// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Finishing buckets: each node's items in one list, sorted, its pairs merged and
//! counted, and the list encoded (`docs/spec/eg-adjacency.md` § 1, § 3.1); then the
//! records, the index and the events written in node order.

use std::io::{self, BufWriter, Write};
use std::sync::{Mutex, PoisonError};

use everygraph_core::{
    ByteSink, ByteSource, Memory, MemoryError, MemoryKind, Reservation, SegColumns, usize_of, widen,
};
use everygraph_format::adjacency::{IndexBuilder, append_record};
use everygraph_format::container::SectionWriter;

use super::columns::{Item, Out, Profile, Stored};
use super::fill::{Chunked, Resident};
use super::layout::{Layout, finish_bytes};
use super::{BuildError, Duplicates, GraphOptions, ONE};
use crate::ids::Scratch;

/// What the pairs of some buckets add up to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct PairCounts {
    /// Pairs, kept loops included, each counted once (at its smaller end when
    /// undirected).
    pub(super) pairs: u64,
    /// Rows of a pair beyond its first, loops included, each counted once.
    pub(super) repeats: u64,
    /// Pairs of a node with itself.
    pub(super) loop_pairs: u64,
    /// Rows of those pairs.
    pub(super) loop_rows: u64,
    /// Entries of the adjacency (pairs at both ends when undirected).
    pub(super) entries: u64,
    /// Stored events of the adjacency: the items of an entry under `"events"`, else 1.
    pub(super) events: u64,
    /// The most items of one entry.
    pub(super) most: u64,
}

impl PairCounts {
    fn add(&mut self, other: &Self) {
        self.pairs += other.pairs;
        self.repeats += other.repeats;
        self.loop_pairs += other.loop_pairs;
        self.loop_rows += other.loop_rows;
        self.entries += other.entries;
        self.events += other.events;
        self.most = self.most.max(other.most);
    }
}

/// One finished bucket.
pub(super) struct BucketOut {
    records: Vec<u8>,
    degrees: Vec<u32>,
    record_lens: Vec<u64>,
    /// Stored events of each entry, under `"events"` when some entry has more than one.
    multiplicities: Option<Vec<u32>>,
    /// The values of each stored column, in entry or event order.
    columns: Vec<Vec<u64>>,
    profile: Profile,
    counts: PairCounts,
    /// What the buffers above hold, given back when they are written out.
    _held: Reservation,
}

fn buffer<T>(len: usize) -> Result<Vec<T>, BuildError> {
    let mut v = Vec::new();
    v.try_reserve_exact(len)
        .map_err(|_| MemoryError::AllocationFailed {
            purpose: "bucket being finished",
            bytes: u64::try_from(len.saturating_mul(size_of::<T>())).unwrap_or(u64::MAX),
        })?;
    Ok(v)
}

/// How the lists of an adjacency are finished.
#[derive(Clone, Copy, Debug)]
pub(super) struct Finish<'c> {
    pub(super) graph: GraphOptions,
    /// The columns stored beside the lists.
    pub(super) columns: &'c [Out],
}

