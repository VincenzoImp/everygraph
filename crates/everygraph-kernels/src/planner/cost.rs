// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The cost model: the memory each operation of [`crate::run`] reserves, and its time.
//!
//! **Memory** is an upper bound of what the run reserves against its budget, read from
//! the code of each kernel and of `run::run`, term by term (below), and of what the open
//! graph file holds beside it ([`Graph::held`](super::Graph::held)). `run`'s operations
//! read no events column, so `everygraph_ingest::graph::Graph::events` (twice its
//! section) is not in the bound. Every per-node
//! array is exact. A frontier is bounded by the nodes it can hold, with the moment its
//! buffer moves (`SegVec`'s growth keeps the old buffer and the new one). The readers'
//! buffers are the edge-map framework's decode batches, one per worker. The bound is
//! checked against `Budget::peak` on the oracle corpus (`tests/planner.rs`). On lj and
//! orkut at 16 threads the runs reserve 0.86–0.99 of it for `PageRank`, BFS and degrees,
//! 0.65–0.66 for components, whose bound holds a root for every node, and 0.59–0.67 for
//! k-core, whose buckets and frontiers are bounded by every node at once.
//!
//! **Time** is per adjacency entry, from the measurements named at each constant, scaled
//! to the thread count by the speedup measured from 1 to 16 threads.

use std::num::NonZeroUsize;
use std::ops::Range;

use everygraph_core::{
    AccessHint, ExactSum, MAX_ALLOCATION_BYTES, Storage, StorageKind, SystemMemory, math,
};
use everygraph_format::adjacency::Batch;

use super::{Estimate, Graph, Regime, Tier};
use crate::edge_map::{
    CompressedStream, Error, FRONTIER_GRAIN, PART_ENTRIES, RUN_ENTRIES, RawCopy, Settings, WAVE,
    Work,
};
use crate::run::{Operation, Precision};

// ---- The constants of the model, with their sources ------------------------------
//
// "Exploratory" rows: `edge_map_bench` (this crate's example) on orkut (234,370,166
// entries), 16 threads unless stated, compressed data part memory-mapped and warm, raw
// CSR in the heap; interleaved A/B rounds, the median of each round's best of three;
// 2026-11-26 on the lab machine (Ryzen 9 7950X) while other work ran (1-minute load 8
// to 26). They are not for publication: the official run of the `planner-*`
// benchmarks in `bench/specs/edge-map.toml` replaces them.

/// Threads at which the per-entry times below were taken.
const REFERENCE_THREADS: f64 = 16.0;

