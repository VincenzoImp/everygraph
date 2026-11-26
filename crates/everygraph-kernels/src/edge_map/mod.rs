// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The edge-map framework (DESIGN D12, ADR 0012). A kernel body is written once, as a
//! closure over one node and its list. It runs over every [`Source`]: raw arrays, the
//! compressed format in the heap or memory-mapped, the raw or the compressed format
//! streamed. It runs in many short calls.
//!
//! # Drivers
//!
//! A [`Plan`] binds a source to the work partition and to a pool of per-worker readers.
//! Its drivers call the body:
//!
//! - [`Plan::scan`] calls it for every node that passes a filter, with the node's list
//!   (pull, scan-shaped). Lists are decoded or read in batches of about
//!   [`Settings::batch_entries`] entries before the body runs on them. A compressed
//!   source decodes only the records of the nodes that pass.
//! - [`Plan::vertices`] calls it for every node, with its degree, without its list.
//! - [`Plan::scan_until`] is a scan whose body reads each list from its start, in
//!   pieces, and may stop early (bottom-up BFS stops at the first parent). A compressed
//!   source then decodes a prefix of each list, and the rest only when the body asks.
//! - [`Plan::frontier`] calls it for the nodes of a frontier, with their lists, by random
//!   access (push, traversal-shaped). Consecutive nodes are decoded together. If
//!   [`Settings::prefetch`] is set, the frontier's byte ranges are advised first.
//!   `Plan::push_with` may also cut a long list into parts that run in parallel, for
//!   bodies whose updates commute; [`Plan::frontier_serial_with`] runs on the calling
//!   thread.
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
//! - **Frontiers.** [`Plan::frontier`] cuts the frontier into groups of
//!   [`FRONTIER_GRAIN`] nodes, and each group into runs of about [`RUN_ENTRIES`] entries
//!   by the degrees of its nodes. `Plan::push_with` also makes a list of more than
//!   [`PART_ENTRIES`] entries a run of its own, cut into parts at multiples of
//!   [`PART_ENTRIES`]. The cuts depend
//!   on the frontier and the degrees alone; the values of parts, runs and groups add in
//!   frontier order.
//! - **Outputs.** The one task that owns a node writes its outputs.
//! - **Lists are whole**, except in the parts of `Plan::push_with` and the pieces of
//!   [`Plan::scan_until`]. A batch, a read buffer or a task never cuts a list. So a
//!   node's own sum over its list runs in one call of the body.
//! - **Floats.** A body's own sums start from `+0.0`; `Iterator::sum` starts from
//!   `-0.0`. A body with several lanes (the four of the `PageRank` gather)
//!   fixes their number in its code, not by target feature. No result goes through
//!   `f64::max`, which may return either zero. A pull and a push form of one kernel add
//!   in different orders, so they are not interchangeable.
//! - **Global scalars** (a dangling mass, a residual, a total) reduce into
//!   [`ExactSum`]: its value does not depend on how the values are grouped, so it does
//!   not rely on the reduction blocks either (`docs/research/sota-numerics.md`, R6).
//!   No float reduces through the blocks: there is no [`Reduce`] for `f32` or `f64`.
//!   The blocks are cut by node count and ids, which a view as a mask and the same
//!   view materialized do not share; integers and [`ExactSum`] give the same value
//!   either way (DESIGN D14).
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
//! The readers' buffers, the blocks' partial values, a frontier's runs and results, and
//! the copies and parts of its long lists are reserved against the plan's [`Memory`], and
//! a reader frees a buffer that a long list grew when it goes back to the pool. Some
//! small allocations are not counted: the task list (`O(n / block)`), the runs and
//! results of a frontier of at most 16 groups, and the pool itself (one entry per
//! worker). The no-OOM suite (`everygraph-memtest`) checks that what is not
//! counted does not grow with the graph.
//!
//! # `unsafe`
//!
//! This is the crate's `unsafe` module (CONTRIBUTING.md, "`unsafe`"). The brand and the
//! unchecked indexing are in `node.rs`. The drivers here turn the lists that a [`Source`]
//! guarantees into nodes. The sources state the guarantee in their `unsafe impl`.

#![allow(unsafe_code)]

