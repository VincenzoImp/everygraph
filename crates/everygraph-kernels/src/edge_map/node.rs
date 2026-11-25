// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Node ids that are known to be in range, and the per-node state they index without
//! bounds checks: [`Node`], [`PerNode`], [`NodeCells`], [`NodeSet`], [`NodeList`].
//!
//! # Why `unsafe`
//!
//! Every list a [`Source`](super::Source) hands out has been checked to name only nodes
//! below the graph's node count (#44: a corrupt file must not make a kernel misbehave).
//! A kernel then indexes its per-node arrays with those targets millions of times, and
//! the compiler cannot know they are in range: removing the bounds checks took WCC from
//! 1.36× to 1.03× of the lab's hand-written loop at one thread and BFS from 1.16× to
//! 0.99× (orkut, raw arrays in the heap, interleaved exploratory runs on a loaded
//! machine, binaries alternated; PR of #44). Without the checks, a target that is out
//! of range would be undefined behaviour, so the proof that it is in range travels in
//! the type.
//!
//! # The proof
//!
//! A [`Plan`](super::Plan) exists only inside [`Plan::with`](super::Plan::with), which
//! gives it a fresh lifetime `'g`, invariant, that no other plan shares (the "generative
//! brand" of `GhostCell` and of the `indexing` crate). A `Node<'g>` can only be made by
//! that plan — from a checked list, from a checked id ([`Plan::node`](super::Plan::node))
//! or from a range of the plan's nodes — so its id is below the plan's node count `n`.
//! A `PerNode<'_, 'g, T>` holds at least `n` values, a `NodeCells<'g>` exactly `n` cells
//! each holding a `Node<'g>`, a `NodeSet<'g>` a bit for each of `n` nodes. Indexing one
//! with a `Node<'g>` is therefore in bounds; a node of another plan has another brand and
//! does not type-check.
//!
//! The raw CSR reader [`csr_list`] is here too: its arrays are checked once when the
//! source is made, and each list then costs one comparison.

use std::fmt;
use std::marker::PhantomData;
use std::num::NonZeroUsize;
use std::ops::{Index, Range};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use everygraph_core::{Memory, MemoryKind, Reservation, SegVec, par};

use super::{Error, Split, ix};

/// The brand: an invariant lifetime.
pub(crate) type Brand<'g> = PhantomData<fn(&'g ()) -> &'g ()>;

/// Words or cells per task when state is made, cleared or listed in parallel.
const GRAIN: NonZeroUsize = NonZeroUsize::new(1 << 12).expect("non-zero");

/// A node of the graph of one [`Plan`](super::Plan): an id below its node count.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct Node<'g> {
    id: u32,
    brand: Brand<'g>,
}

impl fmt::Debug for Node<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.id.fmt(f)
    }
}

impl<'g> Node<'g> {
    /// A node of the plan of brand `'g`.
    ///
    /// # Safety
    ///
    /// `id` is below the node count of that plan.
    pub(crate) const unsafe fn new_unchecked(id: u32) -> Self {
        Self {
            id,
            brand: PhantomData,
        }
    }

    /// The id.
    #[inline]
    #[must_use]
    pub const fn get(self) -> u32 {
        self.id
    }

    /// The id as an index into memory.
    #[inline]
    #[must_use]
    pub fn index(self) -> usize {
        ix(self.id.into())
    }
}

/// The nodes `range` of the plan of brand `'g`.
///
/// # Safety
///
/// `range.end` is at most that plan's node count.
pub(crate) unsafe fn node_range<'g>(range: Range<u32>) -> impl Iterator<Item = Node<'g>> {
    // SAFETY: every id of the range is below its end, which the caller bounds by `n`.
    range.map(|id| unsafe { Node::new_unchecked(id) })
}

/// A range of indices below a node count, as node ids.
fn range_u32(r: Range<usize>) -> Range<u32> {
    let id = |x| u32::try_from(x).expect("below a node count");
    id(r.start)..id(r.end)
}

