// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Where the lists come from: the [`Source`] trait and its three implementations.
//!
//! | Source | Lists | Per-worker reader |
//! |---|---|---|
//! | [`Csr`] | raw offsets and targets, slices in memory | nothing |
//! | [`Compressed`] | the adjacency codec (DESIGN D4) over bytes in memory: a heap buffer or a memory map | a decode batch |
//! | [`Stream`] | raw targets read through a [`ByteSource`] (a file, read sequentially); offsets in memory | a read buffer |
//!
//! Every source checks what it hands out: a list names only nodes below
//! [`nodes`](Source::nodes), so a kernel can index its per-node arrays with any target
//! without a corrupt or hostile file making it panic or read the wrong node (the codec
//! checks record framing, not graph invariants: review of PR #157). [`Csr`] and
//! [`Stream`] check their offsets when made and [`Csr`] its targets too; [`Compressed`]
//! and [`Stream`] check each batch they decode or read. Lists are not checked to be
//! sorted: the kernels must not rely on it for anything but their answer.
//!
//! The trait does not assume one contiguous adjacency: a source hands out each node's
//! whole list, in node order, through callbacks, and says where the entries before a
//! node end ([`Source::entry_offset`], used to cut tasks). A source that merges several
//! runs of a dynamic graph (a base and its deltas, `docs/research/sota-dynamic.md`) in
//! canonical order would implement it here, merging each node's lists into its reader's
//! buffer before the call; nothing in the drivers changes.

use std::num::NonZeroUsize;
use std::ops::Range;

use everygraph_core::{AccessHint, ByteSource, Memory, MemoryKind, NodeId, Reservation, par};
use everygraph_format::adjacency::{Adjacency, Batch};

use super::node::{CheckedCsr, check_csr, check_offsets, csr_list};
use super::{Error, ix};

/// Lists of a graph, read in node order (scans) or by node (frontiers).
///
/// The drivers of [`Plan`](super::Plan) are the only callers; a kernel never calls these
/// methods itself. Each worker holds one [`Reader`](Self::Reader), which owns the
/// buffers that lists are decoded or read into, reserved against the memory budget.
///
/// # Safety
///
/// The drivers index per-node state, without bounds checks, with the nodes these
/// methods hand out, so an implementation guarantees that [`nodes`](Self::nodes) does
/// not change, that every list [`scan`](Self::scan) and [`lists`](Self::lists) pass to
/// `f` names only nodes below it, and that `filter` and `f` of [`scan`](Self::scan) and
/// [`degrees`](Self::degrees), and `f` of [`lists`](Self::lists), are called only with
/// nodes they were asked about.
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
    fn degree(&self, v: u32) -> u32;

    /// Calls `f(v, degree)` for each node of `nodes`, in order.
    fn degrees(&self, nodes: Range<u32>, f: impl FnMut(u32, u32));

    /// A reader for scans of at most `block` nodes in batches of about `batch` entries.
    ///
    /// # Errors
    ///
    /// [`Error::Memory`] if its buffers do not fit in the budget.
    fn reader(&self, memory: &Memory, batch: usize, block: u32) -> Result<Self::Reader, Error>;

    /// Calls `f(v, list)` for each node `v` of `nodes` with `filter(v)`, in order,
    /// decoding or reading a batch of consecutive lists before the calls on it, never
    /// list by list (#118). Nothing is decoded or read when `filter` passes no node.
    ///
    /// # Errors
    ///
    /// The lists could not be read, or are corrupt.
    fn scan(
        &self,
        reader: &mut Self::Reader,
        nodes: Range<u32>,
        filter: impl Fn(u32) -> bool,
        f: impl FnMut(u32, &[u32]),
    ) -> Result<(), Error>;

    /// Calls `f(v, list)` for each node of `nodes`, which is increasing. Lists of
    /// consecutive nodes are decoded or read together.
    ///
    /// # Errors
    ///
    /// The lists could not be read, or are corrupt.
    fn lists(
        &self,
        reader: &mut Self::Reader,
        nodes: &[u32],
        f: impl FnMut(u32, &[u32]),
    ) -> Result<(), Error>;

    /// Tells the backend that the lists of `nodes` (increasing) will be read soon. Advice
    /// only; the default ignores it.
    fn prefetch(&self, nodes: &[u32]) {
        let _ = nodes;
    }
}

