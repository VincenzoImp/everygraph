// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Building a dictionary: the distinct-key pass with spilling, then three phases over
//! what it spilled. The phases are described in the module documentation ("How a
//! dictionary is built").
//!
//! The builder is byte-identical for the same input and options at any thread count
//! and budget; that is a property of this implementation (each step whose grouping
//! depends on threads or budget is followed by an ordering by key, hash or position),
//! not of the format, which a reader needs only to read.

use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};

use everygraph_core::{ByteSource, InsufficientBudget, MemoryAccountant, par};

use super::dense::{DenseBits, RankBitmap};
use super::dict::{Dictionary, Header, Repr, SectionWriters};
use super::hash::{FINE, FINE_BITS, KeyHash, index, top_bits};
use super::kind::{
    ColumnType, Key, KeyKind, MAX_FIXED_WIDTH, MAX_KEY_BYTES, dense_base, dense_index,
};
use super::mphf::{Mphf, Part, PartError, build_part, part_bits_for};
use super::scratch::{
    Reservation, Scatter, Scratch, Spill, WRITE_BUFFER, in_rounds, read_all, widen,
};
use super::table::{Credit, Inserted, Table};
use super::{DictionaryError, IdOrder, Layout, check_node_count};

/// One key in 2^`SAMPLE_BITS` (by hash) is kept in the sample that sizes the phases.
const SAMPLE_BITS: u32 = 10;
/// The smallest table a worker can hold a key of [`MAX_KEY_BYTES`] in: 1,024 entries
/// (40 KiB), 2,048 slots (8 KiB) and a 64 KiB arena.
const TABLE_BASE: u64 = 112 << 10;
/// What a worker needs to spill its table: the write buffers of the run and of its
/// sample, the run index and one record.
const FLUSH_BYTES: u64 = (2 * WRITE_BUFFER + (FINE + 1) * 8 + MAX_KEY_BYTES + 64) as u64;
/// Runs are read through a window of this size (at least one record), so that the
/// memory a reader holds does not depend on how the keys were spread over runs.
const READ_WINDOW: usize = 256 << 10;
/// Memory each worker holds for the whole pass, reserved when the collector is made:
/// with it a worker can always spill and go on, however the rest of the budget is
/// shared out.
const WORKER_BASE: u64 = TABLE_BASE + FLUSH_BYTES;
/// Most workers a collector makes.
const MAX_WORKERS: u64 = 1024;

/// How many workers a collector makes when `available` bytes of the budget are left:
/// one per 32 bases, 1 to 1,024. A function of the budget alone, not of the threads.
pub(crate) fn worker_cap(available: u64) -> u64 {
    (available / (32 * WORKER_BASE)).clamp(1, MAX_WORKERS)
}
/// Invalid keys kept as examples (the ones at the smallest positions).
const MAX_EXAMPLES: usize = 16;
/// Hash seeds tried before [`DictionaryError::NoHashFunction`].
const MAX_SEEDS: u64 = 8;
/// A dense integer dictionary is a bitmap of `max + 1` bits; it is used only when that
/// is at most this many bits per key — 8 bytes, below the 12.3 bytes per key of the
/// hashed form of 8-byte keys (key, slot to id, hash function).
pub(crate) const DENSE_BITS_PER_KEY: u64 = 64;

/// Choices made when a dictionary is built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct BuildOptions {
    /// How dense ids, and with them graph order, are assigned.
    pub order: IdOrder,
    /// How the arrays behind a look-up are arranged.
    pub layout: Layout,
    /// The kind of the keys, instead of detecting it (`ids="string"` is
    /// `Some(KeyKind::Bytes)`).
    pub kind: Option<KeyKind>,
    /// Whether integer keys in sorted order may be collected in a bitmap (then kept as
    /// a rank bitmap when they are dense enough). [`build_dictionary`] turns it off
    /// and starts again when a key is outside `0..2^32`.
    pub dense: bool,
    /// Seed of the key hash. A build that meets two keys with the same 128-bit hash, or
    /// a part no pilot can solve (never seen; possible only with crafted input), is
    /// retried by [`build_dictionary`] with the next seed.
    pub hash_seed: u64,
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            order: IdOrder::Natural,
            layout: Layout::Split,
            kind: None,
            dense: true,
            hash_seed: 0,
        }
    }
}

impl BuildOptions {
    /// These options with `order`.
    #[must_use]
    pub fn with_order(mut self, order: IdOrder) -> Self {
        self.order = order;
        self
    }

    /// These options with `layout`.
    #[must_use]
    pub fn with_layout(mut self, layout: Layout) -> Self {
        self.layout = layout;
        self
    }