/// Finishes the bucket of the `nodes` nodes from `first`: `items` and `locals` are its
/// items and their local sources, in any order.
fn finish_bucket<T: Item>(
    first: u32,
    nodes: usize,
    items: &[T],
    locals: &[u16],
    how: Finish<'_>,
    mut held: Reservation,
) -> Result<BucketOut, BuildError> {
    let graph = how.graph;
    let events = graph.duplicates == Duplicates::Events;
    // The items in node order (a counting sort by local source).
    let mut starts: Vec<u32> = buffer(nodes + 1)?;
    starts.resize(nodes + 1, 0);
    for &l in locals {
        starts[usize::from(l) + 1] += 1;
    }
    for v in 0..nodes {
        starts[v + 1] += starts[v];
    }
    let mut ordered: Vec<T> = buffer(items.len())?;
    ordered.resize(items.len(), T::default());
    {
        let mut next = starts.clone();
        for (&t, &l) in items.iter().zip(locals) {
            let at = &mut next[usize::from(l)];
            ordered[usize::try_from(*at).expect("32 bits")] = t;
            *at += 1;
        }
    }

    let n = items.len();
    let mut columns = Vec::with_capacity(how.columns.len());
    for _ in how.columns {
        columns.push(buffer(n)?);
    }
    let mut out = BucketOut {
        records: buffer(n * 4 + n.div_ceil(4) + nodes)?,
        degrees: buffer(nodes)?,
        record_lens: buffer(nodes)?,
        multiplicities: if events { Some(buffer(n)?) } else { None },
        columns,
        profile: Profile::default(),
        counts: PairCounts::default(),
        _held: Reservation::empty(held.memory(), held.purpose(), held.kind()),
    };
    let mut repeated = false;
    for local in 0..nodes {
        let v = first + u32::try_from(local).expect("buckets hold 2^16 nodes");
        let (s, e) = (usize_of(starts[local]), usize_of(starts[local + 1]));
        let run = &mut ordered[s..e];
        run.sort_unstable_by_key(|x| x.key(events));
        let mut has_loop = false;
        let (mut distinct, mut j) = (0, 0);
        while j < run.len() {
            let t = run[j].target();
            let k = j + run[j..].partition_point(|x| x.target() == t);
            let m = widen(k - j);
            T::pair(&run[j..k], how.columns, &mut out.columns);
            // Each pair once, in place: what is overwritten is done with.
            run[distinct] = run[j];
            distinct += 1;
            j = k;
            has_loop |= v == t;
            let c = &mut out.counts;
            c.entries += 1;
            c.events += if events { m } else { 1 };
            c.most = c.most.max(m);
            if graph.directed || v <= t {
                c.pairs += 1;
                c.repeats += m - 1;
                if v == t {
                    c.loop_pairs += 1;
                    c.loop_rows += m;
                }
            }
            if let Some(ms) = &mut out.multiplicities {
                ms.push(
                    u32::try_from(m)
                        .map_err(|_| BuildError::TooLarge("a pair with 2^32 events or more"))?,
                );
                repeated |= m > 1;
            }
        }
        let targets = run[..distinct].iter().map(|x| x.target());
        let before = out.records.len();
        let degree = append_record(v, targets, &mut out.records)?;
        out.degrees.push(degree);
        out.record_lens
            .push(u64::try_from(out.records.len() - before).expect("64 bits"));
        let loops = u64::from(has_loop && !graph.directed);
        *out.profile.0.entry(u64::from(degree) + loops).or_default() += 1;
    }
    if !repeated {
        out.multiplicities = None;
    }
    let kept = out.records.capacity()
        + out.degrees.capacity() * 4
        + out.record_lens.capacity() * 8
        + out.multiplicities.as_ref().map_or(0, |m| m.capacity() * 4)
        + out.columns.iter().map(|c| c.capacity() * 8).sum::<usize>();
    let _ = held.try_resize(u64::try_from(kept).unwrap_or(u64::MAX));
    out._held = held;
    Ok(out)
}

/// Finishes the buckets of round `round`, whose items are rows `base..` of `columns`
/// (`base` being the items before the round's first bucket, counted from the first
/// bucket the columns hold). Returns the buckets in order.
pub(super) fn finish_round<T: Item>(
    columns: &mut Resident<T>,
    held_from: u64,
    layout: &Layout,
    round: usize,
    how: Finish<'_>,
    memory: &Memory,
) -> Result<Vec<BucketOut>, BuildError> {
    let (first, end) = (layout.rounds[round], layout.rounds[round + 1]);
    // Every bucket's memory is reserved before any is finished, so that a refusal does
    // not depend on the schedule; each keeps what its result holds.
    let mut held = Vec::with_capacity(end - first);
    for b in first..end {
        let items = layout.offsets[b + 1] - layout.offsets[b];
        let nodes = u64::from(layout.starts[b + 1] - layout.starts[b]);
        let bytes = finish_bytes(items, nodes, layout.costs);
        held.push(Some(memory.reserve(
            "buckets being finished",
            MemoryKind::Anonymous,
            bytes,
        )?));
    }
    let held = Mutex::new(held);
    let offsets: Vec<u64> = layout.offsets[first..=end]
        .iter()
        .map(|o| o - held_from)
        .collect();
    let outs: Mutex<Vec<Option<Result<BucketOut, BuildError>>>> =
        Mutex::new((first..end).map(|_| None).collect());
    let pair = (&mut columns.targets, &mut columns.locals);
    pair.for_each_range_mut(&offsets, ONE, |i, (items, locals)| {
        let b = first + i;
        let nodes = usize_of(layout.starts[b + 1] - layout.starts[b]);
        let reservation = held.lock().unwrap_or_else(PoisonError::into_inner)[i]
            .take()
            .expect("one reservation per bucket");
        let out = finish_bucket(layout.starts[b], nodes, items, locals, how, reservation);
        outs.lock().unwrap_or_else(PoisonError::into_inner)[i] = Some(out);
    })?;
    outs.into_inner()
        .unwrap_or_else(PoisonError::into_inner)
        .into_iter()
        .map(|out| out.expect("every bucket was finished"))
        .collect()
}

/// The stored events of every entry, kept from the first bucket in which some pair
/// repeats: before it every entry has one, so a graph without repeated pairs writes
/// nothing. Coded as LEB128 in a scratch file.
pub(super) struct Multiplicities<W: Write> {
    ones: u64,
    out: Option<(BufWriter<W>, Reservation)>,
    memory: Memory,
}