/// Seconds per entry and iteration of `PageRank` (f64), compressed: 20 iterations in
/// 0.590 s (exploratory). f32 state is taken as the same.
const PAGERANK_PER_ENTRY: f64 = 0.126e-9;
/// Speedup of `PageRank` from 1 to 16 threads: 2.71 s against 0.215 s, 5 iterations
/// (exploratory).
const PAGERANK_SPEEDUP: f64 = 12.6;
/// Iterations `PageRank` runs to an L1 change below 1e-6 when it may stop early: 45 on
/// orkut and 46 on lj (a count, which load does not change; `tests/planner.rs`,
/// `the_plan_bounds_what_runs_reserve_on_real_graphs`).
const PAGERANK_ITERATIONS: u32 = 45;
/// Seconds per entry of the components kernel with its numbering, compressed: 0.0207 s
/// (exploratory).
const COMPONENTS_PER_ENTRY: f64 = 0.089e-9;
/// Speedup of components from 1 to 16 threads: 0.256 s against 0.0256 s (exploratory).
const COMPONENTS_SPEEDUP: f64 = 10.0;
/// Seconds per entry of core numbers, compressed: 0.825 s (exploratory).
const KCORE_PER_ENTRY: f64 = 3.52e-9;
/// Speedup of core numbers from 1 to 16 threads: 3.08 s against 0.795 s (exploratory).
const KCORE_SPEEDUP: f64 = 3.9;
/// Seconds per entry of BFS with parents on the compressed lists (0.135 s), on the raw
/// copy (0.106 s), and of `RawCopy::decode` with its new arrays (0.124 s): the medians
/// of five rounds that run the three in turn (exploratory; PR #249 holds the command
/// and its output). The ratio of these medians gives a break-even of 5 traversals; the
/// median of the rounds' own ratios is 3.79 on orkut (2.3–5.6) and 2.12 on lj
/// (1.8–3.1), so 5 decodes later, not earlier, than the rounds suggest.
const BFS_PER_ENTRY: f64 = 0.577e-9;
const BFS_RAW_PER_ENTRY: f64 = 0.45e-9;
const DECODE_PER_ENTRY: f64 = 0.53e-9;
/// Speedup of BFS with parents from 1 to 16 threads: 1.21 s against 0.116 s
/// (exploratory).
const BFS_SPEEDUP: f64 = 10.4;
/// Speedup of the decode from 1 to 16 threads: 0.167 s against 0.046 s, into arrays
/// already allocated (exploratory).
const DECODE_SPEEDUP: f64 = 3.6;
/// Sequential read rate of the lab's `NVMe` drive, bytes per second (FINDINGS § B1:
/// Friendster streamed at 3.9 GB/s). For other storage, placeholders of the model, not
/// measurements.
const NVME_BYTES_PER_SECOND: f64 = 3.9e9;
const SSD_BYTES_PER_SECOND: f64 = 0.5e9;
const OTHER_BYTES_PER_SECOND: f64 = 0.1e9;
/// An estimate's interval: `value / SPREAD` to `value × SPREAD`. A placeholder until the
/// harness measures the coverage of P-7.
const SPREAD: f64 = 2.0;

/// The default budget where free memory is not known (placeholder, as the CLI's).
const UNKNOWN_FREE_BUDGET: u64 = 8_000_000_000;
/// The raw copy is built only if it and the state take at most this share of the
/// budget, so that the rest of the session keeps room: a margin chosen, not measured.
const RAW_SHARE: (u64, u64) = (9, 10);
/// The most a node list of a small graph holds beyond its bound in nodes: its first
/// buffer of 4 ids (`SegVec`'s `MIN_GROWTH`), beside the next while that one is made.
const SMALLEST_LIST: u64 = 2 * 4 * 4;
/// A degree at which k-core samples its hubs and holds two more arrays (`kcore.rs`,
/// `Sampler::min_degree`).
const KCORE_SAMPLED_DEGREE: u32 = 20_000;

// ---- Memory -------------------------------------------------------------------------

/// How a kernel reads its lists (ADR 0029).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Access {
    /// Whole passes in node order.
    Scan,
    /// Frontiers by random access, and scans.
    Mixed,
}

/// The memory of one operation on one graph, its phase and how it reads.
#[derive(Clone, Copy, Debug)]
pub(super) struct Cost {
    /// The phase's name.
    pub phase: &'static str,
    /// How it reads.
    pub access: Access,
    /// Nodes.
    pub nodes: u64,
    /// Threads it runs on.
    pub threads: NonZeroUsize,
    /// Bytes per node of the per-node arrays and their bounded frontiers.
    pub per_node: u64,
    /// Bytes besides, that grow with the nodes: node sets, a few constants.
    pub sets: u64,
    /// The readers' buffers (P-14's fixed buffers).
    pub buffers: u64,
    /// The readers at once that `buffers` holds.
    pub readers: u64,
    /// The raw copy of the adjacency, decoded into the budget.
    pub raw: u64,
    /// Bytes of the result kept in the graph file.
    pub output: u64,
}

