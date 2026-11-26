// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The planner (#45) through `run::execute`, on the oracle's CI corpus:
//!
//! - every choice it can make — tier, regime, threads, advice — gives the bits of the
//!   in-memory run on raw arrays (DESIGN D14), the mapped regime reading a memory map of
//!   a file; a directed graph's operations read its in-lists as a graph file stores
//!   them, and its undirected projection, merged in memory;
//! - a plan's peak bounds what the run reserves, and at a budget of exactly the
//!   smallest budget it names the run fits, while one byte less is refused before any
//!   work; the smallest budget is the one of the fewest threads;
//! - a session decodes the raw copy after the break-even count of traversals, counts
//!   the copy in every later plan, drops it when it does not fit or belongs to another
//!   graph, and serves a kept result by the lookup the plan was made with;
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
    reason = "test code: small graphs; files for the mapped source and the schemas; the runs on real graphs print what they measured"
)]

use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use everygraph_core::{
    AccessHint, Budget, ByteSource, Memory, MemorySource, Storage, StorageKind, SystemMemory, par,
};
use everygraph_format::adjacency::{
    AdjacencyEvents, AdjacencyIndex, Encoded, IndexBuilder, append_record, encode, encode_events,
    merge,
};
use everygraph_format::container::{Container, Kind, Provenance, Writer};
use everygraph_io::MmapSource;
use everygraph_kernels::edge_map::{Csr, Plan as Lanes, RawCopy, Settings};
use everygraph_kernels::pagerank::PageRank;
use everygraph_kernels::planner::{
    self, Force, Graph, Machine, Origin, Plan, Regime, Request, Tier, advice, break_even,
};
use everygraph_kernels::run::{
    self, Column, Direction, Lists, Operation, Output, Precision, RunError, Session, Weight,
    execute,
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

type File = Container<MemorySource<Vec<u8>>>;

/// A graph file in memory whose graph is the adjacency index `index`, keeping the
/// result `kept` of an operation.
fn graph_file(index: &[u8], kept: Option<(&Operation<'_>, &Output)>) -> File {
    let mut w = Writer::create(MemorySource::new(Vec::new())).unwrap();
    w.add_section("adjacency.out.index", Kind::ADJACENCY_INDEX, index, None)
        .unwrap();
    if let Some((op, out)) = kept {
        let graph = w.graph_identity();
        run::store(&mut w, op, graph, &Provenance::default(), out).unwrap();
    }
    w.commit().unwrap();
    Container::open(w.into_sink()).unwrap()
}

/// One adjacency of a test graph, encoded as a graph file holds it: its lists, its
/// index, and its data part in the heap and memory-mapped from a file.
struct Side {
    offsets: Vec<u64>,
    targets: Vec<u32>,
    index_bytes: Vec<u8>,
    index: AdjacencyIndex,
    data: MemorySource<Vec<u8>>,
    map: MmapSource,
    path: PathBuf,
}

impl Drop for Side {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

impl Side {
    fn new(offsets: Vec<u64>, targets: Vec<u32>) -> Self {
        Self::of(offsets, targets, encode)
    }

    /// The in-adjacency of the out-lists `out`, built as the ingest builds it.
    fn inn(out: &Self) -> Self {
        let (offsets, targets) = support::in_lists(&out.offsets, &out.targets);
        Self::of(offsets, targets, |_, _| {
            let encoded = encode(&out.offsets, &out.targets)?;
            Ok::<_, everygraph_format::adjacency::EncodeError>(support::in_adjacency(&encoded))
        })
    }

    fn of<E: std::fmt::Debug>(
        offsets: Vec<u64>,
        targets: Vec<u32>,
        encoder: impl FnOnce(&[u64], &[u32]) -> Result<everygraph_format::adjacency::Encoded, E>,
    ) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let encoded = encoder(&offsets, &targets).unwrap();
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("kernels-planner");
        std::fs::create_dir_all(&dir).unwrap();
        let k = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!("{}-{k}.data", std::process::id()));
        std::fs::write(&path, &encoded.data).unwrap();
        Self {
            index: AdjacencyIndex::parse(&encoded.index).unwrap(),
            index_bytes: encoded.index,
            data: MemorySource::new(encoded.data),
            map: MmapSource::open(&path).unwrap(),
            path,
            offsets,
            targets,
        }
    }

    fn data(&self, mapped: bool) -> &(dyn ByteSource + Sync) {
        if mapped { &self.map } else { &self.data }
    }

    fn csr(&self) -> Csr<'_> {
        Csr::new(&self.offsets, &self.targets).unwrap()
    }
}

/// The lists of `offsets` and `targets`, encoded with the flag of an adjacency that
/// has a `.events` section, and that section, of `counts` events an entry.
fn counted(offsets: &[u64], targets: &[u32], counts: &[u64]) -> (Encoded, &'static [u8]) {
    let nodes = (offsets.len() - 1) as u32;
    let mut builder = IndexBuilder::new(&Memory::unlimited(), nodes).unwrap();
    let mut data = Vec::new();
    for v in 0..nodes as usize {
        let before = data.len();
        let list = targets[offsets[v] as usize..offsets[v + 1] as usize]
            .iter()
            .copied();
        let degree = append_record(v as u32, list, &mut data).unwrap();
        builder.push(degree, (data.len() - before) as u64).unwrap();
    }
    let (index, _) = builder.finish(true).unwrap();
    let total = counts.iter().sum();
    let events = encode_events(targets.len() as u64, total, counts.iter().copied()).unwrap();
    (
        Encoded { index, data },
        Box::leak(events.into_boxed_slice()),
    )
}