mod buckets;
mod compressed_stream;
#[cfg(feature = "frontier-counters")]
pub mod counters;
mod node;
mod source;

use std::error::Error as StdError;
use std::fmt;
use std::num::NonZeroUsize;
use std::ops::{ControlFlow, Range};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use everygraph_core::{
    ExactSum, InsufficientBudget, Memory, MemoryError, MemoryKind, ReadError, Reservation, SegVec,
    par,
};
use everygraph_format::adjacency::{DecodeError, FormatError};

pub use buckets::Buckets;
pub use compressed_stream::{CompressedStream, WindowReader};
pub use node::{Node, NodeCells, NodeList, NodeSet, PerNode, ids};
pub use source::{Compressed, Csr, DecodeReader, RawCopy, Source, Stream, StreamReader};

use node::{Brand, as_nodes, node_range};

/// Nodes of a frontier per group of [`Plan::frontier`]: a group is cut into runs by
/// entries, and the frontier's partial reductions are taken over its runs.
pub const FRONTIER_GRAIN: usize = GRAIN.get();
const GRAIN: NonZeroUsize = NonZeroUsize::new(256).expect("non-zero");

/// Entries of a frontier run, about: a run of [`Plan::frontier`] ends before the node
/// that would take it past this many entries (a node counts its degree plus one). A run
/// is one task, so a body that does little per entry keeps about the work per task of a
/// group of [`FRONTIER_GRAIN`] nodes of degree 64.
pub const RUN_ENTRIES: usize = 1 << 14;

/// Entries of a part of a long list: `Plan::push_with` cuts a list of more entries into
/// parts at multiples of it. `PASGAL` cuts lists at 1,024 entries, Parallel-KCore at 128
/// (`docs/research/sota-review-2026-11.md`, S3).
pub const PART_ENTRIES: usize = PART.get();
const PART: NonZeroUsize = NonZeroUsize::new(1024).expect("non-zero");

/// Nodes of a frontier up to which its runs are cut on the calling thread.
const CUT_ALONE: usize = 16 * FRONTIER_GRAIN;

/// Entries of the first piece of a list in [`Plan::scan_until`], or all of them if the
/// list is shorter. A multiple of four, the codec's group of words.
pub const PREFIX: usize = 16;

/// How much longer each piece of [`Plan::scan_until`] after the first ends than the one
/// before: a piece decoded past the entry a body stops at is at most this many times
/// what it read.
const GROWTH: usize = 4;

/// Fewest nodes in a reduction block.
const MIN_BLOCK: u32 = 64;
/// Most nodes in a reduction block.
const MAX_BLOCK: u32 = 4096;
/// About this many blocks per graph, between the two bounds.
const BLOCKS: u32 = 4096;

/// Entries of the long lists that `Plan::push_with` holds copied out at a time, if the
/// longest list of the frontier is shorter: 256 KiB of nodes.
pub(crate) const WAVE: usize = 1 << 16;

/// The runs of a frontier ([`Plan::frontier_with`]).
struct Cuts {
    /// Positions in the frontier, in order.
    runs: Vec<Range<usize>>,
    /// The number of runs of each group, in order.
    groups: Vec<usize>,
    _held: Reservation,
}

/// How the frontier drivers run.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Runs in parallel, lists whole.
    Parallel,
    /// Runs and the parts of long lists in parallel.
    Parts,
    /// Runs on the calling thread, lists whole.
    Serial,
}

