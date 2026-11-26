// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The planner: how a whole-graph operation runs within its memory budget, chosen before
//! any work, and shown as the plan of `docs/ux-contract.md` § 3.
//!
//! [`plan`] reads only what the graph file's index and the machine say (P-2), and the
//! cost model of each kernel (`cost.rs`). It decides:
//!
//! - **Whether the operation fits** (P-5): the kernel's per-node state, the result
//!   columns and the readers' buffers must fit the budget. If they do not, the plan is
//!   a refusal with the exact smallest budget that fits (P-13, P-14, M-5), and with the
//!   ways forward that exist (P-15): a larger budget, and for `PageRank` the f32 state.
//!   A lower-precision variant is offered, never chosen (DESIGN D14).
//! - **The regime** (DESIGN D11, ux-contract § 3.2): `in_memory` when the compressed
//!   edges fit beside the state, through the graph file's memory map; `streamed` when
//!   only the state does: the edges are read in passes by positional reads
//!   (`CompressedStream`), each reader's window in the budget, so the process holds
//!   the state, the index and the windows only. Fewer threads hold fewer readers, so a
//!   graph whose edges do not fit on the threads first chosen is held or streamed on
//!   fewer; `mapped` when the stream's readers do not fit beside the state on
//!   any thread count: the edges are read from the map, whose pages the budget never
//!   counts (`docs/api-sketch.md` § 2.4). A stream reads the out-lists only: an
//!   operation that reads a directed graph's in-lists too is held or mapped.
//! - **The decode tier** (ADR 0004, condition 5): a BFS of the whole graph reads a raw
//!   copy of the adjacency, decoded into the budget, once the session has run as many
//!   traversals on compressed lists as the decode costs ([`break_even`]), and only when
//!   the copy and the state fit together with a margin. `PageRank`, components, k-core,
//!   degrees and a BFS cut at a depth always read the compressed lists, as ADR 0004
//!   rules: there the copy does not repay its decode (#118). The copy holds the
//!   out-lists only: a directed graph with in-lists searches on the compressed lists,
//!   bottom-up steps included, and an operation that reads the in-lists cannot be forced
//!   onto it. A copy the session holds counts in every later plan; a plan it does not
//!   fit beside drops it first.
//! - **The undirected projection** of a directed graph, for core numbers and a search
//!   in both directions: merged in memory from the out- and in-lists before the kernel
//!   runs, as a phase of its own (`merging`), and counted in the plan's peak.
//! - **Threads** (DESIGN D13): the physical cores for the memory-bound kernels of this
//!   crate, fewer for a small graph, at most what was asked, and fewer again when the
//!   readers of more threads would not fit.
//! - **A kept result** (`run::kept`): served from the file in place of the kernel, by
//!   the lookup `run::execute` serves it with.
//! - **Access advice per phase** (ADR 0029): `MADV_RANDOM` only for a random-access
//!   phase whose working set exceeds the budget; a scan resets `MADV_NORMAL`; a graph
//!   that fits is advised `WillNeed`, so that the map is warm.
//!
//! Strong components hold a set of the pairs their searches find, which grows with the
//! graph's structure: their plan counts the per-node state only, says it is not a bound
//! ([`Plan::bounded`]), and its refusal names no smallest budget.
//!
//! Every choice can be forced ([`Force`]). None changes a result: every source gives
//! the same bits, for any thread count (DESIGN D14, the edge-map framework's rules),
//! and `tests/planner.rs` runs every choice against the in-memory run.
//!
//! [`run::execute`] carries a plan out. A plan is a value: making one reads nothing and
//! writes nothing. It prints as the step table of `explain` (§ 3.4) and serializes to
//! the plan document of § 3.6 ([`Plan::to_json`]).

mod cost;
mod document;

use std::num::NonZeroUsize;

use everygraph_core::{AccessHint, Storage, SystemMemory};
use everygraph_format::adjacency::AdjacencyIndex;

pub use cost::{advice, break_even};
pub use document::{about, typed};

use crate::edge_map::{CompressedStream, Settings};
use crate::run::{self, Operation};

