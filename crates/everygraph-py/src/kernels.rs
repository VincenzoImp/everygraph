// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The whole-graph algorithms of `docs/api-sketch.md` § 3.2, each planned by
//! `everygraph_kernels::planner::plan` and carried out by `run::execute` on the lists
//! the file holds (`run` says which each reads), in the session of its graph.
//!
//! The streamed regime reads the data part by positional reads of the file; the other
//! regimes read the memory map.
//!
//! A result on the whole graph of a file is looked up in the file before it is computed
//! and kept there after (`docs/api-sketch.md` § 2.8). The lookup reads the file's table
//! of contents again, by positional reads, since a result committed after the graph was
//! opened lies beyond the range it maps.

use std::num::NonZeroUsize;
use std::path::Path;

use arrow_array::ArrayRef;
use arrow_array::types::{Float32Type, Float64Type};
use everygraph_core::{ByteSource, Memory, MemoryError, MemoryProbe, Reservation, SegVec};
use everygraph_format::adjacency::AdjacencyEvents;
use everygraph_format::container::{Container, Provenance, Section, Writer};
use everygraph_ingest::build::OUT_DATA;
use everygraph_ingest::graph::OpenError;
use everygraph_kernels::planner::{self, Machine, Origin, Request};
use everygraph_kernels::run::{self, Column, Lists, Operation, Output, RunError, Session, Weight};

use crate::error::{self, Failure};
use crate::graph::Shared;
use crate::run::{Event, Events};
use crate::source::Bytes;
use crate::table::{self, NULL};

/// A value the result's `meta` gains.
pub(crate) enum Extra {
    Count(u64),
    Flag(bool),
}

/// What a call returns: its columns after `node`, the identifier columns after those,
/// and what its `meta` gains.
pub(crate) struct Computed {
    pub(crate) columns: Vec<(&'static str, Vec<ArrayRef>)>,
    pub(crate) ids: Vec<(&'static str, Vec<u32>)>,
    pub(crate) extra: Vec<(&'static str, Extra)>,
}

/// How a call was asked to run (`docs/api-sketch.md` § 2.4): the budget and the most
/// threads, each with where it came from.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Asked {
    pub(crate) memory: Option<(u64, Origin)>,
    pub(crate) threads: Option<(NonZeroUsize, Origin)>,
}

/// Where a setting came from, as the package names it.
pub(crate) fn origin(name: &str) -> Origin {
    match name {
        "argument" => Origin::Argument,
        "graph" => Origin::Graph,
        "environment" => Origin::Environment,
        _ => Origin::Default,
    }
}

/// What the machine has (P-2): the cores and the memory this process may use, and the
/// storage of `dir`.
fn machine(dir: Option<&Path>) -> Machine {
    Machine {
        cores: everygraph_io::physical_cores(),
        hardware_threads: std::thread::available_parallelism().unwrap_or(NonZeroUsize::MIN),
        memory: everygraph_io::ProcMemoryProbe::new().probe().ok(),
        storage: dir.and_then(everygraph_io::storage),
    }
}

/// The events of an adjacency, opened, with the reservation that holds them.
type Opened<'a> = (AdjacencyEvents<'a>, Reservation);

/// The data parts of a graph's adjacencies, the file's results as they are now, and the
/// events of the entries that an operation counts, opened in the graph's budget.
pub(crate) struct Parts<'a> {
    results: Option<Container<Bytes>>,
    out: Section<'a, Bytes>,
    inn: Option<Section<'a, Bytes>>,
    events: [Option<Opened<'a>>; 2],
}

impl<'a> Parts<'a> {
    /// The parts of `g` that `op` reads.
    pub(crate) fn of(g: &'a Shared, op: &Operation<'_>) -> Result<Self, Failure> {
        let file = g.file();
        let counted = matches!(
            op,
            Operation::PageRank {
                weight: Weight::Edges,
                ..
            }
        );
        let open = |events: Result<Option<Opened<'a>>, OpenError>| {
            events.map_err(|e| match e {
                OpenError::Budget(refused) => error::budget(&refused, g.budget_bytes()),
                other => Failure::new("CorruptFile").text("why", other),
            })
        };
        let corrupt = |name, e: &dyn std::fmt::Display| {
            Failure::new("CorruptFile")
                .text("section", name)
                .text("why", e)
        };
        // A file replaced by another graph keeps no result of this one.
        let results = g
            .path()
            .and_then(|p| everygraph_io::FileSource::open(p).ok())
            .and_then(|s| Container::open(Bytes::Read(s)).ok())
            .filter(|c| c.graph_identity() == file.container().graph_identity());
        Ok(Self {
            results,
            out: file.data().map_err(|e| corrupt("adjacency.out", &e))?,
            inn: match file.in_index() {
                Some(_) => Some(file.in_data().map_err(|e| corrupt("adjacency.in", &e))?),
                None => None,
            },
            events: if counted {
                [
                    open(file.events(g.memory()))?,
                    open(file.in_events(g.memory()))?,
                ]
            } else {
                [None, None]
            },
        })
    }

