// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Whole-graph operations by name, as the command line and the Python API ask for them
//! (`docs/api-sketch.md` § 3.2 and § 5.1): each one's name, version and parameters,
//! which make the key a stored result is found by (`docs/spec/eg-container.md` § 6),
//! and the kernel it runs on the lists of a graph file.
//!
//! [`stored`] finds a result in the graph file before it is computed, and [`store`]
//! keeps one there (`docs/api-sketch.md` § 2.8); `bfs`, a personalized `pagerank` and
//! the degrees of a directed graph are never kept.
//!
//! The command line and Python run an operation in three calls: [`kept`] says whether
//! the file serves its result, [`crate::planner::plan`] decides how it runs (its
//! `Display` is `explain`, its `to_json` the plan document, its `refusal` the error
//! before any work), and [`execute`] carries the plan out on its threads: it serves the
//! kept result by the same lookup, or runs the kernel, with a [`Session`] that keeps the
//! decode tier's raw copy from one traversal to the next.
//!
//! An undirected graph's out-lists are its lists. A directed graph's kernels read what
//! each needs (`docs/spec/graph-semantics.md` § 3): `PageRank` pulls along the in-lists;
//! strong components search both ways; a search follows the out- or the in-lists and
//! steps bottom-up along the other; core numbers and a search in both directions read
//! the undirected projection, merged in memory from both; `degrees` counts in- and
//! out-lists and their union. Weak components read the out-lists only. A file built
//! without its in-adjacency answers only what the out-lists answer.

use std::fmt;
use std::sync::atomic::AtomicU32;

use everygraph_core::par::{self, ThreadsError};
use everygraph_core::{AccessHint, ByteSink, ByteSource, Memory, MemoryError, SegVec};
use everygraph_format::adjacency::{
    Adjacency, AdjacencyEvents, AdjacencyIndex, EncodeError, merge, union_degrees,
};
use everygraph_format::container::{
    Container, ContainerError, GraphIdentity, Provenance, ResultKey, StoredResult, Value,
    ValueType, Writer,
};

use crate::components::{self, WccOptions};
use crate::degrees::{self, Loop};
use crate::edge_map::{
    Compressed, CompressedStream, Error, Plan, RawCopy, Source, one_segment_mut,
};
use crate::kcore;
use crate::pagerank::{self, PageRank};
use crate::planner::{self, Request, Tier};
use crate::scc;
use crate::traversal::{self, Bfs, UNREACHED};

/// The state precision of `pagerank` (`docs/api-sketch.md` § 2.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Precision {
    /// f64.
    Full,
    /// f32, only when asked for.
    Low,
}

/// How much a connected pair counts in `pagerank` (`weight`, `docs/api-sketch.md` § 2.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weight {
    /// Every connected pair counts 1.
    Pairs,
    /// The number of edges between the pair: the events of its entry.
    Edges,
}

/// The lists a search follows on a directed graph; an undirected graph has one kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// From a node to the nodes it has an edge to.
    Out,
    /// From a node to the nodes that have an edge to it.
    In,
    /// Both: the undirected projection.
    Both,
}

/// A whole-graph operation and its parameters.
#[derive(Clone, Copy, Debug)]
pub enum Operation<'a> {
    /// `PageRank`.
    PageRank {
        /// Damping, iterations at most, tolerance, and the seed nodes of a personalized
        /// run.
        params: PageRank<'a>,
        /// The state's precision.
        precision: Precision,
        /// What a pair counts.
        weight: Weight,
    },
    /// Connected components, numbered by decreasing size: weak, or `strong` ones of a
    /// directed graph.
    Components {
        /// Strongly connected components.
        strong: bool,
    },
    /// Core numbers, direction ignored.
    KCore,
    /// The degree of each node: the entries of its list, a kept loop counted twice on an
    /// undirected graph (`docs/spec/graph-semantics.md` § 3). On a directed one its in-
    /// and out-degree, and its degree in the undirected projection; only the out-degree
    /// when the file has no in-adjacency.
    Degrees,
    /// Hops from one node.
    Bfs {
        /// The source node.
        source: u32,
        /// Stop at this distance.
        max_hops: Option<u32>,
        /// The lists it follows on a directed graph.
        direction: Direction,
    },
}

/// The names, in the order the help lists them.
pub const NAMES: [&str; 5] = ["pagerank", "components", "kcore", "degrees", "bfs"];

