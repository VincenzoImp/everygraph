// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The edge-map framework (DESIGN D12, ADR 0012). A kernel body is written once, as a
//! closure over one node and its list. It runs over every [`Source`]: raw arrays, the
//! compressed format in the heap or memory-mapped, a stream. It runs in many short calls.
//!
//! # Drivers
//!
//! A [`Plan`] binds a source to the work partition and to a pool of per-worker readers.
//! Its drivers call the body:
//!
//! - [`Plan::scan`] calls it for every node that passes a filter, with the node's list
//!   (pull, scan-shaped). Lists are decoded or read in batches of about
//!   [`Settings::batch_entries`] entries before the body runs on them. A block in which
//!   no node passes is not decoded.
//! - [`Plan::vertices`] calls it for every node, with its degree, without its list.
//! - [`Plan::frontier`] calls it for the nodes of a frontier, with their lists, by random
//!   access (push, traversal-shaped). Consecutive nodes are decoded together. If
//!   [`Settings::prefetch`] is set, the frontier's byte ranges are advised first.
//!
//! The body of a scan receives a [`Row`]: the node, its list, and the list's first entry,
//! which indexes per-entry columns (weights) with [`Row::entries`]. It receives a
//! [`Worker`] too: the kernel's per-worker scratch ([`Plan::scan_with`]), and random
//! access to the list of another node ([`Lists::get`]).
//!
//! The body writes per-node outputs through `out`, any [`Split`]: a `&mut [T]` of one
//! value per node, or a tuple of them. Each task gets the part of `out` for its own
//! nodes, indexed from 0, so the writes need no atomics.
//!
//! [`Buckets`] order nodes by an integer key that goes down (peeling, bucketed shortest
//! paths); [`Buckets::pop`] gives the frontier of the smallest key.
//!
//! # Nodes
//!
//! A plan exists only inside [`Plan::with`]. Its node ids are [`Node`]s of its own
//! brand. Every list is checked to be in range when it is decoded or read. Per-node state
//! made from the plan ([`PerNode`], [`NodeCells`], [`NodeSet`]) is indexed by these nodes
//! without bounds checks. Module `node` gives the proof.
//!
//! # Determinism
//!
//! Every driver returns a reduction: a [`Reduce`] value that the body adds to through
//! `&mut A`. The result does not depend on the thread count, the chunk size, the batch
//! size or the source (DESIGN D14):
//!
//! - **Reduction blocks.** The nodes are cut into fixed blocks of [`Plan::block_nodes`]
//!   nodes, a function of the node count alone. One worker adds the values of one block
//!   in node order, from [`Reduce::ZERO`]. The calling thread then adds the blocks'
//!   values in block order. A parallel task holds whole blocks (about
//!   [`Settings::chunk_entries`] entries). So no reduction follows a task, a batch or a
//!   thread.
//! - **Frontiers.** [`Plan::frontier`] cuts the frontier into runs of [`FRONTIER_GRAIN`]
//!   nodes, a constant, and adds their values in frontier order.
//! - **Outputs.** The one task that owns a node writes its outputs.
//! - **Lists are whole.** A batch, a read buffer or a task never cuts a list. So a node's
//!   own sum over its list runs in one call of the body.
//! - **Floats.** Accumulators start from `+0.0` ([`Reduce::ZERO`]); `Iterator::sum`
//!   starts from `-0.0`. A body with several lanes (the four of the `PageRank` gather)
//!   fixes their number in its code, not by target feature. No result goes through
//!   `f64::max`, which may return either zero. A pull and a push form of one kernel add
//!   in different orders, so they are not interchangeable.
//! - **Global scalars** (a dangling mass, a residual, a total) reduce into
//!   [`ExactSum`]: its value does not depend on how the values are grouped, so it does
//!   not rely on the reduction blocks either (`docs/research/sota-numerics.md`, R6).
//!
//! Shared state that bodies change (atomics, [`NodeCells`]) is the kernel's affair. The
//! kernels of [`crate::proof`] give results that do not depend on the order in which racing
//! updates land.
//!
//! # Work counters
//!
//! [`Plan::work`] counts the lists and entries that the drivers and [`Lists::get`]
//! handed to bodies. The counts depend only on the graph, the kernel and its filters,
//! if the filters read only state that is fixed during the step (BFS: a node's own
//! depth). A filter that reads state other tasks change (Afforest's giant component)
//! makes the counts vary, as the work does.
//!
//! # Memory
//!
//! The readers' buffers, the blocks' partial values and the frontier runs' results are
//! reserved against the plan's [`Memory`], and a reader frees a buffer that a long list
//! grew when it goes back to the pool. Some small allocations are not counted: the task
//! list (`O(n / block)`) and the pool itself (one entry per worker). The no-OOM suite
//! (`everygraph-memtest`) checks that what is not counted does not grow with the graph.
//!
//! # `unsafe`
//!
//! This is the crate's `unsafe` module (CONTRIBUTING.md, "`unsafe`"). The brand and the
//! unchecked indexing are in `node.rs`. The drivers here turn the lists that a [`Source`]
//! guarantees into nodes. The sources state the guarantee in their `unsafe impl`.