    /// Bytes the graph's budget holds for the events.
    fn held(&self) -> u64 {
        self.events.iter().flatten().map(|(_, r)| r.bytes()).sum()
    }

    /// The lists of `g` as `run::execute` reads them.
    pub(crate) fn lists(&'a self, g: &'a Shared) -> Lists<'a, Bytes> {
        let file = g.file();
        Lists {
            file: self.results.as_ref().unwrap_or(file.container()),
            index: file.index(),
            data: &self.out,
            inn: file
                .in_index()
                .zip(self.inn.as_ref())
                .map(|(i, d)| (i, d as &(dyn ByteSource + Sync))),
            events: [0, 1].map(|i| self.events[i].as_ref().map(|(e, _)| e)),
            directed: file.directed(),
        }
    }
}

/// The plan of `op` on `g` as `asked`, for `session`, the graph's session.
///
/// # Errors
///
/// A usage error for a budget above the machine (P-16), with the refusal's document;
/// a damaged file.
pub(crate) fn plan<'a>(
    g: &Shared,
    op: &Operation<'a>,
    parts: &Parts<'_>,
    session: &Session,
    asked: Asked,
) -> Result<planner::Plan<'a>, Failure> {
    let file = g.file();
    let mut facts = g.facts().clone();
    facts.held += parts.held();
    let results = parts.lists(g).file;
    facts.kept =
        run::kept(results, op, file.directed(), file.nodes()).map_err(|e| failure(op, e, 0))?;
    let request = Request {
        memory: asked.memory,
        threads: asked.threads,
        ..Request::default()
    };
    let request = session.request(&parts.lists(g), request);
    let dir = g.path().and_then(Path::parent);
    planner::plan(op, &facts, &machine(dir), &request)
        .map_err(|r| Failure::new("UsageError").text("document", r.to_json()))
}

/// The budget of the run of `plan`: what the plan's budget leaves beside what the
/// graph holds in its own budget and the raw copy `session` keeps.
pub(crate) fn budget_of(
    g: &Shared,
    plan: &planner::Plan<'_>,
    parts: &Parts<'_>,
    session: &Session,
) -> u64 {
    let raw = if plan.drops_raw() {
        0
    } else {
        session.raw_bytes().unwrap_or(0)
    };
    plan.budget()
        .saturating_sub(g.facts().held + parts.held())
        .saturating_sub(raw)
}

/// Runs `plan` on `g` within `memory`, in `session`, sending a phase event to
/// `events` as each phase begins; keeps a result computed on the whole graph of a file
/// in it. Returns the result and, when it could not be kept, why.
pub(crate) fn execute(
    g: &Shared,
    plan: &planner::Plan<'_>,
    parts: &Parts<'_>,
    session: &mut Session,
    memory: &Memory,
    events: &Events,
) -> Result<(Computed, Option<String>), Failure> {
    let started = now();
    let op = plan.operation();
    let begin = |p: &planner::Phase| events.send(Event::Begin(p.name, None));
    let fail = |e| failure(op, e, plan.budget());
    // A stream reads the data part by positional reads of the file, not through the map.
    let streamed = match &parts.results {
        Some(file) if plan.regime() == planner::Regime::Streamed => {
            Some(file.section(OUT_DATA).map_err(|e| {
                Failure::new("CorruptFile")
                    .text("section", OUT_DATA)
                    .text("why", e)
            })?)
        }
        _ => None,
    };
    let lists = parts.lists(g);
    let lists = match &streamed {
        Some(data) => Lists { data, ..lists },
        None => lists,
    };
    let output = run::execute(plan, &lists, memory, session, &begin).map_err(fail)?;
    let directed = g.file().directed();
    let unkept = match g.path() {
        Some(path) if op.stored(directed) && !plan.kept() => {
            keep(g, path, op, &output, started).err()
        }
        _ => None,
    };
    let computed = computed(output, memory).map_err(|e| fail(RunError::from(e)))?;
    Ok((computed, unkept))
}

