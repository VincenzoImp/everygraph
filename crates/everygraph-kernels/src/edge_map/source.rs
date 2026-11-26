// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Where the lists come from: the [`Source`] trait and its three implementations.
//!
//! | Source | Lists | Per-worker reader |
//! |---|---|---|
//! | [`Csr`] | raw offsets and targets, slices in memory | nothing |
//! | [`Compressed`] | the adjacency codec (DESIGN D4) over bytes in memory: a heap buffer or a memory map | a decode batch |
//! | [`Stream`] | raw targets read through a [`ByteSource`]; offsets in memory | a read buffer |
//!
//! Every source checks what it hands out: a list names only nodes below
//! [`nodes`](Source::nodes). The codec checks the framing of its records, not the graph,
//! so a corrupt file can decode to any `u32`. [`Csr`] checks its arrays when it is
//! made. [`Compressed`] and [`Stream`] check their offsets when made, and each batch
//! after they decode or read it. A prefix or a part of a list that [`Compressed`]
//! decodes alone ([`Source::scan_prefixes`], [`Source::list_part`]) is checked the same
//! way, but the rest of its record is not read, so damage there goes unseen. Lists are
//! not checked to be sorted, so a kernel must not rely on order for anything but its
//! answer.

use std::fmt;
use std::num::NonZeroUsize;
use std::ops::Range;

use everygraph_core::{
    AccessHint, ByteSource, Memory, MemoryKind, NodeId, Reservation, SegVec, par,
};
use everygraph_format::adjacency::{Adjacency, AdjacencyIndex, Batch};

use super::{Error, ix};

/// Lists of a graph, read in node order (scans) or by node (frontiers).
///
/// Only the drivers of [`Plan`](super::Plan) call these methods. Each worker holds a
/// [`Reader`](Self::Reader): the buffers that lists are decoded or read into, reserved
/// against the memory budget. A list comes with its first entry: the entries before it in
/// node order, which index the graph's per-entry columns (weights, for example).
///
/// # Safety
///
/// The drivers index per-node state with the nodes these methods hand out, without
/// bounds checks. So, for the life of the value, an implementation guarantees:
///
/// - [`nodes`](Self::nodes) returns the same value.
/// - Every list that [`scan`](Self::scan), [`scan_prefixes`](Self::scan_prefixes) and
///   [`lists`](Self::lists) pass to `f`, and every list that [`list`](Self::list) and
///   [`list_part`](Self::list_part) return, names only nodes below `nodes()`.
/// - That check is made on the memory the list is handed out from, after the last write
///   to that memory. A check of other bytes (the file, the map, an earlier read) does
///   not count.
/// - [`scan`](Self::scan), [`scan_prefixes`](Self::scan_prefixes) and
///   [`degrees`](Self::degrees) call `filter` and `f` only with a `v` in the range
///   `nodes`. [`lists`](Self::lists) calls `f` only with a `v` that
///   is an element of `nodes`. [`list`](Self::list) returns only for `v < nodes()`.
pub unsafe trait Source: Sync {
    /// Per-worker state: the buffers lists are decoded or read into.
    type Reader: Send;

    /// Number of nodes.
    fn nodes(&self) -> u32;

    /// Number of entries (the sum of the degrees).
    fn entries(&self) -> u64;

    /// Entries before the list of `v`, for `v` in `0..=nodes()`.
    fn entry_offset(&self, v: u32) -> u64;

    /// The degree of `v`, by random access.
    ///
    /// # Panics
    ///
    /// If `v` is not a node.
    fn degree(&self, v: u32) -> u32 {
        let d = self.entry_offset(v + 1) - self.entry_offset(v);
        u32::try_from(d).expect("a degree below 2^32")
    }

    /// Calls `f(v, degree)` for each node of `nodes`, in order.
    fn degrees(&self, nodes: Range<u32>, mut f: impl FnMut(u32, u32)) {
        for v in nodes {
            f(v, self.degree(v));
        }
    }

    /// A reader for calls of at most `lists` lists, in batches of about `batch` entries.
    ///
    /// # Errors
    ///
    /// [`Error::Memory`] if its buffers do not fit in the budget.
    fn reader(&self, memory: &Memory, batch: usize, lists: usize) -> Result<Self::Reader, Error>;