#![allow(unsafe_code)]

mod buckets;
mod node;
mod source;

use std::error::Error as StdError;
use std::fmt;
use std::num::NonZeroUsize;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use everygraph_core::{
    ExactSum, InsufficientBudget, Memory, MemoryError, MemoryKind, ReadError, SegVec, par,
};
use everygraph_format::adjacency::{DecodeError, FormatError};

pub use buckets::Buckets;
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

const SEGMENTS: Error = Error::Invalid("node state longer than one segment (#m3-15)");

/// The values of `v` as one slice, as the drivers take per-node state.
///
/// # Errors
///
/// [`Error::Invalid`] if `v` has more than one segment (#m3-15).
pub fn one_segment<T: Copy>(v: &SegVec<T>) -> Result<&[T], Error> {
    v.as_slice().ok_or(SEGMENTS)
}

/// As [`one_segment`], mutable.
///
/// # Errors
///
/// [`Error::Invalid`] if `v` has more than one segment (#m3-15).
pub fn one_segment_mut<T: Copy>(v: &mut SegVec<T>) -> Result<&mut [T], Error> {
    v.as_mut_slice().ok_or(SEGMENTS)
}

/// Why a driver failed.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// Compressed records that do not decode to what the index says.
    Decode(DecodeError),
    /// A data part that does not belong to its index.
    Format(FormatError),
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
    /// Arrays that are not a graph, or a column a kernel cannot use.
    Invalid(&'static str),
    /// A buffer does not fit in the memory budget.
    Memory(MemoryError),
    /// A weight or a cost that the kernel cannot use: negative, NaN or infinite
    /// (`docs/spec/graph-semantics.md` § 4.4). `entry` is its position in the column.
    Weight {
        /// The position of the value in its per-entry column.
        entry: u64,
        /// The value.
        value: f64,
    },
    /// A parameter that the kernel cannot use: an empty seed set, a node that is not in
    /// the graph.
    Argument(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(e) => e.fmt(f),
            Self::Format(e) => e.fmt(f),
            Self::Read(e) => e.fmt(f),
            Self::Target { node, target } => write!(
                f,
                "the list of node {node} names {target}, which is not a node of the graph"
            ),
            Self::Invalid(what) => write!(f, "not a graph: {what}"),
            Self::Memory(e) => e.fmt(f),
            Self::Weight { entry, value } => write!(
                f,
                "entry {entry} has weight {value}; a weight must be finite and not negative"
            ),
            Self::Argument(what) => f.write_str(what),
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Decode(e) => Some(e),
            Self::Format(e) => Some(e),
            Self::Read(e) => Some(e),
            Self::Memory(e) => Some(e),
            Self::Target { .. } | Self::Invalid(_) | Self::Weight { .. } | Self::Argument(_) => {
                None
            }
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

/// What a [`Plan`] handed to kernel bodies: [`Plan::work`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Work {
    /// Lists, one per row handed out.
    pub lists: u64,
    /// Their entries: the edges traversed.
    pub entries: u64,
}

/// How a [`Plan`] cuts and reads. None of it changes a result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Entries per parallel task, about. A task is at least one reduction block.
    pub chunk_entries: u64,
    /// Entries per decoded or read batch, about. A batch holds at least one list.
    pub batch_entries: usize,
    /// Advise the source of a frontier's byte ranges before it reads them: for cold or
    /// out-of-core storage. On a warm memory map it costs system calls for nothing.
    pub prefetch: bool,
}

impl Default for Settings {
    /// Tasks of 2^16 entries and batches of 8,192 entries (FINDINGS § M1-13); no
    /// prefetch.
    fn default() -> Self {
        Self {
            chunk_entries: 1 << 16,
            batch_entries: 8192,
            prefetch: false,
        }
    }
}

/// A value the drivers add up in a fixed order.
pub trait Reduce: Send + Sized {
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

impl Reduce for ExactSum {
    const ZERO: Self = Self::new();
    fn add(mut self, other: Self) -> Self {
        self.merge(&other);
        self
    }
}

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
    /// The part of the first `mid` nodes, and the rest.
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

/// One node and its list, as a body receives them.
#[derive(Clone, Copy, Debug)]
pub struct Row<'r, 'g> {
    /// The node.
    pub node: Node<'g>,
    /// Its list.
    pub list: &'r [Node<'g>],
    /// The entries before the list, in node order.
    pub first_entry: u64,
}

