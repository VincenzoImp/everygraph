// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Whole-graph operations by name, as the command line and the Python API ask for them
//! (`docs/api-sketch.md` § 3.2 and § 5.1): each one's name, version and parameters,
//! which make the key a stored result is found by (`docs/spec/eg-container.md` § 6),
//! and the kernel it runs on the out-lists of a graph file.
//!
//! [`stored`] finds a result in the graph file before it is computed, and [`store`]
//! keeps one there (`docs/api-sketch.md` § 2.8); `bfs` is never kept.
//!
//! The command line and Python run an operation in three calls: [`kept`] says whether
//! the file serves its result, [`crate::planner::plan`] decides how it runs (its
//! `Display` is `explain`, its `to_json` the plan document, its `refusal` the error
//! before any work), and [`execute`] carries the plan out on its threads: it serves the
//! kept result by the same lookup, or runs the kernel, with a [`Session`] that keeps the
//! decode tier's raw copy from one traversal to the next.
//!
//! A graph file holds the out-adjacency only; an undirected graph's out-lists are its
//! lists. On a directed graph, `pagerank` and `kcore` need the in-adjacency or the
//! undirected projection, which no file holds yet (#m2-11): they are
//! [`RunError::Unsupported`] there. `components` reads the out-lists of a directed
//! graph as weak components do, and `bfs` follows them out.

use std::fmt;
use std::sync::atomic::AtomicU32;

use everygraph_core::par::{self, ThreadsError};
use everygraph_core::{AccessHint, ByteSink, ByteSource, Memory, MemoryError, SegVec};
use everygraph_format::adjacency::{Adjacency, AdjacencyIndex};
use everygraph_format::container::{
    Container, ContainerError, GraphIdentity, Provenance, ResultKey, StoredResult, Value,
    ValueType, Writer,
};

use crate::components::{self, WccOptions};
use crate::degrees::{self, Loop};
use crate::edge_map::{Compressed, CompressedStream, Error, Plan, RawCopy, Source};
use crate::kcore;
use crate::pagerank::{self, PageRank};
use crate::planner::{self, Request};
use crate::traversal::{self, Bfs, UNREACHED};

/// The state precision of `pagerank` (`docs/api-sketch.md` § 2.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Precision {
    /// f64.
    Full,
    /// f32, only when asked for.
    Low,
}

/// A whole-graph operation and its parameters.
#[derive(Clone, Copy, Debug)]
pub enum Operation {
    /// `PageRank`.
    PageRank {
        /// Damping, iterations at most, tolerance; no personalization.
        params: PageRank<'static>,
        /// The state's precision.
        precision: Precision,
    },
    /// Weakly connected components, numbered by decreasing size.
    Components,
    /// Core numbers.
    KCore,
    /// The degree of each node: the entries of its list, a kept loop counted twice on an
    /// undirected graph (`docs/spec/graph-semantics.md` § 3); its out-degree on a directed
    /// one.
    Degrees,
    /// Hops from one node.
    Bfs {
        /// The source node.
        source: u32,
        /// Stop at this distance.
        max_hops: Option<u32>,
    },
}

/// The names, in the order the help lists them.
pub const NAMES: [&str; 5] = ["pagerank", "components", "kcore", "degrees", "bfs"];

/// The version of every operation's algorithm: a stored result of another version is
/// not served.
pub const VERSION: u32 = 1;