/// `list` as nodes of the plan of brand `'g`.
///
/// # Safety
///
/// Every element of `list` is below that plan's node count.
#[inline]
pub(crate) unsafe fn as_nodes<'a, 'g>(list: &'a [u32]) -> &'a [Node<'g>] {
    // SAFETY: `Node` is `repr(transparent)` over `u32` (its other field is a
    // zero-sized `PhantomData`), so the two slices have the same layout; the caller
    // guarantees the invariant of every element.
    unsafe { std::slice::from_raw_parts(list.as_ptr().cast::<Node<'g>>(), list.len()) }
}

/// The ids of `nodes`.
#[must_use]
pub fn ids<'a>(nodes: &'a [Node<'_>]) -> &'a [u32] {
    // SAFETY: the same layout, as in `as_nodes`; a `u32` has no invariant.
    unsafe { std::slice::from_raw_parts(nodes.as_ptr().cast::<u32>(), nodes.len()) }
}

/// Per-node values of the plan of brand `'g`: a slice of at least its node count,
/// indexed by its nodes without bounds checks. Made by
/// [`Plan::per_node`](super::Plan::per_node).
pub struct PerNode<'a, 'g, T> {
    values: &'a [T],
    brand: Brand<'g>,
}

impl<T> Clone for PerNode<'_, '_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for PerNode<'_, '_, T> {}

impl<T> fmt::Debug for PerNode<'_, '_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PerNode").field("len", &self.values.len()).finish()
    }
}

impl<'a, 'g, T> PerNode<'a, 'g, T> {
    /// `values` for the plan of brand `'g`, whose node count is `n`; `None` if it is
    /// shorter than `n`. Callers pass the plan's own count.
    pub(crate) fn new(values: &'a [T], n: u32) -> Option<Self> {
        (values.len() as u64 >= u64::from(n)).then_some(Self {
            values,
            brand: PhantomData,
        })
    }

    /// The values.
    #[must_use]
    pub fn as_slice(&self) -> &'a [T] {
        self.values
    }
}

impl<'g, T> Index<Node<'g>> for PerNode<'_, 'g, T> {
    type Output = T;

    #[inline]
    fn index(&self, v: Node<'g>) -> &T {
        // SAFETY: `v` is below the plan's node count (`Node`'s invariant), which is at
        // most `values.len()` (checked in `new`).
        unsafe { self.values.get_unchecked(v.index()) }
    }
}

/// One atomic cell per node of the plan of brand `'g`, each holding a node of that
/// plan: parents of a union-find, for example. Loading a cell gives a [`Node`], so the
/// value can index again without a check.
pub struct NodeCells<'g> {
    cells: Vec<AtomicU32>,
    _reservation: Reservation,
    brand: Brand<'g>,
}

impl fmt::Debug for NodeCells<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NodeCells").field("len", &self.cells.len()).finish()
    }
}

impl<'g> NodeCells<'g> {
    /// Cell `v` holding `v`, for the `n` nodes of the plan of brand `'g`, made in
    /// parallel and reserved against `memory`. Callers pass the plan's own count.
    pub(crate) fn identity(memory: &Memory, n: u32) -> Result<Self, Error> {
        let reservation = memory.reserve("node cells", MemoryKind::Anonymous, u64::from(n) * 4)?;
        let cells = par::collect(ix(n.into()), GRAIN, |i| {
            AtomicU32::new(u32::try_from(i).expect("below n"))
        });
        Ok(Self {
            cells,
            _reservation: reservation,
            brand: PhantomData,
        })
    }

    #[inline]
    fn cell(&self, v: Node<'g>) -> &AtomicU32 {
        // SAFETY: `v` is below the plan's node count (`Node`'s invariant), the number of
        // cells (`identity`).
        unsafe { self.cells.get_unchecked(v.index()) }
    }