impl Row<'_, '_> {
    /// The positions of the list's entries in a per-entry column (one value per entry,
    /// in node order): `&weights[row.entries()]` are the weights of the list.
    ///
    /// # Panics
    ///
    /// If the positions do not fit in memory on this target.
    #[must_use]
    pub fn entries(&self) -> Range<usize> {
        let first = ix(self.first_entry);
        first..first + self.list.len()
    }
}

/// What a body has besides its row: the kernel's per-worker scratch, and random access
/// to other lists. The two are separate fields, so a body can use both at once.
#[derive(Debug)]
pub struct Worker<'w, 'g, S: Source, W> {
    /// The scratch of this worker, made by the `init` of [`Plan::scan_with`] or
    /// [`Plan::frontier_with`] and kept from task to task of one call.
    pub scratch: &'w mut W,
    /// Random access to the lists of other nodes.
    pub lists: Lists<'w, 'g, S>,
}

/// Random access to lists from inside a body: [`Worker::lists`].
pub struct Lists<'w, 'g, S: Source> {
    reader: &'w mut Option<S::Reader>,
    /// What this task read, added to the plan's counters when the task ends: one shared
    /// counter per read would be taken by every worker at once.
    work: &'w mut Work,
    plan: &'w Plan<'w, 'g, S>,
}

impl<S: Source> fmt::Debug for Lists<'_, '_, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Lists").finish_non_exhaustive()
    }
}

impl<'g, S: Source> Lists<'_, 'g, S> {
    /// The row of `v`, by random access, while the body's own row stays valid. It is
    /// decoded or read into a second reader of this worker.
    ///
    /// # Errors
    ///
    /// The list could not be read or is corrupt, or the reader does not fit in the
    /// budget.
    pub fn get(&mut self, v: Node<'g>) -> Result<Row<'_, 'g>, Error> {
        let plan = self.plan;
        let reader = match self.reader {
            Some(r) => r,
            None => self.reader.insert(plan.new_reader()?),
        };
        let (first_entry, list) = plan.source.list(reader, v.get())?;
        self.work.lists += 1;
        self.work.entries += list.len() as u64;
        // SAFETY: the source returns lists of its nodes only (`Source`'s contract).
        let list = unsafe { as_nodes(list) };
        Ok(Row {
            node: v,
            list,
            first_entry,
        })
    }
}

/// A reader of the pool, and the second reader for [`Lists::get`].
struct Slot<R> {
    reader: R,
    side: Option<R>,
}

