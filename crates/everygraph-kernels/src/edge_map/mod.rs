// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The edge-map framework (DESIGN D12, ADR 0012): a kernel body is written once, as a
//! closure over one node and its list, and runs over every [`Source`] — raw arrays,
//! the compressed format in the heap or memory-mapped, a stream — in many short calls.
//!
//! # Drivers
//!
//! A [`Plan`] binds a source to the work partition and the per-worker readers. Its
//! drivers call the body:
//!
//! - [`Plan::scan`]: every node that passes a filter, with its list (pull, scan-shaped).
//!   Lists are decoded or read in batches of about [`Settings::batch_entries`] entries
//!   before the body runs on them; a block in which no node passes is not decoded.
//! - [`Plan::vertices`]: every node, with its degree, without its list.
//! - [`Plan::frontier`]: the nodes of a sorted frontier, with their lists, by random
//!   access (push, traversal-shaped); consecutive nodes are decoded together and, if
//!   [`Settings::prefetch`] is set, the frontier's byte ranges are advised first.
//!
//! The body of [`scan`](Plan::scan) and [`vertices`](Plan::vertices) writes per-node
//! outputs through `out`, any [`Split`] (a `&mut [T]` of one value per node, or a
//! tuple of them): each task receives the part of `out` for its own nodes, indexed from
//! 0, so the writes need neither atomics nor `unsafe`.
//!
//! # Nodes
//!
//! A plan exists only inside [`Plan::with`], and its node ids are [`Node`]s of its own
//! brand: the body receives the node and its list as `Node<'g>` and `&[Node<'g>]`, every
//! one checked to be in range when the list was decoded or read. Per-node state made
//! from the plan — [`PerNode`], [`NodeCells`], [`NodeSet`] — is indexed by them without
//! bounds checks (module `node`, the one `unsafe` module of the crate, says why).
//!
//! # Determinism
//!
//! Every driver returns a reduction, an [`Reduce`] value that the body adds to through
//! `&mut A`. Results do not depend on the thread count, the chunk size, the batch size
//! or the source (DESIGN D14, `design/DECISIONS.md` "What 'repeatable' promises"):
//!
//! - **Reduction blocks.** Nodes are cut into fixed blocks of [`Plan::block_nodes`]
//!   nodes, a function of the number of nodes alone. The body's additions for the nodes
//!   of one block are made in node order, from [`Reduce::ZERO`], by one worker; the
//!   blocks' partial values are then added in block order on the calling thread. The
//!   parallel tasks, which hold about [`Settings::chunk_entries`] entries each, are whole
//!   blocks; no reduction ever follows the tasks, the batches or a thread (the oracle's
//!   finding on the lab prototype, PR #131; rayon's float `sum` is not used).
//! - **Frontiers.** [`Plan::frontier`] cuts the frontier into runs of [`FRONTIER_GRAIN`]
//!   nodes, a constant, and adds their partial values in frontier order.
//! - Per-node outputs are written by the one task that owns the node.
//!
//! What the body does with shared state (atomics, [`NodeCells`]) is its own affair:
//! the kernels of [`crate::proof`] show results that do not depend on the order in which
//! racing updates land.
//!
//! # `unsafe`
//!
//! This module is the crate's one `unsafe` module (CONTRIBUTING.md, "`unsafe`"): the
//! node brand and the unchecked indexing it allows are in `node.rs`, which explains
//! why, with the measurement; the drivers here turn the lists a [`Source`] guarantees
//! into nodes; the sources state that guarantee in their `unsafe impl`.

#![allow(unsafe_code)]

mod node;
mod source;

use std::error::Error as StdError;
use std::fmt;
use std::num::NonZeroUsize;
use std::ops::Range;
use std::sync::Mutex;

use everygraph_core::{InsufficientBudget, Memory, MemoryError, ReadError, par};
use everygraph_format::adjacency::DecodeError;

pub use node::{Node, NodeCells, NodeList, NodeSet, PerNode, ids};
pub use source::{Compressed, Csr, DecodeReader, Source, Stream, StreamReader};

use node::{Brand, as_nodes, node_range};

/// Nodes of a frontier per task of [`Plan::frontier`]. A constant: the frontier's
/// partial reductions are taken over runs of it.
pub const FRONTIER_GRAIN: usize = GRAIN.get();
const GRAIN: NonZeroUsize = NonZeroUsize::new(256).expect("non-zero");

/// Fewest nodes in a reduction block.
const MIN_BLOCK: u32 = 64;
/// Most nodes in a reduction block.
const MAX_BLOCK: u32 = 4096;
/// About this many blocks per graph, between the two bounds.
const BLOCKS: u32 = 4096;