    /// The node in cell `v`.
    #[inline]
    #[must_use]
    pub fn load(&self, v: Node<'g>, order: Ordering) -> Node<'g> {
        // SAFETY: every value ever stored in a cell is a `Node<'g>` (`identity` stores
        // ids below `n`, `store` and `compare_exchange` take nodes), and a load returns
        // one of the values stored in that cell.
        unsafe { Node::new_unchecked(self.cell(v).load(order)) }
    }

    /// Puts `x` in cell `v`.
    #[inline]
    pub fn store(&self, v: Node<'g>, x: Node<'g>, order: Ordering) {
        self.cell(v).store(x.get(), order);
    }

    /// Replaces `current` by `new` in cell `v` if it holds `current`; returns what it
    /// held.
    ///
    /// # Errors
    ///
    /// What cell `v` held instead of `current`.
    #[inline]
    pub fn compare_exchange(
        &self,
        v: Node<'g>,
        current: Node<'g>,
        new: Node<'g>,
        order: Ordering,
    ) -> Result<Node<'g>, Node<'g>> {
        let wrap = |id| {
            // SAFETY: as in `load`: the value was in the cell.
            unsafe { Node::new_unchecked(id) }
        };
        self.cell(v)
            .compare_exchange(current.get(), new.get(), order, Ordering::Relaxed)
            .map(wrap)
            .map_err(wrap)
    }

    /// Points every cell at the end of its chain (`v → cell v → …` until a cell holds
    /// itself), in parallel: a union-find's path compression.
    pub fn flatten(&self) {
        par::for_each_range(self.cells.len(), GRAIN, |r| {
            // SAFETY: `r` is within the cells, one per node.
            for v in unsafe { node_range(range_u32(r)) } {
                loop {
                    let p = self.load(v, Ordering::Relaxed);
                    let pp = self.load(p, Ordering::Relaxed);
                    if p == pp {
                        break;
                    }
                    self.store(v, pp, Ordering::Relaxed);
                }
            }
        });
    }

    /// The ids in the cells, and the reservation that holds them.
    #[must_use]
    pub fn into_ids(self) -> (Vec<AtomicU32>, Reservation) {
        (self.cells, self._reservation)
    }
}

/// A set of nodes of the plan of brand `'g` that workers add to concurrently: a
/// frontier, the nodes found in a step, a view's mask. Listing it gives its nodes in
/// increasing order whatever order they were added in.
///
/// Two levels of bits: one per node, and one per word of those, set when the word gets
/// its first node. Listing and clearing a set of `k` nodes visit the `n / 4096` summary
/// words and the touched words only, on the calling thread, until there are so many
/// that a parallel pass over every word is cheaper: a small frontier stays sparse, and
/// each of the thousands of levels of a traversal of a road graph costs neither a pass
/// over `n / 64` words nor a fork-join (research review of #44, `docs/research/sota-kernels.md`).
pub struct NodeSet<'g> {
    /// One bit per node; bits from the node count up are never set.
    words: Vec<AtomicU64>,
    /// One bit per word of `words`: set once that word may hold a node. A word with a
    /// node always has its bit set.
    summary: Vec<AtomicU64>,
    _reservation: Reservation,
    brand: Brand<'g>,
}

/// Touched words above which listing and clearing go over every word in parallel.
const SPARSE_WORDS: usize = 1 << 14;

impl fmt::Debug for NodeSet<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NodeSet").field("words", &self.words.len()).finish()
    }
}

impl<'g> NodeSet<'g> {
    /// The empty set over the `n` nodes of the plan of brand `'g`, reserved against
    /// `memory`. Callers pass the plan's own count.
    pub(crate) fn empty(memory: &Memory, n: u32) -> Result<Self, Error> {
        let len = ix(n.div_ceil(64).into());
        let summary = len.div_ceil(64);
        let bytes = (len + summary) as u64 * 8;
        let reservation = memory.reserve("node set", MemoryKind::Anonymous, bytes)?;
        Ok(Self {
            words: par::collect(len, GRAIN, |_| AtomicU64::new(0)),
            summary: par::collect(summary, GRAIN, |_| AtomicU64::new(0)),
            _reservation: reservation,
            brand: PhantomData,
        })
    }

