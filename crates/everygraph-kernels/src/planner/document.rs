// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A plan as people and programs read it: the step table of `explain`
//! (`docs/ux-contract.md` § 3.4), the plan document (§ 3.6) and the error document of
//! a refusal (§ 6.1), with the numbers as § 2.6 writes them.

use std::fmt;

use everygraph_core::{AccessHint, SystemMemory};
use serde_json::{Value, json};

use super::{Estimate, Origin, Plan, Refusal, Regime, Shortfall, Tier, Way};
use crate::run::{Operation, Precision};

/// For figures in text and in a document's floats.
fn to_f64(x: u64) -> f64 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "a printed figure of a few digits"
    )]
    let x = x as f64;
    x
}

/// A size in powers of 10, one decimal below 100 (§ 2.6).
fn size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    let (mut value, mut unit) = (to_f64(bytes), 0);
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    match (unit, value) {
        (0, _) => format!("{bytes} B"),
        (_, v) if v < 100.0 => format!("{v:.1} {}", UNITS[unit]),
        (_, v) => format!("{v:.0} {}", UNITS[unit]),
    }
}

/// A budget that can be typed, rounded up (P-14): a multiple of 1 GB from 4 GB, of
/// 0.5 GB from 1 GB, of 100 MB below.
#[must_use]
pub fn typed(bytes: u64) -> String {
    let step = match bytes {
        b if b < 1_000_000_000 => 100_000_000,
        b if b < 4_000_000_000 => 500_000_000,
        _ => 1_000_000_000,
    };
    let up = bytes.div_ceil(step).max(1) * step;
    if up % 1_000_000_000 == 0 {
        format!("{}GB", up / 1_000_000_000)
    } else if up >= 1_000_000_000 {
        format!("{:.1}GB", to_f64(up) / 1e9)
    } else {
        format!("{}MB", up / 1_000_000)
    }
}

/// A count with thousands separators.
fn count(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, d) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(d);
    }
    out
}

/// A time estimate whose interval is wide: "about" and two significant digits (P-8).
fn about(seconds: f64) -> String {
    let round = |x: f64| {
        let digits = if x >= 10.0 { 0 } else { 1 };
        format!("{x:.digits$}")
    };
    if seconds < 1.0 {
        format!("about {} ms", round(seconds * 1000.0))
    } else if seconds < 120.0 {
        format!("about {} s", round(seconds))
    } else if seconds < 7200.0 {
        format!("about {} min", round(seconds / 60.0))
    } else {
        format!("about {} h", round(seconds / 3600.0))
    }
}

fn origin(o: Origin) -> &'static str {
    match o {
        Origin::Argument => "argument",
        Origin::Graph => "graph",
        Origin::Environment => "environment",
        Origin::Default => "default",
    }
}

fn regime_name(r: Regime) -> &'static str {
    match r {
        Regime::InMemory => "in_memory",
        Regime::Mapped => "mapped",
    }
}

fn advice_name(a: AccessHint) -> &'static str {
    match a {
        AccessHint::Sequential => "read in order",
        AccessHint::Random => "random, no read-ahead",
        AccessHint::WillNeed => "warmed ahead",
        _ => "default read-ahead",
    }
}

/// How the Python call of `op` starts, before its keywords.
fn python(op: &Operation) -> String {
    match op {
        Operation::Bfs { .. } => "g.bfs(source, ".to_owned(),
        other => format!("g.{}(", other.name()),
    }
}

fn budget_way(op: &Operation, bytes: u64, note: &str) -> Way {
    let typed = typed(bytes);
    Way {
        command: format!("--memory {typed}"),
        python: format!("{}memory=\"{typed}\")", python(op)),
        note: note.to_owned(),
        needs_bytes: Some(bytes),
    }
}

/// P-16: a budget above the machine's memory in all.
pub(super) fn too_much(op: &Operation, bytes: u64, memory: &SystemMemory) -> Refusal {
    let free = memory.available();
    Refusal {
        error: "UsageError",
        operation: op.name(),
        message: "The memory budget is more than this machine has.".to_owned(),
        why: format!(
            "The budget asks for {}; this machine has {} of memory in all.",
            size(bytes),
            size(memory.total())
        ),
        shortfall: None,
        machine_memory_bytes: Some(memory.total()),
        ways_forward: vec![
            budget_way(op, free, "what is free now"),
            Way {
                command: "leave out --memory".to_owned(),
                python: "leave out memory=".to_owned(),
                note: format!("the default, half of free RAM: {} now", size(free / 2)),
                needs_bytes: None,
            },
        ],
    }
}