/// The events of an adjacency: the section, parsed, and its counts as weights.
struct Events {
    bytes: &'static [u8],
    parsed: AdjacencyEvents<'static>,
    weights: everygraph_core::SegVec<f64>,
}

impl Events {
    fn of(bytes: &'static [u8], index: &AdjacencyIndex, counts: &[u64]) -> Self {
        let mut weights = everygraph_core::SegVec::try_with_len(
            &Memory::unlimited(),
            "weights",
            counts.len(),
            0.0,
        )
        .unwrap();
        for (w, &c) in weights.segments_mut().flatten().zip(counts) {
            *w = c as f64;
        }
        Self {
            bytes,
            parsed: AdjacencyEvents::parse(bytes, index).unwrap(),
            weights,
        }
    }
}

/// A test graph as a graph file holds it: its out-adjacency and, for a directed graph,
/// its in-adjacency; with the events of each when some pair repeats.
struct Stored {
    out: Side,
    inn: Option<Side>,
    events: [Option<Events>; 2],
    file: File,
    directed: bool,
}

impl Stored {
    fn new(offsets: Vec<u64>, targets: Vec<u32>, directed: bool) -> Self {
        let out = Side::new(offsets, targets);
        Self {
            file: graph_file(&out.index_bytes, None),
            inn: directed.then(|| Side::inn(&out)),
            events: [None, None],
            out,
            directed,
        }
    }

    /// The graph whose lists are `lists`, each entry with its number of edges (some
    /// above 1), and, for a directed graph, its in-lists with the same counts.
    fn with_counts(lists: &[Vec<(u32, u64)>], directed: bool) -> Self {
        let csr = |lists: &[Vec<(u32, u64)>]| {
            let mut offsets = vec![0];
            let (mut targets, mut counts) = (Vec::new(), Vec::new());
            for list in lists {
                for &(t, c) in list {
                    targets.push(t);
                    counts.push(c);
                }
                offsets.push(targets.len() as u64);
            }
            (offsets, targets, counts)
        };
        let side = |lists: &[Vec<(u32, u64)>]| {
            let (offsets, targets, counts) = csr(lists);
            let (encoded, bytes) = counted(&offsets, &targets, &counts);
            let side = Side::of(offsets, targets, |_, _| {
                Ok::<_, std::convert::Infallible>(encoded)
            });
            let events = Events::of(bytes, &side.index, &counts);
            (side, events)
        };
        let (out, out_events) = side(lists);
        let (inn, in_events) = if directed {
            let mut back = vec![Vec::new(); lists.len()];
            for (u, list) in lists.iter().enumerate() {
                for &(v, c) in list {
                    back[v as usize].push((u as u32, c));
                }
            }
            let (side, events) = side(&back);
            (Some(side), Some(events))
        } else {
            (None, None)
        };
        Self {
            file: graph_file(&out.index_bytes, None),
            inn,
            events: [Some(out_events), in_events],
            out,
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

    fn nodes(&self) -> u32 {
        self.out.index.nodes()
    }

    /// The lists, their data parts in the heap or `mapped` from the files.
    fn lists(&self, mapped: bool) -> Lists<'_, MemorySource<Vec<u8>>> {
        Lists {
            file: &self.file,
            index: &self.out.index,
            data: self.out.data(mapped),
            inn: self.inn.as_ref().map(|i| (&i.index, i.data(mapped))),
            events: [0, 1].map(|i| self.events[i].as_ref().map(|e| &e.parsed)),
            directed: self.directed,
        }
    }

    fn graph(&self) -> Graph {
        let bytes = self.out.index.data_bytes();
        let graph = Graph {
            directed: self.directed,
            ..Graph::of(
                &self.out.index,
                Some("graphs/test.eg".into()),
                bytes,
                self.out.index.entries(),
            )
        };
        let mut graph = match &self.inn {
            Some(inn) => graph.with_in(&inn.index),
            None => graph,
        };
        // A caller opens the events in the graph's budget, twice their section.
        graph.held += self
            .events
            .iter()
            .flatten()
            .map(|e| 2 * e.bytes.len() as u64)
            .sum::<u64>();
        graph
    }

    /// The weights of the entries of each adjacency, for `op` if it counts edges.
    fn weights(&self, op: &Operation<'_>) -> [Option<&everygraph_core::SegVec<f64>>; 2] {
        let counts = matches!(
            op,
            Operation::PageRank {
                weight: Weight::Edges,
                ..
            }
        );
        [0, 1].map(|i| {
            self.events[i]
                .as_ref()
                .filter(|_| counts)
                .map(|e| &e.weights)
        })
    }

    /// The undirected projection of a directed graph: the sorted union of each node's
    /// two lists.
    fn projection(&self, inn: &Side) -> Side {
        let mut offsets = vec![0];
        let mut targets = Vec::new();
        for v in 0..self.nodes() as usize {
            let list =
                |s: &Side| s.targets[s.offsets[v] as usize..s.offsets[v + 1] as usize].to_vec();
            let mut union = list(&self.out);
            union.extend(list(inn));
            union.sort_unstable();
            union.dedup();
            targets.extend(union);
            offsets.push(targets.len() as u64);
        }
        Side::new(offsets, targets)
    }

    /// `op` on raw arrays in the heap, 1 thread, no budget: the reference. A directed
    /// graph's projection is merged here by sorting, its degrees counted on it.
    fn in_memory(&self, op: &Operation<'_>) -> Result<Output, RunError> {
        let memory = Memory::unlimited();
        let alone = |side: &Side, op: &Operation<'_>, directed, weights| {
            let csr = side.csr();
            Lanes::with(&csr, &memory, Settings::default(), |p| {
                let lanes = run::Lanes::out(&p, directed).weighted([weights, None]);
                run::run(op, &lanes, &memory)
            })
        };
        let weights = self.weights(op);
        par::with_threads(threads(1), || {
            let Some(inn) = self.inn.as_ref() else {
                return alone(&self.out, op, self.directed, weights[0]);
            };
            if run::projects(op) {
                let undirected = op.undirected();
                return alone(&self.projection(inn), &undirected, false, None);
            }
            let (out, back) = (self.out.csr(), inn.csr());
            let mut output = Lanes::with_pair(&out, &back, &memory, Settings::default(), |o, i| {
                let lanes = run::Lanes {
                    inn: Some(&i),
                    ..run::Lanes::out(&o, true).weighted(weights)
                };
                run::run(op, &lanes, &memory)
            })
            .unwrap()?;
            if matches!(op, Operation::Degrees) {
                let mut both = alone(&self.projection(inn), op, false, None)?;
                output.columns.push(("degree", both.columns.remove(0).1));
            }
            Ok(output)
        })
        .unwrap()
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

fn pagerank(precision: Precision) -> Operation<'static> {
    Operation::PageRank {
        params: PageRank::default(),
        precision,
        weight: Weight::Pairs,
    }
}

fn search(source: u32, max_hops: Option<u32>, direction: Direction) -> Operation<'static> {
    Operation::Bfs {
        source,
        max_hops,
        direction,
    }
}