/// A source with its work partition and its pool of readers: what the drivers run on.
/// Made by [`Plan::with`]; `'g` is its brand (see [`Node`]).
pub struct Plan<'s, 'g, S: Source> {
    source: &'s S,
    brand: Brand<'g>,
    block: u32,
    /// Parallel tasks: whole blocks, in order, covering every node.
    tasks: Vec<Range<u32>>,
    batch: usize,
    /// Lists per call of a reader, at most: a block, or a frontier run.
    lists: usize,
    prefetch: bool,
    /// Readers that no task holds. A task takes one or makes one, and gives it back.
    pool: Mutex<Vec<Slot<S::Reader>>>,
    memory: Memory,
    /// [`Work`]: lists, entries.
    work: [AtomicU64; 2],
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

/// Takes a value from `pool`, or makes one.
fn take<T>(pool: &Mutex<Vec<T>>, make: impl FnOnce() -> T) -> T {
    let taken = pool.lock().unwrap_or_else(PoisonError::into_inner).pop();
    taken.unwrap_or_else(make)
}

/// Gives a value back to `pool`.
fn give<T>(pool: &Mutex<Vec<T>>, value: T) {
    pool.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(value);
}

impl<'s, S: Source> Plan<'s, '_, S> {
    /// Cuts `source` into tasks and calls `f` with the plan. Readers are made when they
    /// are first needed, each reserved against `memory`.
    ///
    /// The plan, and every [`Node`] and per-node state made from it, exist only inside
    /// `f`. Their brand `'g` is new for each call, so the nodes of one plan cannot index
    /// the state of another. This compiles, inside one plan:
    ///
    /// ```
    /// # use std::sync::Mutex;
    /// # use std::sync::atomic::Ordering::Relaxed;
    /// # use everygraph_core::Memory;
    /// # use everygraph_kernels::edge_map::{Csr, NodeList, Plan, Settings};
    /// # let (offsets, targets, values) = ([0_u64, 0], [0_u32; 0], [1.0_f64]);
    /// # let csr = Csr::new(&offsets, &targets).unwrap();
    /// # let m = Memory::unlimited();
    /// # let leak = Mutex::new(None);
    /// let id = Plan::with(&csr, &m, Settings::default(), |a| {
    ///     let v = a.node(0).unwrap();
    ///     assert_eq!(a.per_node(&values).unwrap()[v], 1.0);
    ///     assert!(a.node_set(&m).unwrap().insert(v));
    ///     a.node_cells(&m).unwrap().store(v, v, Relaxed);
    ///     let mut list = NodeList::new(&m);
    ///     list.push(v).unwrap();
    ///     a.frontier(list.as_slice().unwrap(), |(): &mut (), _, _| ()).unwrap();
    ///     *leak.lock().unwrap() = Some(v.get());
    ///     v.get()
    /// });
    /// # assert_eq!(id, 0);
    /// ```
    ///
    /// Each of the following uses a node of plan `a` in plan `b`, or outside its plan,
    /// and does not compile:
    ///
    /// ```compile_fail,E0521
    /// # use everygraph_core::Memory;
    /// # use everygraph_kernels::edge_map::{Csr, Plan, Settings};
    /// # let (offsets, targets, values) = ([0_u64, 0], [0_u32; 0], [1.0_f64]);
    /// # let csr = Csr::new(&offsets, &targets).unwrap();
    /// # let m = Memory::unlimited();
    /// Plan::with(&csr, &m, Settings::default(), |a| {
    ///     let v = a.node(0).unwrap();
    ///     Plan::with(&csr, &m, Settings::default(), |b| {
    ///         let _ = b.per_node(&values).unwrap()[v];
    ///     });
    /// });
    /// ```
    ///
    /// ```compile_fail,E0521
    /// # use everygraph_core::Memory;
    /// # use everygraph_kernels::edge_map::{Csr, Plan, Settings};
    /// # let (offsets, targets) = ([0_u64, 0], [0_u32; 0]);
    /// # let csr = Csr::new(&offsets, &targets).unwrap();
    /// # let m = Memory::unlimited();
    /// Plan::with(&csr, &m, Settings::default(), |a| {
    ///     let v = a.node(0).unwrap();
    ///     Plan::with(&csr, &m, Settings::default(), |b| {
    ///         b.node_set(&m).unwrap().insert(v);
    ///     });
    /// });
    /// ```
    ///
    /// ```compile_fail,E0521
    /// # use std::sync::atomic::Ordering::Relaxed;
    /// # use everygraph_core::Memory;
    /// # use everygraph_kernels::edge_map::{Csr, Plan, Settings};
    /// # let (offsets, targets) = ([0_u64, 0], [0_u32; 0]);
    /// # let csr = Csr::new(&offsets, &targets).unwrap();
    /// # let m = Memory::unlimited();
    /// Plan::with(&csr, &m, Settings::default(), |a| {
    ///     let v = a.node(0).unwrap();
    ///     Plan::with(&csr, &m, Settings::default(), |b| {
    ///         b.node_cells(&m).unwrap().store(v, v, Relaxed);
    ///     });
    /// });
    /// ```
    ///
    /// ```compile_fail,E0521
    /// # use everygraph_core::Memory;
    /// # use everygraph_kernels::edge_map::{Csr, NodeList, Plan, Settings};
    /// # let (offsets, targets) = ([0_u64, 0], [0_u32; 0]);
    /// # let csr = Csr::new(&offsets, &targets).unwrap();
    /// # let m = Memory::unlimited();
    /// Plan::with(&csr, &m, Settings::default(), |a| {
    ///     let mut list = NodeList::new(&m);
    ///     list.push(a.node(0).unwrap()).unwrap();
    ///     Plan::with(&csr, &m, Settings::default(), |b| {
    ///         b.frontier(list.as_slice().unwrap(), |(): &mut (), _, _| ()).unwrap();
    ///     });
    /// });
    /// ```
    ///
    /// ```compile_fail,E0521
    /// # use std::sync::Mutex;
    /// # use everygraph_core::Memory;
    /// # use everygraph_kernels::edge_map::{Csr, Plan, Settings};
    /// # let (offsets, targets) = ([0_u64, 0], [0_u32; 0]);
    /// # let csr = Csr::new(&offsets, &targets).unwrap();
    /// # let m = Memory::unlimited();
    /// # let leak = Mutex::new(None);
    /// Plan::with(&csr, &m, Settings::default(), |a| {
    ///     *leak.lock().unwrap() = Some(a.node(0).unwrap());
    /// });
    /// ```
    ///
    /// ```compile_fail
    /// # use everygraph_core::Memory;
    /// # use everygraph_kernels::edge_map::{Csr, Plan, Settings};
    /// # let (offsets, targets) = ([0_u64, 0], [0_u32; 0]);
    /// # let csr = Csr::new(&offsets, &targets).unwrap();
    /// # let m = Memory::unlimited();
    /// let v = Plan::with(&csr, &m, Settings::default(), |a| a.node(0).unwrap());
    /// ```
    pub fn with<R>(
        source: &'s S,
        memory: &Memory,
        settings: Settings,
        f: impl for<'g> FnOnce(Plan<'s, 'g, S>) -> R,
    ) -> R {
        f(Plan::new(source, memory, settings))
    }

