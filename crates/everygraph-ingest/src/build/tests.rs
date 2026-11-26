// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The build against the oracle's projection, on every graph of the CI corpus under
//! every option; the same bytes at any thread count and any budget, in memory and with
//! spills; refusals and write failures that leave no graph behind.

use std::sync::Arc;

use everygraph_core::{Budget, ByteSink, ByteSource, Memory, MemorySource, NodeId, WriteError};
use everygraph_format::adjacency::{Adjacency, AdjacencyEvents, AdjacencyIndex};
use everygraph_format::container::{Container, Provenance};
use everygraph_oracle::corpus::{Tier, specs};
use everygraph_oracle::generators::generate;
use everygraph_oracle::graph::{self as oracle_graph, Graph};
use everygraph_oracle::projection::Projection;

use super::*;
use crate::ids::{MemoryScratch, OwnedKey};
use crate::text::{Delimiter, Header, IdType, TextOptions};

const SMALL: Tuning = Tuning {
    segment_bits: 12,
    bucket_nodes: 5,
    spill_batch: 7,
    resident_items: u64::MAX,
    cache_rows: false,
};

/// [`SMALL`], holding at most 40 items in memory: every graph with more spills.
const SPILLING: Tuning = Tuning {
    resident_items: 40,
    ..SMALL
};

fn text_of(g: &Graph) -> Vec<u8> {
    let mut text = Vec::new();
    for (u, v) in g.src.iter().zip(&g.dst) {
        text.extend_from_slice(format!("{u}\t{v}\n").as_bytes());
    }
    text
}

fn text_options() -> TextOptions {
    TextOptions {
        delimiter: Delimiter::Byte(b'\t'),
        header: Header::Absent,
        ids: IdType::Integer,
        ..TextOptions::default()
    }
}

/// The bytes of the graph built from `text`, and its report.
fn run(
    text: &[u8],
    graph: GraphOptions,
    budget: u64,
    chunk: usize,
    tuning: &Tuning,
) -> Result<(Vec<u8>, Report), BuildError> {
    let reader =
        TextReader::open_with_chunk(MemorySource::new(text), &text_options(), chunk).unwrap();
    let memory = Memory::from(Arc::new(Budget::new(budget)));
    let options = BuildOptions::new(graph, NonZeroUsize::new(8).unwrap());
    let sink = MemorySource::new(Vec::new());
    let unfinished = build_tuned(
        &reader,
        &options,
        &memory,
        &MemoryScratch,
        sink,
        &Silent,
        tuning,
    )?;
    let provenance = Provenance {
        engine: "everygraph test".into(),
        ..Provenance::default()
    };
    let (sink, report) = unfinished.commit(provenance)?;
    Ok((sink.into_inner(), report))
}

/// A graph read back: each node's key, and its list of (neighbour, stored events).
struct Stored {
    keys: Vec<u64>,
    lists: Vec<Vec<(u32, u64)>>,
    events_section: bool,
    parameters: BTreeMap<String, String>,
}

fn read_back(bytes: &[u8]) -> Stored {
    let checked = check(MemorySource::new(bytes), &Memory::unlimited()).unwrap();
    let file = Container::open(MemorySource::new(bytes)).unwrap();
    let section = |name: &str| file.section(name).unwrap().as_slice().unwrap().to_vec();
    let index = AdjacencyIndex::parse(&section(OUT_INDEX)).unwrap();
    let data = section(OUT_DATA);
    let adjacency = Adjacency::new(&index, &data).unwrap();
    let events = index.has_events().then(|| section(OUT_EVENTS));
    let multiplicities: Vec<u64> = match &events {
        Some(bytes) => AdjacencyEvents::parse(bytes, &index)
            .unwrap()
            .multiplicities()
            .unwrap()
            .collect(),
        None => vec![1; usize::try_from(index.entries()).unwrap()],
    };
    assert_eq!(checked.events.is_some(), events.is_some());
    let budget = Budget::unlimited();
    let dictionary = Dictionary::open(&budget, |name| file.section(name).ok()).unwrap();
    let mut keys = Vec::new();
    let mut lists = Vec::new();
    let mut buf = Vec::new();
    let mut entry = 0;
    for v in 0..index.nodes() {
        keys.push(match dictionary.key(NodeId::new(v)).unwrap() {
            OwnedKey::UInt(k) => k,
            other => panic!("integer keys come back as {other:?}"),
        });
        let list = adjacency.neighbors(NodeId::new(v), &mut buf).unwrap();
        lists.push(
            list.iter()
                .map(|&t| {
                    entry += 1;
                    (t, multiplicities[entry - 1])
                })
                .collect(),
        );
    }
    let parameters = file.provenance(OUT_DATA).unwrap().unwrap().parameters;
    Stored {
        keys,
        lists,
        events_section: events.is_some(),
        parameters,
    }
}

