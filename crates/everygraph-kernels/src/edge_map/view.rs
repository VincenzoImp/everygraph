// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A filtered view of a graph as a [`Source`] (#93, ADR 0010): a mask over the parent's
//! lists, or the same view materialized as a [`RawCopy`].
//!
//! A view is made from two optional bitsets in the parent's ids: the nodes it may keep,
//! and the parent's entries (list positions) it may keep. It keeps an entry `u → t` when
//! both bits allow it and both `u` and `t` are kept. With no entry bitset it keeps every
//! node the node bitset allows: the induced subgraph. With an entry bitset it keeps only
//! the allowed nodes that still have a kept entry, in either direction
//! (`design/DECISIONS.md`, "Nodes in a view").
//!
//! Both forms hand the kernels the same lists: the view's nodes are numbered densely in
//! parent order, each list keeps its parent order, and the first entry of a list counts
//! the view's entries before it. So a kernel sees one graph whichever form runs, and the
//! reduction blocks, which the framework cuts by node count, are the same blocks: a
//! deterministic kernel gives the same bits either way (DESIGN D14).

use std::fmt;
use std::num::NonZeroUsize;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use everygraph_core::{Memory, MemoryKind, Reservation, SegVec, par};

use super::source::in_range;
use super::{Error, Plan, RawCopy, Settings, Source, ix, one_segment_mut};

/// Entries a task of [`View::gather`] and [`RawCopy::copy`] reads, about.
pub(super) const CHUNK_ENTRIES: u64 = 1 << 16;
/// Entries per decoded or read batch of those tasks.
const BATCH: usize = 8192;
/// Lists per reader call of those tasks, as a reader's first size.
const LISTS: usize = 1024;
const GRAIN: NonZeroUsize = NonZeroUsize::new(1 << 12).expect("non-zero");

const SEGMENT: Error = Error::Invalid("view state longer than one segment (#m3-15)");

/// Whether bit `i` of `bits` is set.
#[inline]
fn bit(bits: &[u64], i: u64) -> bool {
    bits[ix(i >> 6)] >> (i & 63) & 1 == 1
}

/// The kept nodes of a view that does not keep every node of its parent.
#[derive(Debug)]
struct Nodes {
    /// Bit `p`: parent node `p` is kept.
    bits: SegVec<u64>,
    /// The kept nodes before each word of `bits`: the rank directory.
    before: SegVec<u32>,
    /// The parent id of each node of the view, increasing.
    parents: SegVec<u32>,
}

impl Nodes {
    fn bits(&self) -> &[u64] {
        self.bits.as_slice().unwrap_or_default()
    }

    fn parents(&self) -> &[u32] {
        self.parents.as_slice().unwrap_or_default()
    }

    /// Whether parent node `p` is kept.
    #[inline]
    fn has(&self, p: u32) -> bool {
        bit(self.bits(), p.into())
    }

    /// The view id of kept parent node `p`: the kept nodes before it.
    #[inline]
    fn rank(&self, p: u32) -> u32 {
        let w = ix((p >> 6).into());
        let below = self.bits()[w] & ((1 << (p & 63)) - 1);
        self.before.as_slice().unwrap_or_default()[w] + below.count_ones()
    }

    fn bytes(&self) -> u64 {
        self.bits.reserved_bytes() + self.before.reserved_bytes() + self.parents.reserved_bytes()
    }
}

/// A filtered view of `parent` (module documentation). It runs as a mask over the
/// parent's lists until [`materialize`](Self::materialize) copies its lists into a
/// [`RawCopy`]; [`release`](Self::release) drops the copy.
pub struct View<'s, S: Source> {
    parent: &'s S,
    /// `None` when every parent node is kept: the view's ids are the parent's.
    nodes: Option<Nodes>,
    /// Bit `e`: the parent's entry `e` may be kept. `None`: every entry may.
    entries: Option<SegVec<u64>>,
    /// `n + 1` entry offsets of the view.
    offsets: SegVec<u64>,
    n: u32,
    /// The materialized form: the view's own lists, with the same `n` and offsets.
    raw: Option<RawCopy>,
}

impl<S: Source> fmt::Debug for View<'_, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("View")
            .field("nodes", &self.n)
            .field("entries", &self.entries())
            .field("materialized", &self.raw.is_some())
            .finish_non_exhaustive()
    }
}