/// Where a setting came from (`origin` of `docs/ux-contract.md` § 3.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// The call's argument, or the command's option.
    Argument,
    /// The value given when the graph was opened.
    Graph,
    /// `EVERYGRAPH_MEMORY` or `EVERYGRAPH_THREADS`.
    Environment,
    /// The built-in default.
    Default,
}

/// How an operation runs (`regime.name`, ux-contract § 3.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Regime {
    /// The state and the compressed edges fit the budget.
    InMemory,
    /// The state fits; the edges are read from the map as needed.
    Mapped,
    /// The state fits; the edges are read in passes, in windows in the budget.
    Streamed,
}

/// Where the kernels read lists from (DESIGN D4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// The compressed adjacency of the graph file.
    Compressed,
    /// A raw copy decoded into the budget, kept by the session.
    Raw,
}

/// What the machine has (P-2), as `everygraph_io` probes it.
#[derive(Clone, Copy, Debug)]
pub struct Machine {
    /// Physical cores.
    pub cores: NonZeroUsize,
    /// Hardware threads.
    pub hardware_threads: NonZeroUsize,
    /// Memory, if it could be read.
    pub memory: Option<SystemMemory>,
    /// The storage of the graph file, if it could be read.
    pub storage: Option<Storage>,
}

/// What the graph file says about the graph, without a pass over its edges.
#[derive(Clone, Debug)]
pub struct Graph {
    /// The file, as it was named; `None` for a graph in memory.
    pub path: Option<String>,
    /// Bytes of the file.
    pub bytes: u64,
    /// Nodes.
    pub nodes: u32,
    /// Stored edges: every event counts.
    pub edges: u64,
    /// Entries of the adjacency the kernels read.
    pub entries: u64,
    /// Bytes of its compressed data part.
    pub data_bytes: u64,
    /// The longest list.
    pub max_degree: u32,
    /// Whether the graph is directed.
    pub directed: bool,
    /// Whether the file serves this operation's result: `run::kept`, the lookup that
    /// `run::execute` serves it by.
    pub kept: bool,
    /// Bytes the open graph file holds of the same budget for the whole run: its
    /// adjacency indexes, which `everygraph_ingest::graph::Graph` keeps reserved. `of`
    /// sets the heap of the index it reads; a directed graph's caller adds its
    /// in-adjacency's.
    pub held: u64,
    /// The most the budget held while the file was opened: `Graph::open` reads an index
    /// section whole and parses it, three times its length. A smallest budget below it
    /// cannot open the graph. 0 when the caller does not count the opening.
    pub opened: u64,
    /// The in-adjacency of a directed graph, when the file holds one.
    pub inn: Option<InLists>,
}

/// What the index of a directed graph's in-adjacency says; it has the out-adjacency's
/// nodes and entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InLists {
    /// Bytes of its compressed data part.
    pub data_bytes: u64,
    /// The longest list.
    pub max_degree: u32,
}

impl Graph {
    /// The facts of `index`, the out-adjacency of a graph file of `bytes` bytes at
    /// `path` with `edges` edges. Reads the index's degrees once, in one pass.
    #[must_use]
    pub fn of(index: &AdjacencyIndex, path: Option<String>, bytes: u64, edges: u64) -> Self {
        let n = index.nodes();
        Self {
            path,
            bytes,
            nodes: n,
            edges,
            entries: index.entries(),
            data_bytes: index.data_bytes(),
            max_degree: index.degrees(0..n).max().unwrap_or(0),
            directed: false,
            kept: false,
            held: u64::try_from(index.heap_bytes()).unwrap_or(u64::MAX),
            opened: 0,
            inn: None,
        }
    }

    /// The same graph, with `index` as its in-adjacency, whose parsed index the open
    /// file holds too. Reads the index's degrees once.
    #[must_use]
    pub fn with_in(self, index: &AdjacencyIndex) -> Self {
        Self {
            inn: Some(InLists {
                data_bytes: index.data_bytes(),
                max_degree: index.degrees(0..index.nodes()).max().unwrap_or(0),
            }),
            held: self
                .held
                .saturating_add(u64::try_from(index.heap_bytes()).unwrap_or(u64::MAX)),
            ..self
        }
    }