    /// These options with the kind given instead of detected.
    #[must_use]
    pub fn with_kind(mut self, kind: KeyKind) -> Self {
        self.kind = Some(kind);
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
    /// It must not use [`everygraph_core::par`] itself: the chunk is already one task
    /// of a parallel loop that holds a [`Worker`] while it runs, and a nested parallel
    /// call would let this thread start another chunk meanwhile. (Should it happen,
    /// [`Collector::worker`] does not wait for a worker the same thread holds: it asks
    /// the budget for one more, or fails with [`DictionaryError::Budget`].)
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

/// Keys that did not fit the column's kind: invalid rows (`design/DECISIONS.md`,
/// "Identifier columns"). They are left out of the dictionary; a look-up of one finds
/// nothing. The reader reports them with their lines.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InvalidKeys {
    /// How many were met.
    pub count: u64,
    /// The first ones by position (at most 16), with the key shortened to 64 bytes.
    pub first: Vec<InvalidKey>,
}

/// One key that did not fit the column's kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidKey {
    /// Where it was seen.
    pub position: u64,
    /// The key (its first 64 bytes, as text).
    pub key: String,
}

impl InvalidKeys {
    fn add(&mut self, position: u64, key: Key<'_>) {
        self.count += 1;
        let full = self.first.len() >= MAX_EXAMPLES;
        if full && self.first.last().is_some_and(|l| l.position <= position) {
            return;
        }
        let key = match key {
            Key::Int(v) => v.to_string(),
            Key::UInt(v) => v.to_string(),
            Key::Bytes(b) => String::from_utf8_lossy(&b[..b.len().min(64)]).into_owned(),
        };
        let at = self.first.partition_point(|k| k.position <= position);
        self.first.insert(at, InvalidKey { position, key });
        self.first.truncate(MAX_EXAMPLES);
    }

    fn merge(&mut self, other: &Self) {
        self.count += other.count;
        self.first.extend(other.first.iter().cloned());
        self.first
            .sort_by(|a, b| (a.position, &a.key).cmp(&(b.position, &b.key)));
        self.first.truncate(MAX_EXAMPLES);
    }
}

/// What [`Worker::add`] did with a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Added {
    /// The key is in the dictionary.
    Kept,
    /// The key does not fit the column's kind: an invalid row, left out and counted.
    Invalid,
}

/// A dictionary and what its build left out.
#[derive(Debug)]
#[non_exhaustive]
pub struct Built<'a, S> {
    /// The dictionary.
    pub dictionary: Dictionary<'a, S>,
    /// The keys that were invalid rows.
    pub invalid: InvalidKeys,
}

/// Builds the dictionary of `source`: chooses the key kind from the first
/// [`SAMPLE_SIZE`](super::SAMPLE_SIZE) keys (unless the options give it), reads every
/// chunk in parallel through a [`Collector`], and builds. A key that does not fit the
/// kind is an invalid row, reported in [`Built::invalid`]. When two keys collide on
/// their 128-bit hash or a part cannot be solved, the build is repeated with the next
/// hash seed. The ids and the sections are the same at any thread count and any budget:
/// the bitmap of dense integer keys is chosen from the keys alone, and a budget that
/// cannot hold it is refused, never answered with the hashed form (DESIGN D14).
///
/// # Errors
///
/// [`DictionaryError::Budget`] when even the smallest working set does not fit the
/// accountant's budget, [`DictionaryError::TooManyNodes`] at 2^32 distinct keys,
/// [`DictionaryError::NoHashFunction`] when every seed failed, and read, scratch and
/// input errors.
pub fn build_dictionary<'a, Sc: Scratch>(
    source: &dyn KeySource,
    options: &BuildOptions,
    accountant: &'a dyn MemoryAccountant,
    scratch: &'a Sc,
) -> Result<Built<'a, Sc::Source>, DictionaryError> {
    let column = source.column();
    let (kind, sample_dense) = match options.kind {
        Some(kind) if kind.fits_column(column) => (kind, true),
        Some(_) => {
            return Err(DictionaryError::Unsupported(
                "the kind given does not fit the column",
            ));
        }
        None => detect_from(source)?,
    };
    let mut options = *options;
    options.kind = Some(kind);
    options.dense = options.dense && sample_dense;
    for _ in 0..=MAX_SEEDS {
        let collector = Collector::new(column, kind, &options, accountant, scratch)?;
        match collect(source, &collector) {
            Ok(()) => {}
            // A key outside 0..2^32: the hashed form, which gives the same ids.
            Err(DictionaryError::NotDense) if options.dense => {
                options.dense = false;
                continue;
            }
            Err(e) => return Err(e),
        }
        let distinct = collector.finish()?;
        let invalid = distinct.invalid().clone();
        match distinct.build() {
            Ok(dictionary) => {
                return Ok(Built {
                    dictionary,
                    invalid,
                });
            }
            Err(DictionaryError::HashCollision | DictionaryError::Unsolvable) => {
                options.hash_seed = options.hash_seed.wrapping_add(1);
            }
            Err(e) => return Err(e),
        }
    }
    Err(DictionaryError::NoHashFunction { seeds: MAX_SEEDS })
}