fn oracle_semantics(graph: GraphOptions) -> oracle_graph::Semantics {
    oracle_graph::Semantics {
        self_loops: match graph.self_loops {
            SelfLoops::Drop => oracle_graph::SelfLoops::Drop,
            SelfLoops::Keep => oracle_graph::SelfLoops::Keep,
        },
        duplicates: match graph.duplicates {
            Duplicates::Events => oracle_graph::Duplicates::Events,
            Duplicates::Collapse => oracle_graph::Duplicates::Collapse,
            Duplicates::Sum => oracle_graph::Duplicates::Sum,
        },
    }
}

/// Builds `g` under `options` and compares the stored graph and the report with the
/// oracle's projection of the same rows.
fn compare_with_oracle(g: &Graph, graph: GraphOptions, budget: u64, tuning: &Tuning) {
    let mut g = g.clone();
    g.semantics = oracle_semantics(graph);
    let projection = Projection::new(&g);
    let text = text_of(&g);
    let (bytes, report) =
        run(&text, graph, budget, 64, tuning).unwrap_or_else(|e| panic!("{}: {e}", g.name));
    let stored = read_back(&bytes);

    // Nodes: the identifiers of the rows, in sorted order.
    let mut appearing: Vec<u64> = g.src.iter().chain(&g.dst).map(|&v| u64::from(v)).collect();
    appearing.sort_unstable();
    appearing.dedup();
    assert_eq!(stored.keys, appearing, "{}", g.name);

    let events = graph.duplicates == Duplicates::Events;
    for (id, &key) in stored.keys.iter().enumerate() {
        let v = usize::try_from(key).unwrap();
        let range = projection.out.range(v);
        let mut expected: Vec<(u64, u64)> = range
            .map(|e| {
                let m = if events {
                    projection.out.multiplicity[e]
                } else {
                    1
                };
                (u64::from(projection.out.targets[e]), m)
            })
            .collect();
        if graph.self_loops == SelfLoops::Keep && projection.loops[v] > 0 {
            let m = if events { projection.loops[v] } else { 1 };
            expected.push((key, m));
            expected.sort_unstable();
        }
        let got: Vec<(u64, u64)> = stored.lists[id]
            .iter()
            .map(|&(t, m)| (stored.keys[usize::try_from(t).unwrap()], m))
            .collect();
        assert_eq!(got, expected, "{}: node {key}", g.name);
    }

    // The report: the oracle's, but for nodes that appear in no row.
    let absent = u64::from(projection.n) - u64::try_from(appearing.len()).unwrap();
    let mut oracle = projection.report();
    *oracle.get_mut("counts.nodes").unwrap() -= absent;
    *oracle.get_mut("counts.isolates").unwrap() -= absent;
    let ours: BTreeMap<String, u64> = report
        .counts()
        .iter()
        .map(|&(k, v)| (k.to_owned(), v))
        .collect();
    assert_eq!(ours, oracle, "{}", g.name);
    assert_eq!(
        report.rows,
        report.rejected
            + report.dropped_self_loops
            + report.dropped_duplicates
            + report.merged
            + report.edges
    );
    assert_eq!(report.edges, report.pairs + report.repeated);
    for (key, value) in report.counts() {
        assert_eq!(stored.parameters[key], value.to_string());
    }

    // A .events section exactly when a stored pair has more than one event, and then
    // its multiplicities add up to the events of the adjacency.
    let repeats = stored.lists.iter().flatten().any(|&(_, m)| m > 1);
    assert_eq!(stored.events_section, repeats, "{}", g.name);
    assert!(!repeats || events);
    let stored_events: u64 = stored.lists.iter().flatten().map(|&(_, m)| m).sum();
    if events {
        let loops = report.self_loops;
        let expected = if g.directed {
            report.edges
        } else {
            2 * report.edges - loops
        };
        assert_eq!(stored_events, expected, "{}", g.name);
    }
}