/// Entries per task when a source checks its targets in parallel.
const CHECK_GRAIN: NonZeroUsize = NonZeroUsize::new(1 << 16).expect("non-zero");

/// The largest target of `targets` is below `n`.
fn in_range(targets: &[u32], n: u32) -> bool {
    targets.iter().fold(0, |m, &t| m.max(t)) < n || targets.is_empty()
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

fn degree_of(offsets: &[u64], v: u32) -> u32 {
    let v = ix(v.into());
    u32::try_from(offsets[v + 1] - offsets[v]).expect("checked when made")
}

/// A graph as raw CSR arrays in memory: `offsets` (`n + 1`) and `targets`.
#[derive(Clone, Copy, Debug)]
pub struct Csr<'a> {
    csr: CheckedCsr<'a>,
    offsets: &'a [u64],
    n: u32,
}

impl<'a> Csr<'a> {
    /// Checks the arrays (offsets as a CSR over `targets`, every target below `n`),
    /// in parallel, and wraps them.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] or [`Error::Target`] naming the first problem found.
    pub fn new(offsets: &'a [u64], targets: &'a [u32]) -> Result<Self, Error> {
        let (csr, n) = check_csr(offsets, targets)?;
        let ok = par::map_ranges(targets.len(), CHECK_GRAIN, |r| in_range(&targets[r], n));
        if !ok.into_iter().all(|x| x) {
            let lists = (0..n).map(|v| {
                let v = ix(v.into());
                &targets[ix(offsets[v])..ix(offsets[v + 1])]
            });
            return Err(bad_target(0, lists, n));
        }
        Ok(Self { csr, offsets, n })
    }
}

// SAFETY: every target is below `n`, checked in `new`; the arrays are borrowed, so they
// cannot change.
unsafe impl Source for Csr<'_> {
    type Reader = ();

    fn nodes(&self) -> u32 {
        self.n
    }

    fn entries(&self) -> u64 {
        self.csr.targets().len() as u64
    }

    fn entry_offset(&self, v: u32) -> u64 {
        self.offsets[ix(v.into())]
    }

    fn degree(&self, v: u32) -> u32 {
        degree_of(self.offsets, v)
    }

    #[inline]
    fn degrees(&self, nodes: Range<u32>, mut f: impl FnMut(u32, u32)) {
        for v in nodes {
            f(v, degree_of(self.offsets, v));
        }
    }

    fn reader(&self, _: &Memory, _: usize, _: u32) -> Result<(), Error> {
        Ok(())
    }

    #[inline]
    fn scan(
        &self,
        (): &mut (),
        nodes: Range<u32>,
        filter: impl Fn(u32) -> bool,
        mut f: impl FnMut(u32, &[u32]),
    ) -> Result<(), Error> {
        for v in nodes.filter(|&v| filter(v)) {
            f(v, csr_list(&self.csr, v));
        }
        Ok(())
    }

    fn lists(
        &self,
        (): &mut (),
        nodes: &[u32],
        mut f: impl FnMut(u32, &[u32]),
    ) -> Result<(), Error> {
        for &v in nodes {
            f(v, csr_list(&self.csr, v));
        }
        Ok(())
    }
}

/// Buffers held for one worker, with their reservation.
#[derive(Debug)]
struct Held {
    reservation: Reservation,
    /// Entries the reservation covers.
    entries: usize,
}

impl Held {
    fn new(memory: &Memory, purpose: &'static str, fixed: u64) -> Result<Self, Error> {
        Ok(Self {
            reservation: memory.reserve(purpose, MemoryKind::Anonymous, fixed)?,
            entries: 0,
        })
    }

    /// Makes sure the reservation covers buffers of `entries` entries of `bytes` each.
    fn ensure(&mut self, entries: usize, bytes: u64) -> Result<(), Error> {
        if entries > self.entries {
            let more = (entries - self.entries) as u64 * bytes;
            self.reservation.try_grow(more)?;
            self.entries = entries;
        }
        Ok(())
    }
}

/// The reader of a [`Compressed`] source: one decode batch.
#[derive(Debug)]
pub struct DecodeReader {
    batch: Batch,
    max_entries: usize,
    held: Held,
}