/// The kind of the column, from its first keys, and whether those keys allow the
/// bitmap (integers in `0..2^32`).
fn detect_from(source: &dyn KeySource) -> Result<(KeyKind, bool), DictionaryError> {
    let column = source.column();
    let mut texts: Vec<Vec<u8>> = Vec::new();
    let mut ints: Vec<Key<'static>> = Vec::new();
    let done = AtomicBool::new(false);
    for chunk in 0..source.chunks() {
        let r = source.read_chunk(chunk, &mut |_, key| {
            if texts.len() + ints.len() >= super::SAMPLE_SIZE {
                done.store(true, Ordering::Relaxed);
                return Err(DictionaryError::NotDense);
            }
            match key {
                Key::Int(v) => ints.push(Key::Int(v)),
                Key::UInt(v) => ints.push(Key::UInt(v)),
                Key::Bytes(b) => texts.push(b[..b.len().min(MAX_KEY_BYTES + 1)].to_vec()),
            }
            Ok(())
        });
        match r {
            Ok(()) => {}
            Err(DictionaryError::NotDense) if done.load(Ordering::Relaxed) => break,
            Err(e) => return Err(e),
        }
    }
    let sample = || {
        ints.iter()
            .copied()
            .chain(texts.iter().map(|b| Key::Bytes(b)))
    };
    let kind = KeyKind::detect(column, sample());
    let dense = kind == KeyKind::Integers
        && sample().all(|k| {
            let mut buf = [0_u8; MAX_FIXED_WIDTH];
            kind.normalize(column, k, &mut buf)
                .is_none_or(|n| dense_index(column, n).is_some())
        });
    Ok((kind, dense))
}

fn collect<Sc: Scratch>(
    source: &dyn KeySource,
    collector: &Collector<'_, Sc>,
) -> Result<(), DictionaryError> {
    let stop = AtomicBool::new(false);
    let one = NonZeroUsize::MIN;
    let results = par::map_ranges(source.chunks(), one, |r| -> Result<(), DictionaryError> {
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        let mut worker = collector.worker()?;
        let result = source.read_chunk(r.start, &mut |pos, key| worker.add(pos, key).map(|_| ()));
        if result.is_err() {
            stop.store(true, Ordering::Relaxed);
        }
        result
    });
    results
        .into_iter()
        .find_map(Result::err)
        .map_or(Ok(()), Err)
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What a worker holds between its uses: its table and its memory.
struct WorkerState<'a> {
    table: Table,
    /// The table's memory, never below [`TABLE_BASE`].
    credit: Credit<'a>,
    /// What a spill needs ([`FLUSH_BYTES`]), held throughout.
    _flush: Reservation<'a>,
    invalid: InvalidKeys,
}

struct Pool<'a> {
    idle: Vec<WorkerState<'a>>,
    created: u64,
    /// The bases of the workers not yet made.
    bases: Reservation<'a>,
}

/// The distinct-key pass: gathers the keys of one input from any number of threads,
/// each through a [`Worker`], spilling to the scratch whenever their tables reach the
/// budget.
///
/// At most `available budget / (32 × 312 KiB)` workers exist — one per 9.75 MiB, 1 to
/// 1,024 — their bases reserved when the collector is made, so whether a build fits a budget does not depend on the number
/// of threads: a thread that finds every worker in use waits for one (a native
/// thread pool only; without threads there is one worker and no wait).
pub struct Collector<'a, Sc: Scratch> {
    column: ColumnType,
    kind: KeyKind,
    order: IdOrder,
    layout: Layout,
    seed: u64,
    accountant: &'a dyn MemoryAccountant,
    scratch: &'a Sc,
    runs: Mutex<Vec<Run<Sc::Source>>>,
    /// The sampled records of each run, in the scratch: how many there are depends on
    /// how duplicate keys spread over runs, so they are not held in memory.
    sample_runs: Mutex<Vec<Sc::Source>>,
    pool: Mutex<Pool<'a>>,
    returned: Condvar,
    cap: u64,
    dense: Option<DenseBits<'a>>,
}

impl<Sc: Scratch> std::fmt::Debug for Collector<'_, Sc> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Collector")
            .field("column", &self.column)
            .field("kind", &self.kind)
            .field("order", &self.order)
            .field("workers", &self.cap)
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

/// Bytes a sample of `items` entries and `arena` key bytes holds.
fn sample_bytes(items: usize, arena: usize) -> u64 {
    widen(items * std::mem::size_of::<(KeyHash, u64, usize, usize)>() + arena)
}