impl Cost {
    /// The cost of `op` on `graph` on `threads` threads, reading `tier`.
    pub fn of(op: &Operation, graph: &Graph, threads: NonZeroUsize, tier: Tier) -> Self {
        let n = u64::from(graph.nodes);
        let set = node_set_bytes(graph.nodes);
        // A reader per task running at once: the tasks of a scan hold `chunk_entries`
        // entries each but the last, and a frontier runs in runs of `RUN_ENTRIES`
        // entries or fewer (`runs` below).
        let tasks = graph.entries / Settings::default().chunk_entries.max(1) + 1;
        let groups = n.div_ceil(FRONTIER_GRAIN as u64) + 1;
        // A long list of `push_with` is a run of its own, so it adds a run before and
        // after it: at most two per `PART_ENTRIES` entries.
        let runs = 2 * (graph.entries + n) / RUN_ENTRIES as u64
            + 2 * graph.entries / PART_ENTRIES as u64
            + groups;
        let pieces = match op {
            Operation::Bfs { .. } | Operation::KCore => runs,
            _ => 0,
        };
        let workers = (threads.get() as u64).min(tasks.max(pieces).max(1));
        let reader = match tier {
            Tier::Compressed => reader_bytes(graph),
            Tier::Raw => 0,
        };
        let raw = match tier {
            Tier::Compressed => 0,
            Tier::Raw => 8 * (n + 1) + 4 * graph.entries,
        };
        let (phase, access, per_node, sets, readers, output) = match *op {
            // Strengths (f64), the set of nodes with a kept loop, and three rank arrays
            // (`pagerank::pagerank`); the result is one of them.
            Operation::PageRank { precision, .. } => {
                let f = match precision {
                    Precision::Full => 8,
                    Precision::Low => 4,
                };
                ("pagerank", Access::Scan, 8 + 3 * f, set, 1, f * n)
            }
            // At numbering, the labels, the sizes by node and the numbers (4 bytes each),
            // and per component a root (8), a size (4) and a count by size (4, up to the
            // largest): `12 c + 4 (largest + 2)`, at most `12 n + 12` since a graph of
            // `c` components has one of at most `n + 1 - c` nodes; and the roots' offsets,
            // two words per range of 2^14 nodes and one more (`components::number`).
            Operation::Components => {
                let ranges = n.div_ceil(1 << 14);
                (
                    "components",
                    Access::Scan,
                    24,
                    12 + (2 * ranges + 1) * 8,
                    1,
                    4 * n,
                )
            }
            // Degrees and core numbers, the sample counters when a list is long enough
            // to sample, and four node sets (`kcore::peel`, `Buckets`). The buckets hold
            // each node once, in vectors that double (2 n ids), while one more list moves
            // (n), with the list of moved nodes (n) and its distribution's scratch
            // (2 n); the frontier, its next list, and their move (3 n). 9 n ids, and
            // for a small graph the least of each of the 33 buckets and 7 lists.
            Operation::KCore => {
                let sampled = if graph.max_degree >= KCORE_SAMPLED_DEGREE {
                    8
                } else {
                    0
                };
                let lists = 40 * SMALLEST_LIST;
                (
                    "kcore",
                    Access::Mixed,
                    8 + sampled + 36,
                    4 * set + lists + long_lists(graph),
                    2,
                    4 * n,
                )
            }
            // One column; a directed graph's out-degrees come from the index alone.
            Operation::Degrees => (
                "degrees",
                Access::Scan,
                4,
                0,
                u64::from(!graph.directed),
                4 * n,
            ),
            // Distances and parents (8), the next and the bottom-up set; the frontier of
            // the largest level `m` with its move (8 m) and the counts of `l` levels with
            // their move (24 l, a vector that doubles): `m + l <= n + 1`, so 24 (n + 1)
            // at most, with the least of a list on a small graph. The two result columns
            // (8) come beside the state when the frontiers are gone. A bottom-up step's
            // later pieces of a list are read by a second reader (`Plan::scan_until`).
            Operation::Bfs { .. } => {
                let sets = if graph.directed { set } else { 2 * set } + 24 + SMALLEST_LIST;
                ("bfs", Access::Mixed, 32, sets, 2, 0)
            }
        };
        // Each driver call holds one partial value per reduction block: two `ExactSum`s
        // for `PageRank`'s iterations, a count at most for the others.
        let partial = match op {
            Operation::PageRank { .. } => 2 * size_of::<ExactSum>() as u64,
            _ => 8,
        };
        let mut partials = n.div_ceil(u64::from(block_nodes(graph.nodes))) * partial;
        // A frontier driver holds, per group, its result, and for a group that waits for
        // the waves its index, its count of runs and where its runs start; per run, its
        // range, its end while it is cut and the index of a long list (`Plan::drive`,
        // `Plan::cut`). The `2 * result` per run is the slack for two results per run: in
        // the first wave of `push_with`, a run's result beside the one the wave's map
        // returns. A frontier holds at most every node: two consecutive runs of a group
        // hold more than `RUN_ENTRIES` entries and nodes, so a frontier of `m` entries is
        // cut into at most `2 (m + n) / RUN_ENTRIES` runs, two more per long list and one
        // more per group (`runs` above).
        if access == Access::Mixed {
            let result = size_of::<Option<(u64, Work)>>()
                .max(size_of::<Option<Result<(u64, Work), Error>>>());
            let run = 2 * result + size_of::<Range<usize>>() + 4 + 8;
            let group = result + 3 * size_of::<usize>();
            partials += runs * run as u64 + groups * group as u64;
        }
        Self {
            phase,
            access,
            nodes: n,
            threads,
            per_node,
            sets: sets + partials,
            buffers: readers * workers * reader,
            readers: readers * workers,
            raw,
            output,
        }
    }