/// Bytes a decode batch holds per entry: the target, and its slack of three for the
/// SIMD decoders counted in [`ensure`](Held::ensure)'s caller.
const DECODED: u64 = 4;

/// A graph in the compressed adjacency format (DESIGN D4), its data part in memory:
/// a heap buffer, or a memory-mapped file ([`with_hints`](Self::with_hints)).
#[derive(Clone, Copy)]
pub struct Compressed<'a> {
    adjacency: Adjacency<'a>,
    n: u32,
    /// Where the data part comes from, for prefetch advice.
    hints: Option<&'a (dyn ByteSource + Sync)>,
}

impl std::fmt::Debug for Compressed<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Compressed")
            .field("adjacency", &self.adjacency)
            .field("hints", &self.hints.is_some())
            .finish()
    }
}

impl<'a> Compressed<'a> {
    /// Wraps an adjacency. Lists are checked as they are decoded.
    #[must_use]
    pub fn new(adjacency: Adjacency<'a>) -> Self {
        Self {
            n: adjacency.index().nodes(),
            adjacency,
            hints: None,
        }
    }

    /// The same source, passing prefetch advice for the data part on to `source`, the
    /// [`ByteSource`] whose bytes it is (a memory map, typically).
    #[must_use]
    pub fn with_hints(self, source: &'a (dyn ByteSource + Sync)) -> Self {
        Self {
            hints: Some(source),
            ..self
        }
    }

    /// Decodes from `nodes.start`, as far as `reader`'s batch allows; returns where it
    /// stopped.
    fn decode(&self, r: &mut DecodeReader, nodes: Range<u32>) -> Result<u32, Error> {
        // What the codec's batch grows to: the batch, or the first list if longer, but
        // no more than the range holds; and its SIMD slack of three.
        let first = ix(self.degree(nodes.start).into());
        let span = ix(self.entry_offset(nodes.end) - self.entry_offset(nodes.start));
        r.held
            .ensure(r.max_entries.max(first).min(span) + 3, DECODED)?;
        let end = self
            .adjacency
            .decode_nodes(nodes, r.max_entries, &mut r.batch)?;
        if !in_range(r.batch.targets(), self.n) {
            let lists = (0..r.batch.len()).map(|i| r.batch.list(i));
            return Err(bad_target(r.batch.first(), lists, self.n));
        }
        Ok(end)
    }
}

// SAFETY: every decoded batch is checked to name only nodes below `n` before it is
// handed out (`decode`); the index is borrowed, so `n` cannot change.
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

    fn reader(&self, memory: &Memory, batch: usize, block: u32) -> Result<DecodeReader, Error> {
        // The batch's list ends: two words per list, at most `block` lists a scan.
        let ends = u64::from(block) * 2 * size_of::<usize>() as u64;
        Ok(DecodeReader {
            batch: Batch::default(),
            max_entries: batch,
            held: Held::new(memory, "edge-map decode batch", ends)?,
        })
    }

    #[inline]
    fn scan(
        &self,
        r: &mut DecodeReader,
        nodes: Range<u32>,
        filter: impl Fn(u32) -> bool,
        mut f: impl FnMut(u32, &[u32]),
    ) -> Result<(), Error> {
        if !nodes.clone().any(&filter) {
            return Ok(());
        }
        let mut at = nodes.start;
        while at < nodes.end {
            let next = self.decode(r, at..nodes.end)?;
            for (v, list) in r.batch.iter() {
                if filter(v.get()) {
                    f(v.get(), list);
                }
            }
            at = next;
        }
        Ok(())
    }

    fn lists(
        &self,
        r: &mut DecodeReader,
        nodes: &[u32],
        mut f: impl FnMut(u32, &[u32]),
    ) -> Result<(), Error> {
        let mut i = 0;
        while i < nodes.len() {
            // A run of consecutive nodes is one decode.
            let start = nodes[i];
            let mut end = i + 1;
            while end < nodes.len() && nodes[end] == nodes[end - 1] + 1 {
                end += 1;
            }
            let stop = self.decode(r, start..nodes[end - 1] + 1)?;
            for (v, list) in r.batch.iter() {
                f(v.get(), list);
            }
            i += ix(u64::from(stop - start));
        }
        Ok(())
    }

    fn prefetch(&self, nodes: &[u32]) {
        if let Some(source) = self.hints {
            let index = self.adjacency.index();
            advise_runs(source, nodes, |v| index.record_range(NodeId::new(v)));
        }
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

/// The reader of a [`Stream`] source: the bytes read, and the lists they hold.
#[derive(Debug)]
pub struct StreamReader {
    bytes: Vec<u8>,
    lists: Vec<u32>,
    max_entries: usize,
    held: Held,
}

/// Bytes a stream reader holds per entry: four read, four converted.
const STREAMED: u64 = 8;

/// A raw CSR whose offsets are in memory and whose targets (little-endian `u32`, the
/// `targets.u32` file of a CSR directory) are read through a [`ByteSource`], a batch at a
/// time: the semi-external regime of the lab (FINDINGS § B1), O(n) state in memory.
/// The streaming reader of the compressed format is #46.
#[derive(Clone, Copy)]
pub struct Stream<'a> {
    offsets: &'a [u64],
    targets: &'a (dyn ByteSource + Sync),
    n: u32,
}