/// Calls `f` with the hash, position and key of every record of `source` in
/// `start..start + len`, read through `window` (records of a run or of its sample).
fn for_each_record(
    source: &dyn ByteSource,
    start: u64,
    len: u64,
    fixed: Option<usize>,
    window: &mut [u8],
    mut f: impl FnMut(KeyHash, u64, &[u8]) -> Result<(), DictionaryError>,
) -> Result<(), DictionaryError> {
    let truncated = || DictionaryError::Corrupt("truncated run");
    let end = start + len;
    let mut next = start;
    let mut have = 0_usize;
    let head = 24 + if fixed.is_none() { 4 } else { 0 };
    let word = |b: &[u8]| u64::from_le_bytes(b.try_into().expect("eight bytes"));
    loop {
        let take = index((end - next).min(widen(window.len() - have)));
        source.read_at(next, &mut window[have..have + take])?;
        next += widen(take);
        have += take;
        let mut at = 0;
        while have - at >= head {
            let r = &window[at..have];
            let key_len = match fixed {
                Some(w) => w,
                None => index(u64::from(u32::from_le_bytes(
                    r[24..28].try_into().expect("four bytes"),
                ))),
            };
            if key_len > MAX_KEY_BYTES {
                return Err(truncated());
            }
            if r.len() < head + key_len {
                break;
            }
            let hash = KeyHash {
                hi: word(&r[0..8]),
                lo: word(&r[8..16]),
            };
            f(hash, word(&r[16..24]), &r[head..head + key_len])?;
            at += head + key_len;
        }
        window.copy_within(at..have, 0);
        have -= at;
        if next == end {
            return if have == 0 { Ok(()) } else { Err(truncated()) };
        }
    }
}

impl<'a, Sc: Scratch> Collector<'a, Sc> {
    /// A collector for keys of `kind` from a column of type `column`. With
    /// `options.dense`, integer keys in sorted order are collected in a bitmap; a key
    /// outside `0..2^32` then makes [`Worker::add`] fail with
    /// [`DictionaryError::NotDense`], and the pass must start again without it.
    ///
    /// # Errors
    ///
    /// [`DictionaryError::Budget`] if the workers' bases do not fit the budget.
    pub fn new(
        column: ColumnType,
        kind: KeyKind,
        options: &BuildOptions,
        accountant: &'a dyn MemoryAccountant,
        scratch: &'a Sc,
    ) -> Result<Self, DictionaryError> {
        let order = options.order.resolve(kind);
        let dense = if options.dense && kind == KeyKind::Integers && order == IdOrder::SortedKey {
            Some(DenseBits::new(accountant)?)
        } else {
            None
        };
        let cap = worker_cap(accountant.available());
        let bases = Reservation::new(accountant, "id dictionary distinct keys", cap * WORKER_BASE)?;
        Ok(Self {
            column,
            kind,
            order,
            layout: options.layout,
            seed: options.hash_seed,
            accountant,
            scratch,
            runs: Mutex::new(Vec::new()),
            sample_runs: Mutex::new(Vec::new()),
            pool: Mutex::new(Pool {
                idle: Vec::new(),
                created: 0,
                bases,
            }),
            returned: Condvar::new(),
            cap,
            dense,
        })
    }

    /// A worker for the calling thread: an idle one, a new one while fewer than the
    /// collector's limit exist, or else the next one returned (this waits). A thread
    /// that already holds one of this collector's workers never waits — its own worker
    /// could only come back through it — but gets one more, with a base asked of the
    /// budget.
    ///
    /// # Errors
    ///
    /// [`DictionaryError::Budget`] only in that nested case, when the budget cannot
    /// give the extra base; otherwise every worker's memory was reserved by
    /// [`Collector::new`].
    pub fn worker(&self) -> Result<Worker<'_, 'a, Sc>, DictionaryError> {
        let me = std::ptr::from_ref(self).addr();
        let nested = HOLDING.with(|h| h.borrow().contains(&me));
        let mut pool = lock(&self.pool);
        let base = loop {
            if let Some(state) = pool.idle.pop() {
                drop(pool);
                return Ok(self.lend(state));
            }
            if pool.created < self.cap {
                pool.created += 1;
                break pool.bases.split_off(WORKER_BASE);
            }
            if nested {
                drop(pool);
                break Reservation::new(
                    self.accountant,
                    "id dictionary distinct keys",
                    WORKER_BASE,
                )?;
            }
            pool = self
                .returned
                .wait(pool)
                .unwrap_or_else(PoisonError::into_inner);
        };
        let mut base = base;
        let flush = base.split_off(FLUSH_BYTES);
        Ok(self.lend(WorkerState {
            table: Table::default(),
            credit: Credit::from_reservation(base),
            _flush: flush,
            invalid: InvalidKeys::default(),
        }))
    }

    fn lend(&self, state: WorkerState<'a>) -> Worker<'_, 'a, Sc> {
        HOLDING.with(|h| h.borrow_mut().push(std::ptr::from_ref(self).addr()));
        Worker {
            collector: self,
            state: Some(state),
        }
    }