    /// Calls `f(v, first_entry, list)` for each node `v` of `nodes` with `filter(v)`, in
    /// order. A batch of consecutive lists is decoded or read before the calls on it,
    /// never list by list. Nothing is decoded or read when `filter` passes no node.
    /// `filter` may be called twice for a node.
    ///
    /// # Errors
    ///
    /// The lists could not be read, or are corrupt, or the reader's buffers do not fit
    /// in the budget.
    fn scan(
        &self,
        reader: &mut Self::Reader,
        nodes: Range<u32>,
        filter: impl Fn(u32) -> bool,
        f: impl FnMut(u32, u64, &[u32]),
    ) -> Result<(), Error>;

    /// As [`scan`](Self::scan), but `f(v, first_entry, prefix, degree)` receives at least
    /// the first `min(degree, len)` entries of the list of `v`, and the degree. A source
    /// that decodes may then decode no more of each list; the default hands out whole
    /// lists.
    ///
    /// # Errors
    ///
    /// As [`scan`](Self::scan).
    fn scan_prefixes(
        &self,
        reader: &mut Self::Reader,
        nodes: Range<u32>,
        filter: impl Fn(u32) -> bool,
        len: usize,
        mut f: impl FnMut(u32, u64, &[u32], u32),
    ) -> Result<(), Error> {
        let _ = len;
        self.scan(reader, nodes, filter, |v, first, list| {
            f(
                v,
                first,
                list,
                u32::try_from(list.len()).expect("a degree below 2^32"),
            );
        })
    }

    /// Calls `f(v, first_entry, list)` for each node `v` of `nodes`, which is increasing.
    /// The lists of consecutive nodes are decoded or read together.
    ///
    /// # Errors
    ///
    /// As [`scan`](Self::scan).
    fn lists(
        &self,
        reader: &mut Self::Reader,
        nodes: &[u32],
        f: impl FnMut(u32, u64, &[u32]),
    ) -> Result<(), Error>;

    /// The first entry and the list of `v`.
    ///
    /// # Errors
    ///
    /// As [`scan`](Self::scan).
    fn list<'r>(&'r self, reader: &'r mut Self::Reader, v: u32) -> Result<(u64, &'r [u32]), Error>;

    /// Entries `from..to` of the list of `v`, where `prev` is entry `from − 1` as this
    /// source handed it out, `from` is a multiple of four with `4 ≤ from < to`, and `to`
    /// is at most the degree. A source that decodes then decodes no more of the list;
    /// the default reads all of it.
    ///
    /// # Errors
    ///
    /// As [`scan`](Self::scan).
    fn list_part<'r>(
        &'r self,
        reader: &'r mut Self::Reader,
        v: u32,
        (from, prev): (usize, u32),
        to: usize,
    ) -> Result<&'r [u32], Error> {
        let _ = prev;
        let (_, list) = self.list(reader, v)?;
        Ok(list.get(from..to.min(list.len())).unwrap_or_default())
    }

    /// Tells the backend that the lists of `nodes` (increasing) will be read soon. Advice
    /// only; the default ignores it.
    fn prefetch(&self, nodes: &[u32]) {
        let _ = nodes;
    }

    /// Called when a reader goes back to the pool: it may free buffers beyond its usual
    /// size, which a long list made it take.
    fn idle(&self, reader: &mut Self::Reader) {
        let _ = reader;
    }
}

/// Entries or nodes per task when a source checks its arrays in parallel.
const CHECK_GRAIN: NonZeroUsize = NonZeroUsize::new(1 << 16).expect("non-zero");

/// Whether every target is below `n`.
fn in_range(targets: &[u32], n: u32) -> bool {
    targets.is_empty() || targets.iter().fold(0, |m, &t| m.max(t)) < n
}

/// The first entry of the lists `first..` that names a node `>= n`, as an error.
fn bad_target(first: u32, lists: impl Iterator<Item = impl AsRef<[u32]>>, n: u32) -> Error {
    for (v, list) in (first..).zip(lists) {
        if let Some(&target) = list.as_ref().iter().find(|&&t| t >= n) {
            return Error::Target { node: v, target };
        }
    }
    Error::Invalid("a list names a node outside the graph")
}