/// A `u64` as an index into memory.
pub(crate) fn ix(x: u64) -> usize {
    usize::try_from(x).expect("an index into memory")
}

/// Why a driver failed.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// Compressed records that do not decode to what the index says.
    Decode(DecodeError),
    /// A stream could not be read.
    Read(ReadError),
    /// The list of `node` names `target`, which is not a node of the graph: the input is
    /// corrupt.
    Target {
        /// The node whose list is corrupt.
        node: u32,
        /// The entry that is out of range.
        target: u32,
    },
    /// Arrays that are not a graph.
    Invalid(&'static str),
    /// A buffer does not fit in the memory budget.
    Memory(MemoryError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(e) => e.fmt(f),
            Self::Read(e) => e.fmt(f),
            Self::Target { node, target } => {
                write!(f, "the list of node {node} names {target}, which is not a node of the graph")
            }
            Self::Invalid(what) => write!(f, "not a graph: {what}"),
            Self::Memory(e) => e.fmt(f),
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Decode(e) => Some(e),
            Self::Read(e) => Some(e),
            Self::Memory(e) => Some(e),
            Self::Target { .. } | Self::Invalid(_) => None,
        }
    }
}

impl From<DecodeError> for Error {
    fn from(e: DecodeError) -> Self {
        Self::Decode(e)
    }
}

impl From<ReadError> for Error {
    fn from(e: ReadError) -> Self {
        Self::Read(e)
    }
}

impl From<MemoryError> for Error {
    fn from(e: MemoryError) -> Self {
        Self::Memory(e)
    }
}

impl From<InsufficientBudget> for Error {
    fn from(e: InsufficientBudget) -> Self {
        Self::Memory(e.into())
    }
}

/// How a [`Plan`] cuts and reads; none of it changes a result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Threads the plan runs on: the number of per-worker readers kept. More threads
    /// than this still work, with readers made for the call (and reserved then).
    pub threads: NonZeroUsize,
    /// Entries per parallel task, about (a task is at least one reduction block).
    pub chunk_entries: u64,
    /// Entries per decoded or read batch, about (a batch holds at least one list).
    pub batch_entries: usize,
    /// Advise the source of a frontier's byte ranges before reading them: for cold or
    /// out-of-core storage; on a warm memory map it costs system calls for nothing.
    pub prefetch: bool,
}

impl Default for Settings {
    /// One thread; tasks of 2^16 entries, as the lab's kernels; batches of 8,192
    /// entries, as #118 measured; no prefetch.
    fn default() -> Self {
        Self {
            threads: NonZeroUsize::MIN,
            chunk_entries: 1 << 16,
            batch_entries: 8192,
            prefetch: false,
        }
    }
}

/// A value the drivers add up in a fixed order.
pub trait Reduce: Copy + Send {
    /// The value of no nodes.
    const ZERO: Self;
    /// `self + other`.
    #[must_use]
    fn add(self, other: Self) -> Self;
}

impl Reduce for () {
    const ZERO: Self = ();
    fn add(self, (): Self) -> Self {}
}

macro_rules! reduce_by_add {
    ($($t:ty: $zero:expr),*) => {$(
        impl Reduce for $t {
            const ZERO: Self = $zero;
            #[inline]
            fn add(self, other: Self) -> Self {
                self + other
            }
        }
    )*};
}
reduce_by_add!(u32: 0, u64: 0, f32: 0.0, f64: 0.0);

impl<A: Reduce, B: Reduce> Reduce for (A, B) {
    const ZERO: Self = (A::ZERO, B::ZERO);
    #[inline]
    fn add(self, other: Self) -> Self {
        (self.0.add(other.0), self.1.add(other.1))
    }
}

/// Per-node outputs that can be cut between tasks: a slice of one value per node, a
/// tuple of such, or `()` for none.
pub trait Split: Send + Sized {
    /// The first `mid` nodes' part and the rest.
    fn split_at(self, mid: usize) -> (Self, Self);
}

impl Split for () {
    fn split_at(self, _: usize) -> (Self, Self) {
        ((), ())
    }
}

impl<T: Send> Split for &mut [T] {
    fn split_at(self, mid: usize) -> (Self, Self) {
        self.split_at_mut(mid)
    }
}

impl<A: Split, B: Split> Split for (A, B) {
    fn split_at(self, mid: usize) -> (Self, Self) {
        let (a0, a1) = self.0.split_at(mid);
        let (b0, b1) = self.1.split_at(mid);
        ((a0, b0), (a1, b1))
    }
}