    /// Spills `st`'s table as a run and empties it.
    fn flush(&self, st: &mut WorkerState<'a>) -> Result<(), DictionaryError> {
        if st.table.is_empty() {
            return Ok(());
        }
        let fixed = self.kind.width().is_some();
        st.table.group_entries_by(FINE, |e| e.hash.fine());
        let mut spill = Spill::new(self.scratch, "id dictionary run")?;
        let mut starts = vec![0_u64; FINE + 1];
        let mut record = Vec::with_capacity(32 + MAX_FIXED_WIDTH);
        let mut sample = Spill::new(self.scratch, "id dictionary sample")?;
        let mut fine = 0;
        for e in st.table.entries() {
            while fine < e.hash.fine() {
                fine += 1;
                starts[fine] = spill.bytes;
            }
            let key = st.table.key(e);
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
                sample.write(&record)?;
            }
        }
        while fine < FINE {
            fine += 1;
            starts[fine] = spill.bytes;
        }
        let index_at = spill.bytes;
        for s in &starts {
            spill.write(&s.to_le_bytes())?;
        }
        let source = spill.seal(self.scratch)?;
        lock(&self.runs).push(Run { source, index_at });
        let sample = sample.seal(self.scratch)?;
        lock(&self.sample_runs).push(sample);
        st.table = Table::default();
        st.credit.reset();
        Ok(())
    }

    /// Ends the pass: spills what the workers hold. Every [`Worker`] must have been
    /// dropped.
    ///
    /// # Errors
    ///
    /// Scratch errors.
    pub fn finish(self) -> Result<Distinct<'a, Sc>, DictionaryError> {
        let mut invalid = InvalidKeys::default();
        let states = std::mem::take(&mut lock(&self.pool).idle);
        for mut st in states {
            self.flush(&mut st)?;
            invalid.merge(&st.invalid);
        }
        // One entry per sampled key (a key spilled twice was sampled twice), with its
        // smallest position; then copied into arrays of exactly its size, so that the
        // memory held from here on depends on the input only.
        let fixed = self.kind.width();
        let mut credit = Credit::new(self.accountant, "id dictionary sample", 0)?;
        credit.require(widen(READ_WINDOW))?;
        let mut window = vec![0_u8; READ_WINDOW];
        let mut table = Table::default();
        for run in std::mem::take(&mut *lock(&self.sample_runs)) {
            for_each_record(&run, 0, run.len(), fixed, &mut window, |hash, pos, key| {
                if table.insert(hash, pos, key, Some(&mut credit)) == Inserted::Full {
                    return Err(refused(self.accountant));
                }
                Ok(())
            })?;
        }
        drop(window);
        let key_bytes: usize = table.entries().iter().map(|e| table.key(e).len()).sum();
        let sample_memory = Reservation::new(
            self.accountant,
            "id dictionary sample",
            sample_bytes(table.len(), key_bytes),
        )?;
        let mut sample = Sample {
            items: Vec::with_capacity(table.len()),
            arena: Vec::with_capacity(key_bytes),
        };
        for e in table.entries() {
            let key = table.key(e);
            sample
                .items
                .push((e.hash, e.pos, sample.arena.len(), key.len()));
            sample.arena.extend_from_slice(key);
        }
        drop(table);
        drop(credit);
        Ok(Distinct {
            column: self.column,
            kind: self.kind,
            order: self.order,
            layout: self.layout,
            seed: self.seed,
            accountant: self.accountant,
            scratch: self.scratch,
            runs: std::mem::take(&mut *lock(&self.runs)),
            sample,
            _sample_memory: sample_memory,
            dense: self.dense,
            invalid,
        })
    }
}

/// What one thread uses to add keys to a [`Collector`]. Dropping it hands its table
/// back to the collector for the next thread.
pub struct Worker<'c, 'a, Sc: Scratch> {
    collector: &'c Collector<'a, Sc>,
    state: Option<WorkerState<'a>>,
}

impl<Sc: Scratch> std::fmt::Debug for Worker<'_, '_, Sc> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Worker").finish_non_exhaustive()
    }
}

thread_local! {
    /// The collectors (by address) whose workers this thread holds.
    static HOLDING: std::cell::RefCell<Vec<usize>> = const { std::cell::RefCell::new(Vec::new()) };
}

impl<Sc: Scratch> Drop for Worker<'_, '_, Sc> {
    fn drop(&mut self) {
        let me = std::ptr::from_ref(self.collector).addr();
        HOLDING.with(|h| {
            let mut h = h.borrow_mut();
            if let Some(at) = h.iter().position(|&c| c == me) {
                h.swap_remove(at);
            }
        });
        if let Some(state) = self.state.take() {
            lock(&self.collector.pool).idle.push(state);
            self.collector.returned.notify_one();
        }
    }
}