    /// Whether the kernels can read the in-lists of this graph.
    fn has_in(&self) -> bool {
        self.directed && self.inn.is_some()
    }
}

/// Choices made by the caller instead of the planner: every choice can be forced
/// (#45). A forced choice that does not fit is refused like any other.
#[derive(Clone, Copy, Debug, Default)]
pub struct Force {
    /// The regime.
    pub regime: Option<Regime>,
    /// The decode tier.
    pub tier: Option<Tier>,
    /// The access advice of every phase.
    pub advice: Option<AccessHint>,
    /// Exactly these threads, where `threads` is the most the planner may use.
    pub threads: Option<NonZeroUsize>,
}

/// What was asked, besides the operation.
#[derive(Clone, Copy, Debug, Default)]
pub struct Request {
    /// The memory budget and where it came from; `None`: half of what is free (P-11).
    pub memory: Option<(u64, Origin)>,
    /// The most threads and where that came from; `None`: the physical cores.
    pub threads: Option<(NonZeroUsize, Origin)>,
    /// Traversals the session ran on this graph before this one.
    pub traversals_before: u32,
    /// The raw copy the session holds, if any.
    pub held: Option<Held>,
    /// Choices forced by the caller.
    pub force: Force,
}

/// A raw copy that a session holds, and so counts in every plan (`run::Session`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Held {
    /// Bytes it holds of the budget.
    pub bytes: u64,
    /// Whether it is a copy of this graph; another graph's copy is dropped first.
    pub usable: bool,
}

/// A way forward of a refusal (ux-contract M-5): the same three keys everywhere.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Way {
    /// As the command line shows it.
    pub command: String,
    /// The same, as a Python call.
    pub python: String,
    /// Its consequence, or empty.
    pub note: String,
    /// What it needs, when it is a budget.
    pub needs_bytes: Option<u64>,
}

/// A plan that cannot run, or a budget that cannot be asked for: the error document of
/// ux-contract § 6.1 ([`Refusal::to_json`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    /// `InsufficientBudget`; `UsageError` for a budget above the machine (P-16);
    /// `Unsupported` for an operation this graph or version cannot run.
    pub error: &'static str,
    /// The operation.
    pub operation: &'static str,
    /// The headline.
    pub message: String,
    /// The reason, with its arithmetic.
    pub why: String,
    /// For `InsufficientBudget`: what does not fit.
    pub shortfall: Option<Shortfall>,
    /// The machine's memory in all, if known.
    pub machine_memory_bytes: Option<u64>,
    /// What would work (P-15).
    pub ways_forward: Vec<Way>,
}

/// What a plan needs that the budget does not have (P-13).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shortfall {
    /// All the plan needs: the state, the readers' buffers, and what the open graph and
    /// the session hold.
    pub needs_bytes: u64,
    /// The state: per-node arrays, sets, the raw copy and the projection.
    pub state_bytes: u64,
    /// What the state is, in words: `per-node state`, and the copy it holds.
    pub what: &'static str,
    /// Whether the edges stay on disk, whatever the budget.
    pub on_disk: bool,
    /// The budget.
    pub budget_bytes: u64,
    /// The exact smallest budget that fits, the state and the readers' buffers (P-14);
    /// `None` when no budget on this machine would.
    pub smallest_budget_bytes: Option<u64>,
    /// The nodes of the arithmetic.
    pub nodes: u64,
    /// The bytes each node needs, rounded down: `nodes × bytes_each` is most of
    /// `state_bytes`.
    pub bytes_each: u64,
}

/// An estimate: a value, an interval, and its basis (ux-contract P-7).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Estimate {
    /// The value.
    pub value: f64,
    /// The interval's lower end.
    pub low: f64,
    /// The interval's upper end.
    pub high: f64,
    /// `exact`, `sampled` or `model`.
    pub basis: &'static str,
}

impl Estimate {
    fn exact(value: f64) -> Self {
        Self {
            value,
            low: value,
            high: value,
            basis: "exact",
        }
    }
}

/// One phase of a plan: its name as progress shows it, whether a stored result serves
/// it, its time, its regime and the advice its reads get.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Phase {
    /// The name (`^[a-z]{1,11}$`).
    pub name: &'static str,
    /// Served by a stored result (P-12): zero time.
    pub kept: bool,
    /// The time.
    pub seconds: Estimate,
    /// How its reads are advised.
    pub advice: AccessHint,
}