/// The version of every operation's algorithm: a stored result of another version is
/// not served.
pub const VERSION: u32 = 1;

impl Operation<'_> {
    /// The name, as typed after `everygraph run`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::PageRank { .. } => "pagerank",
            Self::Components { .. } => "components",
            Self::KCore => "kcore",
            Self::Degrees => "degrees",
            Self::Bfs { .. } => "bfs",
        }
    }

    /// The name of the column it computes, the first for `bfs` (`docs/api-sketch.md`
    /// § 3.2): `degrees` of a directed graph read from its out-lists gives `out_degree`.
    #[must_use]
    pub fn column(&self, directed: bool) -> &'static str {
        match self {
            Self::PageRank { .. } => "pagerank",
            Self::Components { .. } => "component",
            Self::KCore => "core",
            Self::Degrees if directed => "out_degree",
            Self::Degrees => "degree",
            Self::Bfs { .. } => "distance",
        }
    }

    /// Whether its result on the whole graph is kept in the graph file, which holds one
    /// column per result: not for `bfs` and a personalized `pagerank`, which are one
    /// table per source (`docs/api-sketch.md` § 2.8), nor for the three degree columns
    /// of a directed graph.
    #[must_use]
    pub fn stored(&self, directed: bool) -> bool {
        match self {
            Self::Bfs { .. } => false,
            Self::PageRank { params, .. } => params.personalize.is_none(),
            Self::Degrees => !directed,
            Self::Components { .. } | Self::KCore => true,
        }
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
    /// (spec § 6: floating-point values as Rust's `{}` writes them). A parameter at the
    /// value every earlier version used is left out, so that their keys stay the same.
    #[must_use]
    pub fn parameters(&self) -> Vec<(&'static str, String)> {
        match self {
            Self::PageRank {
                params,
                precision,
                weight,
            } => {
                let mut p = vec![
                    ("damping", params.damping.to_string()),
                    ("max_iter", params.max_iter.to_string()),
                ];
                p.extend(
                    params
                        .personalize
                        .map(|s| ("personalize", s.len().to_string())),
                );
                p.push((
                    "precision",
                    match precision {
                        Precision::Full => "full",
                        Precision::Low => "low",
                    }
                    .to_owned(),
                ));
                p.push(("tol", params.tol.to_string()));
                if *weight == Weight::Edges {
                    p.push(("weight", "edges".to_owned()));
                }
                p
            }
            Self::Components { strong: true } => vec![("strong", "true".to_owned())],
            Self::Bfs {
                source,
                max_hops,
                direction,
            } => {
                let mut p = vec![("source", source.to_string())];
                p.extend(max_hops.map(|h| ("max_hops", h.to_string())));
                match direction {
                    Direction::Out => {}
                    Direction::In => p.push(("direction", "in".to_owned())),
                    Direction::Both => p.push(("direction", "both".to_owned())),
                }
                p
            }
            Self::Components { strong: false } | Self::KCore | Self::Degrees => Vec::new(),
        }
    }
}

impl Operation<'_> {
    /// The operation on the undirected projection of a directed graph, which `execute`
    /// runs for one that [`projects`]: a search in both directions follows its lists.
    #[must_use]
    pub fn undirected(&self) -> Self {
        match *self {
            Self::Bfs {
                source, max_hops, ..
            } => Self::Bfs {
                source,
                max_hops,
                direction: Direction::Out,
            },
            other => other,
        }
    }
}

/// Whether `op` reads the in-lists of a directed graph whose file holds them. A search
/// along the out-lists reads them only to step bottom-up, which it can do without, and
/// weak components never read them.
#[must_use]
pub fn reads_in(op: &Operation<'_>) -> bool {
    !matches!(
        op,
        Operation::Components { strong: false }
            | Operation::Bfs {
                direction: Direction::Out,
                ..
            }
    )
}

