// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Building a dictionary: the distinct-key pass with spilling, then three phases over
//! what it spilled. The phases are described in the module documentation ("How a
//! dictionary is built").

use std::num::NonZeroUsize;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use everygraph_core::{ByteSource, MemoryAccountant, par};

use super::dense::{DenseBits, RankBitmap};
use super::dict::{Dictionary, Header, Repr, SectionWriters};
use super::hash::{FINE, FINE_BITS, KeyHash, index, top_bits};
use super::kind::{ColumnType, Key, KeyKind, MAX_FIXED_WIDTH, dense_index};
use super::mphf::{Mphf, Part, PartError, build_part, part_bits_for};
use super::scratch::{
    Reservation, Scatter, Scratch, Spill, WRITE_BUFFER, in_rounds, read_all, widen,
};
use super::table::{Credit, Inserted, Table};
use super::{DictionaryError, IdOrder, Layout, check_node_count};

/// One key in 2^`SAMPLE_BITS` (by hash) is kept in the sample that sizes the phases.
const SAMPLE_BITS: u32 = 10;
/// Memory every worker keeps for its table even after it spills, so that it can always
/// make progress however the budget is shared out.
const WORKER_BASE: u64 = 68 << 10;

/// Choices made when a dictionary is built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct BuildOptions {
    /// How dense ids, and with them graph order, are assigned.
    pub order: IdOrder,
    /// How the arrays behind a look-up are arranged.
    pub layout: Layout,
    /// Seed of the key hash. A build that meets two keys with the same 128-bit hash
    /// (never seen; possible only with crafted input) is retried by
    /// [`build_dictionary`] with the next seed.
    pub hash_seed: u64,
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            order: IdOrder::Natural,
            layout: Layout::Split,
            hash_seed: 0,
        }
    }
}

impl BuildOptions {
    /// The default options with `order`.
    #[must_use]
    pub fn with_order(mut self, order: IdOrder) -> Self {
        self.order = order;
        self
    }

    /// The default options with `layout`.
    #[must_use]
    pub fn with_layout(mut self, layout: Layout) -> Self {
        self.layout = layout;
        self
    }
}

/// An input that can be read in chunks, possibly in parallel and more than once.
///
/// Chunk boundaries are a property of the input (a file's blocks, a table's row
/// groups), not of the machine. Positions name where each key was seen: any `u64` that
/// increases along the input (a byte offset, or `2 * row + column`). "First appearance"
/// is the smallest position; two different keys at the same position are ordered by
/// key.
pub trait KeySource: Sync {
    /// The Arrow type of the identifier column.
    fn column(&self) -> ColumnType;

    /// Number of chunks.
    fn chunks(&self) -> usize;

    /// Calls `f` with the position and the key of every identifier of chunk `chunk`, in
    /// input order.
    ///
    /// # Errors
    ///
    /// The input could not be read, or `f` failed (its error is passed through).
    fn read_chunk(
        &self,
        chunk: usize,
        f: &mut dyn FnMut(u64, Key<'_>) -> Result<(), DictionaryError>,
    ) -> Result<(), DictionaryError>;
}

/// Builds the dictionary of `source`: chooses the key kind from the first
/// [`SAMPLE_SIZE`](super::SAMPLE_SIZE) keys, reads every chunk in parallel through a
/// [`Collector`], and builds. When a key does not fit the kind chosen, the pass is
/// repeated with the wider kind; when two keys collide on their 128-bit hash, with the
/// next hash seed. The result is the same at any thread count and any budget.
///
/// # Errors
///
/// [`DictionaryError::Budget`] when even the smallest working set does not fit the
/// accountant's budget, [`DictionaryError::TooManyNodes`] at 2^32 distinct keys, and
/// read, scratch and input errors.
pub fn build_dictionary<'a, Sc: Scratch>(
    source: &dyn KeySource,
    options: &BuildOptions,
    accountant: &'a dyn MemoryAccountant,
    scratch: &'a Sc,
) -> Result<Dictionary<'a, Sc::Source>, DictionaryError> {
    let column = source.column();
    let mut kind = detect_from(source)?;
    let mut options = *options;
    for _ in 0..8 {
        let collector = Collector::new(column, kind, &options, accountant, scratch)?;
        match collect(source, &collector) {
            Ok(()) => {}
            Err(DictionaryError::KindMismatch { .. }) if kind.widen(column) != kind => {
                kind = kind.widen(column);
                continue;
            }
            Err(e) => return Err(e),
        }
        match collector.finish()?.build() {
            Err(DictionaryError::HashCollision) => {
                options.hash_seed = options.hash_seed.wrapping_add(1);
            }
            other => return other,
        }
    }
    Err(DictionaryError::HashCollision)
}