impl Operation {
    /// The name, as typed after `everygraph run`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::PageRank { .. } => "pagerank",
            Self::Components => "components",
            Self::KCore => "kcore",
            Self::Degrees => "degrees",
            Self::Bfs { .. } => "bfs",
        }
    }

    /// The name of the column it computes, the first for `bfs` (`docs/api-sketch.md`
    /// § 3.2): `degrees` of a directed graph gives `out_degree`.
    #[must_use]
    pub fn column(&self, directed: bool) -> &'static str {
        match self {
            Self::PageRank { .. } => "pagerank",
            Self::Components => "component",
            Self::KCore => "core",
            Self::Degrees if directed => "out_degree",
            Self::Degrees => "degree",
            Self::Bfs { .. } => "distance",
        }
    }

    /// Whether its result on the whole graph is kept in the graph file: all but `bfs`,
    /// which is one table per source (`docs/api-sketch.md` § 2.8).
    #[must_use]
    pub fn stored(&self) -> bool {
        !matches!(self, Self::Bfs { .. })
    }

    /// The key its result on the graph `graph` is kept under: every run is
    /// deterministic.
    ///
    /// # Errors
    ///
    /// As [`ResultKey::with`], for a parameter the key cannot hold.
    pub fn key(&self, graph: GraphIdentity) -> Result<ResultKey, ContainerError> {
        self.parameters().into_iter().try_fold(
            ResultKey::new(graph, self.name(), VERSION, true)?,
            |key, (name, value)| key.with(name, &value),
        )
    }

    /// Every parameter that can change the result, with its value as the key holds it
    /// (spec § 6: floating-point values as Rust's `{}` writes them).
    #[must_use]
    pub fn parameters(&self) -> Vec<(&'static str, String)> {
        match self {
            Self::PageRank { params, precision } => vec![
                ("damping", params.damping.to_string()),
                ("max_iter", params.max_iter.to_string()),
                (
                    "precision",
                    match precision {
                        Precision::Full => "full",
                        Precision::Low => "low",
                    }
                    .to_owned(),
                ),
                ("tol", params.tol.to_string()),
            ],
            Self::Bfs { source, max_hops } => {
                let mut p = vec![("source", source.to_string())];
                p.extend(max_hops.map(|h| ("max_hops", h.to_string())));
                p
            }
            Self::Components | Self::KCore | Self::Degrees => Vec::new(),
        }
    }
}

/// One column of a result, one value per node in node order.
#[derive(Debug)]
pub enum Column {
    /// Integers; `null` is the value that stands for none (`bfs` where a node is not
    /// reached).
    U32 {
        /// The values.
        values: SegVec<u32>,
        /// The value that means null, if any.
        null: Option<u32>,
        /// Whether the values are nodes, by their number (`bfs`'s `parent`).
        nodes: bool,
    },
    /// f64 values.
    F64(SegVec<f64>),
    /// f32 values.
    F32(SegVec<f32>),
}

/// What an operation computed.
#[derive(Debug)]
pub struct Output {
    /// The columns after `node`, by name (`docs/api-sketch.md` § 3.2).
    pub columns: Vec<(&'static str, Column)>,
    /// Iterations run, for an iteration.
    pub iterations: Option<u32>,
    /// Whether it stopped below its tolerance, for an iteration.
    pub converged: Option<bool>,
}

/// Why an operation did not run.
#[derive(Debug)]
pub enum RunError {
    /// The kernel failed.
    Kernel(Error),
    /// The operation cannot run on this graph; the reason.
    Unsupported(&'static str),
    /// A stored result could not be read: the file is damaged.
    Stored(ContainerError),
    /// The plan does not fit its budget, or cannot run: [`execute`] did no work.
    Refused(Box<planner::Refusal>),
    /// The threads of the plan could not be started.
    Threads(ThreadsError),
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Kernel(e) => e.fmt(f),
            Self::Unsupported(why) => f.write_str(why),
            Self::Stored(e) => e.fmt(f),
            Self::Refused(r) => write!(f, "{} {}", r.message, r.why),
            Self::Threads(e) => e.fmt(f),
        }
    }
}

impl From<ContainerError> for RunError {
    fn from(e: ContainerError) -> Self {
        Self::Stored(e)
    }
}

impl std::error::Error for RunError {}

impl From<Error> for RunError {
    fn from(e: Error) -> Self {
        Self::Kernel(e)
    }
}

impl From<MemoryError> for RunError {
    fn from(e: MemoryError) -> Self {
        Self::Kernel(Error::Memory(e))
    }
}

const ONE_SEGMENT: &str = "it needs per-node state of more than one segment, which this \
     version does not run (#m3-15): more than 134,217,728 nodes";

const NEEDS_IN_LISTS: &str =
    "on a directed graph it needs the in-adjacency, which this file does not hold";

/// Why `op` cannot run on the out-lists of a graph of `nodes` nodes that is `directed`
/// or not; `None` if it can.
#[must_use]
pub fn unsupported(op: &Operation, directed: bool, nodes: u32) -> Option<&'static str> {
    if u64::from(nodes) * 8 > everygraph_core::MAX_ALLOCATION_BYTES {
        Some(ONE_SEGMENT)
    } else if directed && matches!(op, Operation::PageRank { .. } | Operation::KCore) {
        Some(NEEDS_IN_LISTS)
    } else {
        None
    }
}