/// A bitset of `bits` bits, all clear, reserved against `memory`.
///
/// # Errors
///
/// [`Error::Memory`] if it does not fit; [`Error::Invalid`] beyond one segment.
pub(super) fn empty_bits(
    memory: &Memory,
    purpose: &'static str,
    bits: u64,
) -> Result<SegVec<u64>, Error> {
    let words = ix(bits.div_ceil(64));
    let set = SegVec::try_with_len(memory, purpose, words, 0)?;
    set.as_slice().map(|_| ()).ok_or(SEGMENT)?;
    Ok(set)
}

/// The words of `set`, if any, as one slice.
fn allowed(set: Option<&SegVec<u64>>) -> Result<Option<&[u64]>, Error> {
    set.map(|s| s.as_slice().ok_or(SEGMENT)).transpose()
}

impl<'s, S: Source> View<'s, S> {
    /// The view of `parent` that `nodes` (bit `p`: parent node `p` may be kept) and
    /// `entries` (bit `e`: the parent's entry `e` may be kept) select, made by one pass
    /// over the parent's lists in `memory`. The pass counts the view's nodes and edges;
    /// a bitset that is `None` allows everything.
    ///
    /// # Errors
    ///
    /// [`Error::Argument`] for a bitset with fewer bits than nodes or entries;
    /// [`Error::Memory`] if the view's state does not fit; the parent's errors.
    pub fn new(
        parent: &'s S,
        memory: &Memory,
        settings: Settings,
        nodes: Option<SegVec<u64>>,
        entries: Option<SegVec<u64>>,
    ) -> Result<Self, Error> {
        let n = parent.nodes();
        let words = |bits: u64| bits.div_ceil(64);
        let short = |set: &Option<SegVec<u64>>, bits| {
            set.as_ref().is_some_and(|s| (s.len() as u64) < words(bits))
        };
        if short(&nodes, n.into()) {
            return Err(Error::Argument("the node bitset has fewer bits than nodes"));
        }
        if short(&entries, parent.entries()) {
            return Err(Error::Argument(
                "the entry bitset has fewer bits than entries",
            ));
        }
        let (allow_nodes, allow_entries) = (allowed(nodes.as_ref())?, allowed(entries.as_ref())?);

        // One pass: each allowed node's kept entries, and which nodes have one.
        let mut degrees = SegVec::try_with_len(memory, "view degrees", ix(n.into()), 0_u32)?;
        let touched = match allow_entries {
            Some(_) => Some(Touched::new(memory, n)?),
            None => None,
        };
        Plan::with(parent, memory, settings, |plan| {
            let node_ok = |p: u32| allow_nodes.is_none_or(|b| bit(b, p.into()));
            plan.scan(
                one_segment_mut(&mut degrees)?,
                |v| node_ok(v.get()),
                |(): &mut (), out, i, row, _| {
                    let mut kept = 0_u32;
                    for (e, &t) in (row.first_entry..).zip(row.list) {
                        let t = t.get();
                        if allow_entries.is_none_or(|b| bit(b, e)) && node_ok(t) {
                            kept += 1;
                            if let Some(touched) = &touched {
                                touched.mark(t);
                            }
                        }
                    }
                    if kept > 0 {
                        if let Some(touched) = &touched {
                            touched.mark(row.node.get());
                        }
                    }
                    out[i] = kept;
                },
            )
        })?;

        // The kept nodes: the allowed ones, and with an entry bitset only those touched.
        let mut keep = empty_bits(memory, "view nodes", n.into())?;
        let all_words = one_segment_mut(&mut keep)?;
        par::for_each_chunk_mut(all_words, GRAIN, |first, chunk| {
            for (w, word) in (first..).zip(chunk) {
                let tail = u64::from(n) - 64 * w as u64;
                let valid = if tail >= 64 {
                    u64::MAX
                } else {
                    (1 << tail) - 1
                };
                let allowed = allow_nodes.map_or(u64::MAX, |b| b[w]);
                let touched = touched.as_ref().map_or(u64::MAX, |t| t.word(w));
                *word = allowed & touched & valid;
            }
        });
        drop((nodes, touched));
        let kept_bits = keep.as_slice().ok_or(SEGMENT)?;
        let count: u64 = kept_bits.iter().map(|w| u64::from(w.count_ones())).sum();
        let view_n = u32::try_from(count).map_err(|_| SEGMENT)?;
        let nodes = if view_n == n {
            drop(keep);
            None
        } else {
            Some(Nodes::new(memory, keep, view_n)?)
        };

        let degrees = degrees.as_slice().ok_or(SEGMENT)?;
        let mut offsets = SegVec::try_with_len(memory, "view offsets", ix(view_n.into()) + 1, 0)?;
        let at = one_segment_mut(&mut offsets)?;
        let mut sum = 0;
        for v in 0..view_n {
            let p = nodes.as_ref().map_or(v, |s| s.parents()[ix(v.into())]);
            sum += u64::from(degrees[ix(p.into())]);
            at[ix(v.into()) + 1] = sum;
        }
        Ok(Self {
            parent,
            nodes,
            entries,
            offsets,
            n: view_n,
            raw: None,
        })
    }