fn detect_from(source: &dyn KeySource) -> Result<KeyKind, DictionaryError> {
    let mut sample: Vec<Vec<u8>> = Vec::new();
    let mut ints: Vec<Key<'static>> = Vec::new();
    let enough = || DictionaryError::Corrupt("sample complete");
    'chunks: for chunk in 0..source.chunks() {
        let r = source.read_chunk(chunk, &mut |_, key| {
            match key {
                Key::Int(v) => ints.push(Key::Int(v)),
                Key::UInt(v) => ints.push(Key::UInt(v)),
                Key::Bytes(b) => sample.push(b.to_vec()),
            }
            if sample.len() + ints.len() >= super::SAMPLE_SIZE {
                return Err(enough());
            }
            Ok(())
        });
        match r {
            Ok(()) => {}
            Err(DictionaryError::Corrupt("sample complete")) => break 'chunks,
            Err(e) => return Err(e),
        }
    }
    let keys = ints.into_iter().chain(sample.iter().map(|b| Key::Bytes(b)));
    Ok(KeyKind::detect(source.column(), keys))
}

fn collect<Sc: Scratch>(
    source: &dyn KeySource,
    collector: &Collector<'_, Sc>,
) -> Result<(), DictionaryError> {
    let pool: Mutex<Vec<Worker<'_, '_, Sc>>> = Mutex::new(Vec::new());
    let stop = AtomicBool::new(false);
    let one = NonZeroUsize::MIN;
    let results = par::map_ranges(source.chunks(), one, |r| -> Result<(), DictionaryError> {
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        let worker = pool
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop();
        let mut worker = match worker {
            Some(w) => w,
            None => collector.worker()?,
        };
        let result = source.read_chunk(r.start, &mut |pos, key| worker.add(pos, key));
        if result.is_err() {
            stop.store(true, Ordering::Relaxed);
        }
        pool.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(worker);
        result
    });
    let mut first_error = results.into_iter().find_map(Result::err);
    for worker in pool
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
    {
        if let Err(e) = worker.finish() {
            first_error.get_or_insert(e);
        }
    }
    first_error.map_or(Ok(()), Err)
}

/// The distinct-key pass: gathers the keys of one input from any number of
/// [`Worker`]s, spilling to the scratch whenever their tables reach the budget.
pub struct Collector<'a, Sc: Scratch> {
    column: ColumnType,
    kind: KeyKind,
    order: IdOrder,
    layout: Layout,
    seed: u64,
    accountant: &'a dyn MemoryAccountant,
    scratch: &'a Sc,
    runs: Mutex<Vec<Run<Sc::Source>>>,
    sample: Mutex<Sample>,
    /// Memory of the run indexes and of the sample, split off the workers' tables.
    held: Mutex<Vec<Reservation<'a>>>,
    /// The bases of the first workers, reserved up front so that a worker created late
    /// still finds its base however much the earlier ones have grown.
    bases: Mutex<Reservation<'a>>,
    dense: Option<DenseBits<'a>>,
}

impl<Sc: Scratch> std::fmt::Debug for Collector<'_, Sc> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Collector")
            .field("column", &self.column)
            .field("kind", &self.kind)
            .field("order", &self.order)
            .finish_non_exhaustive()
    }
}

/// A spilled run: records grouped by fine partition, followed by `FINE + 1` `u64`
/// offsets where each partition starts (kept in the scratch, not in memory).
struct Run<S> {
    source: S,
    index_at: u64,
}

impl<S: ByteSource> Run<S> {
    /// Where fine partitions `a..b` are in the run.
    fn span(&self, a: usize, b: usize) -> Result<(u64, u64), DictionaryError> {
        let mut first = [0_u8; 8];
        let mut last = [0_u8; 8];
        self.source
            .read_at(self.index_at + widen(a) * 8, &mut first)?;
        self.source
            .read_at(self.index_at + widen(b) * 8, &mut last)?;
        let (first, last) = (u64::from_le_bytes(first), u64::from_le_bytes(last));
        if last < first || last > self.index_at {
            return Err(DictionaryError::Corrupt("run index"));
        }
        Ok((first, last - first))
    }
}