/// The filter that passes every node.
#[must_use]
pub fn all(_: Node<'_>) -> bool {
    true
}

/// A source with its work partition and its readers: what the drivers run on. Made by
/// [`Plan::with`]; `'g` is its brand (see [`Node`]).
pub struct Plan<'s, 'g, S: Source> {
    source: &'s S,
    brand: Brand<'g>,
    block: u32,
    /// Parallel tasks: whole blocks, in order, covering every node.
    tasks: Vec<Range<u32>>,
    batch: usize,
    prefetch: bool,
    readers: Vec<Mutex<Option<S::Reader>>>,
    memory: Memory,
}

impl<S: Source> fmt::Debug for Plan<'_, '_, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Plan")
            .field("nodes", &self.source.nodes())
            .field("block", &self.block)
            .field("tasks", &self.tasks.len())
            .field("batch", &self.batch)
            .finish_non_exhaustive()
    }
}

/// Nodes per reduction block of a graph of `n` nodes: about [`BLOCKS`] blocks, a power
/// of two between [`MIN_BLOCK`] and [`MAX_BLOCK`].
fn block_nodes(n: u32) -> u32 {
    n.div_ceil(BLOCKS)
        .next_power_of_two()
        .clamp(MIN_BLOCK, MAX_BLOCK)
}

impl<'s, S: Source> Plan<'s, '_, S> {
    /// Cuts `source` into tasks and calls `f` with the plan. Readers are made when first
    /// needed, each reserved against `memory`.
    ///
    /// The plan, and every [`Node`] and per-node state made from it, live only inside
    /// `f`: their brand `'g` is new for this call, so nodes of one plan cannot index the
    /// state of another.
    pub fn with<R>(
        source: &'s S,
        memory: &Memory,
        settings: Settings,
        f: impl for<'g> FnOnce(Plan<'s, 'g, S>) -> R,
    ) -> R {
        f(Plan::new(source, memory, settings))
    }
}

impl<'s, 'g, S: Source> Plan<'s, 'g, S> {
    fn new(source: &'s S, memory: &Memory, settings: Settings) -> Self {
        let n = source.nodes();
        let block = block_nodes(n);
        let mut tasks = Vec::new();
        let mut start = 0;
        while start < n {
            let limit = source.entry_offset(start).saturating_add(settings.chunk_entries.max(1));
            let mut end = start.saturating_add(block).min(n);
            while end < n && source.entry_offset(end) < limit {
                end = end.saturating_add(block).min(n);
            }
            tasks.push(start..end);
            start = end;
        }
        Self {
            source,
            brand: Brand::default(),
            block,
            tasks,
            batch: settings.batch_entries.max(1),
            prefetch: settings.prefetch,
            readers: (0..settings.threads.get()).map(|_| Mutex::new(None)).collect(),
            memory: memory.clone(),
        }
    }