    /// The state: the per-node arrays, the sets and the raw copy.
    pub fn state(&self) -> u64 {
        self.nodes * self.per_node + self.sets + self.raw
    }

    /// The most the run reserves: the smallest budget that fits (P-14).
    pub fn peak(&self) -> u64 {
        self.state() + self.buffers
    }
}

/// `cost` with the readers of a stream (`CompressedStream`, the streamed regime) in place
/// of decode readers, and their window: half of what `budget` leaves beside the open
/// file, the state and the readers' smallest size, shared among them.
pub(super) fn streamed(mut cost: Cost, graph: &Graph, budget: u64) -> (Cost, usize) {
    let batch = Settings::default().batch_entries;
    let each = |window| {
        CompressedStream::reader_bytes(
            graph.nodes,
            graph.entries,
            graph.data_bytes,
            graph.max_degree,
            window,
            batch,
        )
    };
    // What the readers hold besides their windows comes first.
    let fixed = cost.readers.saturating_mul(each(0));
    let left = budget
        .saturating_sub(graph.held.saturating_add(cost.state()))
        .saturating_sub(fixed);
    let readers = usize::try_from(cost.readers).unwrap_or(usize::MAX);
    let window = CompressedStream::window_for(left, readers);
    cost.buffers = cost.readers.saturating_mul(each(window));
    (cost, window)
}

/// Bytes of a `NodeSet` of `n` nodes: a word per 64 nodes, and a summary word per 64
/// words (`edge_map::node`).
fn node_set_bytes(n: u32) -> u64 {
    let words = u64::from(n).div_ceil(64);
    (words + words.div_ceil(64)) * 8
}

/// The most one decode reader of the framework holds (`source.rs`, `DecodeReader`): its
/// usual batch and, while it grows for a long list, the new buffer beside the old, each
/// with the ends of up to a block's lists.
fn reader_bytes(graph: &Graph) -> u64 {
    // The decoders' slack (`decode.rs`, `SLACK`).
    const SLACK: usize = 3;
    // A batch holds no more entries than the graph has.
    let entries = usize::try_from(graph.entries).unwrap_or(usize::MAX);
    let usual = Settings::default().batch_entries.min(entries) + SLACK;
    let longest = usual.max((graph.max_degree as usize).min(entries) + SLACK);
    let lists = (block_nodes(graph.nodes) as usize).max(FRONTIER_GRAIN);
    (Batch::bytes_for(usual, lists) + Batch::bytes_for(longest, lists)) as u64
}