/// Whether `op` on a directed graph whose file holds its in-adjacency reads the
/// undirected projection merged from both.
#[must_use]
pub fn projects(op: &Operation<'_>) -> bool {
    matches!(
        op,
        Operation::KCore
            | Operation::Bfs {
                direction: Direction::Both,
                ..
            }
    )
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

const ONE_SEGMENT: &str = "it needs per-node state of more than one segment (more than \
     134,217,728 nodes), which this version does not run; \
     https://github.com/VincenzoImp/everygraph/issues/169 lifts this limit";

const NEEDS_IN_LISTS: &str =
    "on a directed graph it needs the in-adjacency, which this file does not hold";

const NO_DIRECTION: &str =
    "an undirected graph has no direction to use: its strong components are its components";

const NEEDS_PROJECTION: &str =
    "on a directed graph it reads the undirected projection, which `execute` merges";

/// Why `op` cannot run on a graph of `nodes` nodes that is `directed` or not, whose
/// file holds its in-adjacency (`has_in`) or not; `None` if it can.
#[must_use]
pub fn unsupported(
    op: &Operation<'_>,
    directed: bool,
    has_in: bool,
    nodes: u32,
) -> Option<&'static str> {
    let needs_in = reads_in(op) && !matches!(op, Operation::Degrees);
    if u64::from(nodes) * 8 > everygraph_core::MAX_ALLOCATION_BYTES {
        Some(ONE_SEGMENT)
    } else if !directed && matches!(op, Operation::Components { strong: true }) {
        Some(NO_DIRECTION)
    } else if directed && !has_in && needs_in {
        Some(NEEDS_IN_LISTS)
    } else {
        None
    }
}

/// The adjacency of a graph file, as [`execute`] reads it.
#[expect(
    missing_debug_implementations,
    reason = "a set of borrows; the data parts have no `Debug`"
)]
pub struct Lists<'a, S: ByteSource> {
    /// The graph file: its identity, and its kept results.
    pub file: &'a Container<S>,
    /// The index of its out-adjacency.
    pub index: &'a AdjacencyIndex,
    /// The data part: held in memory (a memory map or a buffer), or in the streamed
    /// regime any source, read by positional reads (a file, for direct reads).
    pub data: &'a (dyn ByteSource + Sync),
    /// The index and data part of the in-adjacency of a directed graph, when the file
    /// holds one, held in memory: the streamed regime reads the out-lists only.
    pub inn: Option<(&'a AdjacencyIndex, &'a (dyn ByteSource + Sync))>,
    /// The events of the entries of the out- and of the in-adjacency, when some pair
    /// repeats; `None`: every entry is one edge. [`Weight::Edges`] counts them.
    pub events: [Option<&'a AdjacencyEvents<'a>>; 2],
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
/// kernel on the plan's tier with its settings and the advice of each phase, calling
/// `begin` with each phase as it starts. It drops the session's raw copy first when the
/// plan says so, or when the copy is of another graph. A traversal counts in `session`,
/// which keeps the raw copy the plan builds.
///
/// # Errors
///
/// [`RunError::Refused`] for a plan that does not fit, and for a kept result that the
/// file no longer serves; [`RunError::Threads`]; [`RunError::Kernel`] with
/// [`Error::Invalid`] when a data part is not in memory and the plan does not stream
/// it; the errors of [`run`] and
/// [`stored`].
pub fn execute<S: ByteSource + Sync>(
    plan: &planner::Plan<'_>,
    lists: &Lists<'_, S>,
    memory: &Memory,
    session: &mut Session,
    begin: &(dyn Fn(&planner::Phase) + Sync),
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
        compute(plan, lists, memory, &mut session.raw, begin)
    };
    let output = par::with_threads(plan.threads(), work).map_err(RunError::Threads)??;
    if planner::traverses(op) && !plan.kept() {
        session.traversals += 1;
    }
    Ok(output)
}