/// Checks CSR offsets over `entries` targets, in parallel: `n + 1` offsets with
/// `n < 2^32`, from 0 to `entries`, never decreasing, every degree below 2^32. Returns
/// `n`.
fn check_offsets(offsets: &[u64], entries: u64) -> Result<u32, Error> {
    let n = offsets
        .len()
        .checked_sub(1)
        .ok_or(Error::Invalid("offsets: none"))?;
    let n = u32::try_from(n).map_err(|_| Error::Invalid("offsets: 2^32 nodes or more"))?;
    if offsets[0] != 0 || offsets[ix(n.into())] != entries {
        return Err(Error::Invalid(
            "offsets: do not run from 0 to the number of entries",
        ));
    }
    let ok = par::map_ranges(offsets.len() - 1, CHECK_GRAIN, |r| {
        offsets[r.start..=r.end]
            .windows(2)
            .all(|w| w[0] <= w[1] && u32::try_from(w[1] - w[0]).is_ok())
    });
    if ok.into_iter().all(|x| x) {
        Ok(n)
    } else {
        Err(Error::Invalid(
            "offsets: decrease, or a degree is 2^32 or more",
        ))
    }
}

/// A graph as raw CSR arrays in memory: `offsets` (`n + 1`) and `targets`.
#[derive(Clone, Copy, Debug)]
pub struct Csr<'a> {
    /// Checked by `new`: a CSR over `targets`.
    offsets: &'a [u64],
    /// Checked by `new`: every target is below `n`.
    targets: &'a [u32],
    n: u32,
}

impl<'a> Csr<'a> {
    /// Checks the arrays, in parallel, and wraps them: `offsets` must be a CSR over
    /// `targets`, and every target below the node count.
    ///
    /// Over a memory map, the soundness of the unchecked reads relies also on the
    /// invariant of `everygraph-io`: no writer changes a mapped file.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] or [`Error::Target`] for the first problem found.
    pub fn new(offsets: &'a [u64], targets: &'a [u32]) -> Result<Self, Error> {
        let n = check_offsets(offsets, targets.len() as u64)?;
        let ok = par::map_ranges(targets.len(), CHECK_GRAIN, |r| in_range(&targets[r], n));
        if !ok.into_iter().all(|x| x) {
            let lists = (0..n).map(|v| {
                let v = ix(v.into());
                &targets[ix(offsets[v])..ix(offsets[v + 1])]
            });
            return Err(bad_target(0, lists, n));
        }
        Ok(Self {
            offsets,
            targets,
            n,
        })
    }

    /// The first entry and the list of `v`.
    ///
    /// # Panics
    ///
    /// If `v` is not a node.
    #[inline]
    fn get(&self, v: u32) -> (u64, &'a [u32]) {
        assert!(v < self.n, "node {v} outside the graph");
        let v = ix(v.into());
        // SAFETY: `v < n`, and `offsets` has `n + 1` elements (`new`).
        let a = *unsafe { self.offsets.get_unchecked(v) };
        // SAFETY: as above.
        let b = *unsafe { self.offsets.get_unchecked(v + 1) };
        // SAFETY: the offsets never decrease and end at `targets.len()` (`new`), so
        // `a <= b <= targets.len()`.
        (a, unsafe { self.targets.get_unchecked(ix(a)..ix(b)) })
    }
}