fn all_options(directed: bool) -> Vec<GraphOptions> {
    let mut out = Vec::new();
    for self_loops in [SelfLoops::Drop, SelfLoops::Keep] {
        for duplicates in [Duplicates::Events, Duplicates::Collapse, Duplicates::Sum] {
            out.push(GraphOptions {
                directed,
                self_loops,
                duplicates,
            });
        }
    }
    out
}

/// Every graph of the CI corpus, under every `self_loops` × `duplicates` option, in
/// memory: the stored lists and multiplicities and the report are the oracle's.
#[test]
fn the_ci_corpus_is_stored_as_the_oracle_projects_it() {
    for spec in specs(Tier::Ci) {
        let g = generate(&spec);
        for graph in all_options(g.directed) {
            compare_with_oracle(&g, graph, 1 << 30, &TUNING);
        }
    }
}

/// The same with tiny buckets, segments and budget: many rounds, ranges across
/// segment ends, and spills.
#[test]
fn the_ci_corpus_is_stored_the_same_with_spills() {
    let mut spilled = 0;
    for spec in specs(Tier::Ci).into_iter().step_by(3) {
        let g = generate(&spec);
        for graph in all_options(g.directed) {
            compare_with_oracle(&g, graph, SMALL_BUDGET, &SPILLING);
            let (_, report) = run(&text_of(&g), graph, SMALL_BUDGET, 64, &SPILLING).unwrap();
            spilled += u64::from(report.regime == Regime::OnDisk);
        }
    }
    assert!(spilled > 0, "no graph spilled");
}

const SMALL_BUDGET: u64 = 3 << 20;

/// A graph with loops, repeated pairs in both directions, and a few hubs.
fn mixed_text(rows: u64) -> Vec<u8> {
    let mut text = Vec::new();
    let mut x = 0x2545_F491_4F6C_DD1D_u64;
    for i in 0..rows {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let u = if i % 7 == 0 { x % 5 } else { x % 3000 };
        let v = (x >> 20) % 3000;
        let (u, v) = if i % 11 == 0 { (v, u) } else { (u, v) };
        text.extend_from_slice(format!("{}\t{}\n", u * 1000 + 7, v * 1000 + 7).as_bytes());
        if i % 13 == 0 {
            text.extend_from_slice(format!("{}\t{}\n", u * 1000 + 7, v * 1000 + 7).as_bytes());
        }
    }
    text
}

/// A digest of the bytes of builds of [`mixed_text`] in memory and with spills, under
/// every option, printed for [`output_does_not_depend_on_threads_or_budget`], which
/// runs it in child processes with different thread counts.
#[test]
#[ignore = "run by output_does_not_depend_on_threads_or_budget in a child process"]
fn digest_probe() {
    let text = mixed_text(20_000);
    let mut digest = 0_u64;
    for directed in [false, true] {
        for graph in all_options(directed) {
            let (large, report) = run(&text, graph, 1 << 30, 4096, &TUNING).unwrap();
            assert_eq!(report.regime, Regime::InMemory);
            assert_eq!(report.spilled_bytes, 0);
            let (small, report) = run(&text, graph, 3 << 20, 4096, &SMALL).unwrap();
            assert_eq!(report.regime, Regime::OnDisk, "{graph:?}");
            assert!(report.spilled_bytes > 0);
            assert!(large == small, "{graph:?}: the budget changed the bytes");
            digest =
                (digest ^ xxhash_rust::xxh3::xxh3_64(&large)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        }
    }
    let threads = std::env::var("RAYON_NUM_THREADS").unwrap_or_default();
    #[expect(
        clippy::print_stdout,
        reason = "the parent test reads the digest from the output"
    )]
    {
        println!("digest threads={threads} value={digest:016x}");
    }
}