/// What `Plan::push_with` holds for the long lists of one wave: their copy (no more
/// entries than the longest list, or `WAVE`), and a range and a result per part.
fn long_lists(graph: &Graph) -> u64 {
    let entries = u64::from(graph.max_degree).max(WAVE as u64);
    let parts = 2 * entries / PART_ENTRIES as u64 + 1;
    let part = size_of::<(usize, Range<usize>)>() + size_of::<Option<(u64, Work)>>();
    4 * entries + parts * part as u64
}

/// Nodes per reduction block, as the framework cuts them (`edge_map::block_nodes`).
fn block_nodes(n: u32) -> u32 {
    n.div_ceil(4096).next_power_of_two().clamp(64, 4096)
}

/// Whether the raw copy and the state fit `budget` with the margin of [`RAW_SHARE`].
pub(super) fn fits_with_margin(peak: u64, budget: u64) -> bool {
    u128::from(peak) * u128::from(RAW_SHARE.1) <= u128::from(budget) * u128::from(RAW_SHARE.0)
}

/// Whether `graph` has a raw copy: its offsets in one segment, and every list within
/// one part of the targets (`RawCopy`).
pub(super) fn raw_fits(graph: &Graph) -> bool {
    8 * (u64::from(graph.nodes) + 1) <= MAX_ALLOCATION_BYTES
        && u64::from(graph.max_degree) <= RawCopy::PART_ENTRIES
}

/// Half of the memory free now (P-11).
pub(super) fn default_budget(memory: Option<SystemMemory>) -> u64 {
    memory.map_or(UNKNOWN_FREE_BUDGET, |m| m.available() / 2)
}

/// Threads for `graph`, at most `cap`: one per task of the framework's partition
/// (`Settings::chunk_entries`), so that no thread starts without work.
pub(super) fn threads(graph: &Graph, cap: NonZeroUsize) -> NonZeroUsize {
    let tasks = graph
        .entries
        .div_ceil(Settings::default().chunk_entries)
        .max(1);
    NonZeroUsize::new(cap.get().min(usize::try_from(tasks).unwrap_or(usize::MAX)))
        .unwrap_or(NonZeroUsize::MIN)
}

/// The advice for a phase in `regime` (ADR 0029): a graph that fits is warmed; a phase
/// of random access only (`random`) whose working set exceeds what the budget leaves
/// (`room`) gets `MADV_RANDOM`; a stream's phase is shown as read in order, the advice its
/// source gives itself before scans; a phase that scans, alone or between frontiers, keeps
/// the kernel's read-ahead; a stream reads in order. Every kernel of this crate scans,
/// so only the first, the second and the last case occur for them.
pub fn advice(random: bool, regime: Regime, working_set: u64, room: u64) -> AccessHint {
    match regime {
        Regime::InMemory => AccessHint::WillNeed,
        // The stream advises its own reads, by phase (`CompressedStream`).
        Regime::Streamed => AccessHint::Sequential,
        _ if random && working_set > room => AccessHint::Random,
        _ => AccessHint::Normal,
    }
}

// ---- Time -----------------------------------------------------------------------------

fn to_f64(x: u64) -> f64 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "an estimate of a few significant digits"
    )]
    let x = x as f64;
    x
}

/// `seconds` measured at 16 threads, on `threads` threads, for a kernel whose speedup
/// from 1 to 16 threads is `speedup`: the speedup is taken to follow a power of the
/// thread count, and no gain past 16.
fn on_threads(seconds: f64, threads: NonZeroUsize, speedup: f64) -> f64 {
    let t = to_f64(threads.get() as u64).min(REFERENCE_THREADS);
    let exponent = math::ln(speedup) / math::ln(REFERENCE_THREADS);
    seconds * math::exp(exponent * math::ln(REFERENCE_THREADS / t))
}