/// A plan that serves a kept result the file does not hold: it was made for another
/// file, or told of a result `run::kept` did not find.
impl Refusal {
    pub(crate) fn not_kept(op: &Operation) -> Self {
        Self {
            error: "Unsupported",
            operation: op.name(),
            message: format!("The kept result of {} is not in this file.", op.name()),
            why: "the plan was made for a kept result that the file does not hold".to_owned(),
            shortfall: None,
            machine_memory_bytes: None,
            ways_forward: Vec::new(),
        }
    }
}

/// An operation that this graph, or this version, cannot run.
pub(super) fn unsupported(op: &Operation, reason: &'static str) -> Refusal {
    Refusal {
        error: "Unsupported",
        operation: op.name(),
        message: format!("{} cannot be done on this graph.", op.name()),
        why: reason.to_owned(),
        shortfall: None,
        machine_memory_bytes: None,
        ways_forward: Vec::new(),
    }
}

/// P-13: the plan's state and buffers do not fit its budget, even on one thread when
/// the planner chose `first` threads.
pub(super) fn refusal(plan: &Plan, first: std::num::NonZeroUsize) -> Refusal {
    let cost = &plan.cost;
    let machine = plan.machine.memory.map(|m| m.total());
    let smallest = plan.peak;
    let possible = machine.is_none_or(|m| smallest <= m);
    let op = &plan.op;
    let state = if plan.graph.kept {
        plan.peak
    } else {
        cost.state()
    };
    let bytes_each = state / cost.nodes.max(1);
    let mut ways = Vec::new();
    if possible {
        let note = if plan.threads < first {
            format!(
                "the smallest budget that works, on {} threads",
                plan.threads
            )
        } else {
            "the smallest budget that works".to_owned()
        };
        ways.push(budget_way(op, smallest, &note));
    } else {
        let at_least = format!("a machine with at least {} of memory", typed(smallest));
        ways.push(Way {
            command: at_least.clone(),
            python: at_least,
            note: String::new(),
            needs_bytes: Some(smallest),
        });
    }
    if let Operation::PageRank {
        params,
        precision: Precision::Full,
    } = *op
    {
        let low = Operation::PageRank {
            params,
            precision: Precision::Low,
        };
        let low = super::cost::Cost::of(&low, &plan.graph, plan.threads, plan.tier).peak();
        let low = (plan.graph.held + low).max(plan.graph.opened);
        if low <= plan.budget {
            ways.push(Way {
                command: "--precision low".to_owned(),
                python: "g.pagerank(precision=\"low\")".to_owned(),
                note: "32-bit state: about half the memory, slightly less exact".to_owned(),
                needs_bytes: Some(low),
            });
        }
    }
    let what = if cost.raw > 0 {
        "per-node state and the raw copy of the edges"
    } else {
        "per-node state"
    };
    let mut why = format!(
        "{} needs {} for {what} ({} nodes × {} bytes).",
        op.name(),
        size(state),
        count(cost.nodes),
        bytes_each
    );
    match machine {
        Some(m) if !possible => why.push_str(&format!(
            " The budget is {}, and this machine has {} of memory in all, so no budget \
             on it is enough.",
            size(plan.budget),
            size(m)
        )),
        _ => why.push_str(&format!(" The budget is {}.", size(plan.budget))),
    }
    if cost.raw == 0 {
        why.push_str(" The edges stay on disk either way.");
    }
    Refusal {
        error: "InsufficientBudget",
        operation: op.name(),
        message: "Not enough memory for this step.".to_owned(),
        why,
        shortfall: Some(Shortfall {
            needs_bytes: state,
            budget_bytes: plan.budget,
            smallest_budget_bytes: possible.then_some(smallest),
            nodes: cost.nodes,
            bytes_each,
        }),
        machine_memory_bytes: machine,
        ways_forward: ways,
    }
}

impl Refusal {
    /// The error document (ux-contract § 6.1, `urn:everygraph:schema:error:1`).
    #[must_use]
    pub fn to_json(&self) -> String {
        self.document().to_string()
    }

    fn document(&self) -> Value {
        let mut doc = json!({
            "schema": 1,
            "error": self.error,
            "message": self.message,
            "why": self.why,
        });
        let fields = doc.as_object_mut().expect("an object");
        let mut put = |key: &str, value: Value| {
            fields.insert(key.to_owned(), value);
        };
        if let Some(s) = &self.shortfall {
            put("step", json!(self.operation));
            put("needs_bytes", json!(s.needs_bytes));
            put("budget_bytes", json!(s.budget_bytes));
            put("smallest_budget_bytes", json!(s.smallest_budget_bytes));
            put(
                "arithmetic",
                json!({"count": s.nodes, "unit": "nodes", "bytes_each": s.bytes_each}),
            );
        }
        if self.error == "Unsupported" {
            put("operation", json!(self.operation));
            put("reason", json!(self.why));
        }
        if let Some(m) = self.machine_memory_bytes {
            put("machine_memory_bytes", json!(m));
        }
        let ways: Vec<Value> = self
            .ways_forward
            .iter()
            .map(|w| {
                let mut way = json!({"command": w.command, "python": w.python, "note": w.note});
                if let Some(b) = w.needs_bytes {
                    way["needs_bytes"] = json!(b);
                }
                way
            })
            .collect();
        put("ways_forward", Value::Array(ways));
        put("changed", json!(false));
        doc
    }
}