/// The out-adjacency of a graph file, as [`execute`] reads it.
#[expect(
    missing_debug_implementations,
    reason = "a set of borrows; the data part has no `Debug`"
)]
pub struct Lists<'a, S: ByteSource> {
    /// The graph file: its identity, and its kept results.
    pub file: &'a Container<S>,
    /// The index of its out-adjacency.
    pub index: &'a AdjacencyIndex,
    /// The data part: held in memory (a memory map or a buffer), or in the streamed
    /// regime any source, read by positional reads (a file, for direct reads).
    pub data: &'a (dyn ByteSource + Sync),
    /// Whether the graph is directed.
    pub directed: bool,
}

/// What a sequence of operations on one graph keeps from one to the next: the
/// traversals run so far, and the raw copy of the adjacency once a plan builds it (the
/// decode tier of `planner`). The copy holds memory of the budget it was built in.
/// Both belong to one graph, by its identity: on another graph they are dropped.
#[derive(Debug, Default)]
pub struct Session {
    graph: Option<GraphIdentity>,
    traversals: u32,
    raw: Option<RawCopy>,
}

impl Session {
    /// A session with nothing run yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `request`, with what the planner must know of this session on the graph of
    /// `lists`: the traversals run on it, and the bytes the session holds, of a raw copy
    /// of this graph or of another.
    #[must_use]
    pub fn request<S: ByteSource>(&self, lists: &Lists<'_, S>, request: Request) -> Request {
        let same = self.graph == Some(lists.file.graph_identity());
        Request {
            traversals_before: if same { self.traversals } else { 0 },
            held: self.raw.as_ref().map(|r| planner::Held {
                bytes: r.held(),
                usable: same,
            }),
            ..request
        }
    }

    /// Bytes the raw copy holds, if the session has one.
    #[must_use]
    pub fn raw_bytes(&self) -> Option<u64> {
        self.raw.as_ref().map(RawCopy::held)
    }

    /// The session, emptied when it belongs to another graph than `graph`.
    fn on(&mut self, graph: GraphIdentity) -> &mut Self {
        if self.graph != Some(graph) {
            *self = Self {
                graph: Some(graph),
                ..Self::default()
            };
        }
        self
    }
}

/// Carries out `plan`, made with [`Session::request`] on `lists`, within `memory`, on
/// `plan.threads()` threads: it serves a kept result through [`stored`], or runs the
/// kernel on the plan's tier with its settings and the advice of each phase. It drops
/// the session's raw copy first when the plan says so, or when the copy is of another
/// graph. A traversal counts in `session`, which keeps the raw copy the plan builds.
///
/// # Errors
///
/// [`RunError::Refused`] for a plan that does not fit, and for a kept result that the
/// file no longer serves; [`RunError::Threads`]; [`RunError::Kernel`] with
/// [`Error::Invalid`] when the data part is not in memory and the plan does not stream
/// it; the errors of [`run`] and
/// [`stored`].
pub fn execute<S: ByteSource + Sync>(
    plan: &planner::Plan,
    lists: &Lists<'_, S>,
    memory: &Memory,
    session: &mut Session,
) -> Result<Output, RunError> {
    let op = plan.operation();
    if let Some(refusal) = plan.refusal() {
        return Err(RunError::Refused(Box::new(refusal.clone())));
    }
    let session = session.on(lists.file.graph_identity());
    if plan.drops_raw() {
        session.raw = None;
    }
    let work = || {
        if plan.kept() {
            let nodes = lists.index.nodes();
            return stored(lists.file, op, lists.directed, nodes, memory)?
                .ok_or_else(|| RunError::Refused(Box::new(planner::Refusal::not_kept(op))));
        }
        compute(plan, lists, memory, &mut session.raw)
    };
    let output = par::with_threads(plan.threads(), work).map_err(RunError::Threads)??;
    if planner::traverses(op) && !plan.kept() {
        session.traversals += 1;
    }
    Ok(output)
}