impl Mode {
    /// [`par::map_ranges`], or the same chunks in order on the calling thread.
    fn map_ranges<T: Send>(
        self,
        len: usize,
        grain: NonZeroUsize,
        f: impl Fn(Range<usize>) -> T + Send + Sync,
    ) -> Vec<T> {
        if self == Self::Serial {
            let g = grain.get();
            (0..len.div_ceil(g))
                .map(|i| f(i * g..((i + 1) * g).min(len)))
                .collect()
        } else {
            par::map_ranges(len, grain, f)
        }
    }
}

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
        /// The column: `weight`, or `out` and `inn` of a directed projection.
        column: &'static str,
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
            Self::Weight {
                column,
                entry,
                value,
            } => write!(
                f,
                "entry {entry} of `{column}` has weight {value}; a weight must be finite and \
                 not negative"
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

impl Reduce for Work {
    const ZERO: Self = Self {
        lists: 0,
        entries: 0,
    };
    fn add(self, other: Self) -> Self {
        Self {
            lists: self.lists + other.lists,
            entries: self.entries + other.entries,
        }
    }
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
    /// Run the tasks of every second scan in reverse order. Tasks run in any order
    /// already, so results do not change. On a stream larger than the page cache, the
    /// next scan then starts with the pages the last one read last, which are still
    /// cached (Grafu's "elevator" order, ASPLOS 2024).
    pub alternate: bool,
}

impl Default for Settings {
    /// Tasks of 2^16 entries and batches of 8,192 entries (FINDINGS § M1-13); no
    /// prefetch; scans in node order.
    fn default() -> Self {
        Self {
            chunk_entries: 1 << 16,
            batch_entries: 8192,
            prefetch: false,
            alternate: false,
        }
    }
}

/// A value the drivers add up in a fixed order: by reduction blocks, then blocks in
/// order (module documentation, "Determinism").
///
/// Implement it only for values whose sum does not depend on the grouping: integers,
/// [`ExactSum`], tuples of these. Never for a type that holds an `f32` or `f64`, a
/// newtype included. The blocks are cut by node count and ids, so a mask and the same
/// view materialized group the values differently, and a float sum would change its
/// bits (DESIGN D14). `tools/numerics_lint.py --check` refuses such an `impl` in a
/// product crate.
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
reduce_by_add!(u32: 0, u64: 0);

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
    alternate: bool,
    /// Scans run so far, for [`Settings::alternate`].
    scans: AtomicU64,
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
/// A value of no size (no scratch, the reader of [`Csr`]) is made, not pooled: frontier
/// runs are short, and every worker would take the one lock.
fn take<T>(pool: &Mutex<Vec<T>>, make: impl FnOnce() -> T) -> T {
    if size_of::<T>() == 0 {
        return make();
    }
    let taken = pool.lock().unwrap_or_else(PoisonError::into_inner).pop();
    taken.unwrap_or_else(make)
}

/// Gives a value back to `pool`.
fn give<T>(pool: &Mutex<Vec<T>>, value: T) {
    if size_of::<T>() == 0 {
        return;
    }
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
            alternate: settings.alternate,
            scans: AtomicU64::new(0),
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

    fn count(&self, work: Work) {
        #[cfg(feature = "frontier-counters")]
        counters::handed(work.entries);
        self.work[0].fetch_add(work.lists, Ordering::Relaxed);
        self.work[1].fetch_add(work.entries, Ordering::Relaxed);
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
        let taken = if size_of::<S::Reader>() == 0 {
            None
        } else {
            self.pool
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop()
        };
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
        if size_of::<S::Reader>() != 0 {
            give(&self.pool, slot);
        }
        result
    }

    /// Runs `task` on every task in parallel, each with its part of `out` and of the
    /// blocks' partial values, and adds the partial values in block order. With
    /// `backward`, the tasks are handed out from the last.
    fn run<O, A>(
        &self,
        out: O,
        backward: bool,
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
        if backward {
            pieces.reverse();
        }
        par::for_each_chunk_mut(&mut pieces, NonZeroUsize::MIN, |_, chunk| {
            for (r, out, sums, result) in chunk.iter_mut() {
                *result = task(r.clone(), out.take().expect("one call each"), sums);
            }
        });
        if backward {
            pieces.reverse();
        }
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
        let backward = self.alternate && self.scans.fetch_add(1, Ordering::Relaxed) % 2 == 1;
        self.run(out, backward, |task, mut out, sums| {
            let mut w = take(&scratch, &init);
            let result = self.with_slot(|Slot { reader, side }| {
                self.source.begin_task(reader, task.clone());
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
                self.count(work.add(Work { lists, entries }));
                #[cfg(feature = "frontier-counters")]
                counters::flush();
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
        self.run(out, false, |task, mut out, sums| {
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
    /// by random access. Returns the sum of the `acc` of each run, in frontier order
    /// (module documentation, "Frontiers"). A frontier of one run is done on the calling
    /// thread, so a level of a traversal with many small levels costs no fork-join.
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
        self.drive(frontier, Mode::Parallel, init, body)
    }

    /// As [`frontier_with`](Self::frontier_with), with the same value, but every run on
    /// the calling thread: for small frontiers whose kernel wants no fork-join and a
    /// memory peak that does not depend on the schedule.
    ///
    /// # Errors
    ///
    /// As [`scan_with`](Self::scan_with).
    pub fn frontier_serial_with<A, W>(
        &self,
        frontier: &[Node<'g>],
        init: impl Fn() -> W + Sync,
        body: impl Fn(&mut A, Row<'_, 'g>, &mut Worker<'_, 'g, S, W>) + Sync,
    ) -> Result<A, Error>
    where
        A: Reduce,
        W: Send,
    {
        self.drive(frontier, Mode::Serial, init, body)
    }

    /// As [`frontier_with`](Self::frontier_with), but a list of more than
    /// [`PART_ENTRIES`] entries is a run of its own, cut into parts at multiples of
    /// [`PART_ENTRIES`], and the
    /// parts run in parallel: `body` is called once per part, with the part as
    /// `row.list` and the part's first entry as `row.first_entry`, and with scratch and
    /// random access of its own. So that a frontier of a few hubs runs on every thread.
    ///
    /// A body may not assume that it is called once per node, that `row.list` is the
    /// whole list or its length the degree, or that its scratch holds what an earlier
    /// part of the same node left there. Its updates must commute (an atomic minimum,
    /// an atomic decrement), a write it makes once per node must give the same value
    /// from every part, and a value that another part of the same node changes may be
    /// read stale. k-core's peel and SSSP's relaxation are such bodies.
    ///
    /// # Errors
    ///
    /// As [`scan_with`](Self::scan_with).
    pub(crate) fn push_with<A, W>(
        &self,
        frontier: &[Node<'g>],
        init: impl Fn() -> W + Sync,
        body: impl Fn(&mut A, Row<'_, 'g>, &mut Worker<'_, 'g, S, W>) + Sync,
    ) -> Result<A, Error>
    where
        A: Reduce,
        W: Send,
    {
        self.drive(frontier, Mode::Parts, init, body)
    }

    /// The frontier drivers (module documentation, "Frontiers"). The drivers' own tasks
    /// that hold a reader slot run to their end and wait for no other task, so they take
    /// no more slots than there are workers. A body that forks may hold its slot while
    /// it waits, and its worker may then take a second one: k-core's exact count splits
    /// a hub's list inside its body, and the planner's two readers per k-core worker
    /// cover that depth. A run reads its lists and calls the body on each. In
    /// [`Mode::Parts`], a long list is a run of its own: it is copied out of a slot,
    /// which is given back, and its parts then take a slot each.
    fn drive<A, W>(
        &self,
        frontier: &[Node<'g>],
        mode: Mode,
        init: impl Fn() -> W + Sync,
        body: impl Fn(&mut A, Row<'_, 'g>, &mut Worker<'_, 'g, S, W>) + Sync,
    ) -> Result<A, Error>
    where
        A: Reduce,
        W: Send,
    {
        let frontier = ids(frontier);
        let scratch = Mutex::new(Vec::new());
        #[cfg(feature = "frontier-counters")]
        let tasks = Mutex::new(Vec::new());
        // One call of the body with a worker's scratch and a slot's side reader; returns
        // what the body read through `Lists::get`.
        let call = |acc: &mut A, row: Row<'_, 'g>, w: &mut W, side: &mut Option<S::Reader>| {
            let mut work = Work::ZERO;
            let mut worker = Worker {
                scratch: w,
                lists: Lists {
                    reader: side,
                    work: &mut work,
                    plan: self,
                },
            };
            body(acc, row, &mut worker);
            work
        };
        // A run: lists read by one reader, each handed to the body whole.
        let run = |nodes: &[u32]| {
            let mut w = take(&scratch, &init);
            let result = self.with_slot(|Slot { reader, side }| {
                if self.prefetch {
                    self.source.prefetch(nodes);
                }
                let mut acc = A::ZERO;
                let mut work = Work {
                    lists: nodes.len() as u64,
                    entries: 0,
                };
                #[cfg(feature = "frontier-counters")]
                let mut tally = counters::Run::default();
                self.source.lists(reader, nodes, |v, first_entry, list| {
                    // SAFETY: `v` is a node of the frontier (`Source`'s contract).
                    let node = unsafe { Node::new_unchecked(v) };
                    // SAFETY: the source hands out lists of its nodes only.
                    let list = unsafe { as_nodes(list) };
                    let row = Row {
                        node,
                        list,
                        first_entry,
                    };
                    let side_work = call(&mut acc, row, &mut w, side);
                    work.entries += list.len() as u64;
                    work = work.add(side_work);
                    #[cfg(feature = "frontier-counters")]
                    {
                        tally.entries += list.len() as u64 + side_work.entries;
                        tally.max_list = tally.max_list.max(list.len() as u64);
                        tally.max_chain = tally.max_chain.max(side_work.entries);
                        tally.side_lists += side_work.lists;
                    }
                })?;
                #[cfg(feature = "frontier-counters")]
                {
                    tally.draws = counters::flush();
                    tasks
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push(tally);
                }
                Ok((acc, work))
            });
            give(&scratch, w);
            result
        };
        // One part of a list that was copied out.
        let part = |node: Node<'g>, first_entry: u64, list: &[Node<'g>]| {
            let mut w = take(&scratch, &init);
            let result = self.with_slot(|Slot { side, .. }| {
                let mut acc = A::ZERO;
                let row = Row {
                    node,
                    list,
                    first_entry,
                };
                let work = call(&mut acc, row, &mut w, side);
                #[cfg(feature = "frontier-counters")]
                tasks
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(counters::Run {
                        entries: list.len() as u64 + work.entries,
                        max_list: list.len() as u64,
                        max_chain: work.entries,
                        side_lists: work.lists,
                        draws: counters::flush(),
                    });
                Ok((acc, work))
            });
            give(&scratch, w);
            result
        };
        let degree = |i: usize| ix(self.source.degree(frontier[i]).into());
        let long = |t: &Range<usize>| {
            mode == Mode::Parts && t.len() == 1 && degree(t.start) > PART_ENTRIES
        };
        let finish = |(acc, work): (A, Work)| {
            self.count(work);
            #[cfg(feature = "frontier-counters")]
            counters::call(
                frontier.len(),
                &tasks.lock().unwrap_or_else(PoisonError::into_inner)[..],
            );
            acc
        };
        // On the calling thread, the runs add as they end: nothing is held.
        if mode == Mode::Serial {
            let mut total = <(A, Work)>::ZERO;
            for start in (0..frontier.len()).step_by(FRONTIER_GRAIN) {
                let g = start..(start + FRONTIER_GRAIN).min(frontier.len());
                let mut group = <(A, Work)>::ZERO;
                for r in self.group_runs(frontier, g, mode) {
                    group = group.add(run(&frontier[r])?);
                }
                total = total.add(group);
            }
            return Ok(finish(total));
        }
        // A small frontier without long lists is cut on the calling thread and its runs
        // run as one fork, as many as there are; one run is done on the calling thread.
        // Its few cuts and results are not reserved (module documentation, "Memory").
        if frontier.len() <= CUT_ALONE {
            let runs: Vec<Range<usize>> = (0..frontier.len().max(1))
                .step_by(FRONTIER_GRAIN)
                .flat_map(|s| {
                    let g = s..(s + FRONTIER_GRAIN).min(frontier.len());
                    self.group_runs(frontier, g, mode)
                })
                .collect();
            if !runs.iter().any(long) {
                if let [one] = &runs[..] {
                    return Ok(finish(run(&frontier[one.clone()])?));
                }
                let done = mode.map_ranges(runs.len(), NonZeroUsize::MIN, |r| {
                    run(&frontier[runs[r.start].clone()])
                });
                let (mut total, mut group, mut at) = (<(A, Work)>::ZERO, <(A, Work)>::ZERO, 0);
                for (r, value) in runs.iter().zip(done) {
                    if r.start / FRONTIER_GRAIN != at {
                        total = total.add(std::mem::replace(&mut group, <(A, Work)>::ZERO));
                        at = r.start / FRONTIER_GRAIN;
                    }
                    group = group.add(value?);
                }
                return Ok(finish(total.add(group)));
            }
        }
        let cuts = self.cut(frontier, mode)?;
        let runs = &cuts.runs[..];
        let (acc, work) = {
            // A result per run, and the first error apart: the results stay small.
            let failed = Mutex::new(None);
            let keep = |r: Result<(A, Work), Error>| match r {
                Ok(x) => Some(x),
                Err(e) => {
                    let mut f = failed.lock().unwrap_or_else(PoisonError::into_inner);
                    f.get_or_insert(e);
                    None
                }
            };
            let _held = self.hold::<Option<(A, Work)>>(runs.len())?;
            let mut results: Vec<Option<(A, Work)>> = (0..runs.len()).map(|_| None).collect();
            let hubs: Vec<usize> = (0..runs.len()).filter(|&i| long(&runs[i])).collect();
            // The long lists, a wave at a time: copied out, then their parts in parallel,
            // those of the first wave with the other runs. A wave holds no more entries
            // than the longest of them, or `WAVE`, so the copies stay within one
            // reader's longest buffer.
            let most = hubs
                .iter()
                .map(|&h| degree(runs[h].start))
                .max()
                .unwrap_or(0)
                .max(WAVE);
            let (mut at, mut first) = (0, true);
            while first || at < hubs.len() {
                let (mut end, mut entries) = (at, 0);
                while end < hubs.len()
                    && (end == at || entries + degree(runs[hubs[end]].start) <= most)
                {
                    entries += degree(runs[hubs[end]].start);
                    end += 1;
                }
                let wave = &hubs[at..end];
                // The wave's lists in one buffer, each copied by a task of its own.
                let hub = |h: usize| frontier[runs[wave[h]].start];
                let mut ends = vec![0];
                for h in 0..wave.len() {
                    ends.push(ends[h] + ix(self.source.degree(hub(h)).into()));
                }
                let mut buffer = SegVec::new(&self.memory, "edge-map long lists");
                if let Some(&v) = wave.first() {
                    // SAFETY: a node of the frontier, to fill the buffer before the copies.
                    let any = unsafe { Node::new_unchecked(frontier[runs[v].start]) };
                    buffer.try_resize(ends[wave.len()], any)?;
                    par::for_each_range_mut(
                        one_segment_mut(&mut buffer)?,
                        &ends,
                        NonZeroUsize::MIN,
                        |h, out| {
                            let copied = self.with_slot(|Slot { reader, .. }| {
                                self.source.lists(reader, &[hub(h)], |_, _, list| {
                                    // SAFETY: the source hands out lists of its nodes only.
                                    out.copy_from_slice(unsafe { as_nodes(list) });
                                })
                            });
                            if let Err(e) = copied {
                                keep(Err(e));
                            }
                        },
                    );
                }
                if let Some(e) = failed.lock().unwrap_or_else(PoisonError::into_inner).take() {
                    return Err(e);
                }
                let buffer = one_segment(&buffer)?;
                let copies: Vec<(u64, &[Node<'g>])> = (0..wave.len())
                    .map(|h| {
                        (
                            self.source.entry_offset(hub(h)),
                            &buffer[ends[h]..ends[h + 1]],
                        )
                    })
                    .collect();
                let pieces: Vec<(usize, Range<usize>)> = copies
                    .iter()
                    .enumerate()
                    .flat_map(|(h, (_, list))| {
                        (0..list.len())
                            .step_by(PART_ENTRIES)
                            .map(move |s| (h, s..(s + PART_ENTRIES).min(list.len())))
                    })
                    .collect();
                let others = if first { runs.len() } else { 0 };
                let _parts = self.hold::<Option<(A, Work)>>(others + pieces.len())?;
                let done = mode.map_ranges(others + pieces.len(), NonZeroUsize::MIN, |r| {
                    let Some((h, range)) = r.start.checked_sub(others).map(|p| &pieces[p]) else {
                        let t = &runs[r.start];
                        return if long(t) {
                            None
                        } else {
                            keep(run(&frontier[t.clone()]))
                        };
                    };
                    let (first_entry, list) = &copies[*h];
                    // SAFETY: a node of the frontier.
                    let node = unsafe { Node::new_unchecked(hub(*h)) };
                    keep(part(
                        node,
                        first_entry + range.start as u64,
                        &list[range.clone()],
                    ))
                });
                let mut done = done.into_iter();
                for (i, r) in done.by_ref().take(others).enumerate() {
                    results[i] = r;
                }
                for (h, (_, list)) in copies.iter().enumerate() {
                    let whole = Work {
                        lists: 1,
                        entries: list.len() as u64,
                    };
                    let mut sum = Some((A::ZERO, whole));
                    for next in done.by_ref().take(list.len().div_ceil(PART_ENTRIES)) {
                        sum = sum.zip(next).map(|(s, x)| s.add(x));
                    }
                    results[wave[h]] = sum;
                }
                (at, first) = (end, false);
            }
            if let Some(e) = failed.into_inner().unwrap_or_else(PoisonError::into_inner) {
                return Err(e);
            }
            // Runs add in order within their group, the groups in order.
            let mut results = results.into_iter().map(|r| r.expect("every run is done"));
            let mut total = <(A, Work)>::ZERO;
            for &count in &cuts.groups {
                let group = results
                    .by_ref()
                    .take(count)
                    .fold(<(A, Work)>::ZERO, Reduce::add);
                total = total.add(group);
            }
            total
        };
        Ok(finish((acc, work)))
    }

    /// The runs of the nodes `g` of `frontier`, a group, as positions in the frontier
    /// ([`cut`](Self::cut)).
    fn group_runs<'a>(
        &'a self,
        frontier: &'a [u32],
        g: Range<usize>,
        mode: Mode,
    ) -> impl Iterator<Item = Range<usize>> + 'a {
        let (mut at, mut done) = (g.start, false);
        std::iter::from_fn(move || {
            if done {
                return None;
            }
            let (start, mut entries) = (at, 0);
            while at < g.end {
                let degree = ix(self.source.degree(frontier[at]).into());
                let long = mode == Mode::Parts && degree > PART_ENTRIES;
                if entries > 0 && (long || entries + degree + 1 > RUN_ENTRIES) {
                    break;
                }
                entries += degree + 1;
                at += 1;
                if long {
                    break;
                }
            }
            // An empty group is one empty run.
            done = at == g.end;
            Some(start..at)
        })
    }

    /// A reservation for `count` values of `T`: the results of parallel tasks.
    fn hold<T>(&self, count: usize) -> Result<Reservation, Error> {
        let bytes = (count as u64).saturating_mul(size_of::<T>() as u64);
        Ok(self
            .memory
            .reserve("edge-map frontier results", MemoryKind::Anonymous, bytes)?)
    }

    /// The runs of `frontier`, as ranges of positions in it, and the number of runs of
    /// each group of [`FRONTIER_GRAIN`] nodes. A run ends before the node that would take
    /// it past [`RUN_ENTRIES`] (a node counts its degree plus one), and holds at least one
    /// node; in [`Mode::Parts`], a node of more than [`PART_ENTRIES`] entries is a run of
    /// its own. The runs are counted first, then written into memory of that size.
    fn cut(&self, frontier: &[u32], mode: Mode) -> Result<Cuts, Error> {
        // A small frontier is cut on the calling thread: a fork costs more than its cuts.
        let how = if frontier.len() <= CUT_ALONE {
            Mode::Serial
        } else {
            Mode::Parallel
        };
        let group = |g: Range<usize>| g.start..g.end.min(frontier.len());
        let groups = how.map_ranges(frontier.len().max(1), GRAIN, |g| {
            self.group_runs(frontier, group(g), mode).count()
        });
        let count: usize = groups.iter().sum();
        let _held = self.hold::<Range<usize>>(count)?;
        // Each group's runs end where the index says: they are written where they go.
        let mut ends = SegVec::try_with_len(&self.memory, "edge-map frontier runs", count, 0_u32)?;
        let at: Vec<usize> = std::iter::once(0)
            .chain(groups.iter().scan(0, |sum, &c| {
                *sum += c;
                Some(*sum)
            }))
            .collect();
        let fill = |g: usize, out: &mut [u32]| {
            let start = g * FRONTIER_GRAIN;
            let runs = self.group_runs(frontier, group(start..start + FRONTIER_GRAIN), mode);
            for (end, r) in out.iter_mut().zip(runs) {
                *end = u32::try_from(r.end).expect("a position in the frontier");
            }
        };
        let out = one_segment_mut(&mut ends)?;
        if how == Mode::Serial {
            for g in 0..groups.len() {
                fill(g, &mut out[at[g]..at[g + 1]]);
            }
        } else {
            par::for_each_range_mut(out, &at, GRAIN, |g, out: &mut [u32]| fill(g, out));
        }
        let ends = one_segment(&ends)?;
        let start = |i: usize| if i == 0 { 0 } else { ix(ends[i - 1].into()) };
        let runs = (0..count).map(|i| start(i)..ix(ends[i].into())).collect();
        Ok(Cuts {
            runs,
            groups,
            _held,
        })
    }

    /// Calls `body(acc, out, i, row)` for every node with `filter(node)`, as
    /// [`scan`](Self::scan) does, but hands its list out in pieces, while `body` returns
    /// [`ControlFlow::Continue`]: the entries before [`PREFIX`], then those before
    /// `GROWTH × PREFIX`, `GROWTH² × PREFIX`, and so on. `row.list` is the piece and
    /// `row.first_entry` the piece's first entry. An empty list is one empty piece. The
    /// pieces are the same on every source.
    ///
    /// A compressed source decodes the first piece of each list in its batches, and each
    /// later piece, by random access, only when the body asks for it: for a body that
    /// stops early, as bottom-up BFS at the first parent.
    ///
    /// # Errors
    ///
    /// As [`scan_with`](Self::scan_with).
    pub fn scan_until<O, A>(
        &self,
        out: O,
        filter: impl Fn(Node<'g>) -> bool + Sync,
        body: impl Fn(&mut A, &mut O, usize, Row<'_, 'g>) -> ControlFlow<()> + Sync,
    ) -> Result<A, Error>
    where
        O: Split,
        A: Reduce,
    {
        let backward = self.alternate && self.scans.fetch_add(1, Ordering::Relaxed) % 2 == 1;
        self.run(out, backward, |task, mut out, sums| {
            self.with_slot(|Slot { reader, side }| {
                self.source.begin_task(reader, task.clone());
                let (mut work, mut failed) = (Work::ZERO, None);
                for (b, sum) in self.blocks(task.clone()).zip(sums) {
                    let mut acc = A::ZERO;
                    let filter = |v| {
                        // SAFETY: the source asks only about nodes of `b` (`Source`'s
                        // contract), which is within the plan's nodes.
                        filter(unsafe { Node::new_unchecked(v) })
                    };
                    let f = |v, first_entry, prefix: &[u32], degree: u32| {
                        if failed.is_some() {
                            return;
                        }
                        // SAFETY: `v` is in `b` (`Source`'s contract).
                        let node = unsafe { Node::new_unchecked(v) };
                        // SAFETY: the source hands out lists of its nodes only.
                        let have = unsafe { as_nodes(prefix) };
                        let degree = ix(degree.into());
                        let i = ix((v - task.start).into());
                        work.lists += 1;
                        let (mut at, mut prev) = (0, 0);
                        loop {
                            let to = degree.min(if at == 0 { PREFIX } else { at * GROWTH });
                            let piece = if to <= have.len() {
                                &have[at..to]
                            } else {
                                let side = match side {
                                    Some(r) => Ok(r),
                                    None => self.new_reader().map(|r| side.insert(r)),
                                };
                                match side.and_then(|r| self.source.list_part(r, v, (at, prev), to))
                                {
                                    // SAFETY: as above.
                                    Ok(part) => unsafe { as_nodes(part) },
                                    Err(e) => {
                                        failed = Some(e);
                                        return;
                                    }
                                }
                            };
                            work.entries += piece.len() as u64;
                            let row = Row {
                                node,
                                list: piece,
                                first_entry: first_entry + at as u64,
                            };
                            let stop = body(&mut acc, &mut out, i, row).is_break();
                            match piece.last() {
                                Some(last) if !stop && to < degree => (at, prev) = (to, last.get()),
                                _ => return,
                            }
                        }
                    };
                    self.source.scan_prefixes(reader, b, filter, PREFIX, f)?;
                    *sum = acc;
                }
                self.count(work);
                #[cfg(feature = "frontier-counters")]
                counters::flush();
                failed.map_or(Ok(()), Err)
            })
        })
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
mod tests;