/// The output is byte-identical at 1, 3 and 8 threads and at two budgets, one of which
/// spills.
#[test]
fn output_does_not_depend_on_threads_or_budget() {
    let exe = std::env::current_exe().unwrap();
    let mut digests = Vec::new();
    for threads in ["1", "3", "8"] {
        let out = std::process::Command::new(&exe)
            .args([
                "--exact",
                "build::tests::digest_probe",
                "--ignored",
                "--nocapture",
                "--test-threads",
                "1",
            ])
            .env("RAYON_NUM_THREADS", threads)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success(),
            "{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let line = stdout
            .lines()
            .find_map(|l| l.find("digest threads=").map(|at| l[at..].to_owned()))
            .unwrap_or_else(|| panic!("no digest in {stdout}"));
        digests.push(line.rsplit('=').next().unwrap().to_owned());
    }
    assert!(digests.windows(2).all(|w| w[0] == w[1]), "{digests:?}");
}

/// Every budget either gives the same bytes or is refused with `Budget`, never a panic
/// or another error; a refused build leaves no commit in its sink.
#[test]
fn every_budget_gives_the_same_bytes_or_a_refusal() {
    let text = mixed_text(3_000);
    let graph = GraphOptions::default();
    let (reference, _) = run(&text, graph, 1 << 30, 1024, &TUNING).unwrap();
    let mut refused = 0;
    let mut spilled = 0;
    for budget in (0..40).map(|i| 60_000_u64 * (1 << (i / 4)) / 4 * (4 + i % 4) / 4) {
        let reader =
            TextReader::open_with_chunk(MemorySource::new(&text[..]), &text_options(), 1024)
                .unwrap();
        let memory = Memory::from(Arc::new(Budget::new(budget)));
        let options = BuildOptions::new(graph, NonZeroUsize::new(4).unwrap());
        let mut sink = MemorySource::new(Vec::new());
        match build_tuned(
            &reader,
            &options,
            &memory,
            &MemoryScratch,
            &mut sink,
            &Silent,
            &SMALL,
        ) {
            Ok(unfinished) => {
                let (_, report) = unfinished
                    .commit(Provenance {
                        engine: "everygraph test".into(),
                        ..Provenance::default()
                    })
                    .unwrap();
                spilled += u64::from(report.regime == Regime::OnDisk);
                assert!(sink.into_inner() == reference, "budget {budget}");
            }
            Err(BuildError::Budget(_)) => {
                refused += 1;
                assert!(Container::open(sink).is_err(), "budget {budget}");
            }
            Err(e) => panic!("budget {budget}: {e}"),
        }
    }
    assert!(
        refused > 0 && spilled > 0,
        "refused {refused}, spilled {spilled}"
    );
}

/// A sink that fails every write after the first `ok` bytes.
#[derive(Debug)]
struct FailingSink {
    inner: MemorySource<Vec<u8>>,
    ok: u64,
    written: u64,
}

impl ByteSource for FailingSink {
    fn len(&self) -> u64 {
        self.inner.len()
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), everygraph_core::ReadError> {
        self.inner.read_at(offset, buf)
    }
}

impl ByteSink for FailingSink {
    fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<(), WriteError> {
        self.written += u64::try_from(data.len()).unwrap();
        if self.written > self.ok {
            return Err(WriteError::Backend("injected failure".into()));
        }
        self.inner.write_at(offset, data)
    }

    fn set_len(&mut self, len: u64) -> Result<(), WriteError> {
        self.inner.set_len(len)
    }

    fn sync(&mut self) -> Result<(), WriteError> {
        self.inner.sync()
    }
}