// SAFETY: every target is below `n` (`new`). The arrays are borrowed, so they do not
// change; over a memory map that also needs the invariant of `everygraph-io` (no writer
// changes a mapped file). `scan` and `lists` call back only with nodes they were given.
unsafe impl Source for Csr<'_> {
    type Reader = ();

    fn nodes(&self) -> u32 {
        self.n
    }

    fn entries(&self) -> u64 {
        self.targets.len() as u64
    }

    fn entry_offset(&self, v: u32) -> u64 {
        self.offsets[ix(v.into())]
    }

    fn reader(&self, _: &Memory, _: usize, _: usize) -> Result<(), Error> {
        Ok(())
    }

    #[inline]
    fn scan(
        &self,
        (): &mut (),
        nodes: Range<u32>,
        filter: impl Fn(u32) -> bool,
        mut f: impl FnMut(u32, u64, &[u32]),
    ) -> Result<(), Error> {
        for v in nodes.filter(|&v| filter(v)) {
            let (first, list) = self.get(v);
            f(v, first, list);
        }
        Ok(())
    }

    fn lists(
        &self,
        (): &mut (),
        nodes: &[u32],
        mut f: impl FnMut(u32, u64, &[u32]),
    ) -> Result<(), Error> {
        for &v in nodes {
            let (first, list) = self.get(v);
            f(v, first, list);
        }
        Ok(())
    }

    fn list<'r>(&'r self, (): &'r mut (), v: u32) -> Result<(u64, &'r [u32]), Error> {
        Ok(self.get(v))
    }
}

/// The reader of a [`Compressed`] source: one decode batch, and the reservation that
/// covers its heap.
#[derive(Debug)]
pub struct DecodeReader {
    batch: Batch,
    max_entries: usize,
    /// Lists per call at most, the usual size of the batch's list ends.
    lists: usize,
    /// Invariant: `held.bytes() >= batch.heap_bytes()`.
    held: Reservation,
}

impl DecodeReader {
    /// Makes room in the batch for `entries` entries in `lists` lists, reserved first.
    fn make_room(&mut self, entries: usize, lists: usize) -> Result<(), Error> {
        let (have_entries, have_lists) = self.batch.capacity();
        if entries <= have_entries && lists <= have_lists {
            return Ok(());
        }
        // The old buffers exist next to the new ones while the data moves.
        self.held
            .try_grow(Batch::bytes_for(entries, lists) as u64)?;
        self.batch.reserve(entries, lists);
        let held = self.batch.heap_bytes() as u64;
        if let Err(e) = self.held.try_resize(held) {
            self.batch.shrink_to(0, 0);
            self.held.shrink(self.held.bytes());
            return Err(e.into());
        }
        Ok(())
    }

    /// Frees the batch beyond its usual size.
    fn release(&mut self) {
        let usual = self.max_entries.saturating_add(8);
        let (entries, lists) = self.batch.capacity();
        if entries > usual || lists > self.lists {
            self.batch.shrink_to(usual, self.lists);
            self.held
                .shrink(self.held.bytes() - self.batch.heap_bytes() as u64);
        }
    }
}

/// A graph in the compressed adjacency format (DESIGN D4), its data part in memory.
#[derive(Clone, Copy)]
pub struct Compressed<'a> {
    adjacency: Adjacency<'a>,
    n: u32,
    /// The [`ByteSource`] of the data part, for prefetch advice.
    hints: Option<&'a (dyn ByteSource + Sync)>,
}

impl fmt::Debug for Compressed<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Compressed")
            .field("adjacency", &self.adjacency)
            .field("hints", &self.hints.is_some())
            .finish_non_exhaustive()
    }
}

impl<'a> Compressed<'a> {
    /// Wraps an adjacency whose data part is in memory. Lists are checked as they are
    /// decoded.
    #[must_use]
    pub fn new(adjacency: Adjacency<'a>) -> Self {
        Self {
            n: adjacency.index().nodes(),
            adjacency,
            hints: None,
        }
    }

    /// The adjacency of `index` over the data part held by `source` (a memory map,
    /// typically), which also receives prefetch advice.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] if `source` does not hold its bytes in memory, and
    /// [`Error::Format`] if they are not the data part of `index`.
    pub fn mapped(
        index: &'a AdjacencyIndex,
        source: &'a (dyn ByteSource + Sync),
    ) -> Result<Self, Error> {
        let data = source
            .as_slice()
            .ok_or(Error::Invalid("the data part is not in memory"))?;
        let adjacency = Adjacency::new(index, data).map_err(Error::Format)?;
        Ok(Self {
            hints: Some(source),
            ..Self::new(adjacency)
        })
    }