const WCC: Operation<'static> = Operation::Components { strong: false };

fn operations(nodes: u32) -> Vec<Operation<'static>> {
    let mut ops = vec![
        pagerank(Precision::Full),
        pagerank(Precision::Low),
        Operation::PageRank {
            params: PageRank::default(),
            precision: Precision::Full,
            weight: Weight::Edges,
        },
        WCC,
        Operation::Components { strong: true },
        Operation::KCore,
        Operation::Degrees,
    ];
    if nodes > 0 {
        for direction in [Direction::Out, Direction::In, Direction::Both] {
            ops.push(search(nodes / 2, None, direction));
        }
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

/// Plans `op` under `budget` with `force` for `session`, and runs it in `memory`; the
/// mapped regime reads the memory maps. Each phase that begins is named in `began`.
fn planned_with<'a>(
    s: &Stored,
    op: &Operation<'a>,
    budget: u64,
    force: Force,
    session: &mut Session,
    memory: &Memory,
    began: &Mutex<Vec<&'static str>>,
) -> (Plan<'a>, Result<Vec<u64>, RunError>) {
    let heap = s.lists(false);
    let plan = planner::plan(
        op,
        &s.graph(),
        &machine(),
        &session.request(&heap, request(budget, force)),
    )
    .unwrap();
    let lists = s.lists(plan.regime() == Regime::Mapped);
    let begin = |p: &planner::Phase| began.lock().unwrap().push(p.name);
    let out = execute(&plan, &lists, memory, session, &begin).map(|o| bits(&o));
    (plan, out)
}

/// As [`planned_with`], checking that the phases began in the plan's order.
fn planned_in<'a>(
    s: &Stored,
    op: &Operation<'a>,
    budget: u64,
    force: Force,
    session: &mut Session,
    memory: &Memory,
) -> (Plan<'a>, Result<Vec<u64>, RunError>) {
    let began = Mutex::new(Vec::new());
    let (plan, out) = planned_with(s, op, budget, force, session, memory, &began);
    if out.is_ok() && !plan.kept() {
        let names: Vec<&str> = plan.phases().iter().map(|p| p.name).collect();
        assert_eq!(began.into_inner().unwrap(), names, "{}", op.name());
    }
    (plan, out)
}

/// As [`planned_in`], in a budget of `budget` bytes of its own; with the budget's peak.
fn planned<'a>(
    s: &Stored,
    op: &Operation<'a>,
    budget: u64,
    force: Force,
    session: &mut Session,
) -> (Plan<'a>, Result<Vec<u64>, RunError>, u64) {
    // As Python does, the run's own budget leaves out what the plan counts of the open
    // graph (`held`), which the graph holds in its own.
    let accountant = Arc::new(Budget::new(budget.saturating_sub(s.graph().held)));
    let memory = Memory::from(Arc::clone(&accountant));
    let (plan, out) = planned_in(s, op, budget, force, session, &memory);
    (plan, out, accountant.peak())
}

const PLENTY: u64 = 1 << 40;