/// Keys whose hash has its low `SAMPLE_BITS` bits zero, with their first position.
#[derive(Default)]
struct Sample {
    items: Vec<(KeyHash, u64, usize, usize)>,
    arena: Vec<u8>,
}

impl<'a, Sc: Scratch> Collector<'a, Sc> {
    /// A collector for keys of `kind` from a column of type `column`.
    ///
    /// # Errors
    ///
    /// [`DictionaryError::Budget`] if the accountant refuses the collector's first
    /// reservation.
    pub fn new(
        column: ColumnType,
        kind: KeyKind,
        options: &BuildOptions,
        accountant: &'a dyn MemoryAccountant,
        scratch: &'a Sc,
    ) -> Result<Self, DictionaryError> {
        let order = options.order.resolve(kind);
        let dense = if kind == (KeyKind::Integers { dense: true }) && order == IdOrder::SortedKey {
            Some(DenseBits::new(accountant)?)
        } else {
            None
        };
        Ok(Self {
            column,
            kind,
            order,
            layout: options.layout,
            seed: options.hash_seed,
            accountant,
            scratch,
            runs: Mutex::new(Vec::new()),
            sample: Mutex::new(Sample::default()),
            held: Mutex::new(Vec::new()),
            bases: Mutex::new(Reservation::new(
                accountant,
                "id dictionary distinct keys",
                (accountant.available() / 4).min(64 * WORKER_BASE) / WORKER_BASE * WORKER_BASE,
            )?),
            dense,
        })
    }

    /// A worker: what one thread uses to add keys.
    ///
    /// # Errors
    ///
    /// [`DictionaryError::Budget`] if the worker's first reservation is refused.
    pub fn worker(&self) -> Result<Worker<'_, 'a, Sc>, DictionaryError> {
        Ok(Worker {
            collector: self,
            table: Table::default(),
            credit: {
                let mut bases = self
                    .bases
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if bases.bytes() >= WORKER_BASE {
                    Credit::from_reservation(bases.split_off(WORKER_BASE))
                } else {
                    Credit::new(self.accountant, "id dictionary distinct keys", WORKER_BASE)?
                }
            },
            buf: [0; MAX_FIXED_WIDTH],
        })
    }

    /// Ends the pass. Every worker must have been finished.
    ///
    /// # Errors
    ///
    /// None today; the signature leaves room for checks of the runs.
    pub fn finish(self) -> Result<Distinct<'a, Sc>, DictionaryError> {
        let mut sample = self
            .sample
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let arena = std::mem::take(&mut sample.arena);
        let key = |&(_, _, off, len): &(KeyHash, u64, usize, usize)| &arena[off..off + len];
        // One entry per key (a key spilled twice was sampled twice), with its smallest
        // position.
        sample
            .items
            .sort_unstable_by(|a, b| (a.0, key(a), a.1).cmp(&(b.0, key(b), b.1)));
        sample.items.dedup_by(|b, a| a.0 == b.0 && key(a) == key(b));
        sample.arena = arena;
        Ok(Distinct {
            column: self.column,
            kind: self.kind,
            order: self.order,
            layout: self.layout,
            seed: self.seed,
            accountant: self.accountant,
            scratch: self.scratch,
            runs: self
                .runs
                .into_inner()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            sample,
            _held: self
                .held
                .into_inner()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            dense: self.dense,
        })
    }
}

/// What one thread uses to add keys to a [`Collector`].
pub struct Worker<'c, 'a, Sc: Scratch> {
    collector: &'c Collector<'a, Sc>,
    table: Table,
    credit: Credit<'a>,
    buf: [u8; MAX_FIXED_WIDTH],
}

impl<Sc: Scratch> std::fmt::Debug for Worker<'_, '_, Sc> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Worker")
            .field("keys", &self.table.len())
            .finish_non_exhaustive()
    }
}