    /// Decodes the first `prefix` entries of each list from `nodes.start`, as far as the
    /// reader's batch size allows, and checks what the batch hands out; returns the first
    /// node not decoded. A batch that fails a check is emptied.
    fn decode(&self, r: &mut DecodeReader, nodes: Range<u32>, prefix: usize) -> Result<u32, Error> {
        let entries = self
            .adjacency
            .prefix_entries(nodes.clone(), prefix, r.max_entries);
        r.make_room(entries, nodes.len())?;
        let checked = self
            .adjacency
            .decode_prefixes(nodes.clone(), prefix, r.max_entries, &mut r.batch)
            .map_err(Error::from)
            .and_then(|end| self.check(&r.batch, nodes.start, end).map(|()| end));
        if checked.is_err() {
            r.batch.clear();
        }
        #[cfg(feature = "frontier-counters")]
        super::counters::decoded(r.batch.targets().len());
        checked
    }

    /// Checks that `batch` holds the lists of `first..end`, that together they are
    /// exactly `batch.targets()`, and that those name only nodes of the graph.
    fn check(&self, batch: &Batch, first: u32, end: u32) -> Result<(), Error> {
        let lists = batch.len();
        let targets = batch.targets().as_ptr_range();
        let shape = batch.first() == first
            && lists == ix((end - first).into())
            && lists > 0
            && batch.list(0).as_ptr_range().start == targets.start
            && batch.list(lists - 1).as_ptr_range().end == targets.end;
        if !shape {
            return Err(Error::Invalid(
                "a decoded batch does not hold the lists asked for",
            ));
        }
        if !in_range(batch.targets(), self.n) {
            return Err(bad_target(first, (0..lists).map(|i| batch.list(i)), self.n));
        }
        Ok(())
    }

    /// Calls `f` for each list of the reader's batch, with its node and first entry.
    fn hand_out(&self, r: &DecodeReader, mut f: impl FnMut(u32, u64, &[u32])) {
        let mut entry = self.entry_offset(r.batch.first());
        for (v, list) in r.batch.iter() {
            f(v.get(), entry, list);
            entry += list.len() as u64;
        }
    }

    /// Calls `decode` on runs of `nodes` that hold every node with `filter`, in order. Two
    /// such nodes are in one run when the nodes between them hold at most [`GAP`] entries,
    /// `cost(degree)` each: those records are decoded and not handed out.
    fn runs(
        &self,
        nodes: Range<u32>,
        filter: &impl Fn(u32) -> bool,
        cost: impl Fn(u32) -> usize,
        mut decode: impl FnMut(Range<u32>) -> Result<(), Error>,
    ) -> Result<(), Error> {
        // Where every node passes (a scan of the whole graph), no degree is read.
        let Some(fail) = nodes.clone().find(|&v| !filter(v)) else {
            return if nodes.is_empty() {
                Ok(())
            } else {
                decode(nodes)
            };
        };
        let mut run = (nodes.start < fail).then_some(nodes.start..fail);
        let mut gap = 0_usize;
        let rest = fail..nodes.end;
        for (v, d) in rest.clone().zip(self.adjacency.index().degrees(rest)) {
            if !filter(v) {
                gap = gap.saturating_add(cost(d));
                continue;
            }
            run = match run {
                Some(r) if gap <= GAP => Some(r.start..v + 1),
                Some(r) => {
                    decode(r)?;
                    Some(v..v + 1)
                }
                None => Some(v..v + 1),
            };
            gap = 0;
        }
        run.map_or(Ok(()), decode)
    }
}

/// Entries of nodes that a scan's filter leaves out, between two nodes it passes, that
/// are decoded rather than skipped: a new decode starts with two Elias–Fano selects.
const GAP: usize = 64;