fn estimate(e: Estimate) -> Value {
    json!({"value": e.value, "low": e.low, "high": e.high, "basis": e.basis})
}

fn bytes(b: u64, basis: &str) -> Value {
    json!({"value": b, "low": b, "high": b, "basis": basis})
}

/// One row of the step table: what, where, the memory it holds, the work.
struct Step {
    what: &'static str,
    place: &'static str,
    memory: Option<u64>,
    work: Option<String>,
}

impl Plan {
    fn steps(&self) -> Vec<Step> {
        let cost = &self.cost;
        if self.graph.kept {
            return vec![Step {
                what: "read the kept result",
                place: "memory",
                memory: Some(cost.output),
                work: None,
            }];
        }
        let mut steps = vec![Step {
            what: "keep per-node state",
            place: "memory",
            memory: Some(cost.state() - cost.raw),
            work: None,
        }];
        if self.tier == Tier::Raw {
            steps.push(Step {
                what: "decode the edges into memory",
                place: "memory",
                memory: Some(cost.raw),
                work: Some(
                    if self.build_raw {
                        "once, kept for the next searches"
                    } else {
                        "kept from an earlier search"
                    }
                    .to_owned(),
                ),
            });
        }
        let (what, work) = match self.op {
            Operation::PageRank { params, .. } if params.tol > 0.0 => (
                "compute ranks",
                format!("{} iterations at most", params.max_iter),
            ),
            Operation::PageRank { params, .. } => {
                ("compute ranks", format!("{} iterations", params.max_iter))
            }
            Operation::Components => ("join components", "2 passes".to_owned()),
            Operation::KCore => ("peel cores", "a round per core number".to_owned()),
            Operation::Degrees => ("count degrees", "1 pass".to_owned()),
            Operation::Bfs { .. } => ("search from the source", "a level at a time".to_owned()),
        };
        let place = if self.regime == Regime::Mapped && self.tier == Tier::Compressed {
            "edges from disk"
        } else {
            "memory"
        };
        steps.push(Step {
            what,
            place,
            memory: Some(cost.buffers),
            work: Some(work),
        });
        if self.op.stored() {
            steps.push(Step {
                what: "write the result",
                place: "disk",
                memory: None,
                work: None,
            });
        }
        steps
    }

    /// P-6: the regime with its arithmetic.
    fn reason(&self) -> String {
        let ratio = self.ratio();
        match self.regime {
            Regime::InMemory => "the graph fits the memory budget".to_owned(),
            _ if ratio >= 1.0 => format!("the graph is {ratio:.1}× the memory budget"),
            _ => format!(
                "the graph is {ratio:.1}× the memory budget, and the state leaves {} of it",
                size(self.budget.saturating_sub(self.peak))
            ),
        }
    }

    fn ratio(&self) -> f64 {
        to_f64(self.graph.data_bytes) / to_f64(self.budget.max(1))
    }

    fn parameters(&self) -> Value {
        let mut p = serde_json::Map::new();
        for (name, value) in self.op.parameters() {
            let v = match (value.parse::<u64>(), value.parse::<f64>()) {
                (Ok(i), _) => json!(i),
                (_, Ok(x)) => json!(x),
                _ => json!(value),
            };
            p.insert(name.to_owned(), v);
        }
        Value::Object(p)
    }