/// Nanoseconds since 1970-01-01 UTC, for provenance.
fn now() -> Option<i64> {
    let since = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?;
    i64::try_from(since.as_nanos()).ok()
}

/// Commits `output`, the result of `op` on the graph of `g`, to its file at `path`,
/// unless another process writes it or it holds another graph now; the reason it could
/// not.
fn keep(
    g: &Shared,
    path: &Path,
    op: &Operation<'_>,
    output: &Output,
    started: Option<i64>,
) -> Result<(), String> {
    let sink = everygraph_io::FileSink::open(path).map_err(|e| e.to_string())?;
    let mut writer = Writer::update(sink).map_err(|e| e.to_string())?;
    let graph = g.file().container().graph_identity();
    if writer.graph_identity() != graph {
        return Err(format!("{} holds another graph now", path.display()));
    }
    let about = Provenance {
        engine: format!("everygraph {}", env!("CARGO_PKG_VERSION")),
        started,
        finished: now(),
        ..Provenance::default()
    };
    run::store(&mut writer, op, graph, &about, output).map_err(|e| e.to_string())?;
    writer.commit().map(drop).map_err(|e| e.to_string())
}

/// The failure of `op` within a budget of `budget` bytes.
fn failure(op: &Operation<'_>, e: RunError, budget: u64) -> Failure {
    match e {
        RunError::Kernel(e) => error::kernel(op.name(), e, budget),
        RunError::Unsupported(why) => Failure::new("Unsupported")
            .text("operation", op.name())
            .text("reason", why),
        RunError::Stored(e) => Failure::new("CorruptFile")
            .text("section", "results")
            .text("why", e),
        RunError::Refused(r) => Failure::new("Refusal").text("document", r.to_json()),
        RunError::Threads(e) => Failure::new("EverygraphError")
            .text("message", format!("Cannot start the threads: {e}.")),
    }
}

/// The largest value of `values` that is not `null`.
fn largest(values: &SegVec<u32>, null: Option<u32>) -> Option<u32> {
    values
        .segments()
        .flatten()
        .copied()
        .filter(|&v| Some(v) != null)
        .max()
}

/// The table of `output`, its bitmaps of nulls held under `memory`, and what its `meta`
/// gains: a `PageRank`'s iterations, the number of components, the largest core, the
/// deepest hop.
fn computed(output: Output, memory: &Memory) -> Result<Computed, MemoryError> {
    let mut out = Computed {
        columns: Vec::new(),
        ids: Vec::new(),
        extra: Vec::new(),
    };
    if let Some(i) = output.iterations {
        out.extra.push(("iterations", Extra::Count(u64::from(i))));
    }
    if let Some(c) = output.converged {
        out.extra.push(("converged", Extra::Flag(c)));
    }
    for (name, column) in output.columns {
        match column {
            Column::U32 {
                values,
                null,
                nodes: true,
            } => {
                let ids = values
                    .segments()
                    .flatten()
                    .map(|&v| if Some(v) == null { NULL } else { v })
                    .collect();
                out.ids.push((name, ids));
            }
            Column::U32 { values, null, .. } => {
                let most = largest(&values, null).map_or(0, u64::from);
                let fact = match name {
                    "component" => {
                        Some(("components", if values.is_empty() { 0 } else { most + 1 }))
                    }
                    "core" => Some(("largest_core", most)),
                    "distance" => Some(("hops", most)),
                    _ => None,
                };
                if let Some((key, value)) = fact {
                    out.extra.push((key, Extra::Count(value)));
                }
                out.columns
                    .push((name, table::lend_u32(values, null, memory)?));
            }
            Column::F64(values) => out.columns.push((name, table::lend::<Float64Type>(values))),
            Column::F32(values) => out.columns.push((name, table::lend::<Float32Type>(values))),
        }
    }
    Ok(out)
}