impl<'a, Sc: Scratch> Worker<'_, 'a, Sc> {
    /// Adds `key`, seen at `position`. A key that does not fit the collector's kind
    /// (or is longer than [`MAX_KEY_BYTES`]) is an invalid row: counted, not added.
    ///
    /// # Errors
    ///
    /// [`DictionaryError::NotDense`] when the collector gathers a bitmap and the key is
    /// outside `0..2^32`; budget and scratch errors.
    ///
    /// # Panics
    ///
    /// Never: a worker holds its state from [`Collector::worker`] until it is dropped.
    pub fn add(&mut self, position: u64, key: Key<'_>) -> Result<Added, DictionaryError> {
        let c = self.collector;
        let st = self
            .state
            .as_mut()
            .expect("a worker holds its state until dropped");
        let mut copy = [0_u8; MAX_FIXED_WIDTH];
        let Some(len) = c.kind.normalize(c.column, key, &mut copy).map(<[u8]>::len) else {
            st.invalid.add(position, key);
            return Ok(Added::Invalid);
        };
        // Bytes keys are their own normalized form; the others are in `copy`.
        let norm: &[u8] = match (c.kind, key) {
            (KeyKind::Bytes, Key::Bytes(b)) => b,
            _ => &copy[..len],
        };
        if let Some(dense) = &c.dense {
            let Some(bit) = dense_index(c.column, norm) else {
                return Err(DictionaryError::NotDense);
            };
            dense.set(bit)?;
            return Ok(Added::Kept);
        }
        Self::insert(c, st, position, norm)?;
        Ok(Added::Kept)
    }

    /// Adds an already normalized key.
    pub(crate) fn add_normalized(
        &mut self,
        position: u64,
        norm: &[u8],
    ) -> Result<(), DictionaryError> {
        let c = self.collector;
        let st = self
            .state
            .as_mut()
            .expect("a worker holds its state until dropped");
        Self::insert(c, st, position, norm)
    }

    fn insert(
        c: &Collector<'a, Sc>,
        st: &mut WorkerState<'a>,
        position: u64,
        norm: &[u8],
    ) -> Result<(), DictionaryError> {
        let hash = KeyHash::of(norm, c.seed);
        if st.table.insert(hash, position, norm, Some(&mut st.credit)) == Inserted::Done {
            return Ok(());
        }
        // The budget is taken (by this table or by others): spill, keep the base, retry.
        // The base holds a table for any key up to MAX_KEY_BYTES, so this succeeds.
        c.flush(st)?;
        if st.table.insert(hash, position, norm, Some(&mut st.credit)) == Inserted::Done {
            return Ok(());
        }
        Err(DictionaryError::Budget(InsufficientBudget::new(
            "id dictionary distinct keys",
            widen(norm.len()) + TABLE_BASE,
            c.accountant.available(),
        )))
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
    _sample_memory: Reservation<'a>,
    dense: Option<DenseBits<'a>>,
    invalid: InvalidKeys,
}

impl<Sc: Scratch> std::fmt::Debug for Distinct<'_, Sc> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Distinct")
            .field("kind", &self.kind)
            .field("runs", &self.runs.len())
            .field("invalid", &self.invalid.count)
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

/// The dense rule: a bitmap of `max + 1` bits for `n` keys is kept only when it takes
/// at most [`DENSE_BITS_PER_KEY`] bits per key.
pub(crate) fn dense_enough(n: u64, max: Option<u32>) -> bool {
    max.is_none_or(|m| u64::from(m) < DENSE_BITS_PER_KEY.saturating_mul(n))
}

impl<'a, Sc: Scratch> Distinct<'a, Sc> {
    /// The keys that were invalid rows.
    #[must_use]
    pub fn invalid(&self) -> &InvalidKeys {
        &self.invalid
    }