/// A plan (ux-contract § 3.2): what an operation will do and cost, or why it cannot.
#[derive(Clone, Debug)]
pub struct Plan<'a> {
    op: Operation<'a>,
    graph: Graph,
    machine: Machine,
    budget: u64,
    budget_from: Origin,
    threads: NonZeroUsize,
    threads_from: Origin,
    regime: Regime,
    tier: Tier,
    build_raw: bool,
    drop_raw: bool,
    settings: Settings,
    /// The read window of the streamed regime; 0 in the others.
    window: usize,
    cost: cost::Cost,
    peak: u64,
    phases: Vec<Phase>,
    seconds: Estimate,
    refusal: Option<Refusal>,
}

/// Why a forced streamed regime cannot read the raw tier.
const RAW_STREAMED: &str = "the raw copy is decoded into memory, so it cannot be streamed";

/// Why a forced raw tier cannot run (`RawCopy`).
const NO_RAW_COPY: &str = "a raw copy needs its offsets in one segment and every list \
     within one part of at most 1 GiB, and this graph has more nodes or a longer list";

/// Why a forced streamed regime cannot run an operation that reads the in-lists.
const NO_STREAM_IN: &str = "the streamed regime reads the out-lists only, and on a \
     directed graph this operation reads the in-lists too";

/// Why a forced raw tier cannot run an operation that reads the in-lists.
const NO_RAW_IN: &str = "a raw copy holds the out-lists only, and on a directed graph \
     this operation reads the in-lists too";

/// Decides how `op` runs on `graph` (module documentation). When the plan does not fit
/// on the threads it would choose, it takes fewer: each holds a reader, and the thread
/// count never changes a result (DESIGN D14). A refusal names the smallest budget over
/// every thread count, which is the one of a single thread.
///
/// # Errors
///
/// A usage error (P-16) when the budget asked for is more than the machine's memory in
/// all: it is never lowered.
pub fn plan<'a>(
    op: &Operation<'a>,
    graph: &Graph,
    machine: &Machine,
    request: &Request,
) -> Result<Plan<'a>, Box<Refusal>> {
    let memory = machine.memory;
    let (budget, budget_from) = match request.memory {
        Some((bytes, from)) => {
            if let Some(m) = memory.filter(|m| bytes > m.total()) {
                return Err(Box::new(document::too_much(op, bytes, &m)));
            }
            (bytes, from)
        }
        None => (cost::default_budget(memory), Origin::Default),
    };
    let (cap, threads_from) = request.threads.unwrap_or((machine.cores, Origin::Default));
    let at = |threads| Plan {
        budget_from,
        threads_from,
        ..choose(op, graph, machine, request, budget, threads)
    };
    let first = request
        .force
        .threads
        .unwrap_or_else(|| cost::threads(graph, cap));
    let mut plan = at(first);
    if request.force.threads.is_none() {
        let mut t = first.get();
        while plan.peak > budget && t > 1 {
            t -= 1;
            plan = at(NonZeroUsize::new(t).unwrap_or(NonZeroUsize::MIN));
        }
        // Edges that do not fit are held or streamed on fewer threads rather than
        // mapped: fewer threads hold fewer readers, and the readers and the windows are
        // in the budget, where the map's pages are not.
        if plan.regime == Regime::Mapped && request.force.regime.is_none() {
            let fewer = (1..plan.threads.get())
                .rev()
                .filter_map(NonZeroUsize::new)
                .map(&at)
                .find(|p| p.regime != Regime::Mapped);
            plan = fewer.unwrap_or(plan);
        }
    }
    if let Some(reason) = run::unsupported(op, graph.directed, graph.has_in(), graph.nodes) {
        plan.refusal = Some(document::unsupported(op, reason));
    } else if plan.tier == Tier::Raw && !cost::raw_fits(graph) {
        plan.refusal = Some(document::unsupported(op, NO_RAW_COPY));
    } else if plan.tier == Tier::Raw && graph.has_in() && run::reads_in(op) {
        plan.refusal = Some(document::unsupported(op, NO_RAW_IN));
    } else if plan.tier == Tier::Raw && plan.regime == Regime::Streamed {
        plan.refusal = Some(document::unsupported(op, RAW_STREAMED));
    } else if plan.regime == Regime::Streamed && !streams(op, graph) {
        plan.refusal = Some(document::unsupported(op, NO_STREAM_IN));
    } else if plan.peak > budget {
        plan.refusal = Some(document::refusal(&plan, first));
    }
    Ok(plan)
}