// SAFETY: every list handed out comes from the reader's batch after `check` accepted it
// in the same call: the batch holds the lists asked for, they lie inside
// `batch.targets()`, and every target there is below `n`. The index is borrowed, so `n`
// does not change.
unsafe impl Source for Compressed<'_> {
    type Reader = DecodeReader;

    fn nodes(&self) -> u32 {
        self.n
    }

    fn entries(&self) -> u64 {
        self.adjacency.index().entries()
    }

    fn entry_offset(&self, v: u32) -> u64 {
        self.adjacency.index().entry_offset(NodeId::new(v)).get()
    }

    fn degree(&self, v: u32) -> u32 {
        self.adjacency.index().degree(NodeId::new(v))
    }

    #[inline]
    fn degrees(&self, nodes: Range<u32>, mut f: impl FnMut(u32, u32)) {
        for (v, d) in nodes.clone().zip(self.adjacency.index().degrees(nodes)) {
            f(v, d);
        }
    }

    fn reader(&self, memory: &Memory, batch: usize, lists: usize) -> Result<DecodeReader, Error> {
        let mut r = DecodeReader {
            batch: Batch::default(),
            max_entries: batch,
            lists,
            held: Reservation::empty(memory, "edge-map decode batch", MemoryKind::Anonymous),
        };
        r.make_room(0, lists)?;
        Ok(r)
    }

    #[inline]
    fn scan(
        &self,
        r: &mut DecodeReader,
        nodes: Range<u32>,
        filter: impl Fn(u32) -> bool,
        mut f: impl FnMut(u32, u64, &[u32]),
    ) -> Result<(), Error> {
        self.runs(
            nodes,
            &filter,
            |d| ix(d.into()),
            |run| {
                let mut at = run.start;
                while at < run.end {
                    let next = self.decode(r, at..run.end, usize::MAX)?;
                    self.hand_out(r, |v, entry, list| {
                        if filter(v) {
                            f(v, entry, list);
                        }
                    });
                    at = next;
                }
                Ok(())
            },
        )
    }

    fn scan_prefixes(
        &self,
        r: &mut DecodeReader,
        nodes: Range<u32>,
        filter: impl Fn(u32) -> bool,
        len: usize,
        mut f: impl FnMut(u32, u64, &[u32], u32),
    ) -> Result<(), Error> {
        let index = self.adjacency.index();
        self.runs(
            nodes,
            &filter,
            |d| ix(d.into()).min(len),
            |run| {
                let mut at = run.start;
                while at < run.end {
                    let next = self.decode(r, at..run.end, len)?;
                    let mut entry = self.entry_offset(at);
                    for ((v, list), d) in r.batch.iter().zip(index.degrees(at..next)) {
                        if filter(v.get()) {
                            f(v.get(), entry, list, d);
                        }
                        entry += u64::from(d);
                    }
                    at = next;
                }
                Ok(())
            },
        )
    }

    fn lists(
        &self,
        r: &mut DecodeReader,
        nodes: &[u32],
        mut f: impl FnMut(u32, u64, &[u32]),
    ) -> Result<(), Error> {
        let mut i = 0;
        while i < nodes.len() {
            // A run of consecutive nodes is one decode.
            let start = nodes[i];
            let mut end = i + 1;
            while end < nodes.len() && nodes[end] == nodes[end - 1] + 1 {
                end += 1;
            }
            let stop = self.decode(r, start..nodes[end - 1] + 1, usize::MAX)?;
            self.hand_out(r, &mut f);
            i += ix(u64::from(stop - start));
        }
        Ok(())
    }

    fn list<'r>(&'r self, r: &'r mut DecodeReader, v: u32) -> Result<(u64, &'r [u32]), Error> {
        self.decode(r, v..v + 1, usize::MAX)?;
        Ok((self.entry_offset(v), r.batch.list(0)))
    }

    fn list_part<'r>(
        &'r self,
        r: &'r mut DecodeReader,
        v: u32,
        from: (usize, u32),
        to: usize,
    ) -> Result<&'r [u32], Error> {
        r.make_room(self.adjacency.part_entries(from.0, to), 1)?;
        let checked = self
            .adjacency
            .decode_part(NodeId::new(v), from, to, &mut r.batch)
            .map_err(Error::from)
            .and_then(|()| self.check(&r.batch, v, v + 1));
        if checked.is_err() {
            r.batch.clear();
        }
        #[cfg(feature = "frontier-counters")]
        super::counters::decoded(r.batch.targets().len());
        checked.map(|()| r.batch.list(0))
    }

    fn prefetch(&self, nodes: &[u32]) {
        if let Some(source) = self.hints {
            let index = self.adjacency.index();
            advise_runs(source, nodes, |v| index.record_range(NodeId::new(v)));
        }
    }

    fn idle(&self, r: &mut DecodeReader) {
        r.release();
    }
}