    /// Builds the dictionary.
    ///
    /// # Errors
    ///
    /// [`DictionaryError::TooManyNodes`] at 2^32 distinct keys;
    /// [`DictionaryError::HashCollision`] if two keys share their 128-bit hash and
    /// [`DictionaryError::Unsolvable`] if a part cannot be solved (retry with another
    /// seed in both cases); budget, read and scratch errors.
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
        // The hash function stays in memory with the dictionary: each task reserves
        // its parts and hands the reservation over with them.
        let mut resident: Vec<Reservation<'a>> = Vec::with_capacity(units + 1);
        // The start of each part's slots (the parts themselves are reserved by the
        // tasks that solve them).
        resident.push(Reservation::new(
            self.accountant,
            "id dictionary hash function",
            (1_u64 << part_bits) * 4,
        )?);
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
            // A guess, the task reserves what it really needs as it goes: the unit's
            // share of the sampled keys, at the memory per key the task needs.
            |&u| unit_sampled[u] * (100 + 2 * avg_key) + (1 << 16),
            |&u, grant| self.unit_task(u, fine_per_unit, part_bits, unit_bits, &splitters, grant),
            |_, out: UnitOut<'a>| {
                unit_base.push(u32::try_from(total).map_err(|_| super::too_many_nodes(total))?);
                total += u64::from(out.n);
                check_node_count(total)?;
                for p in out.parts {
                    mphf.push(p);
                }
                resident.push(out.resident);
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
        let mut out = SectionWriters::new(self.scratch, self.accountant, self.layout, width)?;
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
        let header_bytes = |order: IdOrder| -> u64 {
            (if order == IdOrder::SortedKey { 0 } else { 8 })
                + 4
                + 4
                + if width.is_none() { 4 } else { 0 }
        };
        // Exactly what a phase-C task holds: the bucket read, the records, and the
        // outputs (all allocated at their final size).
        let phase_c_bytes = |r: usize| -> u64 {
            let bytes = bucket_sources[r].len();
            let count = bucket_counts[r];
            let keys = bytes.saturating_sub(count * header_bytes(order));
            let located = count * widen(std::mem::size_of::<Located<'_>>());
            let outputs = if slot_layout {
                // id_to_slot, and records of slot + (length) + payload.
                count * 4 + count * (4 + 4 + if width.is_none() { 4 } else { 0 }) + keys
            } else {
                // keys, lengths, and records of slot + id.
                keys + if width.is_none() { count * 8 } else { 0 } + count * 8
            };
            // Plus the payload buffer (one key) and small vectors.
            bytes
                + located
                + outputs
                + widen(range_count) * (24 + 8)
                + widen(MAX_KEY_BYTES)
                + (1 << 12)
        };
        let mut key_offset = 0_u64;
        in_rounds(
            self.accountant,
            "id dictionary order bucket",
            &bucket_tasks,
            |&r| phase_c_bytes(r),
            |&r, grant| {
                let count = index(bucket_counts[r]);
                let bytes = read_all(&bucket_sources[r])?;
                let mut items = parse_located(&bytes, order, width, count)?;
                match order {
                    IdOrder::SortedKey => items.sort_unstable_by(|a, b| a.key.cmp(b.key)),
                    _ => items.sort_unstable_by(|a, b| (a.pos, a.key).cmp(&(b.pos, b.key))),
                }
                let slot_of = |it: &Located<'_>| unit_base[index(u64::from(it.unit))] + it.local;
                // Exact sizes of every output.
                let key_bytes: usize = items.iter().map(|it| it.key.len()).sum();
                let record_len = |it: &Located<'_>| {
                    4 + usize::from(payload_width.is_none()) * 4
                        + match (slot_layout, width) {
                            (false, _) => 4,
                            (true, Some(w)) => w + 4,
                            (true, None) => 4 + it.key.len(),
                        }
                };
                let mut range_sizes = vec![0_usize; range_count];
                for it in &items {
                    range_sizes[index(u64::from(slot_of(it)) / range_len)] += record_len(it);
                }
                let mut ranges: Vec<Vec<u8>> =
                    range_sizes.iter().map(|&s| Vec::with_capacity(s)).collect();
                let mut keys = Vec::with_capacity(if slot_layout { 0 } else { key_bytes });
                let mut lens = Vec::with_capacity(if slot_layout || width.is_some() {
                    0
                } else {
                    count
                });
                let mut id_to_slot = Vec::with_capacity(if slot_layout { 4 * count } else { 0 });
                let mut payload = Vec::with_capacity(4 + MAX_KEY_BYTES);
                for (i, it) in items.iter().enumerate() {
                    let id = u32::try_from(bucket_base[r] + widen(i)).expect("checked below 2^32");
                    let slot = slot_of(it);
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
    ) -> Result<(UnitOut<'a>, Reservation<'a>), DictionaryError> {
        let fixed = self.kind.width();
        let (a, b) = (unit * fine_per_unit, (unit + 1) * fine_per_unit);
        // Memory beyond the round's estimate is taken as the task goes; a refusal
        // defers the task to a smaller round (`in_rounds`).
        let mut credit = Credit::from_reservation(grant);
        let mut table = Table::default();
        credit.require(widen(READ_WINDOW))?;
        let mut window = vec![0_u8; READ_WINDOW];
        for run in &self.runs {
            let (start, len) = run.span(a, b)?;
            for_each_record(
                &run.source,
                start,
                len,
                fixed,
                &mut window,
                |hash, pos, key| {
                    if table.insert(hash, pos, key, Some(&mut credit)) == Inserted::Full {
                        return Err(refused(self.accountant));
                    }
                    Ok(())
                },
            )?;
        }
        drop(window);
        credit.give(widen(READ_WINDOW));
        // Everything else the task holds, now that its keys are known: the hashes by
        // part, the solver's working memory for the largest part, the solved parts,
        // and the records for phase C — each allocated at its exact size.
        let parts_per_unit = 1_usize << (part_bits - unit_bits);
        let part_of = |h: KeyHash| top_bits(h.hi, part_bits) - (unit << (part_bits - unit_bits));
        let mut part_sizes = vec![0_u32; parts_per_unit];
        let mut bucket_sizes = vec![0_usize; splitters.len() + 1];
        let header = (if self.order == IdOrder::SortedKey {
            0
        } else {
            8
        }) + 4
            + 4
            + usize::from(fixed.is_none()) * 4;
        for e in table.entries() {
            part_sizes[part_of(e.hash)] += 1;
            let key = table.key(e);
            bucket_sizes[Self::bucket_of(splitters, self.order, e.pos, key)] += header + key.len();
        }
        let distinct = widen(table.len());
        let largest = part_sizes.iter().copied().max().unwrap_or(0);
        let parts_bytes: u64 = part_sizes.iter().map(|&p| Part::bytes_for(p)).sum();
        let hashes_bytes = distinct * 16 + widen(parts_per_unit) * 24;
        let outputs: u64 = bucket_sizes.iter().map(|&s| widen(s) + 24).sum::<u64>()
            + widen(bucket_sizes.len()) * 8
            + widen(parts_per_unit) * 12;
        credit.require(hashes_bytes + Part::solver_bytes(largest) + parts_bytes + outputs)?;
        let mut by_part: Vec<Vec<KeyHash>> = part_sizes
            .iter()
            .map(|&s| Vec::with_capacity(index(u64::from(s))))
            .collect();
        for e in table.entries() {
            by_part[part_of(e.hash)].push(e.hash);
        }
        let mut parts = Vec::with_capacity(parts_per_unit);
        let mut part_base = Vec::with_capacity(parts_per_unit);
        let mut n = 0_u32;
        for hashes in &by_part {
            let part = build_part(hashes).map_err(|e| match e {
                PartError::Collision => DictionaryError::HashCollision,
                PartError::Unsolvable => DictionaryError::Unsolvable,
            })?;
            part_base.push(n);
            n += part.n;
            parts.push(part);
        }
        drop(by_part);
        let mut buckets: Vec<Vec<u8>> = bucket_sizes
            .iter()
            .map(|&s| Vec::with_capacity(s))
            .collect();
        let mut counts = vec![0_u64; splitters.len() + 1];
        let unit_u32 = u32::try_from(unit).expect("at most 2^10 units");
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
        // The parts stay in memory with the dictionary: their reservation goes with
        // them.
        let resident = credit.split_off(parts_bytes);
        Ok((
            UnitOut {
                parts,
                n,
                buckets,
                counts,
                resident,
            },
            credit.into_reservation(),
        ))
    }

    fn build_dense(mut self) -> Result<Dictionary<'a, Sc::Source>, DictionaryError> {
        let bits = self.dense.take().expect("dense path");
        // The rule first, from the segments in place: a sparse set is never copied
        // into a bitmap spanning its largest key.
        let (n, max) = bits.count_and_max();
        if !dense_enough(n, max) {
            return Self::rehash(
                self.column,
                self.kind,
                self.layout,
                self.seed,
                self.accountant,
                self.scratch,
                self.invalid,
                &bits,
            );
        }
        // The rank bitmap is a copy of the segments, made while they still exist.
        let copy = Reservation::new(
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
            vec![copy],
        )
    }

    /// Builds the hashed form of the keys of a bitmap.
    #[expect(
        clippy::too_many_arguments,
        reason = "the fields of a consumed Distinct"
    )]
    fn rehash(
        column: ColumnType,
        kind: KeyKind,
        layout: Layout,
        seed: u64,
        accountant: &'a dyn MemoryAccountant,
        scratch: &'a Sc,
        invalid: InvalidKeys,
        bits: &DenseBits<'a>,
    ) -> Result<Dictionary<'a, Sc::Source>, DictionaryError> {
        let options = BuildOptions {
            order: IdOrder::SortedKey,
            layout,
            kind: Some(kind),
            dense: false,
            hash_seed: seed,
        };
        let collector = Collector::new(column, kind, &options, accountant, scratch)?;
        let base = dense_base(column);
        let grain = NonZeroUsize::new(64).expect("non-zero");
        let results = par::map_ranges(
            DenseBits::SEGMENTS,
            grain,
            |r| -> Result<(), DictionaryError> {
                let mut worker = collector.worker()?;
                bits.for_each_key(r, |bit| {
                    let code = base + u64::from(bit);
                    worker.add_normalized(u64::from(bit), &code.to_be_bytes())
                })
            },
        );
        results
            .into_iter()
            .find_map(Result::err)
            .map_or(Ok(()), Err)?;
        let mut distinct = collector.finish()?;
        distinct.invalid = invalid;
        distinct.build_hashed()
    }
}

fn refused(accountant: &dyn MemoryAccountant) -> DictionaryError {
    DictionaryError::Budget(InsufficientBudget::new(
        "id dictionary distinct keys",
        1 << 16,
        accountant.available(),
    ))
}

struct Splitter {
    pos: u64,
    key: Vec<u8>,
}

struct UnitOut<'a> {
    parts: Vec<Part>,
    n: u32,
    buckets: Vec<Vec<u8>>,
    counts: Vec<u64>,
    resident: Reservation<'a>,
}

fn parse_located(
    bytes: &[u8],
    order: IdOrder,
    width: Option<usize>,
    count: usize,
) -> Result<Vec<Located<'_>>, DictionaryError> {
    let mut items = Vec::with_capacity(count);
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
        if items.len() == count {
            return Err(DictionaryError::Corrupt("order bucket longer than counted"));
        }
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
/// with each piece's first record index and bytes. Holds a buffer of about 1 MiB.
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