    /// The source.
    #[must_use]
    pub fn source(&self) -> &'s S {
        self.source
    }

    /// Number of nodes.
    #[must_use]
    pub fn nodes(&self) -> u32 {
        self.source.nodes()
    }

    /// Nodes per reduction block, a function of the number of nodes alone.
    #[must_use]
    pub fn block_nodes(&self) -> u32 {
        self.block
    }

    /// Node `id`, if it is one.
    #[must_use]
    pub fn node(&self, id: u32) -> Option<Node<'g>> {
        // SAFETY: `id` is below the node count, checked here.
        (id < self.nodes()).then(|| unsafe { Node::new_unchecked(id) })
    }

    /// Every node, in order.
    pub fn all_nodes(&self) -> impl Iterator<Item = Node<'g>> + use<'g, S> {
        // SAFETY: the range ends at the node count.
        unsafe { node_range(0..self.nodes()) }
    }

    /// The degree of `v`.
    #[must_use]
    pub fn degree(&self, v: Node<'g>) -> u32 {
        self.source.degree(v.get())
    }

    /// `values`, one per node, indexed by the plan's nodes without bounds checks.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] if `values` is shorter than the node count.
    pub fn per_node<'a, T>(&self, values: &'a [T]) -> Result<PerNode<'a, 'g, T>, Error> {
        PerNode::new(values, self.nodes())
            .ok_or(Error::Invalid("per-node values: fewer than the nodes"))
    }

    /// Cell `v` holding `v`, for every node: a union-find's parents.
    ///
    /// # Errors
    ///
    /// [`Error::Memory`] if the cells do not fit in the budget.
    pub fn node_cells(&self, memory: &Memory) -> Result<NodeCells<'g>, Error> {
        NodeCells::identity(memory, self.nodes())
    }

    /// The empty set of the plan's nodes.
    ///
    /// # Errors
    ///
    /// [`Error::Memory`] if the set does not fit in the budget.
    pub fn node_set(&self, memory: &Memory) -> Result<NodeSet<'g>, Error> {
        NodeSet::empty(memory, self.nodes())
    }

    /// Runs `f` with a reader of the pool, or one made for the call when every reader
    /// is in use.
    fn with_reader<R>(&self, f: impl FnOnce(&mut S::Reader) -> Result<R, Error>) -> Result<R, Error> {
        for slot in &self.readers {
            if let Ok(mut guard) = slot.try_lock() {
                if guard.is_none() {
                    *guard = Some(self.source.reader(&self.memory, self.batch, self.block)?);
                }
                return f(guard.as_mut().expect("made above"));
            }
        }
        f(&mut self.source.reader(&self.memory, self.batch, self.block)?)
    }

    /// Runs `task` on every task in parallel, each with its part of `out` and of the
    /// blocks' partial values, and adds the partial values in block order.
    fn run<O, A>(
        &self,
        out: O,
        task: impl Fn(Range<u32>, O, &mut [A]) -> Result<(), Error> + Sync,
    ) -> Result<A, Error>
    where
        O: Split,
        A: Reduce,
    {
        let n = self.source.nodes();
        let blocks = ix(n.div_ceil(self.block).into());
        let mut partials: Vec<A> = (0..blocks).map(|_| A::ZERO).collect();
        let mut pieces = Vec::new();
        let (mut out, mut rest) = (out, partials.as_mut_slice());
        for r in &self.tasks {
            let (mine, others) = out.split_at(ix((r.end - r.start).into()));
            let (sums, more) = rest.split_at_mut(ix(((r.end - r.start).div_ceil(self.block)).into()));
            pieces.push((r.clone(), Some(mine), sums, Ok(())));
            (out, rest) = (others, more);
        }
        par::for_each_chunk_mut(&mut pieces, NonZeroUsize::MIN, |_, chunk| {
            for (r, out, sums, result) in chunk.iter_mut() {
                *result = task(r.clone(), out.take().expect("one call each"), sums);
            }
        });
        pieces.into_iter().try_for_each(|p| p.3)?;
        Ok(partials.into_iter().fold(A::ZERO, A::add))
    }

    /// Calls `body(acc, out, i, v, list)` for every node `v` with `filter(v)`, where
    /// `list` is its list, `out` the task's part of the outputs and `i` the index of `v`
    /// in it; returns the sum of the reduction blocks' `acc`.
    ///
    /// `filter` is called once or twice per node: sources that decode or read (all but
    /// [`Csr`]) first check that it passes some node of a reduction block, and skip the
    /// block's decode when it passes none. It may read state that the body changes, if
    /// the kernel's answer does not depend on when such a change is seen.
    ///
    /// # Errors
    ///
    /// The lists could not be read or are corrupt, or a reader does not fit in the
    /// budget; the outputs are then partly written.
    pub fn scan<O, A>(
        &self,
        out: O,
        filter: impl Fn(Node<'g>) -> bool + Sync,
        body: impl Fn(&mut A, &mut O, usize, Node<'g>, &[Node<'g>]) + Sync,
    ) -> Result<A, Error>
    where
        O: Split,
        A: Reduce,
    {
        self.run(out, |task, mut out, sums| {
            self.with_reader(|reader| {
                for (b, sum) in self.blocks(task.clone()).zip(sums) {
                    let mut acc = A::ZERO;
                    let filter = |v| {
                        // SAFETY: the source asks only about nodes of `b` (`Source`'s
                        // contract), which is within the plan's nodes.
                        filter(unsafe { Node::new_unchecked(v) })
                    };
                    self.source.scan(reader, b, filter, |v, list| {
                        // SAFETY: `v` is in `b` (`Source`'s contract).
                        let node = unsafe { Node::new_unchecked(v) };
                        // SAFETY: the source hands out lists of its nodes only (`Source`'s
                        // contract).
                        let list = unsafe { as_nodes(list) };
                        body(&mut acc, &mut out, ix((v - task.start).into()), node, list);
                    })?;
                    *sum = acc;
                }
                Ok(())
            })
        })
    }

    /// Calls `body(acc, out, i, v, degree)` for every node `v`; as [`scan`](Self::scan)
    /// otherwise, without lists.
    ///
    /// # Errors
    ///
    /// None from the sources of this crate; the signature leaves room for one that
    /// reads degrees from storage.
    pub fn vertices<O, A>(
        &self,
        out: O,
        body: impl Fn(&mut A, &mut O, usize, Node<'g>, u32) + Sync,
    ) -> Result<A, Error>
    where
        O: Split,
        A: Reduce,
    {
        self.run(out, |task, mut out, sums| {
            for (b, sum) in self.blocks(task.clone()).zip(sums) {
                let mut acc = A::ZERO;
                self.source.degrees(b, |v, d| {
                    // SAFETY: `v` is in `b` (`Source`'s contract), within the plan's nodes.
                    let node = unsafe { Node::new_unchecked(v) };
                    body(&mut acc, &mut out, ix((v - task.start).into()), node, d);
                });
                *sum = acc;
            }
            Ok(())
        })
    }

    /// Calls `body(acc, v, list)` for each node `v` of `frontier`, which must be
    /// increasing (a [`NodeList`] made by [`NodeSet::list_into`]), with its list,
    /// decoded by random access;
    /// returns the sum of the `acc` of each run of [`FRONTIER_GRAIN`] nodes, in order. A
    /// frontier of one run is done on the calling thread: a level of a high-diameter
    /// traversal costs no fork-join.
    ///
    /// # Errors
    ///
    /// As [`scan`](Self::scan).
    pub fn frontier<A>(
        &self,
        frontier: &[Node<'g>],
        body: impl Fn(&mut A, Node<'g>, &[Node<'g>]) + Sync,
    ) -> Result<A, Error>
    where
        A: Reduce,
    {
        let frontier = ids(frontier);
        debug_assert!(frontier.windows(2).all(|w| w[0] < w[1]), "frontier not increasing");
        let run = |r: Range<usize>| {
            self.with_reader(|reader| {
                let nodes = &frontier[r];
                if self.prefetch {
                    self.source.prefetch(nodes);
                }
                let mut acc = A::ZERO;
                self.source.lists(reader, nodes, |v, list| {
                    // SAFETY: `v` is a node of the frontier (`Source`'s contract).
                    let node = unsafe { Node::new_unchecked(v) };
                    // SAFETY: the source hands out lists of its nodes only (`Source`'s
                    // contract).
                    let list = unsafe { as_nodes(list) };
                    body(&mut acc, node, list);
                })?;
                Ok(acc)
            })
        };
        if frontier.len() <= FRONTIER_GRAIN {
            return run(0..frontier.len());
        }
        par::map_ranges(frontier.len(), GRAIN, run)
            .into_iter()
            .try_fold(A::ZERO, |sum, part| Ok(sum.add(part?)))
    }

    /// The reduction blocks of `task`.
    fn blocks(&self, task: Range<u32>) -> impl Iterator<Item = Range<u32>> {
        let block = self.block;
        (task.start..task.end)
            .step_by(ix(block.into()))
            .map(move |s| s..s.saturating_add(block).min(task.end))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_follow_the_number_of_nodes_only() {
        assert_eq!(block_nodes(0), MIN_BLOCK);
        assert_eq!(block_nodes(1000), MIN_BLOCK);
        assert_eq!(block_nodes(64 * 4096 + 1), 128);
        assert_eq!(block_nodes(3_072_441), 1024);
        assert_eq!(block_nodes(u32::MAX), MAX_BLOCK);
    }

    /// Tasks are whole blocks, in order, covering every node, whatever the chunk size.
    #[test]
    fn tasks_are_whole_blocks_covering_the_graph() {
        let n: u32 = 1000;
        let offsets: Vec<u64> = (0..=u64::from(n))
            .scan(0, |sum, v| {
                let at = *sum;
                *sum += v % 7;
                Some(at)
            })
            .collect();
        let targets: Vec<u32> = (0..offsets[ix(n.into())]).map(|e| (e % 1000) as u32).collect();
        let csr = Csr::new(&offsets, &targets).unwrap();
        for chunk in [1, 64, 1000, 1 << 16, u64::MAX] {
            let settings = Settings {
                chunk_entries: chunk,
                ..Settings::default()
            };
            Plan::with(&csr, &Memory::unlimited(), settings, |plan| {
                let mut at = 0;
                for t in &plan.tasks {
                    assert_eq!(t.start, at);
                    assert!(t.end > t.start && (t.end % plan.block == 0 || t.end == n));
                    at = t.end;
                }
                assert_eq!(at, n, "chunk {chunk}");
            });
        }
    }
}