    /// As [`with`](Self::with), with two sources over the same nodes, such as the out-
    /// and the in-adjacency of a directed graph. The two plans share the brand `'g`, so
    /// a node of one indexes the state of the other.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] if the two sources do not have the same node count.
    pub fn with_pair<T: Source, R>(
        source: &'s S,
        other: &'s T,
        memory: &Memory,
        settings: Settings,
        f: impl for<'g> FnOnce(Plan<'s, 'g, S>, Plan<'s, 'g, T>) -> R,
    ) -> Result<R, Error> {
        if source.nodes() != other.nodes() {
            return Err(Error::Invalid("two adjacencies with different node counts"));
        }
        Ok(f(
            Plan::new(source, memory, settings),
            Plan::new(other, memory, settings),
        ))
    }
}

impl<'s, 'g, S: Source> Plan<'s, 'g, S> {
    fn new(source: &'s S, memory: &Memory, settings: Settings) -> Self {
        let n = source.nodes();
        let block = block_nodes(n);
        let mut tasks = Vec::new();
        let mut start = 0;
        while start < n {
            let limit = source
                .entry_offset(start)
                .saturating_add(settings.chunk_entries.max(1));
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
            lists: ix(block.into()).max(FRONTIER_GRAIN),
            prefetch: settings.prefetch,
            pool: Mutex::new(Vec::new()),
            memory: memory.clone(),
            work: [AtomicU64::new(0), AtomicU64::new(0)],
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

    /// Nodes per reduction block, a function of the node count alone.
    #[must_use]
    pub fn block_nodes(&self) -> u32 {
        self.block
    }

    /// What the drivers and [`Lists::get`] handed to bodies so far (module
    /// documentation, "Work counters").
    #[must_use]
    pub fn work(&self) -> Work {
        Work {
            lists: self.work[0].load(Ordering::Relaxed),
            entries: self.work[1].load(Ordering::Relaxed),
        }
    }

    fn count(&self, lists: u64, entries: u64) {
        self.work[0].fetch_add(lists, Ordering::Relaxed);
        self.work[1].fetch_add(entries, Ordering::Relaxed);
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

    fn new_reader(&self) -> Result<S::Reader, Error> {
        self.source.reader(&self.memory, self.batch, self.lists)
    }

    /// Runs `f` with a slot of the pool, or a new one, and gives it back.
    fn with_slot<R>(
        &self,
        f: impl FnOnce(&mut Slot<S::Reader>) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let taken = self
            .pool
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop();
        let mut slot = match taken {
            Some(slot) => slot,
            None => Slot {
                reader: self.new_reader()?,
                side: None,
            },
        };
        let result = f(&mut slot);
        self.source.idle(&mut slot.reader);
        if let Some(side) = &mut slot.side {
            self.source.idle(side);
        }
        give(&self.pool, slot);
        result
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
        let blocks = ix(self.nodes().div_ceil(self.block).into());
        // An accumulator can be large (two `ExactSum`s are 1.2 KB), and below 2^18
        // nodes a block is 64 of them.
        let _held = self.memory.reserve(
            "edge-map partial values",
            MemoryKind::Anonymous,
            (blocks as u64).saturating_mul(size_of::<A>() as u64),
        )?;
        let mut partials: Vec<A> = (0..blocks).map(|_| A::ZERO).collect();
        let mut pieces = Vec::new();
        let (mut out, mut rest) = (out, partials.as_mut_slice());
        for r in &self.tasks {
            let (mine, others) = out.split_at(ix((r.end - r.start).into()));
            let sums = ix((r.end - r.start).div_ceil(self.block).into());
            let (sums, more) = rest.split_at_mut(sums);
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

    /// [`scan_with`](Self::scan_with) without scratch.
    ///
    /// # Errors
    ///
    /// As [`scan_with`](Self::scan_with).
    pub fn scan<O, A>(
        &self,
        out: O,
        filter: impl Fn(Node<'g>) -> bool + Sync,
        body: impl Fn(&mut A, &mut O, usize, Row<'_, 'g>, &mut Worker<'_, 'g, S, ()>) + Sync,
    ) -> Result<A, Error>
    where
        O: Split,
        A: Reduce,
    {
        self.scan_with(out, filter, || (), body)
    }

    /// Calls `body(acc, out, i, row, worker)` for every node with `filter(node)`, where
    /// `row` holds the node and its list, `out` is the task's part of the outputs, `i`
    /// the node's index in it, and `worker` the worker's scratch (made by `init`) and its
    /// random access. Returns the sum of the reduction blocks' `acc`.
    ///
    /// `filter` is called once or twice per node: sources that decode or read (all but
    /// [`Csr`]) first check that it passes some node of a reduction block, and skip the
    /// block when it passes none. It may read state that the body changes, if the
    /// kernel's answer does not depend on when it sees the change.
    ///
    /// # Errors
    ///
    /// The lists could not be read or are corrupt, or a reader does not fit in the
    /// budget. The outputs are then partly written.
    pub fn scan_with<O, A, W>(
        &self,
        out: O,
        filter: impl Fn(Node<'g>) -> bool + Sync,
        init: impl Fn() -> W + Sync,
        body: impl Fn(&mut A, &mut O, usize, Row<'_, 'g>, &mut Worker<'_, 'g, S, W>) + Sync,
    ) -> Result<A, Error>
    where
        O: Split,
        A: Reduce,
        W: Send,
    {
        let scratch = Mutex::new(Vec::new());
        self.run(out, |task, mut out, sums| {
            let mut w = take(&scratch, &init);
            let result = self.with_slot(|Slot { reader, side }| {
                let (mut lists, mut entries, mut work) = (0, 0, Work::default());
                for (b, sum) in self.blocks(task.clone()).zip(sums) {
                    let mut acc = A::ZERO;
                    let filter = |v| {
                        // SAFETY: the source asks only about nodes of `b` (`Source`'s
                        // contract), which is within the plan's nodes.
                        filter(unsafe { Node::new_unchecked(v) })
                    };
                    self.source
                        .scan(reader, b, filter, |v, first_entry, list| {
                            // SAFETY: `v` is in `b` (`Source`'s contract).
                            let node = unsafe { Node::new_unchecked(v) };
                            // SAFETY: the source hands out lists of its nodes only.
                            let list = unsafe { as_nodes(list) };
                            lists += 1;
                            entries += list.len() as u64;
                            let row = Row {
                                node,
                                list,
                                first_entry,
                            };
                            let mut worker = Worker {
                                scratch: &mut w,
                                lists: Lists {
                                    reader: &mut *side,
                                    work: &mut work,
                                    plan: self,
                                },
                            };
                            let i = ix((v - task.start).into());
                            body(&mut acc, &mut out, i, row, &mut worker);
                        })?;
                    *sum = acc;
                }
                self.count(lists + work.lists, entries + work.entries);
                Ok(())
            });
            give(&scratch, w);
            result
        })
    }

    /// Calls `body(acc, out, i, v, degree)` for every node `v`; as [`scan`](Self::scan)
    /// otherwise, without lists.
    ///
    /// # Errors
    ///
    /// None from the sources of this crate. The signature leaves room for a source that
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
                    // SAFETY: `v` is in `b` (`Source`'s contract), within the nodes.
                    let node = unsafe { Node::new_unchecked(v) };
                    body(&mut acc, &mut out, ix((v - task.start).into()), node, d);
                });
                *sum = acc;
            }
            Ok(())
        })
    }

    /// [`frontier_with`](Self::frontier_with) without scratch.
    ///
    /// # Errors
    ///
    /// As [`frontier_with`](Self::frontier_with).
    pub fn frontier<A>(
        &self,
        frontier: &[Node<'g>],
        body: impl Fn(&mut A, Row<'_, 'g>, &mut Worker<'_, 'g, S, ()>) + Sync,
    ) -> Result<A, Error>
    where
        A: Reduce,
    {
        self.frontier_with(frontier, || (), body)
    }

    /// Calls `body(acc, row, worker)` for each node of `frontier`, with its list, decoded
    /// by random access. Returns the sum of the `acc` of each run of [`FRONTIER_GRAIN`]
    /// nodes, in order. A frontier of one run is done on the calling thread, so a level
    /// of a traversal with many small levels costs no fork-join.
    ///
    /// `frontier` should be increasing (a [`NodeList`] made by [`NodeSet::list_into`]):
    /// then consecutive nodes are decoded together. That is needed for speed only: a
    /// frontier in any order is still read correctly and safely.
    ///
    /// # Errors
    ///
    /// As [`scan_with`](Self::scan_with).
    pub fn frontier_with<A, W>(
        &self,
        frontier: &[Node<'g>],
        init: impl Fn() -> W + Sync,
        body: impl Fn(&mut A, Row<'_, 'g>, &mut Worker<'_, 'g, S, W>) + Sync,
    ) -> Result<A, Error>
    where
        A: Reduce,
        W: Send,
    {
        let frontier = ids(frontier);
        let scratch = Mutex::new(Vec::new());
        let run = |r: Range<usize>| {
            let mut w = take(&scratch, &init);
            let result = self.with_slot(|Slot { reader, side }| {
                let nodes = &frontier[r];
                if self.prefetch {
                    self.source.prefetch(nodes);
                }
                let (mut acc, mut entries, mut work) = (A::ZERO, 0, Work::default());
                self.source.lists(reader, nodes, |v, first_entry, list| {
                    // SAFETY: `v` is a node of the frontier (`Source`'s contract).
                    let node = unsafe { Node::new_unchecked(v) };
                    // SAFETY: the source hands out lists of its nodes only.
                    let list = unsafe { as_nodes(list) };
                    entries += list.len() as u64;
                    let row = Row {
                        node,
                        list,
                        first_entry,
                    };
                    let mut worker = Worker {
                        scratch: &mut w,
                        lists: Lists {
                            reader: &mut *side,
                            work: &mut work,
                            plan: self,
                        },
                    };
                    body(&mut acc, row, &mut worker);
                })?;
                self.count(nodes.len() as u64 + work.lists, entries + work.entries);
                Ok(acc)
            });
            give(&scratch, w);
            result
        };
        if frontier.len() <= FRONTIER_GRAIN {
            return run(0..frontier.len());
        }
        // One result per run, and an accumulator can be large (Leiden's).
        let runs = frontier.len().div_ceil(GRAIN.get()) as u64;
        let _held = self.memory.reserve(
            "edge-map frontier results",
            MemoryKind::Anonymous,
            runs.saturating_mul(size_of::<Result<A, Error>>() as u64),
        )?;
        par::map_ranges(frontier.len(), GRAIN, run)
            .into_iter()
            .try_fold(A::ZERO, |sum, part| Ok(sum.add(part?)))
    }

    /// The reduction blocks of `task`.
    fn blocks(&self, task: Range<u32>) -> impl Iterator<Item = Range<u32>> + use<S> {
        let block = self.block;
        (task.start..task.end)
            .step_by(ix(block.into()))
            .map(move |s| s..s.saturating_add(block).min(task.end))
    }
}

#[cfg(test)]
mod tests {
    use everygraph_core::MemorySource;
    use everygraph_format::adjacency::{Adjacency, AdjacencyIndex, encode};

    use super::*;

    #[test]
    fn blocks_follow_the_node_count_only() {
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
        let targets: Vec<u32> = (0..offsets[ix(n.into())])
            .map(|e| u32::try_from(e % 1000).unwrap())
            .collect();
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

    /// A small graph: node `v` lists `v + 1 .. v + 1 + v % 4` (mod `n`), and node `bad`
    /// lists `n + 5` if `bad` is given.
    fn lists(n: u32, bad: Option<u32>) -> (Vec<u64>, Vec<u32>) {
        let (mut offsets, mut targets) = (vec![0], Vec::new());
        for v in 0..n {
            if bad == Some(v) {
                targets.push(n + 5);
            } else {
                let mut list: Vec<u32> = (1..=v % 4).map(|k| (v + k) % n).collect();
                list.sort_unstable();
                list.dedup();
                targets.extend(list);
            }
            offsets.push(targets.len() as u64);
        }
        (offsets, targets)
    }

    /// Every driver, over every source built in memory: each target indexes a
    /// `PerNode`, so Miri sees every unchecked read the drivers allow.
    fn drive<S: Source>(source: &S, offsets: &[u64]) -> Result<(u64, u64, u64), Error> {
        let memory = Memory::unlimited();
        let settings = Settings {
            chunk_entries: 16,
            batch_entries: 5,
            prefetch: true,
        };
        Plan::with(source, &memory, settings, |plan| {
            let values: Vec<u64> = (0..u64::from(plan.nodes())).collect();
            let per = plan.per_node(&values)?;
            let mut out: Vec<u64> = values.iter().map(|_| 0).collect();
            let scanned: u64 = plan.scan(&mut out[..], all, |acc: &mut u64, out, i, row, w| {
                out[i] = row.list.iter().map(|&u| per[u]).sum();
                *acc += out[i];
                if let Some(&u) = row.list.first() {
                    let side = w.lists.get(u).expect("a good list");
                    *acc += side.list.iter().map(|&t| per[t]).sum::<u64>();
                }
                assert_eq!(row.first_entry, offsets[row.node.index()]);
            })?;
            let frontier: Vec<Node<'_>> = plan.all_nodes().filter(|v| v.get() % 3 != 0).collect();
            let pushed: u64 = plan.frontier(&frontier, |acc: &mut u64, row, _| {
                *acc += row.list.iter().map(|&u| per[u]).sum::<u64>();
            })?;
            let degrees: u64 = plan.vertices((), |acc: &mut u64, (), _, v, d| {
                *acc += per[v] * u64::from(d);
            })?;
            Ok((scanned, pushed, degrees))
        })
    }

    #[test]
    fn drivers_over_every_source_in_memory() {
        let n = 70;
        let (offsets, targets) = lists(n, None);
        let encoded = encode(&offsets, &targets).unwrap();
        let index = AdjacencyIndex::parse(&encoded.index).unwrap();
        let bytes: Vec<u8> = targets.iter().flat_map(|t| t.to_le_bytes()).collect();
        let stream_bytes = MemorySource::new(bytes);
        let data = MemorySource::new(encoded.data.clone());
        let csr = drive(&Csr::new(&offsets, &targets).unwrap(), &offsets).unwrap();
        let heap = Adjacency::new(&index, &encoded.data).unwrap();
        assert_eq!(drive(&Compressed::new(heap), &offsets).unwrap(), csr);
        let mapped = Compressed::mapped(&index, &data).unwrap();
        assert_eq!(drive(&mapped, &offsets).unwrap(), csr);
        let stream = Stream::new(&offsets, &stream_bytes).unwrap();
        assert_eq!(drive(&stream, &offsets).unwrap(), csr);
    }

    /// One corrupt list: the scan and the frontier that reach it fail and hand out no
    /// list of it; a scan that skips its block, and a frontier without it, then succeed
    /// with the same reader.
    #[test]
    fn a_corrupt_list_is_refused_and_the_reader_stays_usable() {
        let (n, bad) = (70, 40);
        let (offsets, targets) = lists(n, Some(bad));
        let encoded = encode(&offsets, &targets).unwrap();
        let index = AdjacencyIndex::parse(&encoded.index).unwrap();
        let bytes: Vec<u8> = targets.iter().flat_map(|t| t.to_le_bytes()).collect();
        let stream_bytes = MemorySource::new(bytes);
        let heap = Compressed::new(Adjacency::new(&index, &encoded.data).unwrap());
        let stream = Stream::new(&offsets, &stream_bytes).unwrap();
        check_corrupt(&heap, bad);
        check_corrupt(&stream, bad);
    }

    fn check_corrupt<S: Source>(source: &S, bad: u32) {
        let memory = Memory::unlimited();
        Plan::with(source, &memory, Settings::default(), |plan| {
            let values: Vec<u8> = (0..plan.nodes()).map(|_| 0).collect();
            let per = plan.per_node(&values).unwrap();
            fn sum<'g>(per: PerNode<'_, 'g, u8>, acc: &mut u64, row: Row<'_, 'g>) {
                *acc += row.list.iter().map(|&u| u64::from(per[u])).sum::<u64>();
            }
            let good: Vec<Node<'_>> = (0..10).filter_map(|v| plan.node(v)).collect();
            let with_bad: Vec<Node<'_>> = (bad - 2..bad + 2).filter_map(|v| plan.node(v)).collect();
            for _ in 0..2 {
                assert!(
                    plan.frontier(&good, |a: &mut u64, r, _| sum(per, a, r))
                        .is_ok()
                );
                let e = plan
                    .frontier(&with_bad, |a: &mut u64, r, _| sum(per, a, r))
                    .unwrap_err();
                assert!(
                    matches!(e, Error::Target { node, .. } if node == bad),
                    "{e}"
                );
                let e = plan
                    .scan((), all, |a: &mut u64, (), _, r, _| sum(per, a, r))
                    .unwrap_err();
                assert!(
                    matches!(e, Error::Target { node, .. } if node == bad),
                    "{e}"
                );
                let far = |v: Node<'_>| v.get().abs_diff(bad) > 64;
                assert!(
                    plan.scan((), far, |a: &mut u64, (), _, r, _| sum(per, a, r))
                        .is_ok()
                );
            }
        });
    }
}