    #[inline]
    fn word(&self, v: Node<'g>) -> &AtomicU64 {
        // SAFETY: `v` is below the node count `n` (`Node`'s invariant), so `v / 64` is
        // below `ceil(n / 64)`, the number of words (`empty`).
        unsafe { self.words.get_unchecked(ix((v.get() >> 6).into())) }
    }

    /// Adds `v`; true if it was not in the set.
    #[inline]
    pub fn insert(&self, v: Node<'g>) -> bool {
        let bit = 1 << (v.get() & 63);
        let old = self.word(v).fetch_or(bit, Ordering::Relaxed);
        if old == 0 {
            let w = v.get() >> 6;
            self.summary[ix((w >> 6).into())].fetch_or(1 << (w & 63), Ordering::Relaxed);
        }
        old & bit == 0
    }

    /// Whether `v` is in the set.
    #[inline]
    #[must_use]
    pub fn contains(&self, v: Node<'g>) -> bool {
        self.word(v).load(Ordering::Relaxed) >> (v.get() & 63) & 1 != 0
    }

    /// The touched words, in increasing order, if there are at most [`SPARSE_WORDS`].
    fn touched(&self) -> Option<Vec<usize>> {
        let count: usize = self
            .summary
            .iter()
            .map(|s| s.load(Ordering::Relaxed).count_ones() as usize)
            .sum();
        if count > SPARSE_WORDS {
            return None;
        }
        let mut out = Vec::new();
        for (i, s) in self.summary.iter().enumerate() {
            let mut x = s.load(Ordering::Relaxed);
            while x != 0 {
                out.push(i * 64 + x.trailing_zeros() as usize);
                x &= x - 1;
            }
        }
        Some(out)
    }

    /// Removes every node.
    pub fn clear(&mut self) {
        match self.touched() {
            Some(touched) => {
                for w in touched {
                    *self.words[w].get_mut() = 0;
                }
            }
            None => par::for_each_chunk_mut(&mut self.words, GRAIN, |_, words| {
                for w in words {
                    *w.get_mut() = 0;
                }
            }),
        }
        for s in &mut self.summary {
            *s.get_mut() = 0;
        }
    }

    /// Replaces `out`'s nodes with those of the set, in increasing order.
    ///
    /// # Errors
    ///
    /// [`Error::Memory`] if they do not fit in `out`'s budget, or in one segment (more
    /// than 2^28 nodes: #m3-15).
    ///
    /// # Panics
    ///
    /// Never: every bit set is a node id.
    pub fn list_into(&self, out: &mut NodeList<'g>) -> Result<(), Error> {
        let words = &self.words;
        let ids = &mut out.ids;
        ids.clear();
        // The nodes of word `w`, holding `x`, in order.
        let bits = |w: usize, mut x: u64, put: &mut dyn FnMut(u32)| {
            let base = u32::try_from(w * 64).expect("a node id");
            while x != 0 {
                put(base | x.trailing_zeros());
                x &= x - 1;
            }
        };
        if let Some(touched) = self.touched() {
            let mut result = Ok(());
            for w in touched {
                bits(w, words[w].load(Ordering::Relaxed), &mut |id| {
                    if result.is_ok() {
                        result = ids.try_push(id);
                    }
                });
            }
            return Ok(result?);
        }
        let counts = par::map_ranges(words.len(), GRAIN, |r| {
            words[r]
                .iter()
                .map(|w| w.load(Ordering::Relaxed).count_ones() as usize)
                .sum::<usize>()
        });
        ids.try_resize(counts.iter().sum(), 0)?;
        let all = ids
            .as_mut_slice()
            .ok_or(Error::Invalid("a node list longer than one segment (#m3-15)"))?;
        let mut pieces = Vec::new();
        let mut rest = all;
        for (k, &c) in counts.iter().enumerate() {
            let (mine, others) = Split::split_at(rest, c);
            pieces.push((k, mine));
            rest = others;
        }
        par::for_each_chunk_mut(&mut pieces, NonZeroUsize::MIN, |_, chunk| {
            for (k, dst) in chunk.iter_mut() {
                let first = *k * GRAIN.get();
                let mine = &words[first..(first + GRAIN.get()).min(words.len())];
                let mut at = 0;
                for (w, word) in (first..).zip(mine) {
                    bits(w, word.load(Ordering::Relaxed), &mut |id| {
                        dst[at] = id;
                        at += 1;
                    });
                }
            }
        });
        Ok(())
    }
}