    /// The plan document (ux-contract § 3.6, `urn:everygraph:schema:plan:1`).
    #[must_use]
    pub fn to_json(&self) -> String {
        let m = &self.machine;
        let memory = m.memory;
        let storage = m.storage;
        let free_disk = storage.map_or(0, |s| s.free_bytes);
        let g = &self.graph;
        let steps: Vec<Value> = self
            .steps()
            .into_iter()
            .map(|s| json!({"step": s.what, "where": s.place, "memory_bytes": s.memory, "work": s.work}))
            .collect();
        let phases: Vec<Value> = self
            .phases
            .iter()
            .map(|p| {
                json!({
                    "name": p.name,
                    "kept": p.kept,
                    "seconds": estimate(p.seconds),
                    "regime": regime_name(self.regime),
                })
            })
            .collect();
        let output_path = (self.op.stored() && !g.kept)
            .then(|| g.path.clone())
            .flatten();
        let output = if output_path.is_some() {
            json!({"path": output_path, "kind": "result", "free_bytes": free_disk})
        } else {
            json!({"path": null, "kind": "none"})
        };
        // Scratch would go beside the graph file (`docs/api-sketch.md` § 2.4); these
        // operations write none.
        let scratch_dir = g
            .path
            .as_deref()
            .and_then(|p| p.rsplit_once(['/', '\\']))
            .map_or(".", |(dir, _)| if dir.is_empty() { "/" } else { dir });
        let doc = json!({
            "schema": 1,
            "operation": self.op.name(),
            "parameters": self.parameters(),
            "input": null,
            "graph": {
                "path": g.path,
                "bytes": g.bytes,
                "nodes": g.nodes,
                "edges": g.edges,
                "directed": g.directed,
            },
            "detected": null,
            "machine": {
                "cores": m.cores.get(),
                "hardware_threads": m.hardware_threads.get(),
                "memory_bytes": memory.map_or(0, |x| x.total()),
                "memory_free_bytes": memory.map_or(0, |x| x.available()),
                "memory_limit_bytes": memory.and_then(|x| x.limit),
                "storage": {
                    "kind": storage.map_or_else(|| "unknown".to_owned(), |s| s.kind.to_string()),
                    "free_bytes": free_disk,
                },
            },
            "budget": {
                "memory_bytes": self.budget,
                "memory_from": origin(self.budget_from),
                "above_free": memory.is_some_and(|x| self.budget > x.available()),
                "threads": self.threads.get(),
                "threads_from": origin(self.threads_from),
            },
            "regime": {
                "name": regime_name(self.regime),
                "reason": self.reason(),
                "working_bytes": g.data_bytes,
                "ratio": self.ratio(),
            },
            "steps": steps,
            "phases": phases,
            "estimate": {
                "seconds": estimate(self.seconds),
                "peak_memory_bytes": bytes(self.peak, "model"),
                "scratch_bytes": bytes(0, "exact"),
                "output_bytes": bytes(if output_path.is_some() { self.cost.output } else { 0 }, "model"),
            },
            "scratch": {
                "path": scratch_dir,
                "from": "default",
                "memory_backed": storage.is_some_and(|s| s.kind == everygraph_core::StorageKind::Memory),
                "free_bytes": free_disk,
            },
            "output": output,
            "fits": self.fits(),
            "refusal": self.refusal.as_ref().map(Refusal::document),
        });
        doc.to_string()
    }
}

impl fmt::Display for Plan {
    /// The step table of `explain` and `--dry-run` (ux-contract § 3.4).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = self
            .graph
            .path
            .as_deref()
            .map_or("the graph", |p| p.rsplit(['/', '\\']).next().unwrap_or(p));
        let params: Vec<String> = self
            .op
            .parameters()
            .into_iter()
            .map(|(k, v)| format!("{k} {v}"))
            .collect();
        write!(f, "Plan for {}() on {name}", self.op.name())?;
        if !params.is_empty() {
            write!(f, " — {}", params.join(", "))?;
        }
        writeln!(f)?;
        let budget = size(self.budget);
        match &self.refusal {
            None => writeln!(f, "  fits     peak {} of {budget} budget", size(self.peak))?,
            Some(r) if r.shortfall.is_some() => writeln!(
                f,
                "  fits     no — needs {}, the budget is {budget}",
                size(self.peak)
            )?,
            Some(r) => writeln!(f, "  fits     no — {}", r.why)?,
        }
        writeln!(f)?;
        for (i, s) in self.steps().iter().enumerate() {
            let memory = s.memory.map_or_else(|| "—".to_owned(), size);
            let work = s.work.as_deref().unwrap_or("");
            let row = format!(
                "  {}  {:<32} {:<16} {memory:>8}   {work}",
                i + 1,
                s.what,
                s.place
            );
            writeln!(f, "{}", row.trim_end())?;
        }
        writeln!(f)?;
        let c = &self.cost;
        writeln!(
            f,
            "  why      {} nodes × {} bytes = {} of state",
            count(c.nodes),
            c.state() / c.nodes.max(1),
            size(c.state())
        )?;
        let edges = size(self.graph.data_bytes);
        match self.regime {
            Regime::InMemory => {
                writeln!(f, "           the edges ({edges}) fit the memory budget")?;
            }
            _ => writeln!(
                f,
                "           the edges ({edges}) stay on disk and are read as needed"
            )?,
        }
        let advice: Vec<String> = self
            .phases
            .iter()
            .map(|p| format!("{} {}", p.name, advice_name(p.advice)))
            .collect();
        writeln!(
            f,
            "  threads  {} · reads: {}",
            self.threads,
            advice.join(", ")
        )?;
        write!(f, "  estimate {}", about(self.seconds.value))?;
        if let Some(r) = &self.refusal {
            write!(f, "\n\n  Ways forward:")?;
            for w in &r.ways_forward {
                let line = format!("    {:<24} {}", w.command, w.note);
                write!(f, "\n{}", line.trim_end())?;
            }
        }
        Ok(())
    }
}