/// A write that fails at any point of the output ends the build with an error and
/// leaves no graph in the sink.
#[test]
fn a_failed_write_leaves_no_graph() {
    let text = mixed_text(2_000);
    let build_into = |sink: &mut FailingSink| {
        let reader =
            TextReader::open_with_chunk(MemorySource::new(&text[..]), &text_options(), 1024)
                .unwrap();
        let options = BuildOptions::new(GraphOptions::default(), NonZeroUsize::new(4).unwrap());
        build_tuned(
            &reader,
            &options,
            &Memory::unlimited(),
            &MemoryScratch,
            sink,
            &Silent,
            &TUNING,
        )
        .and_then(|unfinished| unfinished.commit(Provenance::default()))
        .map(|(_, report)| report)
    };
    let mut counting = FailingSink {
        inner: MemorySource::new(Vec::new()),
        ok: u64::MAX,
        written: 0,
    };
    build_into(&mut counting).unwrap();
    let total = counting.written;
    for ok in (0..total).step_by(usize::try_from(total / 37).unwrap()) {
        let mut sink = FailingSink {
            inner: MemorySource::new(Vec::new()),
            ok,
            written: 0,
        };
        let result = build_into(&mut sink);
        assert!(
            matches!(result, Err(BuildError::Container(_))),
            "failing after {ok} bytes: {result:?}"
        );
        assert!(
            Container::open(sink.inner).is_err(),
            "failing after {ok} bytes"
        );
    }
}

/// A bucket longer than a segment of the bucket columns cannot be lent as one slice:
/// the build reports it instead of aborting.
#[test]
fn a_bucket_longer_than_a_segment_is_an_error() {
    let mut text = Vec::new();
    for v in 1..=80 {
        text.extend_from_slice(format!("0\t{v}\n").as_bytes());
    }
    let tuning = Tuning {
        segment_bits: 4,
        bucket_nodes: 1,
        ..SMALL
    };
    let graph = GraphOptions {
        directed: true,
        ..GraphOptions::default()
    };
    let result = run(&text, graph, 1 << 30, 64, &tuning);
    assert!(
        matches!(
            result,
            Err(BuildError::Memory(MemoryError::AllocationFailed { .. }))
        ),
        "{result:?}"
    );
    // With segments of 2^7 rows it fits.
    let tuning = Tuning {
        segment_bits: 7,
        ..tuning
    };
    assert!(run(&text, graph, 1 << 30, 64, &tuning).is_ok());
}

/// Invalid rows and identifiers that do not fit their kind are refused before anything
/// is written.
#[test]
fn invalid_input_is_refused() {
    let reader =
        TextReader::open(MemorySource::new(&b"1\t2\nx\t3\n"[..]), &text_options()).unwrap();
    let options = BuildOptions::new(GraphOptions::default(), NonZeroUsize::new(2).unwrap());
    let mut sink = MemorySource::new(Vec::new());
    let result = build(
        &reader,
        &options,
        &Memory::unlimited(),
        &MemoryScratch,
        &mut sink,
        &Silent,
    );
    assert!(
        matches!(result, Err(BuildError::Text(TextError::InvalidRows(_)))),
        "{result:?}"
    );
    assert_eq!(sink.len(), 0);
}

/// An empty input is a graph with no nodes.
#[test]
fn an_empty_input_is_an_empty_graph() {
    let (bytes, report) = run(b"", GraphOptions::default(), 1 << 20, 64, &TUNING).unwrap();
    assert_eq!((report.nodes, report.edges, report.rows), (0, 0, 0));
    let stored = read_back(&bytes);
    assert!(stored.keys.is_empty());
}

/// Direction from the column names, else undirected.
#[test]
fn direction_is_inferred_from_the_column_names() {
    let layout = |header: &str| {
        let text = format!("{header}\n1,2\n");
        TextReader::open(
            MemorySource::new(text.into_bytes()),
            &TextOptions::default(),
        )
        .unwrap()
        .layout()
        .clone()
    };
    assert_eq!(
        infer_direction(&layout("From,To"), None),
        (true, DirectedFrom::ColumnNames)
    );
    assert_eq!(
        infer_direction(&layout("source,target"), None),
        (true, DirectedFrom::ColumnNames)
    );
    assert_eq!(
        infer_direction(&layout("a,b"), None),
        (false, DirectedFrom::Default)
    );
    assert_eq!(
        infer_direction(&layout("from,to"), Some(false)),
        (false, DirectedFrom::Caller)
    );
}
