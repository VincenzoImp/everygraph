// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The planner (#45) through `run::execute`, on the oracle's CI corpus:
//!
//! - every choice it can make — tier, regime, threads, advice — gives the bits of the
//!   in-memory run on raw arrays (DESIGN D14);
//! - a plan's peak bounds what the run reserves, and at a budget of exactly the
//!   smallest budget it names the run fits, while one byte less is refused before any
//!   work;
//! - a session decodes the raw copy after the break-even count of traversals, not
//!   before;
//! - plan and error documents are valid against the closed schemas of
//!   `docs/ux-contract.md`.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::print_stderr,
    reason = "test code: small graphs; the schemas are read from the docs; the run on real graphs prints what it measured"
)]

use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::sync::{Arc, OnceLock};

use everygraph_core::{
    AccessHint, Budget, Memory, MemorySource, Storage, StorageKind, SystemMemory, par,
};
use everygraph_format::adjacency::{AdjacencyIndex, encode};
use everygraph_kernels::edge_map::{Csr, Plan as Lanes, Settings};
use everygraph_kernels::pagerank::PageRank;
use everygraph_kernels::planner::{
    self, Force, Graph, Machine, Origin, Plan, Regime, Request, Tier, advice, break_even,
};
use everygraph_kernels::run::{
    self, Column, Lists, Operation, Output, Precision, RunError, Session, execute,
};
use everygraph_oracle::corpus::{Tier as Corpus, entries};
use everygraph_oracle::projection::Projection;
use serde_json::Value;

mod support;

fn threads(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).unwrap()
}

/// A machine of 16 cores whose memory is not known, so that no budget is above it.
fn machine() -> Machine {
    Machine {
        cores: threads(16),
        hardware_threads: threads(32),
        memory: None,
        storage: Some(Storage {
            kind: StorageKind::Nvme,
            free_bytes: 1 << 40,
        }),
    }
}

/// The adjacency of a test graph, encoded as a graph file holds it.
struct Stored {
    offsets: Vec<u64>,
    targets: Vec<u32>,
    index: AdjacencyIndex,
    data: MemorySource<Vec<u8>>,
    directed: bool,
}

impl Stored {
    fn new(offsets: Vec<u64>, targets: Vec<u32>, directed: bool) -> Self {
        let encoded = encode(&offsets, &targets).unwrap();
        Self {
            index: AdjacencyIndex::parse(&encoded.index).unwrap(),
            data: MemorySource::new(encoded.data),
            offsets,
            targets,
            directed,
        }
    }

    fn of(p: &Projection) -> Self {
        let adj = if p.directed { &p.out } else { &p.undirected };
        let l = support::with_loops(adj, p);
        assert_eq!(
            (l.weight.len(), l.cost.len()),
            (l.targets.len(), l.targets.len())
        );
        Self::new(l.offsets, l.targets, p.directed)
    }

    fn lists(&self) -> Lists<'_> {
        Lists {
            index: &self.index,
            data: &self.data,
            directed: self.directed,
        }
    }

    fn graph(&self) -> Graph {
        let bytes = self.index.data_bytes();
        Graph {
            directed: self.directed,
            ..Graph::of(
                &self.index,
                Some("graphs/test.eg".into()),
                bytes,
                self.index.entries(),
            )
        }
    }

    /// `op` on raw arrays in the heap, 1 thread, no budget: the reference.
    fn in_memory(&self, op: &Operation) -> Result<Vec<u64>, RunError> {
        let csr = Csr::new(&self.offsets, &self.targets).unwrap();
        let memory = Memory::unlimited();
        par::with_threads(threads(1), || {
            Lanes::with(&csr, &memory, Settings::default(), |p| {
                run::run(op, &p, self.directed, &memory)
            })
        })
        .unwrap()
        .map(|o| bits(&o))
    }
}