/// Checks one graph: every forced choice against the reference, the plan's peak, and
/// the smallest budget. A raw copy holds the out-lists only, so an operation that reads
/// the in-lists is refused on it; strong components have no bound to check.
fn check(name: &str, s: &Stored) -> usize {
    let mut checked = 0;
    for op in operations(s.nodes()) {
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
        let reference = bits(&reference.unwrap_or_else(|e| panic!("{name} {}: {e}", op.name())));
        let no_raw = s.inn.is_some() && run::reads_in(&op);
        let no_stream = s.inn.is_some() && !matches!(op, Operation::Components { strong: false });
        for tier in [Tier::Compressed, Tier::Raw] {
            for regime in [Regime::InMemory, Regime::Mapped, Regime::Streamed] {
                for t in [1, 3] {
                    let force = Force {
                        tier: Some(tier),
                        regime: Some(regime),
                        threads: Some(threads(t)),
                        advice: None,
                    };
                    let (plan, got, peak) = planned(s, &op, PLENTY, force, &mut Session::new());
                    let refused = (tier, regime) == (Tier::Raw, Regime::Streamed)
                        || (tier == Tier::Raw && no_raw)
                        || (regime == Regime::Streamed && no_stream);
                    if refused {
                        let r = plan.refusal().map(|r| r.error);
                        assert_eq!(r, Some("Unsupported"), "{name} {}", op.name());
                        assert!(matches!(got, Err(RunError::Refused(_))) && peak == 0);
                        continue;
                    }
                    let got = got.unwrap_or_else(|e| panic!("{name} {}: {e}", op.name()));
                    assert!(
                        got == reference,
                        "{name} {} {tier:?} {regime:?} {t}",
                        op.name()
                    );
                    assert!(
                        peak + s.graph().held <= plan.peak_bytes() || !plan.bounded(),
                        "{name} {} {tier:?} {t} threads: reserved {peak}, planned {}",
                        op.name(),
                        plan.peak_bytes()
                    );
                }
            }
        }
        // The smallest budget is the one of a single thread: it runs; a byte less is
        // refused before any reservation, naming it.
        let one = Force {
            threads: Some(threads(1)),
            ..Force::default()
        };
        let smallest = planned(s, &op, PLENTY, one, &mut Session::new())
            .0
            .peak_bytes();
        let (plan, got, _) = planned(s, &op, smallest, Force::default(), &mut Session::new());
        assert!(plan.fits(), "{name} {}", op.name());
        assert!(
            !plan.bounded() || got.as_ref().is_ok_and(|g| *g == reference),
            "{name} {} at its smallest budget {smallest}: {:?}",
            op.name(),
            got.err()
        );
        if smallest > 0 {
            let (plan, got, peak) =
                planned(s, &op, smallest - 1, Force::default(), &mut Session::new());
            let r = plan.refusal().expect("refused");
            let named = plan.bounded().then_some(smallest);
            assert_eq!(r.shortfall.unwrap().smallest_budget_bytes, named);
            assert!(
                matches!(got, Err(RunError::Refused(_))) && peak == 0,
                "{name}"
            );
        }
        checked += 1;
    }
    checked
}

/// A directed graph of `n` nodes with `m` seeded arcs, a kept loop, and every arc
/// sorted into its list.
fn directed(n: u32, m: usize, seed: u64) -> Stored {
    let mut x = seed;
    let mut next = || {
        x = x
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (x >> 33) as u32 % n
    };
    let mut lists = vec![Vec::new(); n as usize];
    for _ in 0..m {
        let (u, v) = (next(), next());
        lists[u as usize].push(v);
    }
    lists[1].push(1);
    let mut offsets = vec![0];
    let mut targets = Vec::new();
    for mut l in lists {
        l.sort_unstable();
        l.dedup();
        targets.extend(l);
        offsets.push(targets.len() as u64);
    }
    Stored::new(offsets, targets, true)
}

/// Directed graphs read their in-lists and their projection, small enough for a debug
/// build: every choice gives the in-memory bits, and the bounds hold.
#[test]
fn every_choice_gives_the_bits_of_the_in_memory_run_on_directed_graphs() {
    for (n, m, seed) in [(40, 120, 1), (300, 900, 2), (1, 0, 3)] {
        let s = directed(n.max(2), m, seed);
        assert!(check(&format!("directed {n}"), &s) > 0);
    }
}

/// A directed graph's core numbers merge the projection in a phase of its own, which
/// the plan counts; strong components plan their per-node state only and name no
/// smallest budget.
#[test]
fn a_directed_graph_plans_its_merge_and_its_strong_components() {
    let s = directed(300, 900, 4);
    let one = Force {
        threads: Some(threads(1)),
        ..Force::default()
    };
    let (plan, got, _) = planned(&s, &Operation::KCore, PLENTY, one, &mut Session::new());
    let names: Vec<&str> = plan.phases().iter().map(|p| p.name).collect();
    assert_eq!(names, ["merging", "kcore"]);
    assert!(
        got.is_ok()
            && plan
                .to_string()
                .contains("merge both directions into memory")
    );
    let strong = Operation::Components { strong: true };
    let plan = planner::plan(&strong, &s.graph(), &machine(), &request(10, one)).unwrap();
    assert!(!plan.bounded());
    let r = plan.refusal().unwrap();
    assert_eq!(r.shortfall.unwrap().smallest_budget_bytes, None);
    assert_eq!(r.ways_forward.len(), 1);
    assert!(
        r.ways_forward[0]
            .python
            .starts_with("g.components(strong=True, memory=")
    );
    // Without its in-adjacency the file answers what the out-lists answer.
    let mut bare = s.graph();
    bare.inn = None;
    let plan = planner::plan(&Operation::KCore, &bare, &machine(), &request(PLENTY, one)).unwrap();
    assert_eq!(plan.refusal().map(|r| r.error), Some("Unsupported"));
    let plan = planner::plan(
        &Operation::Degrees,
        &bare,
        &machine(),
        &request(PLENTY, one),
    )
    .unwrap();
    assert!(plan.fits());
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
    for (name, s) in [("star", star(20_001)), ("path", path(20_001))] {
        assert!(check(name, &s) > 0);
    }
}