/// Write buffer in front of the multiplicities file.
pub(super) const MULTIPLICITY_BUFFER: usize = 64 << 10;

impl<W: Write> Multiplicities<W> {
    pub(super) fn new(memory: &Memory) -> Self {
        Self {
            ones: 0,
            out: None,
            memory: memory.clone(),
        }
    }

    fn push<Sc: Scratch<Writer = W>>(
        &mut self,
        out: &BucketOut,
        scratch: &Sc,
    ) -> Result<(), BuildError> {
        if self.out.is_none() && out.multiplicities.is_some() {
            let buffer = u64::try_from(MULTIPLICITY_BUFFER).expect("64 KiB");
            let reservation =
                self.memory
                    .reserve("multiplicities buffer", MemoryKind::Anonymous, buffer)?;
            let file = scratch.create("multiplicities")?;
            let writer = BufWriter::with_capacity(MULTIPLICITY_BUFFER, file);
            self.out = Some((writer, reservation));
        }
        let Some((file, _)) = &mut self.out else {
            self.ones += out.counts.entries;
            return Ok(());
        };
        let mut code = [0_u8; 5];
        match &out.multiplicities {
            Some(ms) => {
                for &m in ms {
                    let n = leb128(m, &mut code);
                    file.write_all(&code[..n])?;
                }
            }
            None => {
                for _ in 0..out.counts.entries {
                    file.write_all(&[1])?;
                }
            }
        }
        Ok(())
    }

    /// The entries that come before the file, and the file; `None` when no pair
    /// repeats.
    pub(super) fn finish<Sc: Scratch<Writer = W>>(
        self,
        scratch: &Sc,
    ) -> io::Result<Option<(u64, Sc::Source)>> {
        let Some((file, _)) = self.out else {
            return Ok(None);
        };
        let file = file.into_inner().map_err(io::IntoInnerError::into_error)?;
        Ok(Some((self.ones, scratch.seal(file)?)))
    }
}

fn leb128(mut m: u32, out: &mut [u8; 5]) -> usize {
    let mut n = 0;
    loop {
        let byte = u8::try_from(m & 0x7f).expect("7 bits");
        m >>= 7;
        if m == 0 {
            out[n] = byte;
            return n + 1;
        }
        out[n] = byte | 0x80;
        n += 1;
    }
}

/// The multiplicities of [`Multiplicities::finish`] as a sequence: `ones` ones, then
/// the file. A read error ends it early and is kept in `error`.
pub(super) fn multiplicities<'a, R: ByteSource>(
    ones: u64,
    file: &'a R,
    memory: &Memory,
    error: &'a Mutex<Option<BuildError>>,
) -> Result<impl Iterator<Item = u64> + 'a, BuildError> {
    let mut reader = Chunked::new(file, memory)?;
    let coded = std::iter::from_fn(move || {
        let mut value = 0_u64;
        for shift in (0..35).step_by(7) {
            match reader.take(1) {
                Ok(Some(b)) => {
                    value |= u64::from(b[0] & 0x7f) << shift;
                    if b[0] & 0x80 == 0 {
                        return Some(value);
                    }
                }
                Ok(None) if shift == 0 => return None,
                Ok(None) => break,
                Err(e) => {
                    *error.lock().unwrap_or_else(PoisonError::into_inner) = Some(e);
                    return None;
                }
            }
        }
        *error.lock().unwrap_or_else(PoisonError::into_inner) = Some(BuildError::Scratch(
            io::Error::other("the multiplicities file is damaged"),
        ));
        None
    });
    Ok(std::iter::repeat_n(1, usize::try_from(ones).unwrap_or(usize::MAX)).chain(coded))
}

/// Where finished buckets go, in node order.
pub(super) struct Output<'w, K: ByteSink, W: Write> {
    pub(super) data: SectionWriter<'w, K>,
    pub(super) index: IndexBuilder,
    pub(super) multiplicities: Multiplicities<W>,
    pub(super) columns: Stored<W>,
    pub(super) profile: Profile,
    pub(super) counts: PairCounts,
}

impl<K: ByteSink, W: Write> Output<'_, K, W> {
    pub(super) fn take<Sc: Scratch<Writer = W>>(
        &mut self,
        out: BucketOut,
        scratch: &Sc,
    ) -> Result<(), BuildError> {
        self.data.write(&out.records)?;
        for (&degree, &len) in out.degrees.iter().zip(&out.record_lens) {
            self.index.push(degree, len)?;
        }
        self.multiplicities.push(&out, scratch)?;
        self.columns.push(&out.columns)?;
        self.profile.add(&out.profile);
        self.counts.add(&out.counts);
        Ok(())
    }
}