/// Every value of every column, as bits, and the iteration facts.
fn bits(o: &Output) -> Vec<u64> {
    let mut out = vec![
        u64::from(o.iterations.unwrap_or(u32::MAX)),
        u64::from(o.converged.unwrap_or(false)),
    ];
    for (_, column) in &o.columns {
        match column {
            Column::U32 { values, .. } => {
                out.extend(values.segments().flatten().map(|&x| u64::from(x)));
            }
            Column::F64(values) => out.extend(values.segments().flatten().map(|x| x.to_bits())),
            Column::F32(values) => {
                out.extend(values.segments().flatten().map(|x| u64::from(x.to_bits())));
            }
        }
    }
    out
}

fn operations(nodes: u32) -> Vec<Operation> {
    let params = PageRank::default();
    let mut ops = vec![
        Operation::PageRank {
            params,
            precision: Precision::Full,
        },
        Operation::PageRank {
            params,
            precision: Precision::Low,
        },
        Operation::Components,
        Operation::KCore,
        Operation::Degrees,
    ];
    if nodes > 0 {
        ops.push(Operation::Bfs {
            source: nodes / 2,
            max_hops: None,
        });
    }
    ops
}

fn request(memory: u64, force: Force) -> Request {
    Request {
        memory: Some((memory, Origin::Argument)),
        force,
        ..Request::default()
    }
}

/// Plans `op` under `budget` with `force`, runs it on the plan's threads in a budget of
/// that size, and returns its bits with the plan and the budget's peak.
fn planned(
    s: &Stored,
    op: &Operation,
    budget: u64,
    force: Force,
    session: &mut Session,
) -> (Plan, Result<Vec<u64>, RunError>, u64) {
    let plan = planner::plan(
        op,
        &s.graph(),
        &machine(),
        &session.request(request(budget, force)),
    )
    .unwrap();
    let accountant = Arc::new(Budget::new(budget));
    let memory = Memory::from(Arc::clone(&accountant));
    let out = par::with_threads(plan.threads(), || {
        execute(&plan, s.lists(), &memory, session)
    })
    .unwrap()
    .map(|o| bits(&o));
    (plan, out, accountant.peak())
}

const PLENTY: u64 = 1 << 40;

/// Checks one graph: every forced choice against the reference, the plan's peak, and
/// the smallest budget.
fn check(name: &str, s: &Stored) -> usize {
    let mut checked = 0;
    for op in operations(s.index.nodes()) {
        let reference = s.in_memory(&op);
        if let Err(RunError::Unsupported(_)) = reference {
            let plan = planner::plan(
                &op,
                &s.graph(),
                &machine(),
                &request(PLENTY, Force::default()),
            )
            .unwrap();
            assert_eq!(
                plan.refusal().map(|r| r.error),
                Some("Unsupported"),
                "{name}"
            );
            continue;
        }
        let reference = reference.unwrap_or_else(|e| panic!("{name} {}: {e}", op.name()));
        for tier in [Tier::Compressed, Tier::Raw] {
            for regime in [Regime::InMemory, Regime::Mapped] {
                for t in [1, 3] {
                    let force = Force {
                        tier: Some(tier),
                        regime: Some(regime),
                        threads: Some(threads(t)),
                        advice: None,
                    };
                    let (plan, got, peak) = planned(s, &op, PLENTY, force, &mut Session::new());
                    let got = got.unwrap_or_else(|e| panic!("{name} {}: {e}", op.name()));
                    assert!(
                        got == reference,
                        "{name} {} {tier:?} {regime:?} {t}",
                        op.name()
                    );
                    assert!(
                        peak <= plan.peak_bytes(),
                        "{name} {} {tier:?} {t} threads: reserved {peak}, planned {}",
                        op.name(),
                        plan.peak_bytes()
                    );
                }
            }
        }
        // The smallest budget runs; a byte less is refused, before any reservation.
        let force = Force {
            threads: Some(threads(3)),
            ..Force::default()
        };
        let smallest = planned(s, &op, PLENTY, force, &mut Session::new())
            .0
            .peak_bytes();
        let (plan, got, _) = planned(s, &op, smallest, force, &mut Session::new());
        assert!(plan.fits(), "{name} {}", op.name());
        assert!(
            got.as_ref().is_ok_and(|g| *g == reference),
            "{name} {} at its smallest budget {smallest}: {:?}",
            op.name(),
            got.err()
        );
        if smallest > 0 {
            let (plan, got, peak) = planned(s, &op, smallest - 1, force, &mut Session::new());
            let r = plan.refusal().expect("refused");
            assert_eq!(r.shortfall.unwrap().smallest_budget_bytes, Some(smallest));
            assert!(
                matches!(got, Err(RunError::Refused(_))) && peak == 0,
                "{name}"
            );
        }
        checked += 1;
    }
    checked
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "minutes in a debug build; `oracle/run-ci.sh` (the oracle job) runs it in release"
)]
fn every_choice_gives_the_bits_of_the_in_memory_run_on_the_corpus() {
    let mut checked = 0;
    for entry in entries(Corpus::Ci) {
        let p = Projection::new(&entry.graph);
        checked += check(&entry.graph.name, &Stored::of(&p));
    }
    assert!(checked > 1000, "{checked}");
}