impl std::fmt::Debug for Stream<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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

    /// Reads the lists of `nodes` (whole lists, in order) into the reader.
    fn read(&self, r: &mut StreamReader, nodes: Range<u32>) -> Result<(), Error> {
        let (a, b) = (
            self.offsets[ix(nodes.start.into())],
            self.offsets[ix(nodes.end.into())],
        );
        let len = ix(b - a);
        r.held.ensure(len, STREAMED)?;
        // Sized by `extend`, not `resize`: the reservation above covers it.
        r.bytes.clear();
        r.bytes.extend(std::iter::repeat_n(0, len * 4));
        self.targets.read_at(a * 4, &mut r.bytes)?;
        r.lists.clear();
        r.lists.extend(
            r.bytes
                .chunks_exact(4)
                .map(|c| u32::from_le_bytes(c.try_into().expect("four bytes"))),
        );
        if !in_range(&r.lists, self.n) {
            let lists = nodes.clone().map(|v| self.slice(r, nodes.start, v));
            return Err(bad_target(nodes.start, lists, self.n));
        }
        Ok(())
    }

    /// The list of `v` among lists read from `first` on.
    fn slice<'r>(&self, r: &'r StreamReader, first: u32, v: u32) -> &'r [u32] {
        let base = self.offsets[ix(first.into())];
        let v = ix(v.into());
        &r.lists[ix(self.offsets[v] - base)..ix(self.offsets[v + 1] - base)]
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

// SAFETY: every list read is checked to name only nodes below `n` before it is handed
// out (`read`); the offsets are borrowed, so `n` cannot change.
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

    fn degree(&self, v: u32) -> u32 {
        degree_of(self.offsets, v)
    }

    fn degrees(&self, nodes: Range<u32>, mut f: impl FnMut(u32, u32)) {
        for v in nodes {
            f(v, degree_of(self.offsets, v));
        }
    }

    fn reader(&self, memory: &Memory, batch: usize, _: u32) -> Result<StreamReader, Error> {
        Ok(StreamReader {
            bytes: Vec::new(),
            lists: Vec::new(),
            max_entries: batch,
            held: Held::new(memory, "edge-map stream buffer", 0)?,
        })
    }

    fn scan(
        &self,
        r: &mut StreamReader,
        nodes: Range<u32>,
        filter: impl Fn(u32) -> bool,
        mut f: impl FnMut(u32, &[u32]),
    ) -> Result<(), Error> {
        if !nodes.clone().any(&filter) {
            return Ok(());
        }
        let mut at = nodes.start;
        while at < nodes.end {
            let stop = self.batch_end(at, nodes.end, r.max_entries);
            self.read(r, at..stop)?;
            for v in (at..stop).filter(|&v| filter(v)) {
                f(v, self.slice(r, at, v));
            }
            at = stop;
        }
        Ok(())
    }

    fn lists(
        &self,
        r: &mut StreamReader,
        nodes: &[u32],
        mut f: impl FnMut(u32, &[u32]),
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
                f(v, self.slice(r, start, v));
            }
            i = end;
        }
        Ok(())
    }

    fn prefetch(&self, nodes: &[u32]) {
        advise_runs(self.targets, nodes, |v| {
            let v = ix(v.into());
            self.offsets[v] * 4..self.offsets[v + 1] * 4
        });
    }
}