/// Nodes of the plan of brand `'g` in a list, in increasing order when made by
/// [`NodeSet::list_into`]: a frontier.
#[derive(Debug)]
pub struct NodeList<'g> {
    /// Every id is a node of the plan: only `list_into` and `push` write it.
    ids: SegVec<u32>,
    brand: Brand<'g>,
}

impl<'g> NodeList<'g> {
    /// An empty list, its memory counted against `memory`.
    #[must_use]
    pub fn new(memory: &Memory) -> Self {
        Self {
            ids: SegVec::new(memory, "node list"),
            brand: PhantomData,
        }
    }

    /// Appends `v`.
    ///
    /// # Errors
    ///
    /// [`Error::Memory`] if it does not fit in the budget.
    pub fn push(&mut self, v: Node<'g>) -> Result<(), Error> {
        Ok(self.ids.try_push(v.get())?)
    }

    /// Number of nodes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// Whether there are none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// The nodes.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] if they are in more than one segment (#m3-15).
    pub fn as_slice(&self) -> Result<&[Node<'g>], Error> {
        let ids = self
            .ids
            .as_slice()
            .ok_or(Error::Invalid("a node list longer than one segment (#m3-15)"))?;
        // SAFETY: every id was a `Node<'g>` when written (field invariant).
        Ok(unsafe { as_nodes(ids) })
    }
}

/// The list of `v` in a CSR whose arrays [`check_csr`] accepted.
///
/// # Panics
///
/// If `v` is not below `offsets.len() - 1`.
#[inline]
pub(crate) fn csr_list<'a>(csr: &CheckedCsr<'a>, v: u32) -> &'a [u32] {
    let v = ix(v.into());
    assert!(v < csr.offsets.len() - 1, "node {v} outside the graph");
    // SAFETY: `v + 1 < offsets.len()`, checked above.
    let a = ix(*unsafe { csr.offsets.get_unchecked(v) });
    // SAFETY: as above.
    let b = ix(*unsafe { csr.offsets.get_unchecked(v + 1) });
    // SAFETY: the offsets never decrease and end at `targets.len()` (checked by
    // `check_csr`), so `a <= b <= targets.len()`.
    unsafe { csr.targets.get_unchecked(a..b) }
}

/// CSR arrays that [`check_csr`] accepted: `offsets` from 0, never decreasing, ending at
/// `targets.len()`, every degree below 2^32, at most 2^32 − 1 nodes.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CheckedCsr<'a> {
    offsets: &'a [u64],
    targets: &'a [u32],
}

impl<'a> CheckedCsr<'a> {
    pub(crate) fn targets(&self) -> &'a [u32] {
        self.targets
    }
}

/// Checks CSR offsets over `entries` targets (in parallel); returns the node count.
///
/// # Errors
///
/// [`Error::Invalid`] naming the problem.
pub(crate) fn check_offsets(offsets: &[u64], entries: u64) -> Result<u32, Error> {
    let n = offsets
        .len()
        .checked_sub(1)
        .ok_or(Error::Invalid("offsets: none"))?;
    let n = u32::try_from(n).map_err(|_| Error::Invalid("offsets: more than 2^32 - 1 nodes"))?;
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
        Err(Error::Invalid("offsets: decrease, or a degree is 2^32 or more"))
    }
}