/// A star (one hub of degree 20,000, so k-core samples it and readers grow) and a path:
/// the extremes of the frontier and reader bounds, small enough for a debug build.
#[test]
fn every_choice_gives_the_bits_of_the_in_memory_run_on_a_star_and_a_path() {
    let n: u32 = 20_001;
    let mut star = (vec![0_u64], Vec::new());
    for v in 0..n {
        if v == 0 {
            star.1.extend(1..n);
        } else {
            star.1.push(0);
        }
        star.0.push(star.1.len() as u64);
    }
    let mut path = (vec![0_u64], Vec::new());
    for v in 0..n {
        if v > 0 {
            path.1.push(v - 1);
        }
        if v + 1 < n {
            path.1.push(v + 1);
        }
        path.0.push(path.1.len() as u64);
    }
    for (name, (offsets, targets)) in [("star", star), ("path", path)] {
        assert!(check(name, &Stored::new(offsets, targets, false)) > 0);
    }
}

fn small() -> Stored {
    // Two triangles joined by an edge, and an isolated node.
    let lists: [&[u32]; 7] = [
        &[1, 2],
        &[0, 2],
        &[0, 1, 3],
        &[2, 4, 5],
        &[3, 5],
        &[3, 4],
        &[],
    ];
    let mut offsets = vec![0];
    let mut targets = Vec::new();
    for l in lists {
        targets.extend_from_slice(l);
        offsets.push(targets.len() as u64);
    }
    Stored::new(offsets, targets, false)
}

#[test]
fn a_session_decodes_the_raw_copy_after_the_break_even_count_of_traversals() {
    let s = small();
    let bfs = Operation::Bfs {
        source: 0,
        max_hops: None,
    };
    let reference = s.in_memory(&bfs).unwrap();
    let t = threads(4);
    let k = break_even(t);
    let mut session = Session::new();
    let force = Force {
        threads: Some(t),
        ..Force::default()
    };
    for run in 0..k + 3 {
        let (plan, got, _) = planned(&s, &bfs, PLENTY, force, &mut session);
        assert_eq!(got.unwrap(), reference);
        let want = if run < k { Tier::Compressed } else { Tier::Raw };
        assert_eq!(plan.tier(), want, "run {run}, break-even {k}");
        assert_eq!(plan.builds_raw(), run == k, "run {run}");
    }
    // A scan never reads the copy.
    let (plan, _, _) = planned(&s, &Operation::Components, PLENTY, force, &mut session);
    assert_eq!(plan.tier(), Tier::Compressed);
    // A copy that does not fit with its margin is not built.
    let raw = Force {
        tier: Some(Tier::Raw),
        ..force
    };
    let tight = planned(&s, &bfs, PLENTY, raw, &mut Session::new())
        .0
        .peak_bytes();
    let mut late = Session::new();
    for _ in 0..k {
        let _ = planned(&s, &bfs, PLENTY, force, &mut late);
    }
    assert_eq!(
        planned(&s, &bfs, tight, force, &mut late).0.tier(),
        Tier::Compressed
    );
    assert_eq!(
        planned(&s, &bfs, tight * 10 / 9 + 1, force, &mut late)
            .0
            .tier(),
        Tier::Raw
    );
}