    /// The graph the view filters.
    #[must_use]
    pub fn parent(&self) -> &'s S {
        self.parent
    }

    /// Entries of the view (the sum of its degrees).
    #[must_use]
    pub fn entries(&self) -> u64 {
        self.at()[ix(self.n.into())]
    }

    /// The parent's id of view node `v`.
    ///
    /// # Panics
    ///
    /// If `v` is not a node of the view.
    #[must_use]
    pub fn parent_id(&self, v: u32) -> u32 {
        assert!(v < self.n, "node {v} outside the view");
        self.nodes.as_ref().map_or(v, |s| s.parents()[ix(v.into())])
    }

    /// The view's id of parent node `p`, if the view keeps it.
    #[must_use]
    pub fn view_id(&self, p: u32) -> Option<u32> {
        match &self.nodes {
            _ if p >= self.parent.nodes() => None,
            None => Some(p),
            Some(s) => s.has(p).then(|| s.rank(p)),
        }
    }

    /// Bytes the mask holds of its budget: the node and entry bitsets, the rank
    /// directory, the map to parent ids and the offsets.
    #[must_use]
    pub fn mask_bytes(&self) -> u64 {
        self.nodes.as_ref().map_or(0, Nodes::bytes)
            + self.entries.as_ref().map_or(0, SegVec::reserved_bytes)
            + self.offsets.reserved_bytes()
    }

    /// Bytes the materialized lists hold of their budget; 0 for a mask.
    #[must_use]
    pub fn materialized_bytes(&self) -> u64 {
        self.raw.as_ref().map_or(0, RawCopy::held)
    }

    /// Whether the view runs on its own lists.
    #[must_use]
    pub fn is_materialized(&self) -> bool {
        self.raw.is_some()
    }

    /// Copies the view's lists into a [`RawCopy`] in `memory`: from then on the view
    /// reads its own lists. The mask stays, for [`gather`](Self::gather).
    ///
    /// # Errors
    ///
    /// As [`RawCopy::copy`].
    pub fn materialize(&mut self, memory: &Memory) -> Result<(), Error> {
        if self.raw.is_none() {
            let raw = RawCopy::copy(&*self, memory)?;
            debug_assert_eq!(raw.nodes(), self.n);
            self.raw = Some(raw);
        }
        Ok(())
    }

    /// Drops the materialized lists: the view runs as a mask again.
    pub fn release(&mut self) {
        self.raw = None;
    }

    /// One value per entry of the view, in the view's entry order: `value(e)` of the
    /// parent's entry `e` that each entry of the view is. A per-entry column of the
    /// parent becomes the view's with `|e| column[e]`.
    ///
    /// # Errors
    ///
    /// [`Error::Memory`] if the column does not fit; [`Error::Invalid`] beyond one segment;
    /// the parent's errors.
    pub fn gather<T>(
        &self,
        memory: &Memory,
        purpose: &'static str,
        value: impl Fn(u64) -> T + Sync,
    ) -> Result<SegVec<T>, Error>
    where
        T: Copy + Default + Send + 'static,
    {
        let mut out = SegVec::try_with_len(memory, purpose, ix(self.entries()), T::default())?;
        let at = self.at();
        by_chunks(one_segment_mut(&mut out)?, at, 0..self.n, |nodes, slice| {
            let mut reader = self.parent.reader(memory, BATCH, LISTS)?;
            let base = at[ix(nodes.start.into())];
            let mut k = 0;
            let mut each = |first: u64, list: &[u32]| {
                self.keep(first, list, |e, _| {
                    if let Some(slot) = slice.get_mut(k) {
                        *slot = value(e);
                    }
                    k += 1;
                });
            };
            match &self.nodes {
                None => {
                    self.parent
                        .scan(&mut reader, nodes.clone(), |_| true, |_, f, l| each(f, l))?;
                }
                Some(s) => {
                    let parents = &s.parents()[ix(nodes.start.into())..ix(nodes.end.into())];
                    self.parent
                        .lists(&mut reader, parents, |_, f, l| each(f, l))?;
                }
            }
            if base + k as u64 == at[ix(nodes.end.into())] {
                Ok(())
            } else {
                Err(Error::Invalid("the parent's lists changed under the view"))
            }
        })?;
        Ok(out)
    }

    fn at(&self) -> &[u64] {
        self.offsets.as_slice().unwrap_or_default()
    }

    /// Calls `f(e, t)` for each kept entry of a parent list whose first entry is
    /// `first`: its parent entry and its target's view id, in list order.
    #[inline]
    fn keep(&self, first: u64, list: &[u32], mut f: impl FnMut(u64, u32)) {
        let entries = (first..).zip(list);
        match (
            self.entries.as_ref().and_then(SegVec::as_slice),
            &self.nodes,
        ) {
            (None, None) => entries.for_each(|(e, &t)| f(e, t)),
            (Some(b), None) => entries
                .filter(|(e, _)| bit(b, *e))
                .for_each(|(e, &t)| f(e, t)),
            (None, Some(s)) => entries
                .filter(|(_, t)| s.has(**t))
                .for_each(|(e, &t)| f(e, s.rank(t))),
            (Some(b), Some(s)) => entries
                .filter(|(e, t)| bit(b, *e) && s.has(**t))
                .for_each(|(e, &t)| f(e, s.rank(t))),
        }
    }

    /// The kept entries of a parent list, as view ids, written into `buf` and checked
    /// to name nodes of the view.
    fn filter<'b>(
        &self,
        first: u64,
        list: &[u32],
        buf: &'b mut SegVec<u32>,
    ) -> Result<&'b [u32], Error> {
        if buf.len() < list.len() {
            buf.try_resize(list.len(), 0)?;
        }
        let out = buf.as_mut_slice().ok_or(SEGMENT)?;
        let mut k = 0;
        self.keep(first, list, |_, t| {
            out[k] = t;
            k += 1;
        });
        let kept = &out[..k];
        if in_range(kept, self.n) {
            Ok(kept)
        } else {
            Err(Error::Invalid("a view list names a node outside the view"))
        }
    }
}