/// Runs the kernel of `plan` on its tier: the raw copy of the out-lists in `raw`, built
/// first if the plan builds it, or the compressed lists of `lists`, held in memory or
/// streamed.
fn compute<S: ByteSource + Sync>(
    plan: &planner::Plan<'_>,
    lists: &Lists<'_, S>,
    memory: &Memory,
    raw: &mut Option<RawCopy>,
    begin: &(dyn Fn(&planner::Phase) + Sync),
) -> Result<Output, RunError> {
    let (op, directed) = (plan.operation(), lists.directed);
    // A search along the out-lists steps bottom-up along the in-lists.
    let inn = lists
        .inn
        .filter(|_| directed && !matches!(op, Operation::Components { strong: false }));
    let advise = |hint| {
        lists.data.advise(0..lists.data.len(), hint);
        if let Some((_, data)) = inn {
            data.advise(0..data.len(), hint);
        }
    };
    let mut phases = plan.phases().iter();
    let mut next = |advice: bool| {
        let phase = phases.next();
        if let Some(p) = phase {
            begin(p);
            if advice {
                advise(p.advice);
            }
        }
        phase
    };
    if plan.tier() == Tier::Raw {
        if raw.is_none() || plan.builds_raw() {
            if plan.builds_raw() {
                next(true);
            }
            *raw = None;
            let built = RawCopy::decode(&adjacency(lists.index, lists.data)?, memory);
            advise(AccessHint::Normal);
            *raw = Some(built?);
        }
        next(false);
        let source = raw.as_ref().ok_or(Error::Invalid("no raw copy"))?;
        let weights = edge_weights(op, lists.events[0], memory)?;
        let out = Plan::with(source, memory, plan.settings(), |p| {
            let lanes = Lanes::out(&p, directed).weighted([weights.as_ref(), None]);
            run(op, &lanes, memory)
        });
        return out;
    }
    if plan.regime() == planner::Regime::Streamed {
        // The stream advises its own reads, by phase.
        next(false);
        let source = CompressedStream::new(lists.index, lists.data, plan.window())?;
        let weights = edge_weights(op, lists.events[0], memory)?;
        return Plan::with(&source, memory, plan.settings(), |p| {
            let lanes = Lanes::out(&p, directed).weighted([weights.as_ref(), None]);
            run(op, &lanes, memory)
        });
    }
    let out = Compressed::mapped(lists.index, lists.data)?;
    let output = match inn {
        Some((index, data)) if projects(op) => {
            next(true);
            let merged = project(lists, index, data, memory);
            advise(AccessHint::Normal);
            let (index, data, _held) = merged?;
            next(false);
            let lists = Compressed::new(Adjacency::new(&index, &data).map_err(Error::Format)?);
            let undirected = op.undirected();
            return Plan::with(&lists, memory, plan.settings(), |p| {
                run(&undirected, &Lanes::out(&p, false), memory)
            });
        }
        Some((index, data)) => {
            next(true);
            let weights = [
                edge_weights(op, lists.events[0], memory)?,
                edge_weights(op, lists.events[1], memory)?,
            ];
            let back = Compressed::mapped(index, data)?;
            let mut output = Plan::with_pair(&out, &back, memory, plan.settings(), |o, i| {
                let lanes = Lanes {
                    inn: Some(&i),
                    ..Lanes::out(&o, true).weighted([weights[0].as_ref(), weights[1].as_ref()])
                };
                run(op, &lanes, memory)
            })??;
            if matches!(op, Operation::Degrees) {
                output
                    .columns
                    .push(("degree", union(lists, index, data, memory)?));
            }
            output
        }
        None => {
            next(true);
            let weights = edge_weights(op, lists.events[0], memory)?;
            Plan::with(&out, memory, plan.settings(), |p| {
                let lanes = Lanes::out(&p, directed).weighted([weights.as_ref(), None]);
                run(op, &lanes, memory)
            })?
        }
    };
    advise(AccessHint::Normal);
    Ok(output)
}

/// The adjacency of `index` over `data`, held in memory.
fn adjacency<'a>(
    index: &'a AdjacencyIndex,
    data: &'a (dyn ByteSource + Sync),
) -> Result<Adjacency<'a>, Error> {
    let bytes = data
        .as_slice()
        .ok_or(Error::Invalid("the data part is not in memory"))?;
    Adjacency::new(index, bytes).map_err(Error::Format)
}

fn merge_error(e: EncodeError) -> Error {
    match e {
        EncodeError::Budget(refused) => Error::Memory(refused.into()),
        EncodeError::OutOfMemory { bytes } => Error::Memory(MemoryError::AllocationFailed {
            purpose: "undirected projection",
            bytes,
        }),
        EncodeError::Decode(e) => Error::Decode(e),
        EncodeError::Offsets(what) => Error::Invalid(what),
        _ => Error::Invalid("the in-adjacency is not of the same graph"),
    }
}