fn model(value: f64, high: f64) -> Estimate {
    Estimate {
        value,
        low: value / SPREAD,
        high: high * SPREAD,
        basis: "model",
    }
}

/// The time of `op` with `cost` on `graph`, in `regime` under `budget`: the kernel's
/// work, and in the mapped regime the reads of what the page cache cannot keep.
pub(super) fn seconds(
    op: &Operation,
    cost: &Cost,
    graph: &Graph,
    regime: Regime,
    budget: u64,
    storage: Option<Storage>,
) -> Estimate {
    let entries = to_f64(graph.entries);
    let raw = cost.raw > 0;
    let (per_entry, speedup, passes, most) = match *op {
        Operation::PageRank { params, .. } => {
            let expected = if params.tol > 0.0 {
                params.max_iter.min(PAGERANK_ITERATIONS)
            } else {
                params.max_iter
            };
            // The strengths and the kept loops are a pass each.
            let passes = |iterations: u32| f64::from(iterations) + 2.0;
            (
                PAGERANK_PER_ENTRY,
                PAGERANK_SPEEDUP,
                passes(expected),
                passes(params.max_iter),
            )
        }
        Operation::Components => (COMPONENTS_PER_ENTRY, COMPONENTS_SPEEDUP, 2.0, 2.0),
        Operation::KCore => (KCORE_PER_ENTRY, KCORE_SPEEDUP, 1.0, 1.0),
        Operation::Degrees => (PAGERANK_PER_ENTRY, PAGERANK_SPEEDUP, 1.0, 1.0),
        Operation::Bfs { .. } if raw => (BFS_RAW_PER_ENTRY, BFS_SPEEDUP, 1.0, 1.0),
        Operation::Bfs { .. } => (BFS_PER_ENTRY, BFS_SPEEDUP, 1.0, 1.0),
    };
    let work = |passes: f64| on_threads(entries * per_entry * passes, cost.threads, speedup);
    let uncached = graph
        .data_bytes
        .saturating_sub(budget.saturating_sub(cost.peak()));
    let reads = |passes: f64| match regime {
        Regime::Mapped | Regime::Streamed if !raw => to_f64(uncached) * passes / read_rate(storage),
        _ => 0.0,
    };
    model(work(passes) + reads(passes), work(most) + reads(most))
}

/// Bytes per second read from `storage`.
fn read_rate(storage: Option<Storage>) -> f64 {
    match storage.map(|s| s.kind) {
        Some(StorageKind::Nvme | StorageKind::Memory) => NVME_BYTES_PER_SECOND,
        Some(StorageKind::Ssd) => SSD_BYTES_PER_SECOND,
        _ => OTHER_BYTES_PER_SECOND,
    }
}

/// The time to decode `graph`'s adjacency into the raw copy on `threads` threads.
pub(super) fn decode_seconds(graph: &Graph, threads: NonZeroUsize) -> Estimate {
    let s = on_threads(
        to_f64(graph.entries) * DECODE_PER_ENTRY,
        threads,
        DECODE_SPEEDUP,
    );
    model(s, s)
}

/// Traversals on compressed lists that cost as much as decoding the raw copy once, on
/// `threads` threads: a session builds the copy for the traversal after that many
/// (ski rental: it never pays more than twice the best choice made in hindsight).
#[must_use]
pub fn break_even(threads: NonZeroUsize) -> u32 {
    let decode = on_threads(DECODE_PER_ENTRY, threads, DECODE_SPEEDUP);
    let saved = on_threads(BFS_PER_ENTRY - BFS_RAW_PER_ENTRY, threads, BFS_SPEEDUP);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a small positive count"
    )]
    let runs = (decode / saved).ceil().max(1.0) as u32;
    runs
}