/// Nodes per task when offsets are checked in parallel.
const CHECK_GRAIN: NonZeroUsize = NonZeroUsize::new(1 << 16).expect("non-zero");

/// Checks `offsets` as a CSR over `targets` ([`check_offsets`]).
///
/// # Errors
///
/// As [`check_offsets`].
pub(crate) fn check_csr<'a>(
    offsets: &'a [u64],
    targets: &'a [u32],
) -> Result<(CheckedCsr<'a>, u32), Error> {
    let n = check_offsets(offsets, targets.len() as u64)?;
    Ok((CheckedCsr { offsets, targets }, n))
}

#[cfg(test)]
mod tests {
    //! Every `unsafe` block of the module, reached from safe code (and run under Miri).

    use std::sync::atomic::Ordering::Relaxed;

    use super::*;

    #[test]
    fn nodes_index_state_of_their_plan() {
        let memory = Memory::unlimited();
        // SAFETY: a test of 5 nodes.
        let nodes: Vec<Node<'_>> = unsafe { node_range(0..5) }.collect();
        let values = [10, 11, 12, 13, 14, 15];
        let per = PerNode::new(&values, 5).unwrap();
        assert!(PerNode::new(&values[..4], 5).is_none());
        assert_eq!(nodes.iter().map(|&v| per[v]).sum::<i32>(), 60);
        // SAFETY: every id below 5.
        let list = unsafe { as_nodes(&[4, 0, 2]) };
        assert_eq!(ids(list), &[4, 0, 2]);
        assert_eq!(per[list[0]], 14);

        let cells = NodeCells::identity(&memory, 5).unwrap();
        assert_eq!(cells.load(list[0], Relaxed), list[0]);
        cells.store(list[0], list[1], Relaxed);
        assert_eq!(cells.load(list[0], Relaxed), list[1]);
        assert_eq!(cells.compare_exchange(list[0], list[1], list[2], Relaxed), Ok(list[1]));
        assert_eq!(cells.compare_exchange(list[0], list[1], list[2], Relaxed), Err(list[2]));
        let (ids, _) = cells.into_ids();
        assert_eq!(ids[4].load(Relaxed), 2);

        let mut set = NodeSet::empty(&memory, 5).unwrap();
        assert!(set.insert(list[0]) && !set.insert(list[0]) && set.insert(list[2]));
        assert!(set.contains(list[2]) && !set.contains(nodes[1]));
        let mut out = NodeList::new(&memory);
        out.push(nodes[3]).unwrap();
        set.list_into(&mut out).unwrap();
        assert_eq!(out.as_slice().unwrap(), &[nodes[2], nodes[4]]);
        set.clear();
        set.list_into(&mut out).unwrap();
        assert!(out.is_empty());

        let (offsets, targets) = ([0, 2, 2, 3], [1, 2, 0]);
        let (csr, n) = check_csr(&offsets, &targets).unwrap();
        assert_eq!(n, 3);
        let lists: [&[u32]; 3] = [&[1, 2], &[], &[0]];
        assert_eq!([csr_list(&csr, 0), csr_list(&csr, 1), csr_list(&csr, 2)], lists);
        assert!(check_csr(&[0, 2, 1], &targets).is_err());
        assert!(check_csr(&[0, 1], &targets).is_err());
    }

    #[test]
    #[should_panic = "outside the graph"]
    fn a_csr_list_outside_the_graph_panics() {
        let (offsets, targets) = ([0, 1], [0]);
        let (csr, _) = check_csr(&offsets, &targets).unwrap();
        let _ = csr_list(&csr, 1);
    }
}