#[test]
fn the_break_even_follows_the_cost_model() {
    // Exploratory constants (cost.rs): decode 0.197 ns an entry, BFS 0.537 compressed
    // and 0.287 raw at 16 threads.
    assert_eq!(break_even(threads(16)), 1);
    assert!(break_even(threads(1)) >= 1);
}

#[test]
fn threads_are_the_physical_cores_and_fewer_for_a_small_graph() {
    let s = small();
    let op = Operation::Components;
    let plan = |r: Request| planner::plan(&op, &s.graph(), &machine(), &r).unwrap();
    assert_eq!(
        plan(request(PLENTY, Force::default())).threads(),
        threads(1)
    );
    let mut big = s.graph();
    big.entries = 1 << 40;
    let plan = |r: Request| planner::plan(&op, &big, &machine(), &r).unwrap();
    assert_eq!(plan(Request::default()).threads(), threads(16));
    let asked = Request {
        threads: Some((threads(4), Origin::Argument)),
        ..Request::default()
    };
    assert_eq!(plan(asked).threads(), threads(4));
}

#[test]
fn advice_is_random_only_for_random_access_over_its_budget() {
    assert_eq!(advice(true, Regime::Mapped, 10, 9), AccessHint::Random);
    assert_eq!(advice(true, Regime::Mapped, 9, 9), AccessHint::Normal);
    assert_eq!(advice(false, Regime::Mapped, 10, 9), AccessHint::Normal);
    assert_eq!(advice(true, Regime::InMemory, 10, 9), AccessHint::WillNeed);
}

#[test]
fn the_regime_follows_the_edges_beside_the_state() {
    let s = small();
    let op = Operation::Degrees;
    let plan =
        |b| planner::plan(&op, &s.graph(), &machine(), &request(b, Force::default())).unwrap();
    let state = plan(PLENTY).peak_bytes();
    let edges = s.index.data_bytes();
    assert_eq!(plan(state + edges).regime(), Regime::InMemory);
    assert_eq!(plan(state + edges - 1).regime(), Regime::Mapped);
    assert!(plan(state).fits() && !plan(state - 1).fits());
}

#[test]
fn a_budget_above_the_machine_is_a_usage_error_and_above_free_memory_is_honoured() {
    let s = small();
    let mut m = machine();
    m.memory = Some(SystemMemory::new(
        64_000_000_000,
        16_000_000_000,
        None,
        None,
    ));
    let op = Operation::KCore;
    let refused = planner::plan(
        &op,
        &s.graph(),
        &m,
        &request(80_000_000_000, Force::default()),
    )
    .unwrap_err();
    assert_eq!(refused.error, "UsageError");
    check_schema("error", &json(&refused.to_json()));
    let plan = planner::plan(
        &op,
        &s.graph(),
        &m,
        &request(40_000_000_000, Force::default()),
    )
    .unwrap();
    assert!(plan.fits());
    assert_eq!(
        json(&plan.to_json())["budget"]["above_free"],
        Value::Bool(true)
    );
    // The default is half of what is free (P-11).
    let plan = planner::plan(&op, &s.graph(), &m, &Request::default()).unwrap();
    assert_eq!(plan.budget(), 8_000_000_000);
}

#[test]
fn a_refusal_offers_only_what_works() {
    let s = small();
    let full = Operation::PageRank {
        params: PageRank::default(),
        precision: Precision::Full,
    };
    let low = Operation::PageRank {
        params: PageRank::default(),
        precision: Precision::Low,
    };
    let plan = |op: &Operation, b| {
        planner::plan(op, &s.graph(), &machine(), &request(b, Force::default())).unwrap()
    };
    let need_low = plan(&low, PLENTY).peak_bytes();
    let refused = plan(&full, need_low);
    let ways: Vec<&str> = refused
        .refusal()
        .unwrap()
        .ways_forward
        .iter()
        .map(|w| w.command.as_str())
        .collect();
    assert_eq!(ways.len(), 2, "{ways:?}");
    assert!(ways[0].starts_with("--memory ") && ways[1] == "--precision low");
    let refused = plan(&full, need_low - 1);
    assert_eq!(refused.refusal().unwrap().ways_forward.len(), 1);
    // No budget on a machine smaller than the smallest: no `--memory` line (P-15).
    let mut m = machine();
    m.memory = Some(SystemMemory::new(100, 50, None, None));
    let r = planner::plan(&full, &s.graph(), &m, &request(10, Force::default())).unwrap();
    let r = r.refusal().unwrap();
    assert_eq!(r.shortfall.unwrap().smallest_budget_bytes, None);
    assert!(
        r.ways_forward[0]
            .command
            .starts_with("a machine with at least")
    );
}