/// Byte gap below which two ranges are advised, or read, as one.
const COALESCE_GAP: u64 = 16 << 10;

/// Passes [`AccessHint::WillNeed`] to `source` for the byte ranges of `nodes`, merged
/// where they are less than [`COALESCE_GAP`] apart.
fn advise_runs(source: &dyn ByteSource, nodes: &[u32], range: impl Fn(u32) -> Range<u64>) {
    let mut run: Option<Range<u64>> = None;
    for &v in nodes {
        let r = range(v);
        if r.is_empty() {
            continue;
        }
        run = match run {
            Some(cur) if r.start <= cur.end + COALESCE_GAP => Some(cur.start..r.end.max(cur.end)),
            Some(cur) => {
                source.advise(cur, AccessHint::WillNeed);
                Some(r)
            }
            None => Some(r),
        };
    }
    if let Some(cur) = run {
        source.advise(cur, AccessHint::WillNeed);
    }
}

/// The reader of a [`Stream`] source: the bytes read and the lists they hold, each in a
/// [`SegVec`], which reserves what it allocates.
#[derive(Debug)]
pub struct StreamReader {
    bytes: SegVec<u8>,
    lists: SegVec<u32>,
    max_entries: usize,
}

/// A raw CSR whose offsets are in memory and whose targets (little-endian `u32`, the
/// `targets.u32` file of a CSR directory) are read through a [`ByteSource`], a batch at a
/// time: the semi-external regime (FINDINGS § B1), O(n) state in memory.
#[derive(Clone, Copy)]
pub struct Stream<'a> {
    offsets: &'a [u64],
    targets: &'a (dyn ByteSource + Sync),
    n: u32,
}

impl fmt::Debug for Stream<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Stream")
            .field("nodes", &self.n)
            .finish_non_exhaustive()
    }
}

impl<'a> Stream<'a> {
    /// Checks the offsets against the length of `targets` and wraps both. Targets are
    /// checked as they are read.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] if the offsets are not those of a CSR over `targets`.
    pub fn new(offsets: &'a [u64], targets: &'a (dyn ByteSource + Sync)) -> Result<Self, Error> {
        if !targets.len().is_multiple_of(4) {
            return Err(Error::Invalid("targets: not a whole number of u32"));
        }
        let n = check_offsets(offsets, targets.len() / 4)?;
        Ok(Self {
            offsets,
            targets,
            n,
        })
    }

    /// Reads the lists of `nodes` into the reader and checks them.
    fn read(&self, r: &mut StreamReader, nodes: Range<u32>) -> Result<(), Error> {
        let (a, b) = (
            self.offsets[ix(nodes.start.into())],
            self.offsets[ix(nodes.end.into())],
        );
        let len = ix(b - a);
        let bytes = len
            .checked_mul(4)
            .ok_or(Error::Invalid("a read longer than the address space"))?;
        r.bytes.try_resize(bytes, 0)?;
        r.lists.try_resize(len, 0)?;
        let too_long = || Error::Invalid("a read longer than one segment (#m3-15)");
        let raw = r.bytes.as_mut_slice().ok_or_else(too_long)?;
        self.targets.read_at(a * 4, raw)?;
        let lists = r.lists.as_mut_slice().ok_or_else(too_long)?;
        for (t, c) in lists.iter_mut().zip(raw.chunks_exact(4)) {
            *t = u32::from_le_bytes(c.try_into().expect("four bytes"));
        }
        if !in_range(lists, self.n) {
            let lists = nodes.clone().map(|v| self.slice(r, nodes.start, v));
            return Err(bad_target(nodes.start, lists, self.n));
        }
        Ok(())
    }