/// A star of `n` nodes: node 0 lists every other node.
fn star(n: u32) -> Stored {
    let mut offsets = vec![0_u64];
    let mut targets = Vec::new();
    for v in 0..n {
        if v == 0 {
            targets.extend(1..n);
        } else {
            targets.push(0);
        }
        offsets.push(targets.len() as u64);
    }
    Stored::new(offsets, targets, false)
}

/// A path of `n` nodes.
fn path(n: u32) -> Stored {
    let mut offsets = vec![0_u64];
    let mut targets = Vec::new();
    for v in 0..n {
        if v > 0 {
            targets.push(v - 1);
        }
        if v + 1 < n {
            targets.push(v + 1);
        }
        offsets.push(targets.len() as u64);
    }
    Stored::new(offsets, targets, false)
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
    let bfs = search(0, None, Direction::Out);
    let reference = bits(&s.in_memory(&bfs).unwrap());
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
    // A scan never reads the copy, nor does a BFS cut at a depth.
    let (plan, _, _) = planned(&s, &WCC, PLENTY, force, &mut session);
    assert_eq!(plan.tier(), Tier::Compressed);
    let near = search(0, Some(1), Direction::Out);
    let mut shallow = Session::new();
    for _ in 0..k + 2 {
        let (plan, got, _) = planned(&s, &near, PLENTY, force, &mut shallow);
        assert!(got.is_ok() && plan.tier() == Tier::Compressed && !plan.builds_raw());
    }
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
    // The exploratory constants of cost.rs, at 16 threads.
    assert_eq!(break_even(threads(16)), 5);
    assert!(break_even(threads(1)) >= 1);
}

/// Traversals build the raw copy, then `PageRank` runs in the same budget: the copy
/// counts in its plan, it does not fit beside `PageRank`, so the plan drops it first and
/// the run fits. Beside an operation it fits with, the copy is kept and counted.
#[test]
fn a_held_copy_counts_in_every_plan_and_is_dropped_when_it_does_not_fit() {
    let s = path(20_001);
    let bfs = search(0, None, Direction::Out);
    let pagerank = pagerank(Precision::Full);
    let one = Force {
        threads: Some(threads(1)),
        ..Force::default()
    };
    let raw = Force {
        tier: Some(Tier::Raw),
        ..one
    };
    let with_raw = planned(&s, &bfs, PLENTY, raw, &mut Session::new())
        .0
        .peak_bytes();
    let alone = planned(&s, &pagerank, PLENTY, one, &mut Session::new())
        .0
        .peak_bytes();
    let budget = (with_raw * 10 / 9 + 1).max(alone);
    let accountant = Arc::new(Budget::new(budget));
    let memory = Memory::from(Arc::clone(&accountant));
    let mut session = Session::new();
    for _ in 0..=break_even(threads(1)) {
        let (plan, got) = planned_in(&s, &bfs, budget, one, &mut session, &memory);
        assert!(got.is_ok(), "{:?}", plan.tier());
    }
    let held = session.raw_bytes().expect("the copy is built");
    assert!(
        alone + held > budget,
        "the test needs a copy that does not fit beside"
    );
    let (plan, got) = planned_in(&s, &pagerank, budget, one, &mut session, &memory);
    assert!(plan.fits() && plan.drops_raw(), "{plan}");
    assert_eq!(got.unwrap(), bits(&s.in_memory(&pagerank).unwrap()));
    assert!(session.raw_bytes().is_none());
    // Beside an operation that fits with it, the copy is kept and counted.
    let mut kept = Session::new();
    for _ in 0..=break_even(threads(1)) {
        let _ = planned(&s, &bfs, PLENTY, one, &mut kept);
    }
    let held = kept.raw_bytes().unwrap();
    let (plan, _, _) = planned(&s, &Operation::Degrees, PLENTY, one, &mut kept);
    let degrees = planned(&s, &Operation::Degrees, PLENTY, one, &mut Session::new()).0;
    assert!(!plan.drops_raw() && plan.peak_bytes() == degrees.peak_bytes() + held);
    assert!(kept.raw_bytes().is_some());
}