/// The plan of `op` on `threads` threads within `budget`, before the refusal.
fn choose<'a>(
    op: &Operation<'a>,
    graph: &Graph,
    machine: &Machine,
    request: &Request,
    budget: u64,
    threads: NonZeroUsize,
) -> Plan<'a> {
    let usable = request.held.filter(|h| h.usable);
    let compressed = cost::Cost::of(op, graph, threads, Tier::Compressed);
    let raw = cost::Cost::of(op, graph, threads, Tier::Raw);
    let tier = match request.force.tier {
        Some(t) => t,
        None => {
            let wanted = traverses(op)
                && !graph.has_in()
                && (usable.is_some() || request.traversals_before >= break_even(threads));
            if wanted
                && cost::raw_fits(graph)
                && cost::fits_with_margin(graph.held + raw.peak(), budget)
            {
                Tier::Raw
            } else {
                Tier::Compressed
            }
        }
    };
    let cost = if tier == Tier::Raw { raw } else { compressed };
    let build_raw = tier == Tier::Raw && usable.is_none() && !graph.kept;
    // A kept result is read into the budget instead of computed; the open file holds its
    // indexes beside either. A copy the session holds counts beside the run, unless it
    // is the run's own tier; when it does not fit beside it, or is of another graph, the
    // run drops it first.
    let peak_of = |cost: &cost::Cost| {
        let own = graph.held + if graph.kept { cost.output } else { cost.peak() };
        let (peak, drop_raw) = match request.held {
            None => (own, false),
            Some(_) if tier == Tier::Raw && !build_raw && !graph.kept => (own, false),
            Some(h) if h.usable && own.saturating_add(h.bytes) <= budget => (own + h.bytes, false),
            Some(_) => (own, true),
        };
        (peak.max(graph.opened), drop_raw)
    };
    let (peak, drop_raw) = peak_of(&cost);

    // The raw copy is in memory, and the kernel does not read the map.
    let room = budget.saturating_sub(peak);
    // The edges that do not fit are streamed, when the stream's readers fit; else read
    // from the map, whose pages the budget does not count. The raw copy is in memory.
    let stream = (tier == Tier::Compressed && streams(op, graph)).then(|| {
        let (cost, window) = cost::streamed(cost, graph, budget);
        (cost, window, peak_of(&cost))
    });
    let regime = request.force.regime.unwrap_or_else(|| {
        if tier == Tier::Raw || cost.data <= room {
            Regime::InMemory
        } else if stream.is_some_and(|(_, _, (peak, _))| peak <= budget) {
            Regime::Streamed
        } else {
            Regime::Mapped
        }
    });
    let (cost, window, peak, drop_raw) = match stream {
        Some((cost, window, (peak, drop_raw))) if regime == Regime::Streamed => {
            (cost, window, peak, drop_raw)
        }
        _ => (cost, 0, peak, drop_raw),
    };
    let settings = if window > 0 {
        CompressedStream::settings_for(graph.entries, graph.data_bytes, window)
    } else {
        Settings {
            prefetch: regime != Regime::InMemory && cost.access != cost::Access::Scan,
            ..Settings::default()
        }
    };
    let advice = request.force.advice.unwrap_or(match tier {
        Tier::Raw => AccessHint::Normal,
        Tier::Compressed => advice(false, regime, cost.data, room),
    });

    let mut phases = Vec::new();
    if build_raw {
        phases.push(Phase {
            name: "decoding",
            kept: false,
            seconds: cost::decode_seconds(graph, threads),
            advice: AccessHint::Sequential,
        });
    }
    if cost.projection > 0 && !graph.kept {
        phases.push(Phase {
            name: "merging",
            kept: false,
            seconds: cost::merge_seconds(graph, threads),
            advice,
        });
    }
    phases.push(Phase {
        name: cost.phase,
        kept: graph.kept,
        seconds: if graph.kept {
            Estimate::exact(0.0)
        } else {
            cost::seconds(op, &cost, regime, budget, machine.storage)
        },
        advice,
    });
    let seconds = phases.iter().fold(Estimate::exact(0.0), |sum, p| Estimate {
        value: sum.value + p.seconds.value,
        low: sum.low + p.seconds.low,
        high: sum.high + p.seconds.high,
        basis: if p.seconds.basis == "exact" {
            sum.basis
        } else {
            "model"
        },
    });
    Plan {
        op: *op,
        graph: graph.clone(),
        machine: *machine,
        budget,
        budget_from: Origin::Default,
        threads,
        threads_from: Origin::Default,
        regime,
        tier,
        build_raw,
        drop_raw,
        settings,
        window,
        cost,
        peak,
        phases,
        seconds,
        refusal: None,
    }
}