/// The undirected projection of a directed graph, merged in memory from its out-lists
/// and the in-lists of `index` over `data`: the index parsed, the data part, and the
/// reservation that holds both.
fn project<S: ByteSource>(
    lists: &Lists<'_, S>,
    index: &AdjacencyIndex,
    data: &(dyn ByteSource + Sync),
    memory: &Memory,
) -> Result<(AdjacencyIndex, Vec<u8>, everygraph_core::Reservation), Error> {
    let (merged, mut held) = merge(
        &adjacency(lists.index, lists.data)?,
        &adjacency(index, data)?,
        memory,
    )
    .map_err(merge_error)?;
    // The parsed index beside its bytes, then alone.
    let bytes = merged.index.len() as u64;
    held.try_grow(2 * bytes)
        .map_err(|e| Error::Memory(e.into()))?;
    let parsed = AdjacencyIndex::parse(&merged.index).map_err(Error::Format)?;
    held.shrink((2 * bytes).saturating_sub(parsed.heap_bytes() as u64));
    Ok((parsed, merged.data, held))
}

/// The degree of each node of a directed graph in its undirected projection, from its
/// out-lists and the in-lists of `index` over `data`, without building the projection.
fn union<S: ByteSource>(
    lists: &Lists<'_, S>,
    index: &AdjacencyIndex,
    data: &(dyn ByteSource + Sync),
    memory: &Memory,
) -> Result<Column, Error> {
    let n = everygraph_core::usize_of(lists.index.nodes());
    let mut values = SegVec::try_with_len(memory, "degrees", n, 0_u32)?;
    union_degrees(
        &adjacency(lists.index, lists.data)?,
        &adjacency(index, data)?,
        memory,
        one_segment_mut(&mut values)?,
    )
    .map_err(merge_error)?;
    Ok(Column::U32 {
        values,
        null: None,
        nodes: false,
    })
}

/// The weight of each entry for `op`: its number of edges, from `events`, when `op` is a
/// `PageRank` that counts them and some pair repeats.
fn edge_weights(
    op: &Operation<'_>,
    events: Option<&AdjacencyEvents<'_>>,
    memory: &Memory,
) -> Result<Option<SegVec<f64>>, Error> {
    let (
        Operation::PageRank {
            weight: Weight::Edges,
            ..
        },
        Some(events),
    ) = (op, events)
    else {
        return Ok(None);
    };
    let entries = usize::try_from(events.entries())
        .map_err(|_| Error::Invalid("more entries than the address space"))?;
    let mut w = SegVec::try_with_len(memory, "pair weights", entries, 0.0)?;
    let counts = events.multiplicities().map_err(Error::Format)?;
    #[expect(
        clippy::cast_precision_loss,
        reason = "a count of events, exact below 2^53"
    )]
    for (slot, m) in one_segment_mut(&mut w)?.iter_mut().zip(counts) {
        *slot = m as f64;
    }
    Ok(Some(w))
}

/// The lists [`run`] reads, by plans of one brand: the out-lists, and the in-lists of a
/// directed graph when the operation reads them; with one weight per entry of each,
/// when the operation counts them.
#[derive(Clone, Copy, Debug)]
pub struct Lanes<'p, 's, 'g, S: Source> {
    /// The out-lists; the lists of an undirected graph.
    pub out: &'p Plan<'s, 'g, S>,
    /// The in-lists of a directed graph.
    pub inn: Option<&'p Plan<'s, 'g, S>>,
    /// Whether the graph is directed.
    pub directed: bool,
    /// The weights of the entries of `out` and of `inn`.
    pub weights: [Option<&'p SegVec<f64>>; 2],
}

impl<'p, 's, 'g, S: Source> Lanes<'p, 's, 'g, S> {
    /// The out-lists `out` of a graph that is `directed` or not, alone.
    #[must_use]
    pub fn out(out: &'p Plan<'s, 'g, S>, directed: bool) -> Self {
        Self {
            out,
            inn: None,
            directed,
            weights: [None; 2],
        }
    }