/// A forced choice meets the checks of an automatic one: a forced raw tier on a graph
/// with no raw copy is refused, and a held copy counts beside a kept result.
#[test]
fn a_forced_choice_is_checked_as_an_automatic_one() {
    let s = path(1000);
    let bfs = search(0, None, Direction::Out);
    let raw = Force {
        tier: Some(Tier::Raw),
        threads: Some(threads(1)),
        ..Force::default()
    };
    let mut hub = s.graph();
    hub.max_degree = u32::MAX;
    let plan = planner::plan(&bfs, &hub, &machine(), &request(PLENTY, raw)).unwrap();
    assert_eq!(plan.refusal().map(|r| r.error), Some("Unsupported"));
    let held = planner::Held {
        bytes: 1000,
        usable: true,
    };
    let mut kept = s.graph();
    kept.kept = true;
    let op = WCC;
    let at = |force, held| {
        let r = Request {
            held,
            ..request(PLENTY, force)
        };
        planner::plan(&op, &kept, &machine(), &r)
            .unwrap()
            .peak_bytes()
    };
    assert_eq!(at(raw, Some(held)), at(raw, None) + 1000);
    assert_eq!(
        at(Force::default(), Some(held)),
        at(Force::default(), None) + 1000
    );
}

/// A session that built the raw copy of a path, asked for a BFS on a star, drops the
/// path's copy and gives the star's distances.
#[test]
fn a_session_belongs_to_one_graph() {
    let (p, s) = (path(1000), star(1000));
    let bfs = search(0, None, Direction::Out);
    let force = Force {
        threads: Some(threads(2)),
        ..Force::default()
    };
    let mut session = Session::new();
    for _ in 0..=break_even(threads(2)) {
        let _ = planned(&p, &bfs, PLENTY, force, &mut session);
    }
    assert!(session.raw_bytes().is_some());
    let (plan, got, _) = planned(&s, &bfs, PLENTY, force, &mut session);
    assert!(plan.drops_raw() && plan.tier() == Tier::Compressed);
    assert_eq!(got.unwrap(), bits(&s.in_memory(&bfs).unwrap()));
    assert!(session.raw_bytes().is_none());
}

/// A plan sized for serving a kept result serves it from the file, by the lookup it was
/// made with; a plan told of a result that the file does not keep is refused, with
/// nothing reserved.
#[test]
fn a_kept_result_is_served_by_the_lookup_the_plan_was_made_with() {
    let mut s = path(20_001);
    let op = WCC;
    let reference = s.in_memory(&op).unwrap();
    let one = Force {
        threads: Some(threads(1)),
        ..Force::default()
    };
    assert!(!run::kept(&s.file, &op, false, s.nodes()).unwrap());
    s.file = graph_file(&s.out.index_bytes, Some((&op, &reference)));
    assert!(run::kept(&s.file, &op, false, s.nodes()).unwrap());
    assert!(!run::kept(&s.file, &Operation::KCore, false, s.nodes()).unwrap());
    let mut graph = s.graph();
    graph.kept = true;
    let heap = s.lists(false);
    let plan = planner::plan(&op, &graph, &machine(), &request(PLENTY, one)).unwrap();
    let served = plan.peak_bytes();
    let plan = planner::plan(&op, &graph, &machine(), &request(served, one)).unwrap();
    assert!(plan.fits() && plan.kept());
    let accountant = Arc::new(Budget::new(served));
    let memory = Memory::from(Arc::clone(&accountant));
    let got = execute(&plan, &heap, &memory, &mut Session::new(), &|_| {}).unwrap();
    assert_eq!(bits(&got), bits(&reference));
    // A kept result the file does not hold: refused before any reservation.
    let other = Stored::new(s.out.offsets.clone(), s.out.targets.clone(), false);
    let accountant = Arc::new(Budget::new(served));
    let memory = Memory::from(Arc::clone(&accountant));
    let lists = other.lists(false);
    let refused = execute(&plan, &lists, &memory, &mut Session::new(), &|_| {});
    assert!(matches!(refused, Err(RunError::Refused(_))), "{refused:?}");
    assert_eq!(accountant.peak(), 0);
}

/// The planner takes fewer threads before it refuses, since each holds a reader of a
/// hub's length (a star), and the refusal names the one-thread budget.
#[test]
fn the_smallest_budget_is_the_one_of_the_fewest_threads() {
    let s = star(200_001);
    let op = Operation::KCore;
    let at = |t| {
        let force = Force {
            threads: Some(threads(t)),
            ..Force::default()
        };
        planner::plan(&op, &s.graph(), &machine(), &request(PLENTY, force))
            .unwrap()
            .peak_bytes()
    };
    let (many, one) = (at(4), at(1));
    assert!(one < many);
    let ask =
        |b| planner::plan(&op, &s.graph(), &machine(), &request(b, Force::default())).unwrap();
    let chosen = ask(PLENTY).threads();
    assert!(chosen.get() > 1);
    let plan = ask(one);
    assert!(plan.fits() && plan.threads() == threads(1));
    let refused = ask(one - 1);
    let r = refused.refusal().unwrap();
    assert_eq!(r.shortfall.unwrap().smallest_budget_bytes, Some(one));
    assert!(
        r.ways_forward[0].note.ends_with("on 1 thread"),
        "{:?}",
        r.ways_forward
    );
}

#[test]
fn threads_are_the_physical_cores_and_fewer_for_a_small_graph() {
    let s = small();
    let op = WCC;
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
    let edges = s.out.index.data_bytes();
    assert_eq!(plan(state + edges).regime(), Regime::InMemory);
    assert_eq!(plan(state + edges - 1).regime(), Regime::Mapped);
    assert!(plan(state).fits() && !plan(state - 1).fits());
}