#[test]
fn smallest_budgets_are_typed_rounded_up() {
    assert_eq!(planner::typed(5_598_053_768), "6GB");
    assert_eq!(planner::typed(1_200_000_001), "1.5GB");
    assert_eq!(planner::typed(1), "100MB");
}

// ---- Documents ----------------------------------------------------------------------

fn json(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

/// Every ```` ```json ```` block with a `$schema` key in `docs/ux-contract.md`, by `$id`.
fn schemas() -> &'static BTreeMap<String, Value> {
    static SCHEMAS: OnceLock<BTreeMap<String, Value>> = OnceLock::new();
    SCHEMAS.get_or_init(|| {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/ux-contract.md");
        let text = std::fs::read_to_string(path).unwrap().replace("\r\n", "\n");
        let mut found = BTreeMap::new();
        for block in text.split("```json\n").skip(1) {
            let value: Value = serde_json::from_str(block.split("\n```").next().unwrap()).unwrap();
            if let (Some(id), Some(_)) = (
                value.get("$id").and_then(Value::as_str),
                value.get("$schema"),
            ) {
                found.insert(id.to_owned(), value.clone());
            }
        }
        found
    })
}

/// Checks `doc` against the closed schema `urn:everygraph:schema:NAME:1`.
fn check_schema(name: &str, doc: &Value) {
    let id = format!("urn:everygraph:schema:{name}:1");
    let mut registry = jsonschema::Registry::new();
    for (uri, s) in schemas() {
        registry = registry.add(uri, s.clone()).unwrap();
    }
    let registry = registry.prepare().unwrap();
    let validator = jsonschema::options()
        .with_registry(&registry)
        .build(&schemas()[&id])
        .unwrap();
    let errors: Vec<String> = validator
        .iter_errors(doc)
        .map(|e| format!("{}: {e}", e.instance_path()))
        .collect();
    assert!(errors.is_empty(), "{id}: {errors:?}\n{doc}");
}

#[test]
fn plans_and_refusals_are_valid_against_the_closed_schemas() {
    let s = small();
    let mut m = machine();
    m.memory = Some(SystemMemory::new(
        64_000_000_000,
        16_000_000_000,
        Some(32_000_000_000),
        None,
    ));
    let mut directed = s.graph();
    directed.directed = true;
    let mut kept = s.graph();
    kept.kept = true;
    let raw = Force {
        tier: Some(Tier::Raw),
        ..Force::default()
    };
    for op in operations(7) {
        for (graph, budget, force) in [
            (s.graph(), PLENTY.min(1 << 30), Force::default()),
            (s.graph(), 1 << 30, raw),
            (s.graph(), 100, Force::default()),
            (directed.clone(), 1 << 30, Force::default()),
            (kept.clone(), 1 << 30, Force::default()),
        ] {
            let plan = planner::plan(&op, &graph, &m, &request(budget, force)).unwrap();
            let doc = json(&plan.to_json());
            check_schema("plan", &doc);
            assert_eq!(doc["fits"], Value::Bool(plan.fits()));
            if let Some(r) = plan.refusal() {
                check_schema("error", &json(&r.to_json()));
                assert_eq!(doc["refusal"], json(&r.to_json()));
            }
            // The text form is the step table of `explain`.
            let text = plan.to_string();
            assert!(
                text.starts_with(&format!("Plan for {}() on test.eg", op.name())),
                "{text}"
            );
        }
    }
}