impl<Sc: Scratch> Worker<'_, '_, Sc> {
    /// Adds `key`, seen at `position`.
    ///
    /// # Errors
    ///
    /// [`DictionaryError::KindMismatch`] if the key does not fit the collector's kind
    /// (the pass is then repeated with [`KeyKind::widen`]); budget and scratch errors.
    pub fn add(&mut self, position: u64, key: Key<'_>) -> Result<(), DictionaryError> {
        let c = self.collector;
        let Some(norm) = c.kind.normalize(c.column, key, &mut self.buf) else {
            return Err(DictionaryError::mismatch(position, key));
        };
        if let Some(dense) = &c.dense {
            let Some(bit) = dense_index(c.column, norm) else {
                return Err(DictionaryError::mismatch(position, key));
            };
            return dense.set(bit);
        }
        let hash = KeyHash::of(norm, c.seed);
        if self
            .table
            .insert(hash, position, norm, Some(&mut self.credit))
            == Inserted::Done
        {
            return Ok(());
        }
        // The budget is taken (by this table or by others): spill, keep the base, retry.
        let norm = norm.to_vec();
        let was_empty = self.table.is_empty();
        self.flush()?;
        if !was_empty
            && self
                .table
                .insert(hash, position, &norm, Some(&mut self.credit))
                == Inserted::Done
        {
            return Ok(());
        }
        Err(DictionaryError::Budget(
            everygraph_core::InsufficientBudget::new(
                "id dictionary distinct keys",
                widen(norm.len()) + WORKER_BASE,
                c.accountant.available(),
            ),
        ))
    }

    /// Spills the table as a run and empties it.
    fn flush(&mut self) -> Result<(), DictionaryError> {
        if self.table.is_empty() {
            return Ok(());
        }
        let c = self.collector;
        let fixed = c.kind.width().is_some();
        self.table.group_entries_by(FINE, |e| e.hash.fine());
        let mut spill = Spill::new(c.scratch, "id dictionary run")?;
        let mut starts = vec![0_u64; FINE + 1];
        let mut record = Vec::with_capacity(32 + MAX_FIXED_WIDTH);
        let mut sample = Vec::new();
        let mut sample_arena = Vec::new();
        let mut fine = 0;
        for e in self.table.entries() {
            while fine < e.hash.fine() {
                fine += 1;
                starts[fine] = spill.bytes;
            }
            let key = self.table.key(e);
            record.clear();
            record.extend_from_slice(&e.hash.hi.to_le_bytes());
            record.extend_from_slice(&e.hash.lo.to_le_bytes());
            record.extend_from_slice(&e.pos.to_le_bytes());
            if !fixed {
                let len = u32::try_from(key.len()).expect("a key shorter than 4 GiB");
                record.extend_from_slice(&len.to_le_bytes());
            }
            record.extend_from_slice(key);
            spill.write(&record)?;
            if e.hash.lo.trailing_zeros() >= SAMPLE_BITS {
                sample.push((e.hash, e.pos, sample_arena.len(), key.len()));
                sample_arena.extend_from_slice(key);
            }
        }
        while fine < FINE {
            fine += 1;
            starts[fine] = spill.bytes;
        }
        let index_at = spill.bytes;
        let index: Vec<u8> = starts.iter().flat_map(|s| s.to_le_bytes()).collect();
        spill.write(&index)?;
        let source = spill.seal(c.scratch)?;
        // The sample outlives the table: its memory is moved off the table's
        // reservation, so that no other worker can take it meanwhile.
        let kept = self
            .credit
            .split_off(widen(sample.len() * 48 + sample_arena.len()));
        c.held
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(kept);
        c.runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(Run { source, index_at });
        {
            let mut s = c
                .sample
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let base = s.arena.len();
            s.items.extend(
                sample
                    .into_iter()
                    .map(|(h, p, off, len)| (h, p, off + base, len)),
            );
            s.arena.extend_from_slice(&sample_arena);
        }
        self.table = Table::default();
        self.credit.reset();
        Ok(())
    }

    /// Spills what is left and returns the worker's memory.
    ///
    /// # Errors
    ///
    /// Scratch errors.
    pub fn finish(mut self) -> Result<(), DictionaryError> {
        self.flush()
    }
}

/// The distinct keys of an input, spilled; [`Distinct::build`] turns them into a
/// dictionary.
pub struct Distinct<'a, Sc: Scratch> {
    column: ColumnType,
    kind: KeyKind,
    order: IdOrder,
    layout: Layout,
    seed: u64,
    accountant: &'a dyn MemoryAccountant,
    scratch: &'a Sc,
    runs: Vec<Run<Sc::Source>>,
    sample: Sample,
    _held: Vec<Reservation<'a>>,
    dense: Option<DenseBits<'a>>,
}

impl<Sc: Scratch> std::fmt::Debug for Distinct<'_, Sc> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Distinct")
            .field("kind", &self.kind)
            .field("runs", &self.runs.len())
            .finish_non_exhaustive()
    }
}