impl Nodes {
    /// The kept nodes `bits`, `count` of them: their rank directory and parent ids.
    fn new(memory: &Memory, bits: SegVec<u64>, count: u32) -> Result<Self, Error> {
        let words = bits.as_slice().ok_or(SEGMENT)?;
        let mut before = SegVec::try_with_len(memory, "view rank directory", words.len(), 0_u32)?;
        let mut parents = SegVec::try_with_len(memory, "view parent ids", ix(count.into()), 0_u32)?;
        let (b, p) = (
            one_segment_mut(&mut before)?,
            one_segment_mut(&mut parents)?,
        );
        let mut seen = 0_u32;
        for (w, &word) in words.iter().enumerate() {
            b[w] = seen;
            let mut x = word;
            while x != 0 {
                let id = u32::try_from(64 * w as u64 + u64::from(x.trailing_zeros()))
                    .expect("a node id");
                p[ix(seen.into())] = id;
                seen += 1;
                x &= x - 1;
            }
        }
        Ok(Self {
            bits,
            before,
            parents,
        })
    }
}

/// Nodes that have a kept entry, set from many tasks at once.
struct Touched {
    words: Vec<AtomicU64>,
    _held: Reservation,
}

impl Touched {
    fn new(memory: &Memory, n: u32) -> Result<Self, Error> {
        let len = ix(u64::from(n).div_ceil(64));
        let held = memory.reserve("view touched nodes", MemoryKind::Anonymous, 8 * len as u64)?;
        Ok(Self {
            words: par::collect(len, GRAIN, |_| AtomicU64::new(0)),
            _held: held,
        })
    }

    #[inline]
    fn mark(&self, v: u32) {
        let (w, b) = (&self.words[ix((v >> 6).into())], 1_u64 << (v & 63));
        // A load first: a hub's word is read by every task, written once.
        if w.load(Ordering::Relaxed) & b == 0 {
            w.fetch_or(b, Ordering::Relaxed);
        }
    }