/// Whether `op` on `graph` can be streamed: it reads the out-lists alone. On a directed
/// graph whose file holds its in-lists, everything but weak components reads them too.
fn streams(op: &Operation<'_>, graph: &Graph) -> bool {
    !graph.has_in() || matches!(op, Operation::Components { strong: false })
}

/// Whether `op` is a traversal of the whole graph, which the raw copy speeds up
/// (ADR 0004). A BFS cut at a depth reads a few lists: it never repays the decode.
#[must_use]
pub fn traverses(op: &Operation<'_>) -> bool {
    matches!(op, Operation::Bfs { max_hops: None, .. })
}

impl<'a> Plan<'a> {
    /// Whether it can run inside the limits.
    #[must_use]
    pub fn fits(&self) -> bool {
        self.refusal.is_none()
    }

    /// Why not, when it does not fit.
    #[must_use]
    pub fn refusal(&self) -> Option<&Refusal> {
        self.refusal.as_ref()
    }

    /// The operation.
    #[must_use]
    pub fn operation(&self) -> &Operation<'a> {
        &self.op
    }

    /// The memory budget in bytes.
    #[must_use]
    pub fn budget(&self) -> u64 {
        self.budget
    }

    /// The threads to run on.
    #[must_use]
    pub fn threads(&self) -> NonZeroUsize {
        self.threads
    }

    /// The regime.
    #[must_use]
    pub fn regime(&self) -> Regime {
        self.regime
    }

    /// The decode tier.
    #[must_use]
    pub fn tier(&self) -> Tier {
        self.tier
    }

    /// Whether the run decodes the raw copy first, for the session to keep.
    #[must_use]
    pub fn builds_raw(&self) -> bool {
        self.build_raw
    }

    /// Whether the run drops the session's raw copy first: it does not fit beside the
    /// run, or it is of another graph.
    #[must_use]
    pub fn drops_raw(&self) -> bool {
        self.drop_raw
    }

    /// Whether the run serves the result the file keeps instead of computing it.
    #[must_use]
    pub fn kept(&self) -> bool {
        self.graph.kept
    }

    /// How the edge-map framework cuts and reads.
    #[must_use]
    pub fn settings(&self) -> Settings {
        self.settings
    }

    /// The read window of each reader in the streamed regime, in bytes; 0 in the
    /// others.
    #[must_use]
    pub fn window(&self) -> usize {
        self.window
    }

    /// The phases in order.
    #[must_use]
    pub fn phases(&self) -> &[Phase] {
        &self.phases
    }

    /// The most memory the run holds: the estimate's peak, an upper bound of what the
    /// kernel reserves when the plan is [`bounded`](Self::bounded).
    #[must_use]
    pub fn peak_bytes(&self) -> u64 {
        self.peak
    }

    /// Whether the peak bounds what the run reserves: not for strong components, whose
    /// set of pairs grows with the graph's structure.
    #[must_use]
    pub fn bounded(&self) -> bool {
        self.cost.bounded
    }

    /// Wall time.
    #[must_use]
    pub fn seconds(&self) -> Estimate {
        self.seconds
    }
}