    /// The list of `v` among the lists read from `first` on.
    fn slice<'r>(&self, r: &'r StreamReader, first: u32, v: u32) -> &'r [u32] {
        let base = self.offsets[ix(first.into())];
        let v = ix(v.into());
        let lists = r.lists.as_slice().unwrap_or_default();
        &lists[ix(self.offsets[v] - base)..ix(self.offsets[v + 1] - base)]
    }

    /// The end of the batch that starts at `start`: as many lists as fit in `max`
    /// entries, at least one, at most up to `end`.
    fn batch_end(&self, start: u32, end: u32, max: usize) -> u32 {
        let limit = self.offsets[ix(start.into())].saturating_add(max as u64);
        let mut stop = start + 1;
        while stop < end && self.offsets[ix(u64::from(stop) + 1)] <= limit {
            stop += 1;
        }
        stop
    }
}

// SAFETY: every list handed out is a slice of the reader's `lists` after `read` checked
// all of them, in that buffer, in the same call. The offsets are borrowed, so `n` does
// not change. `scan` and `lists` call back only with nodes they were given.
unsafe impl Source for Stream<'_> {
    type Reader = StreamReader;

    fn nodes(&self) -> u32 {
        self.n
    }

    fn entries(&self) -> u64 {
        self.offsets[ix(self.n.into())]
    }

    fn entry_offset(&self, v: u32) -> u64 {
        self.offsets[ix(v.into())]
    }

    fn reader(&self, memory: &Memory, batch: usize, _: usize) -> Result<StreamReader, Error> {
        Ok(StreamReader {
            bytes: SegVec::new(memory, "edge-map stream bytes"),
            lists: SegVec::new(memory, "edge-map stream lists"),
            max_entries: batch,
        })
    }

    fn scan(
        &self,
        r: &mut StreamReader,
        nodes: Range<u32>,
        filter: impl Fn(u32) -> bool,
        mut f: impl FnMut(u32, u64, &[u32]),
    ) -> Result<(), Error> {
        if !nodes.clone().any(&filter) {
            return Ok(());
        }
        let mut at = nodes.start;
        while at < nodes.end {
            let stop = self.batch_end(at, nodes.end, r.max_entries);
            self.read(r, at..stop)?;
            for v in (at..stop).filter(|&v| filter(v)) {
                f(v, self.entry_offset(v), self.slice(r, at, v));
            }
            at = stop;
        }
        Ok(())
    }

    fn lists(
        &self,
        r: &mut StreamReader,
        nodes: &[u32],
        mut f: impl FnMut(u32, u64, &[u32]),
    ) -> Result<(), Error> {
        let mut i = 0;
        while i < nodes.len() {
            // Lists less than COALESCE_GAP bytes apart are one read, within the batch.
            let start = nodes[i];
            let limit = self.offsets[ix(start.into())].saturating_add(r.max_entries as u64);
            let mut end = i + 1;
            while end < nodes.len() {
                let (prev, next) = (ix(nodes[end - 1].into()), ix(nodes[end].into()));
                let gap = (self.offsets[next] - self.offsets[prev + 1]) * 4;
                if gap > COALESCE_GAP || self.offsets[next + 1] > limit {
                    break;
                }
                end += 1;
            }
            self.read(r, start..nodes[end - 1] + 1)?;
            for &v in &nodes[i..end] {
                f(v, self.entry_offset(v), self.slice(r, start, v));
            }
            i = end;
        }
        Ok(())
    }

    fn list<'r>(&'r self, r: &'r mut StreamReader, v: u32) -> Result<(u64, &'r [u32]), Error> {
        assert!(v < self.n, "node {v} outside the graph");
        self.read(r, v..v + 1)?;
        Ok((self.entry_offset(v), self.slice(r, v, v)))
    }

    fn prefetch(&self, nodes: &[u32]) {
        advise_runs(self.targets, nodes, |v| {
            let v = ix(v.into());
            self.offsets[v] * 4..self.offsets[v + 1] * 4
        });
    }

    fn idle(&self, r: &mut StreamReader) {
        let usual = r.max_entries.saturating_add(8);
        if r.lists.capacity() > usual {
            r.lists.truncate(usual);
            r.lists.shrink_to_fit();
            r.bytes.truncate(usual * 4);
            r.bytes.shrink_to_fit();
        }
    }
}