/// A record of phase C: a distinct key, where its slot is, and its first position.
struct Located<'r> {
    pos: u64,
    unit: u32,
    local: u32,
    key: &'r [u8],
}

impl<'a, Sc: Scratch> Distinct<'a, Sc> {
    /// Builds the dictionary.
    ///
    /// # Errors
    ///
    /// [`DictionaryError::TooManyNodes`] at 2^32 distinct keys,
    /// [`DictionaryError::HashCollision`] if two keys share their 128-bit hash (retry
    /// with another seed), budget, read and scratch errors.
    pub fn build(self) -> Result<Dictionary<'a, Sc::Source>, DictionaryError> {
        if self.dense.is_some() {
            self.build_dense()
        } else {
            self.build_hashed()
        }
    }

    fn build_hashed(self) -> Result<Dictionary<'a, Sc::Source>, DictionaryError> {
        let width = self.kind.width();
        let sampled = widen(self.sample.items.len());
        let estimate = sampled << SAMPLE_BITS;
        let part_bits = part_bits_for(estimate);
        let unit_bits = part_bits.min(FINE_BITS);
        let units = 1_usize << unit_bits;
        let fine_per_unit = FINE >> unit_bits;
        // Every phase splits its work into tasks of about this many bytes; the split
        // follows the budget and never changes the result.
        let task_bytes = (self.accountant.available() / 8).clamp(1 << 20, 1 << 30);
        let avg_key = widen(self.sample.arena.len())
            .checked_div(sampled)
            .unwrap_or(16);

        // Phase C's buckets: ranges of the order value, cut at quantiles of the sample.
        let record_c = 8 + 4 + 4 + 4 + avg_key;
        let buckets = (estimate * record_c * 3)
            .div_ceil(task_bytes)
            .clamp(1, widen(self.sample.items.len()) + 1);
        let splitters = self.splitters(index(buckets));
        let bucket_count = splitters.len() + 1;

        // Phase B: per unit, dedupe across runs, solve the unit's parts, and send each
        // key to its phase-C bucket.
        let unit_tasks: Vec<usize> = (0..units).collect();
        let mut mphf = Mphf {
            part_bits,
            ..Mphf::default()
        };
        // The hash function is kept in memory by the dictionary: reserved as it grows.
        let mut resident = Reservation::new(self.accountant, "id dictionary hash function", 0)?;
        let mut unit_base: Vec<u32> = Vec::with_capacity(units + 1);
        let mut total = 0_u64;
        let _bucket_buffers = Reservation::new(
            self.accountant,
            "id dictionary write buffers",
            widen(bucket_count * WRITE_BUFFER),
        )?;
        let mut bucket_spills = (0..bucket_count)
            .map(|_| Spill::new(self.scratch, "id dictionary order bucket"))
            .collect::<std::io::Result<Vec<_>>>()?;
        let mut bucket_counts = vec![0_u64; bucket_count];
        let mut unit_sampled = vec![0_u64; units];
        for (h, ..) in &self.sample.items {
            unit_sampled[top_bits(h.hi, unit_bits)] += 1 << SAMPLE_BITS;
        }
        in_rounds(
            self.accountant,
            "id dictionary distinct keys",
            &unit_tasks,
            // A guess (the task reserves what it really needs as it goes): the unit's
            // share of the sampled keys, at 200 bytes each.
            |&u| unit_sampled[u] * 200 + (1 << 16),
            |&u, grant| self.unit_task(u, fine_per_unit, part_bits, unit_bits, &splitters, grant),
            |_, out: UnitOut| {
                unit_base.push(u32::try_from(total).map_err(|_| super::too_many_nodes(total))?);
                total += u64::from(out.n);
                check_node_count(total)?;
                let before = mphf.bytes();
                for p in &out.parts {
                    mphf.push(p);
                }
                resident.grow(mphf.bytes() - before)?;
                for (r, bytes) in out.buckets.iter().enumerate() {
                    bucket_spills[r].write(bytes)?;
                    bucket_counts[r] += out.counts[r];
                }
                Ok(())
            },
        )?;
        drop(self.runs);
        unit_base.push(u32::try_from(total).map_err(|_| super::too_many_nodes(total))?);
        let n = total;

        // Phase C: per bucket, sort by order value and assign ids; write the arrays that
        // are in id order, and send each (slot, …) to phase D's scatter.
        let slot_layout = self.layout == Layout::SlotOrdered;
        let payload_width = match (slot_layout, width) {
            (false, _) => Some(4),
            (true, Some(w)) => Some(w + 4),
            (true, None) => None,
        };
        let mut scatter = Scatter::new(
            self.scratch,
            self.accountant,
            n,
            payload_width,
            8 + avg_key,
            task_bytes / 3,
        )?;
        let mut out = SectionWriters::new(self.scratch, self.layout, width)?;
        let bucket_sources = bucket_spills
            .into_iter()
            .map(|s| s.seal(self.scratch))
            .collect::<std::io::Result<Vec<_>>>()?;
        let mut bucket_base = Vec::with_capacity(bucket_count);
        let mut acc = 0_u64;
        for c in &bucket_counts {
            bucket_base.push(acc);
            acc += c;
        }
        debug_assert_eq!(acc, n);
        let bucket_tasks: Vec<usize> = (0..bucket_count).collect();
        let order = self.order;
        let (range_len, range_count) = (scatter.range_len(), scatter.range_count());
        let mut key_offset = 0_u64;
        in_rounds(
            self.accountant,
            "id dictionary order bucket",
            &bucket_tasks,
            |&r| {
                bucket_sources[r]
                    .len()
                    .saturating_mul(4)
                    .saturating_add(1 << 16)
            },
            |&r, grant| {
                let bytes = read_all(&bucket_sources[r])?;
                let mut items = parse_located(&bytes, order, width)?;
                match order {
                    IdOrder::SortedKey => items.sort_unstable_by(|a, b| a.key.cmp(b.key)),
                    _ => items.sort_unstable_by(|a, b| (a.pos, a.key).cmp(&(b.pos, b.key))),
                }
                let mut keys = Vec::new();
                let mut lens = Vec::new();
                let mut id_to_slot = Vec::new();
                let mut ranges: Vec<Vec<u8>> = vec![Vec::new(); range_count];
                let mut payload = Vec::with_capacity(4 + MAX_FIXED_WIDTH);
                for (i, it) in items.iter().enumerate() {
                    let id = u32::try_from(bucket_base[r] + widen(i)).expect("checked below 2^32");
                    let slot = unit_base[index(u64::from(it.unit))] + it.local;
                    payload.clear();
                    if slot_layout {
                        id_to_slot.extend_from_slice(&slot.to_le_bytes());
                        if width.is_some() {
                            payload.extend_from_slice(it.key);
                            payload.extend_from_slice(&id.to_le_bytes());
                        } else {
                            payload.extend_from_slice(&id.to_le_bytes());
                            payload.extend_from_slice(it.key);
                        }
                    } else {
                        keys.extend_from_slice(it.key);
                        if width.is_none() {
                            lens.push(widen(it.key.len()));
                        }
                        payload.extend_from_slice(&id.to_le_bytes());
                    }
                    let range = index(u64::from(slot) / range_len);
                    Scatter::<'_, Sc>::encode(payload_width, slot, &payload, &mut ranges[range]);
                }
                Ok(((keys, lens, id_to_slot, ranges), grant))
            },
            |_, (keys, lens, id_to_slot, ranges)| {
                if let Some(w) = out.keys.as_mut() {
                    w.write(&keys)?;
                }
                if let Some(offsets) = out.key_offsets.as_mut() {
                    for l in lens {
                        key_offset += l;
                        offsets.write(&key_offset.to_le_bytes())?;
                    }
                }
                if let Some(w) = out.id_to_slot.as_mut() {
                    w.write(&id_to_slot)?;
                }
                for (range, bytes) in ranges.iter().enumerate() {
                    scatter.append(range, bytes)?;
                }
                Ok(())
            },
        )?;
        drop(bucket_sources);

        // Phase D: arrays in slot order.
        let (slot_main, slot_offsets) = out.slot_ordered();
        scatter.finish(self.scratch, self.accountant, slot_main, slot_offsets)?;

        let header = Header {
            kind: self.kind,
            column: self.column,
            order: self.order,
            layout: self.layout,
            repr: Repr::Hashed,
            renumbered: false,
            n,
            hash_seed: self.seed,
            part_bits,
            parts: u32::try_from(mphf.parts.len()).expect("at most 2^16 parts"),
        };
        out.finish(
            self.scratch,
            header,
            Some(mphf),
            None,
            self.accountant,
            resident,
        )
    }

    /// `buckets - 1` order values that cut the sample into `buckets` equal ranges.
    fn splitters(&self, buckets: usize) -> Vec<Splitter> {
        let items = &self.sample.items;
        let key = |i: usize| {
            let (_, _, off, len) = items[i];
            self.sample.arena[off..off + len].to_vec()
        };
        let mut order: Vec<usize> = (0..items.len()).collect();
        match self.order {
            IdOrder::SortedKey => order.sort_unstable_by_key(|&i| key(i)),
            _ => order.sort_unstable_by_key(|&i| (items[i].1, key(i))),
        }
        let mut out: Vec<Splitter> = (1..buckets)
            .map(|b| {
                let i = order[b * order.len() / buckets];
                Splitter {
                    pos: items[i].1,
                    key: key(i),
                }
            })
            .collect();
        out.dedup_by(|b, a| a.pos == b.pos && a.key == b.key);
        out
    }

    fn bucket_of(splitters: &[Splitter], order: IdOrder, pos: u64, key: &[u8]) -> usize {
        match order {
            IdOrder::SortedKey => splitters.partition_point(|s| s.key.as_slice() <= key),
            _ => splitters.partition_point(|s| (s.pos, s.key.as_slice()) <= (pos, key)),
        }
    }

    fn unit_task(
        &self,
        unit: usize,
        fine_per_unit: usize,
        part_bits: u32,
        unit_bits: u32,
        splitters: &[Splitter],
        grant: Reservation<'a>,
    ) -> Result<(UnitOut, Reservation<'a>), DictionaryError> {
        let fixed = self.kind.width();
        let (a, b) = (unit * fine_per_unit, (unit + 1) * fine_per_unit);
        // Memory beyond the round's estimate is taken as the task goes; a refusal
        // defers the task to a smaller round (`in_rounds`).
        let mut credit = Credit::from_reservation(grant);
        let mut table = Table::default();
        let mut bytes = Vec::new();
        for run in &self.runs {
            let (start, len) = run.span(a, b)?;
            let len_usize = index(len);
            if len_usize > bytes.capacity() {
                credit.require(widen(len_usize - bytes.capacity()))?;
            }
            bytes.resize(len_usize, 0);
            run.source.read_at(start, &mut bytes)?;
            let mut at = 0;
            while at < bytes.len() {
                let field = |at: &mut usize, n: usize| -> Result<&[u8], DictionaryError> {
                    let f = bytes
                        .get(*at..*at + n)
                        .ok_or(DictionaryError::Corrupt("truncated run"))?;
                    *at += n;
                    Ok(f)
                };
                let word = |f: &[u8]| u64::from_le_bytes(f.try_into().expect("eight bytes"));
                let hi = word(field(&mut at, 8)?);
                let lo = word(field(&mut at, 8)?);
                let pos = word(field(&mut at, 8)?);
                let len = match fixed {
                    Some(w) => w,
                    None => index(u64::from(u32::from_le_bytes(
                        field(&mut at, 4)?.try_into().expect("four bytes"),
                    ))),
                };
                let key = field(&mut at, len)?;
                if table.insert(KeyHash { hi, lo }, pos, key, Some(&mut credit)) == Inserted::Full {
                    return Err(refused(self.accountant));
                }
            }
        }
        drop(bytes);
        // The hash function's working memory and the outputs, now that the number of
        // keys is known: about 48 bytes per key for the solver, one record per key out.
        let distinct = widen(table.len());
        let record = 8 + 4 + 4 + 4 + widen(fixed.unwrap_or(32));
        credit.require(distinct * (48 + record) + widen(splitters.len() + 1) * 32)?;
        // The unit's parts, in hash order.
        let parts_per_unit = 1_usize << (part_bits - unit_bits);
        let mut by_part: Vec<Vec<KeyHash>> = vec![Vec::new(); parts_per_unit];
        let part_of = |h: KeyHash| top_bits(h.hi, part_bits) - (unit << (part_bits - unit_bits));
        for e in table.entries() {
            by_part[part_of(e.hash)].push(e.hash);
        }
        let mut parts = Vec::with_capacity(parts_per_unit);
        let mut part_base = Vec::with_capacity(parts_per_unit);
        let mut n = 0_u32;
        for hashes in &by_part {
            let part = build_part(hashes).map_err(|e| match e {
                PartError::Collision => DictionaryError::HashCollision,
                PartError::Unsolvable => {
                    DictionaryError::Corrupt("a hash function part could not be solved")
                }
            })?;
            part_base.push(n);
            n += part.n;
            parts.push(part);
        }
        drop(by_part);
        let mut buckets: Vec<Vec<u8>> = vec![Vec::new(); splitters.len() + 1];
        let mut counts = vec![0_u64; splitters.len() + 1];
        let unit_u32 = u32::try_from(unit).expect("at most 2^12 units");
        for e in table.entries() {
            let p = part_of(e.hash);
            let local = part_base[p] + parts[p].eval(e.hash);
            let key = table.key(e);
            let r = Self::bucket_of(splitters, self.order, e.pos, key);
            let out = &mut buckets[r];
            if self.order != IdOrder::SortedKey {
                out.extend_from_slice(&e.pos.to_le_bytes());
            }
            out.extend_from_slice(&unit_u32.to_le_bytes());
            out.extend_from_slice(&local.to_le_bytes());
            if fixed.is_none() {
                out.extend_from_slice(&u32::try_from(key.len()).expect("short key").to_le_bytes());
            }
            out.extend_from_slice(key);
            counts[r] += 1;
        }
        Ok((
            UnitOut {
                parts,
                n,
                buckets,
                counts,
            },
            credit.into_reservation(),
        ))
    }

    fn build_dense(self) -> Result<Dictionary<'a, Sc::Source>, DictionaryError> {
        let bits = self.dense.expect("dense path");
        // The rank bitmap is a copy of the segments, made while they still exist.
        let resident = Reservation::new(
            self.accountant,
            "id dictionary dense bitmap",
            RankBitmap::bytes(bits.word_len()),
        )?;
        let bitmap = RankBitmap::new(bits.into_words())?;
        let header = Header {
            kind: self.kind,
            column: self.column,
            order: IdOrder::SortedKey,
            layout: self.layout,
            repr: Repr::Dense,
            renumbered: false,
            n: u64::from(bitmap.len()),
            hash_seed: self.seed,
            part_bits: 0,
            parts: 0,
        };
        let out = SectionWriters::new_dense();
        out.finish(
            self.scratch,
            header,
            None,
            Some(bitmap),
            self.accountant,
            resident,
        )
    }
}