    fn word(&self, w: usize) -> u64 {
        self.words[w].load(Ordering::Relaxed)
    }
}

/// Cuts `nodes` into tasks of about [`CHUNK_ENTRIES`] entries by the offsets `at`, and
/// runs `work(task, rows)` on each, possibly in parallel, where `rows` is the part of
/// `out` (the rows `at[nodes.start]..at[nodes.end]`) that holds the task's entries.
/// Returns the error of the first task that failed.
pub(super) fn by_chunks<T: Send>(
    out: &mut [T],
    at: &[u64],
    nodes: Range<u32>,
    work: impl Fn(Range<u32>, &mut [T]) -> Result<(), Error> + Sync,
) -> Result<(), Error> {
    let base = at[ix(nodes.start.into())];
    let mut cuts = vec![nodes.start];
    let mut v = nodes.start;
    while v < nodes.end {
        let limit = at[ix(v.into())].saturating_add(CHUNK_ENTRIES);
        let end = at[ix(v.into())..=ix(nodes.end.into())].partition_point(|&o| o <= limit);
        v = (v + u32::try_from(end).unwrap_or(u32::MAX).saturating_sub(1)).clamp(v + 1, nodes.end);
        cuts.push(v);
    }
    let rows: Vec<u64> = cuts.iter().map(|&c| at[ix(c.into())] - base).collect();
    let failed = Mutex::new(None);
    par::for_each_range_mut(out, &rows, NonZeroUsize::MIN, |i, slice| {
        if let Err(e) = work(cuts[i]..cuts[i + 1], slice) {
            let mut first = failed.lock().unwrap_or_else(PoisonError::into_inner);
            if first.as_ref().is_none_or(|(j, _)| i < *j) {
                *first = Some((i, e));
            }
        }
    });
    match failed.into_inner().unwrap_or_else(PoisonError::into_inner) {
        Some((_, e)) => Err(e),
        None => Ok(()),
    }
}

/// The reader of a [`View`]: the parent's reader for a mask, and the buffers of the
/// view's lists and of the parent ids asked for.
pub struct ViewReader<R> {
    inner: Option<R>,
    list: SegVec<u32>,
    nodes: SegVec<u32>,
    max_entries: usize,
}

impl<R> fmt::Debug for ViewReader<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ViewReader")
            .field("mask", &self.inner.is_some())
            .field("list", &self.list.len())
            .finish_non_exhaustive()
    }
}

const FORM: Error = Error::Invalid("a reader of a view in another form");

