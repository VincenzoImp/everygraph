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
//!   edges fit beside the state, `mapped` when only the state does. Both read the graph
//!   file through its memory map; the budget never counts mapped pages
//!   (`docs/api-sketch.md` § 2.4). `streamed` needs a stream source over the compressed
//!   format (#46), and is not chosen yet.
//! - **The decode tier** (ADR 0004, condition 5): traversals read a raw copy of the
//!   adjacency, decoded into the budget, once the session has run as many traversals on
//!   compressed lists as the decode costs ([`break_even`]), and only when the copy and
//!   the state fit together with a margin. `PageRank`, components, k-core and degrees
//!   always read the compressed lists, as ADR 0004 rules: there the copy does not repay
//!   its decode (#118).
//! - **Threads** (DESIGN D13): the physical cores for the memory-bound kernels of this
//!   crate, fewer for a small graph, at most what was asked.
//! - **Access advice per phase** (ADR 0029): `MADV_RANDOM` only for a random-access
//!   phase whose working set exceeds the budget; a scan resets `MADV_NORMAL`; a graph
//!   that fits is advised `WillNeed`, so that the map is warm.
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
pub use document::typed;

use crate::edge_map::Settings;
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
    /// The state fits; the edges are read in passes (#46).
    Streamed,
    /// A build whose working data does not fit.
    OnDisk,
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
    /// Whether the file already keeps this operation's result (`run::stored`).
    pub kept: bool,
}

impl Graph {
    /// The facts of `index`, the out-adjacency of a graph file of `bytes` bytes at
    /// `path` with `edges` edges. Reads the index's degrees once: about a nanosecond a
    /// node.
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
        }
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
    /// Whether the session holds the raw copy of the graph already.
    pub raw_held: bool,
    /// Choices forced by the caller.
    pub force: Force,
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
    /// The state: per-node arrays, sets and the raw copy.
    pub needs_bytes: u64,
    /// The budget.
    pub budget_bytes: u64,
    /// The exact smallest budget that fits, the state and the readers' buffers (P-14);
    /// `None` when no budget on this machine would.
    pub smallest_budget_bytes: Option<u64>,
    /// The nodes of the arithmetic.
    pub nodes: u64,
    /// The bytes each node needs, rounded down: `nodes × bytes_each` is most of
    /// `needs_bytes`.
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
pub struct Plan {
    op: Operation,
    graph: Graph,
    machine: Machine,
    budget: u64,
    budget_from: Origin,
    threads: NonZeroUsize,
    threads_from: Origin,
    regime: Regime,
    tier: Tier,
    build_raw: bool,
    settings: Settings,
    cost: cost::Cost,
    peak: u64,
    phases: Vec<Phase>,
    seconds: Estimate,
    refusal: Option<Refusal>,
}

/// Decides how `op` runs on `graph` (module documentation).
///
/// # Errors
///
/// A usage error (P-16) when the budget asked for is more than the machine's memory in
/// all: it is never lowered.
pub fn plan(
    op: &Operation,
    graph: &Graph,
    machine: &Machine,
    request: &Request,
) -> Result<Plan, Box<Refusal>> {
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
    let threads = request
        .force
        .threads
        .unwrap_or_else(|| cost::threads(graph, cap));

    let compressed = cost::Cost::of(op, graph, threads, Tier::Compressed);
    let raw = cost::Cost::of(op, graph, threads, Tier::Raw);
    let tier = request.force.tier.unwrap_or_else(|| {
        let wanted = cost::traverses(op)
            && (request.raw_held || request.traversals_before >= break_even(threads));
        if wanted && cost::fits_with_margin(raw.peak(), budget) {
            Tier::Raw
        } else {
            Tier::Compressed
        }
    });
    let cost = if tier == Tier::Raw { raw } else { compressed };
    let build_raw = tier == Tier::Raw && !request.raw_held && !graph.kept;

    // The raw copy is in memory, and the kernel does not read the map.
    let room = budget.saturating_sub(cost.peak());
    let regime = request
        .force
        .regime
        .unwrap_or(if tier == Tier::Raw || graph.data_bytes <= room {
            Regime::InMemory
        } else {
            Regime::Mapped
        });
    let settings = Settings {
        prefetch: regime != Regime::InMemory && cost.access != cost::Access::Scan,
        ..Settings::default()
    };
    let advice = request.force.advice.unwrap_or(match tier {
        Tier::Raw => AccessHint::Normal,
        Tier::Compressed => advice(false, regime, graph.data_bytes, room),
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
    phases.push(Phase {
        name: cost.phase,
        kept: graph.kept,
        seconds: if graph.kept {
            Estimate::exact(0.0)
        } else {
            cost::seconds(op, &cost, graph, regime, budget, machine.storage)
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

    // A kept result is read into the budget instead of computed.
    let peak = if graph.kept { cost.output } else { cost.peak() };
    let mut plan = Plan {
        op: *op,
        graph: graph.clone(),
        machine: *machine,
        budget,
        budget_from,
        threads,
        threads_from,
        regime,
        tier,
        build_raw,
        settings,
        cost,
        peak,
        phases,
        seconds,
        refusal: None,
    };
    if let Some(reason) = run::unsupported(op, graph.directed, graph.nodes) {
        plan.refusal = Some(document::unsupported(op, reason));
    } else if peak > budget {
        plan.refusal = Some(document::refusal(&plan));
    }
    Ok(plan)
}

impl Plan {
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
    pub fn operation(&self) -> &Operation {
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

    /// How the edge-map framework cuts and reads.
    #[must_use]
    pub fn settings(&self) -> Settings {
        self.settings
    }

    /// The phases in order.
    #[must_use]
    pub fn phases(&self) -> &[Phase] {
        &self.phases
    }

    /// The most memory the run holds: the estimate's peak, an upper bound of what the
    /// kernel reserves.
    #[must_use]
    pub fn peak_bytes(&self) -> u64 {
        self.peak
    }

    /// Wall time.
    #[must_use]
    pub fn seconds(&self) -> Estimate {
        self.seconds
    }
}