fn refused(accountant: &dyn MemoryAccountant) -> DictionaryError {
    DictionaryError::Budget(everygraph_core::InsufficientBudget::new(
        "id dictionary distinct keys",
        1 << 16,
        accountant.available(),
    ))
}

struct Splitter {
    pos: u64,
    key: Vec<u8>,
}

struct UnitOut {
    parts: Vec<Part>,
    n: u32,
    buckets: Vec<Vec<u8>>,
    counts: Vec<u64>,
}

fn parse_located(
    bytes: &[u8],
    order: IdOrder,
    width: Option<usize>,
) -> Result<Vec<Located<'_>>, DictionaryError> {
    let mut items = Vec::new();
    let mut at = 0;
    let truncated = || DictionaryError::Corrupt("truncated order bucket");
    while at < bytes.len() {
        let mut take = |n: usize| -> Result<&[u8], DictionaryError> {
            let f = bytes.get(at..at + n).ok_or_else(truncated)?;
            at += n;
            Ok(f)
        };
        let pos = if order == IdOrder::SortedKey {
            0
        } else {
            u64::from_le_bytes(take(8)?.try_into().expect("eight bytes"))
        };
        let unit = u32::from_le_bytes(take(4)?.try_into().expect("four bytes"));
        let local = u32::from_le_bytes(take(4)?.try_into().expect("four bytes"));
        let len = match width {
            Some(w) => w,
            None => index(u64::from(u32::from_le_bytes(
                take(4)?.try_into().expect("four bytes"),
            ))),
        };
        let key = take(len)?;
        items.push(Located {
            pos,
            unit,
            local,
            key,
        });
    }
    Ok(items)
}

/// Reads `source` in pieces of `record` bytes (a few thousand at a time) and calls `f`
/// with each piece's first record index and bytes.
pub(crate) fn for_each_record_chunk(
    source: &dyn ByteSource,
    record: usize,
    mut f: impl FnMut(u64, &[u8]) -> Result<(), DictionaryError>,
) -> Result<(), DictionaryError> {
    let per_read = ((1 << 20) / record.max(1)).max(1);
    let total = source.len() / widen(record.max(1));
    let mut buf = vec![0_u8; per_read * record];
    let mut i = 0_u64;
    while i < total {
        let count = index((total - i).min(widen(per_read)));
        let bytes = &mut buf[..count * record];
        source.read_at(i * widen(record), bytes)?;
        f(i, bytes)?;
        i += widen(count);
    }
    Ok(())
}