/// A graph whose compressed edges do not fit beside the state is streamed when the
/// stream's readers do: the plan fits, gives the bits of the in-memory run, and the
/// run reserves no more than the plan's peak. The edges of the path are not in the
/// budget; only the windows are. Without a forced thread count the same budget is in
/// memory on fewer threads, which the planner takes before the map.
#[test]
fn edges_that_do_not_fit_are_streamed_within_the_planned_peak() {
    let s = path(200_000);
    let op = pagerank(Precision::Full);
    let reference = bits(&s.in_memory(&op).unwrap());
    // On 4 threads: on fewer, the decode readers are smaller and the edges fit.
    let four = Force {
        threads: Some(threads(4)),
        ..Force::default()
    };
    let in_memory = planner::plan(&op, &s.graph(), &machine(), &request(PLENTY, four)).unwrap();
    assert_eq!(in_memory.regime(), Regime::InMemory);
    let budget = in_memory.peak_bytes() + s.out.index.data_bytes() - 1;
    let (plan, got, peak) = planned(&s, &op, budget, four, &mut Session::new());
    assert_eq!(plan.regime(), Regime::Streamed, "{plan}");
    assert!(plan.fits() && plan.window() > 0, "{plan}");
    assert!(plan.to_string().contains("read in passes"), "{plan}");
    assert_eq!(got.unwrap(), reference);
    assert!(
        peak <= plan.peak_bytes() && plan.peak_bytes() <= budget,
        "reserved {peak}, planned {}, budget {budget}",
        plan.peak_bytes()
    );
    let free = planned(&s, &op, budget, Force::default(), &mut Session::new());
    assert_ne!(free.0.regime(), Regime::Mapped, "{}", free.0);
    assert!(free.0.fits() && free.2 <= free.0.peak_bytes());
    assert_eq!(free.1.unwrap(), reference);
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
    let full = pagerank(Precision::Full);
    let low = pagerank(Precision::Low);
    let plan = |op: &Operation<'static>, b| {
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
    assert_eq!(planner::typed(100_000_001), "200MB");
    // Below 100 MB, two significant digits.
    assert_eq!(planner::typed(2_860_001), "2.9MB");
    assert_eq!(planner::typed(64), "64B");
    assert_eq!(planner::typed(1_501), "1.6kB");
    assert_eq!(planner::typed(99_000_001), "100MB");
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
    let op = WCC;
    let plan = planner::plan(&op, &s.graph(), &machine(), &request(10, Force::default())).unwrap();
    let doc = json(&plan.refusal().unwrap().to_json());
    assert_eq!(doc["error"], "InsufficientBudget");
    assert_eq!(doc["arithmetic"]["count"], 7);
    // 7 × 24 bytes, 12 bytes, the roots' offsets (24) and one partial value (8): 212
    // bytes, 30 a node.
    assert_eq!(doc["arithmetic"]["bytes_each"], 30);
    assert_eq!(doc["smallest_budget_bytes"], plan.peak_bytes());
    assert_eq!(doc["changed"], Value::Bool(false));
}