#[test]
fn a_refusal_states_its_arithmetic() {
    let s = small();
    let op = Operation::Components;
    let plan = planner::plan(&op, &s.graph(), &machine(), &request(10, Force::default())).unwrap();
    let doc = json(&plan.refusal().unwrap().to_json());
    assert_eq!(doc["error"], "InsufficientBudget");
    assert_eq!(doc["arithmetic"]["count"], 7);
    assert_eq!(doc["arithmetic"]["bytes_each"], 25);
    assert_eq!(doc["smallest_budget_bytes"], plan.peak_bytes());
    assert_eq!(doc["changed"], Value::Bool(false));
}

/// This machine, as `everygraph_io` probes it, with the storage of `dir`.
fn probed(dir: &std::path::Path) -> Machine {
    use everygraph_core::MemoryProbe;
    let hardware = std::thread::available_parallelism().unwrap();
    Machine {
        cores: everygraph_io::physical_cores()
            .and_then(NonZeroUsize::new)
            .unwrap_or(hardware),
        hardware_threads: hardware,
        memory: everygraph_io::ProcMemoryProbe::new().probe().ok(),
        storage: everygraph_io::storage(dir),
    }
}

/// Exploratory, on the graphs of the dataset registry: what each operation reserves
/// against what its plan names, and the iterations `PageRank` runs. Both are counts that
/// do not depend on the machine's load.
#[test]
#[ignore = "reads csr/lj and csr/orkut under $EVERYGRAPH_DATA; run under a memory limit"]
fn the_plan_bounds_what_runs_reserve_on_real_graphs() {
    let data = std::path::PathBuf::from(std::env::var("EVERYGRAPH_DATA").unwrap());
    let words = |path: std::path::PathBuf, width: usize| -> Vec<u64> {
        let bytes = std::fs::read(path).unwrap();
        bytes
            .chunks_exact(width)
            .map(|c| c.iter().rev().fold(0, |x, &b| x << 8 | u64::from(b)))
            .collect()
    };
    for name in ["lj", "orkut"] {
        let dir = data.join("csr").join(name);
        let offsets = words(dir.join("offsets.u64"), 8);
        let targets = words(dir.join("targets.u32"), 4)
            .into_iter()
            .map(|t| t as u32)
            .collect();
        let s = Stored::new(offsets, targets, false);
        let force = Force {
            threads: Some(threads(16)),
            ..Force::default()
        };
        for op in operations(s.index.nodes()) {
            let (plan, out, peak) = planned(&s, &op, PLENTY, force, &mut Session::new());
            assert!(
                out.is_ok() && peak <= plan.peak_bytes(),
                "{name} {}",
                op.name()
            );
            let out = out.unwrap();
            let iterations = out[0];
            // The other choices give the same bits, on a graph of the registry too.
            for (tier, regime, t) in [
                (Tier::Raw, Regime::InMemory, 3),
                (Tier::Compressed, Regime::Mapped, 5),
            ] {
                let other = Force {
                    tier: Some(tier),
                    regime: Some(regime),
                    threads: Some(threads(t)),
                    advice: None,
                };
                let got = planned(&s, &op, PLENTY, other, &mut Session::new()).1;
                assert!(
                    got.unwrap() == out,
                    "{name} {} {tier:?} {regime:?}",
                    op.name()
                );
            }
            if name == "orkut" && op.name() == "pagerank" {
                let probed = probed(&dir);
                for budget in [None, Some((50_000_000, Origin::Argument))] {
                    let r = Request {
                        memory: budget,
                        ..Request::default()
                    };
                    let plan = planner::plan(&op, &s.graph(), &probed, &r).unwrap();
                    eprintln!("{plan}\n{}\n", plan.to_json());
                }
            }
            eprintln!(
                "{name} {:10} {:?} planned {:>12} reserved {:>12} ({:.2}) iterations {iterations}",
                op.name(),
                op.parameters(),
                plan.peak_bytes(),
                peak,
                peak as f64 / plan.peak_bytes() as f64,
            );
        }
    }
}