/// Runs the kernel of `plan` on its tier: the raw copy in `raw`, built first if the plan
/// builds it, or the compressed lists of `lists`.
fn compute<S: ByteSource + Sync>(
    plan: &planner::Plan,
    lists: &Lists<'_, S>,
    memory: &Memory,
    raw: &mut Option<RawCopy>,
) -> Result<Output, RunError> {
    let (op, directed) = (plan.operation(), lists.directed);
    let whole = 0..lists.data.len();
    let advise = |hint| lists.data.advise(whole.clone(), hint);
    let mut phases = plan.phases().iter();
    let compressed = plan.tier() == planner::Tier::Compressed;
    if compressed && plan.regime() == planner::Regime::Streamed {
        // The stream advises its own reads, by phase.
        let source = CompressedStream::new(lists.index, lists.data, plan.window())?;
        return Plan::with(&source, memory, plan.settings(), |p| {
            run(op, &p, directed, memory)
        });
    }
    if compressed {
        let source = Compressed::mapped(lists.index, lists.data)?;
        if let Some(phase) = phases.next() {
            advise(phase.advice);
        }
        let out = Plan::with(&source, memory, plan.settings(), |p| {
            run(op, &p, directed, memory)
        });
        advise(AccessHint::Normal);
        return out;
    }
    if raw.is_none() || plan.builds_raw() {
        if let Some(decoding) = phases.next().filter(|p| p.name == "decoding") {
            advise(decoding.advice);
        }
        *raw = None;
        let data = lists
            .data
            .as_slice()
            .ok_or(Error::Invalid("the data part is not in memory"))?;
        let adjacency = Adjacency::new(lists.index, data).map_err(Error::Format)?;
        let built = RawCopy::decode(&adjacency, memory);
        advise(AccessHint::Normal);
        *raw = Some(built?);
    }
    let source = raw.as_ref().ok_or(Error::Invalid("no raw copy"))?;
    Plan::with(source, memory, plan.settings(), |p| {
        run(op, &p, directed, memory)
    })
}

/// Runs `op` on the out-lists that `out` reads, of a graph that is `directed` or not,
/// within `memory`.
///
/// # Errors
///
/// [`RunError::Unsupported`] for `pagerank` and `kcore` on a directed graph; the
/// kernel's errors.
pub fn run<S: Source>(
    op: &Operation,
    out: &Plan<'_, '_, S>,
    directed: bool,
    memory: &Memory,
) -> Result<Output, RunError> {
    let plain = |name, column| Output {
        columns: vec![(name, column)],
        iterations: None,
        converged: None,
    };
    let labels = |values| Column::U32 {
        values,
        null: None,
        nodes: false,
    };
    if let Some(why) = unsupported(op, directed, out.nodes()) {
        return Err(RunError::Unsupported(why));
    }
    match *op {
        Operation::PageRank { params, precision } => {
            let g = pagerank::Graph {
                lists: out,
                weights: None,
                directed,
            };
            let strength = pagerank::strength(out, None, directed, memory)?;
            let strength = strength
                .as_slice()
                .ok_or(RunError::Unsupported(ONE_SEGMENT))?;
            let (column, iterations, change) = match precision {
                Precision::Full => {
                    let r = pagerank::pagerank::<_, f64>(&g, strength, &params, memory)?;
                    (Column::F64(r.values), r.iterations, r.change)
                }
                Precision::Low => {
                    let r = pagerank::pagerank::<_, f32>(&g, strength, &params, memory)?;
                    (Column::F32(r.values), r.iterations, r.change)
                }
            };
            Ok(Output {
                iterations: Some(iterations),
                converged: Some(change < params.tol),
                ..plain("pagerank", column)
            })
        }
        Operation::Components => {
            let options = WccOptions {
                symmetric: !directed,
                sample: true,
            };
            let (found, _held) = components::wcc(out, memory, options)?;
            let numbered = components::number(&found, memory)?;
            Ok(plain("component", labels(numbered.component)))
        }
        Operation::KCore => {
            let (core, _held) = kcore::kcore(out, memory)?;
            Ok(plain("core", labels(to_column(&core, memory)?)))
        }
        Operation::Degrees => {
            let own = if directed { Loop::Once } else { Loop::Twice };
            let values = degrees::degrees(out, memory, own)?;
            Ok(plain(op.column(directed), labels(values)))
        }
        Operation::Bfs { source, max_hops } => {
            let p = Bfs {
                max_hops,
                parents: true,
            };
            let inn = (!directed).then_some(out);
            let hops = traversal::bfs(out, inn, source, &p, memory)?;
            let mut columns = vec![(
                "distance",
                Column::U32 {
                    values: to_column(&hops.distance, memory)?,
                    null: Some(UNREACHED),
                    nodes: false,
                },
            )];
            if let Some(parent) = &hops.parent {
                columns.push((
                    "parent",
                    Column::U32 {
                        values: to_column(parent, memory)?,
                        null: Some(UNREACHED),
                        nodes: true,
                    },
                ));
            }
            Ok(Output {
                columns,
                iterations: None,
                converged: None,
            })
        }
    }
}