// SAFETY: `n` is fixed at `new` and the fields are private. A materialized view hands
// out the lists of its `RawCopy`, made from this view with the same `n` (its own `Source`
// guarantee). A mask hands out only a prefix of the reader's `list` buffer, which
// `filter` checks to be below `n` after writing it, in the same call. `scan` and `lists`
// call `f` only with nodes they were given: the parent's ids when every parent node is
// kept (then `n` is the parent's), or, for a run of kept nodes, the rank of a parent id
// of that run, which is checked to lie in the run.
unsafe impl<S: Source> Source for View<'_, S> {
    type Reader = ViewReader<S::Reader>;

    fn nodes(&self) -> u32 {
        self.n
    }

    fn entries(&self) -> u64 {
        View::entries(self)
    }

    fn entry_offset(&self, v: u32) -> u64 {
        self.at()[ix(v.into())]
    }

    fn reader(&self, memory: &Memory, batch: usize, lists: usize) -> Result<Self::Reader, Error> {
        let inner = match self.raw {
            Some(_) => None,
            None => Some(self.parent.reader(memory, batch, lists)?),
        };
        Ok(ViewReader {
            inner,
            list: SegVec::new(memory, "view list"),
            nodes: SegVec::new(memory, "view parent nodes"),
            max_entries: batch,
        })
    }

    fn begin_task(&self, r: &mut Self::Reader, nodes: Range<u32>) {
        if let (Some(inner), false) = (r.inner.as_mut(), nodes.is_empty()) {
            let parents = match &self.nodes {
                None => nodes,
                Some(s) => {
                    let p = s.parents();
                    p[ix(nodes.start.into())]..p[ix((nodes.end - 1).into())] + 1
                }
            };
            self.parent.begin_task(inner, parents);
        }
    }

    fn scan(
        &self,
        r: &mut Self::Reader,
        nodes: Range<u32>,
        filter: impl Fn(u32) -> bool,
        mut f: impl FnMut(u32, u64, &[u32]),
    ) -> Result<(), Error> {
        if let Some(raw) = &self.raw {
            return raw.scan(&mut (), nodes, filter, f);
        }
        let ViewReader { inner, list, .. } = r;
        let inner = inner.as_mut().ok_or(FORM)?;
        let mut failed = None;
        match &self.nodes {
            None => self.parent.scan(inner, nodes, &filter, |v, first, l| {
                match self.filter(first, l, list) {
                    Ok(kept) => f(v, self.at()[ix(v.into())], kept),
                    Err(e) => {
                        failed.get_or_insert(e);
                    }
                }
            })?,
            Some(s) => {
                let mut v = nodes.start;
                while v < nodes.end {
                    if !filter(v) {
                        v += 1;
                        continue;
                    }
                    let start = v;
                    v += 1;
                    while v < nodes.end && filter(v) {
                        v += 1;
                    }
                    let run = start..v;
                    let parents = &s.parents()[ix(start.into())..ix(v.into())];
                    self.parent.lists(inner, parents, |p, first, l| {
                        let w = s.rank(p);
                        match self.filter(first, l, list) {
                            Ok(kept) if run.contains(&w) => f(w, self.at()[ix(w.into())], kept),
                            Ok(_) => {
                                failed.get_or_insert(FORM);
                            }
                            Err(e) => {
                                failed.get_or_insert(e);
                            }
                        }
                    })?;
                }
            }
        }
        failed.map_or(Ok(()), Err)
    }

    fn lists(
        &self,
        r: &mut Self::Reader,
        nodes: &[u32],
        mut f: impl FnMut(u32, u64, &[u32]),
    ) -> Result<(), Error> {
        if let Some(raw) = &self.raw {
            return raw.lists(&mut (), nodes, f);
        }
        let ViewReader {
            inner,
            list,
            nodes: asked,
            ..
        } = r;
        let inner = inner.as_mut().ok_or(FORM)?;
        let mut failed = None;
        match &self.nodes {
            None => self.parent.lists(inner, nodes, |v, first, l| {
                match self.filter(first, l, list) {
                    Ok(kept) => f(v, self.at()[ix(v.into())], kept),
                    Err(e) => {
                        failed.get_or_insert(e);
                    }
                }
            })?,
            Some(s) => {
                asked.try_resize(nodes.len(), 0)?;
                let parents = asked.as_mut_slice().ok_or(SEGMENT)?;
                for (p, &v) in parents.iter_mut().zip(nodes) {
                    *p = self.parent_id(v);
                }
                self.parent.lists(inner, parents, |p, first, l| {
                    let w = s.rank(p);
                    match self.filter(first, l, list) {
                        Ok(kept) if nodes.binary_search(&w).is_ok() => {
                            f(w, self.at()[ix(w.into())], kept);
                        }
                        Ok(_) => {
                            failed.get_or_insert(FORM);
                        }
                        Err(e) => {
                            failed.get_or_insert(e);
                        }
                    }
                })?;
            }
        }
        failed.map_or(Ok(()), Err)
    }

    fn list<'r>(&'r self, r: &'r mut Self::Reader, v: u32) -> Result<(u64, &'r [u32]), Error> {
        if let Some(raw) = &self.raw {
            assert!(v < self.n, "node {v} outside the view");
            return Ok(raw.get_in(raw.part_of(v), v));
        }
        let p = self.parent_id(v);
        let ViewReader { inner, list, .. } = r;
        let inner = inner.as_mut().ok_or(FORM)?;
        let (first, l) = self.parent.list(inner, p)?;
        Ok((self.at()[ix(v.into())], self.filter(first, l, list)?))
    }

    fn prefetch(&self, nodes: &[u32]) {
        if self.raw.is_none() && self.nodes.is_none() {
            self.parent.prefetch(nodes);
        }
    }

    fn idle(&self, r: &mut Self::Reader) {
        if let Some(inner) = r.inner.as_mut() {
            self.parent.idle(inner);
        }
        let usual = r.max_entries.saturating_add(8);
        if r.list.capacity() > usual {
            r.list.truncate(usual);
            r.list.shrink_to_fit();
        }
        r.nodes.clear();
        r.nodes.shrink_to_fit();
    }
}