/// This machine, as `everygraph_io` probes it, with the storage of `dir`.
fn probed(dir: &std::path::Path) -> Machine {
    use everygraph_core::MemoryProbe;
    let hardware = std::thread::available_parallelism().unwrap();
    Machine {
        cores: everygraph_io::physical_cores(),
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
        // A session of traversals switches to the raw copy after the break-even count.
        let bfs = search(0, None, Direction::Out);
        let mut session = Session::new();
        let mut first = None;
        for run in 0..=break_even(threads(16)) + 1 {
            let (plan, out, peak) = planned(&s, &bfs, 8 << 30, force, &mut session);
            let out = out.unwrap();
            assert_eq!(*first.get_or_insert_with(|| out.clone()), out);
            eprintln!(
                "{name} bfs run {run}: {:?}, builds {}, reserved {peak} of {}",
                plan.tier(),
                plan.builds_raw(),
                plan.peak_bytes()
            );
        }
        for op in operations(s.nodes()) {
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

/// Exploratory, on twitter-sym's offsets (2.4 B entries): the planner switches a session
/// to the raw copy after the break-even count of traversals, and the copy's parts each
/// fit one allocation of at most `MAX_ALLOCATION_BYTES`.
#[test]
#[ignore = "reads csr/twitter-sym/offsets.u64 under $EVERYGRAPH_DATA (333 MB); run under a memory limit"]
fn twitter_sym_switches_to_a_raw_copy_in_parts_of_one_allocation() {
    let data = std::path::PathBuf::from(std::env::var("EVERYGRAPH_DATA").unwrap());
    let bytes = std::fs::read(data.join("csr/twitter-sym/offsets.u64")).unwrap();
    let offsets: Vec<u64> = bytes
        .chunks_exact(8)
        .map(|c| u64::from_le_bytes(c.try_into().unwrap()))
        .collect();
    let cap = everygraph_core::MAX_ALLOCATION_BYTES;
    assert!(8 * offsets.len() as u64 <= cap);
    let parts = RawCopy::parts(&offsets).unwrap();
    for r in &parts {
        let entries = offsets[r.end as usize] - offsets[r.start as usize];
        assert!(4 * entries <= cap, "{r:?}: {entries} entries");
    }
    let entries = *offsets.last().unwrap();
    let graph = Graph {
        path: Some("twitter-sym.eg".into()),
        bytes: entries * 2,
        nodes: (offsets.len() - 1) as u32,
        edges: entries / 2,
        entries,
        // About 16 bits an entry (ADR 0004, #118): an estimate, for the regime only.
        data_bytes: entries * 2,
        max_degree: offsets.windows(2).map(|w| w[1] - w[0]).max().unwrap() as u32,
        directed: false,
        kept: false,
        held: 0,
        opened: 0,
        inn: None,
    };
    let bfs = search(0, None, Direction::Out);
    let budget = 16 << 30;
    let k = break_even(threads(16));
    for before in 0..=k {
        let r = Request {
            traversals_before: before,
            threads: Some((threads(16), Origin::Argument)),
            ..request(budget, Force::default())
        };
        let plan = planner::plan(&bfs, &graph, &machine(), &r).unwrap();
        eprintln!(
            "twitter-sym bfs after {before} traversals: {:?}, builds {}, peak {}",
            plan.tier(),
            plan.builds_raw(),
            plan.peak_bytes()
        );
        assert_eq!(plan.tier() == Tier::Raw, before >= k);
        assert!(plan.fits() && plan.peak_bytes() <= budget);
    }
    eprintln!("twitter-sym raw copy: {} parts", parts.len());
}

/// Lists of `n` nodes from `m` seeded arcs (both ways when not `directed`), each pair
/// with the number of its arcs, a loop at node 1 among them.
fn multigraph(n: u32, m: usize, seed: u64, directed: bool) -> Vec<Vec<(u32, u64)>> {
    let mut x = seed;
    let mut next = || {
        x = x
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (x >> 33) as u32 % n
    };
    let mut pairs = BTreeMap::new();
    let mut add = |u: u32, v: u32| {
        *pairs.entry((u, v)).or_insert(0_u64) += 1;
        if !directed && u != v {
            *pairs.entry((v, u)).or_insert(0) += 1;
        }
    };
    for _ in 0..m {
        let (u, v) = (next(), next());
        add(u, v);
        // A pair in three of every four arcs repeats.
        if next() % 4 != 0 {
            add(u, v);
        }
    }
    add(1, 1);
    let mut lists = vec![Vec::new(); n as usize];
    for ((u, v), c) in pairs {
        lists[u as usize].push((v, c));
    }
    lists
}

/// `PageRank` that counts the edges of a pair reads the `.events` of both lists: every
/// choice gives the bits of the in-memory run with the counts as weights, within the
/// plan's peak.
#[test]
fn counted_edges_give_the_bits_of_the_in_memory_run() {
    for directed in [true, false] {
        let s = Stored::with_counts(&multigraph(200, 700, 5, directed), directed);
        assert!(s.events[0].is_some() && s.events[1].is_some() == directed);
        let edges = Operation::PageRank {
            params: PageRank::default(),
            precision: Precision::Full,
            weight: Weight::Edges,
        };
        let pairs = bits(&s.in_memory(&pagerank(Precision::Full)).unwrap());
        assert_ne!(
            bits(&s.in_memory(&edges).unwrap()),
            pairs,
            "the counts change the ranks"
        );
        assert!(check(&format!("counted, directed {directed}"), &s) > 0);
    }
}

/// The projection's data is at most both data parts and 3 bytes a node: here an even
/// node's out-list starts just above it and its in-list far below, so the first entry of
/// each union becomes a long gap. The plan's step holds the merged data and index.
#[test]
fn the_projection_bound_holds_where_first_entries_become_gaps() {
    let n: u32 = 1 << 19;
    let mut offsets = vec![0];
    let mut targets = Vec::new();
    for v in 0..n {
        let base = if v % 2 == 0 { v } else { v + (1 << 17) };
        targets.extend(
            [1, 3, 5, 7]
                .map(|d| base + d)
                .into_iter()
                .filter(|&t| t < n),
        );
        offsets.push(targets.len() as u64);
    }
    let s = Stored::new(offsets, targets, true);
    let inn = s.inn.as_ref().unwrap();
    let [out, back] = [&s.out, inn].map(|side| {
        everygraph_format::adjacency::Adjacency::new(&side.index, side.data.as_slice().unwrap())
            .unwrap()
    });
    let (merged, _) = merge(&out, &back, &Memory::unlimited()).unwrap();
    let both = s.out.index.data_bytes() + inn.index.data_bytes();
    assert!(
        merged.data.len() as u64 > both,
        "the graph must defeat the old bound"
    );
    assert!(merged.data.len() as u64 <= both + 3 * u64::from(n));
    let plan = planner::plan(
        &Operation::KCore,
        &s.graph(),
        &machine(),
        &request(PLENTY, Force::default()),
    )
    .unwrap();
    let doc = json(&plan.to_json());
    let step = doc["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["step"] == "merge both directions into memory")
        .unwrap();
    let held = (merged.data.len() + merged.index.len()) as u64;
    assert!(step["memory_bytes"].as_u64().unwrap() >= held);
}