    /// The same, with `weights`.
    #[must_use]
    pub fn weighted(self, weights: [Option<&'p SegVec<f64>>; 2]) -> Self {
        Self { weights, ..self }
    }
}

/// Runs `op` on `lanes` within `memory`. On a directed graph, core numbers and a search
/// in both directions read the undirected projection, which the caller makes (as
/// [`execute`] does) and passes as the lists of an undirected graph; `degree` in the
/// projection is the caller's column too.
///
/// # Errors
///
/// [`RunError::Unsupported`] as [`unsupported`] says; the kernel's errors.
pub fn run<S: Source>(
    op: &Operation<'_>,
    lanes: &Lanes<'_, '_, '_, S>,
    memory: &Memory,
) -> Result<Output, RunError> {
    let Lanes {
        out,
        inn,
        directed,
        weights,
    } = *lanes;
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
    if let Some(why) = unsupported(op, directed, inn.is_some(), out.nodes()) {
        return Err(RunError::Unsupported(why));
    }
    match *op {
        Operation::PageRank {
            params, precision, ..
        } => {
            let (pull, pull_weights) = match inn {
                Some(inn) => (inn, weights[1]),
                None => (out, weights[0]),
            };
            let g = pagerank::Graph {
                lists: pull,
                weights: one_slice(pull_weights)?,
                directed,
            };
            let strength = pagerank::strength(out, one_slice(weights[0])?, directed, memory)?;
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
        Operation::Components { strong } => {
            let (found, _held) = match inn {
                Some(inn) if strong => scc::scc(out, inn, memory)?,
                _ => {
                    let options = WccOptions {
                        symmetric: !directed,
                        sample: true,
                    };
                    components::wcc(out, memory, options)?
                }
            };
            let numbered = components::number(&found, memory)?;
            Ok(plain("component", labels(numbered.component)))
        }
        Operation::KCore if directed => Err(RunError::Unsupported(NEEDS_PROJECTION)),
        Operation::KCore => {
            let (core, _held) = kcore::kcore(out, memory)?;
            Ok(plain("core", labels(to_column(&core, memory)?)))
        }
        Operation::Degrees => {
            let own = if directed { Loop::Once } else { Loop::Twice };
            let mut columns = Vec::new();
            if let Some(inn) = inn {
                columns.push(("in_degree", labels(degrees::degrees(inn, memory, own)?)));
            }
            let values = degrees::degrees(out, memory, own)?;
            columns.push((op.column(directed), labels(values)));
            Ok(Output {
                columns,
                iterations: None,
                converged: None,
            })
        }
        Operation::Bfs {
            source,
            max_hops,
            direction,
        } => {
            let (lists, back) = match (direction, inn) {
                _ if !directed => (out, Some(out)),
                (Direction::Both, _) => return Err(RunError::Unsupported(NEEDS_PROJECTION)),
                (Direction::In, Some(inn)) => (inn, Some(out)),
                (_, back) => (out, back),
            };
            let p = Bfs {
                max_hops,
                parents: true,
            };
            let hops = traversal::bfs(lists, back, source, &p, memory)?;
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

/// The values of `weights` in one slice.
fn one_slice(weights: Option<&SegVec<f64>>) -> Result<Option<&[f64]>, RunError> {
    weights
        .map(|w| w.as_slice().ok_or(RunError::Unsupported(ONE_SEGMENT)))
        .transpose()
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
    op: &Operation<'_>,
    directed: bool,
    nodes: u32,
    memory: &Memory,
) -> Result<Option<Output>, RunError> {
    let Some(found) = find(file, op, directed, nodes)? else {
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

/// Whether [`stored`] serves `op` on the graph of `file`, of `nodes` nodes and
/// `directed` or not, without reading its values: what [`planner::Graph::kept`] must be.
///
/// # Errors
///
/// As [`stored`], for the key and the table of the file.
pub fn kept<S: ByteSource>(
    file: &Container<S>,
    op: &Operation<'_>,
    directed: bool,
    nodes: u32,
) -> Result<bool, RunError> {
    Ok(find(file, op, directed, nodes)?.is_some())
}

/// The kept result of `op` that [`stored`] serves: one value per node, of a type a
/// column holds.
fn find<'f, S: ByteSource>(
    file: &'f Container<S>,
    op: &Operation<'_>,
    directed: bool,
    nodes: u32,
) -> Result<Option<StoredResult<'f, S>>, RunError> {
    if !op.stored(directed) {
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
    op: &Operation<'_>,
    graph: GraphIdentity,
    about: &Provenance,
    output: &Output,
) -> Result<(), ContainerError> {
    let key = op.key(graph)?;
    let Some((_, column)) = output.columns.first() else {
        return Ok(());
    };
    let long = ContainerError::InvalidResult(
        "a result longer than one segment, which this version does not store; \
         https://github.com/VincenzoImp/everygraph/issues/169 lifts this limit",
    );
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