/// The result of `op` on the graph of `file`, a graph of `nodes` nodes that is
/// `directed` or not, if the file keeps one with one value per node: checked against
/// its hash and read into `memory`.
///
/// # Errors
///
/// [`RunError::Stored`] for a result that is damaged; [`RunError::Kernel`] with
/// [`Error::Memory`] when its values do not fit the budget; [`RunError::Unsupported`]
/// for a graph whose node state is longer than one segment.
pub fn stored<S: ByteSource>(
    file: &Container<S>,
    op: &Operation,
    directed: bool,
    nodes: u32,
    memory: &Memory,
) -> Result<Option<Output>, RunError> {
    let Some(found) = find(file, op, nodes)? else {
        return Ok(None);
    };
    file.verify_result(&found)?;
    let len = usize::try_from(found.len())
        .map_err(|_| Error::Invalid("a result longer than the address space"))?;
    let column = match found.value_type() {
        ValueType::U32 => Column::U32 {
            values: read(&found, len, memory)?,
            null: None,
            nodes: false,
        },
        ValueType::F64 => Column::F64(read(&found, len, memory)?),
        _ => Column::F32(read(&found, len, memory)?),
    };
    Ok(Some(Output {
        columns: vec![(op.column(directed), column)],
        iterations: None,
        converged: None,
    }))
}

/// Whether [`stored`] serves `op` on the graph of `file`, of `nodes` nodes, without
/// reading its values: what [`planner::Graph::kept`] must be.
///
/// # Errors
///
/// As [`stored`], for the key and the table of the file.
pub fn kept<S: ByteSource>(
    file: &Container<S>,
    op: &Operation,
    nodes: u32,
) -> Result<bool, RunError> {
    Ok(find(file, op, nodes)?.is_some())
}

/// The kept result of `op` that [`stored`] serves: one value per node, of a type a
/// column holds.
fn find<'f, S: ByteSource>(
    file: &'f Container<S>,
    op: &Operation,
    nodes: u32,
) -> Result<Option<StoredResult<'f, S>>, RunError> {
    if !op.stored() {
        return Ok(None);
    }
    if u64::from(nodes) * 8 > everygraph_core::MAX_ALLOCATION_BYTES {
        return Err(RunError::Unsupported(ONE_SEGMENT));
    }
    let found = file.find_result(&op.key(file.graph_identity())?)?;
    Ok(found.filter(|f| {
        f.len() == u64::from(nodes)
            && matches!(
                f.value_type(),
                ValueType::U32 | ValueType::F64 | ValueType::F32
            )
    }))
}

fn read<S: ByteSource, T: Value + Default>(
    found: &StoredResult<'_, S>,
    len: usize,
    memory: &Memory,
) -> Result<SegVec<T>, RunError> {
    let mut values = SegVec::try_with_len(memory, "stored result", len, T::default())?;
    let mut first = 0;
    for segment in values.segments_mut() {
        found.read(first, segment)?;
        first += segment.len() as u64;
    }
    Ok(values)
}

/// Adds the first column of `output` to `writer` as the result of `op` on the graph
/// `graph` it was computed on, with `about` for the engine and the times;
/// [`Writer::commit`] makes it stored.
///
/// # Errors
///
/// [`ContainerError::InvalidResult`] when the writer's file holds another graph than
/// `graph` (it was replaced during the run), or for a column longer than one segment;
/// the other errors of [`Writer::add_result`].
pub fn store<K: ByteSink>(
    writer: &mut Writer<K>,
    op: &Operation,
    graph: GraphIdentity,
    about: &Provenance,
    output: &Output,
) -> Result<(), ContainerError> {
    let key = op.key(graph)?;
    let Some((_, column)) = output.columns.first() else {
        return Ok(());
    };
    let long = ContainerError::InvalidResult("a result longer than one segment (#m3-15)");
    match column {
        Column::U32 { values, .. } => {
            writer.add_result(&key, about, values.as_slice().ok_or(long)?)
        }
        Column::F64(values) => writer.add_result(&key, about, values.as_slice().ok_or(long)?),
        Column::F32(values) => writer.add_result(&key, about, values.as_slice().ok_or(long)?),
    }
    .map(|_| ())
}

/// The values of `atomics`, in a column of the budget.
fn to_column(atomics: &[AtomicU32], memory: &Memory) -> Result<SegVec<u32>, RunError> {
    let mut out = SegVec::try_with_len(memory, "result column", atomics.len(), 0_u32)?;
    for (slot, a) in out.segments_mut().flatten().zip(atomics) {
        *slot = a.load(std::sync::atomic::Ordering::Relaxed);
    }
    Ok(out)
}
