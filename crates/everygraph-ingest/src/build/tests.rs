// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The build against the oracle's projection, on every graph of the CI corpus under
//! every option, with integer and text identifiers; the same bytes at any thread count
//! and any budget, in memory and with spills; an input that changes between passes,
//! refusals and write failures that leave no graph behind.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use everygraph_core::{
    Budget, ByteSink, ByteSource, Memory, MemorySource, NodeId, ReadError, WriteError,
};
use everygraph_format::adjacency::{Adjacency, AdjacencyEvents, AdjacencyIndex};
use everygraph_format::container::{Container, Provenance};
use everygraph_oracle::corpus::{Tier, specs};
use everygraph_oracle::generators::generate;
use everygraph_oracle::graph::{self as oracle_graph, Graph};
use everygraph_oracle::projection::Projection;

use super::*;
use crate::ids::{KeyKind, MemoryScratch, OwnedKey};
use crate::text::{Delimiter, Header, IdType, TextOptions};

pub(super) const SMALL: Tuning = Tuning {
    segment_bits: 12,
    bucket_nodes: 5,
    spill_batch: 7,
    resident_items: u64::MAX,
    cache_rows: false,
};

/// [`SMALL`], holding at most 40 items in memory: every graph with more spills.
pub(super) const SPILLING: Tuning = Tuning {
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

/// How one test build runs.
#[derive(Clone, Copy)]
struct Run<'t> {
    graph: GraphOptions,
    ids: IdType,
    budget: u64,
    chunk: usize,
    tuning: &'t Tuning,
    threads: usize,
}

impl Run<'static> {
    fn new(graph: GraphOptions) -> Self {
        Self {
            graph,
            ids: IdType::Integer,
            budget: 1 << 30,
            chunk: 64,
            tuning: &TUNING,
            threads: 8,
        }
    }
}

impl Run<'_> {
    fn text_options(&self) -> TextOptions {
        TextOptions {
            delimiter: Delimiter::Byte(b'\t'),
            header: Header::Absent,
            ids: self.ids,
            ..TextOptions::default()
        }
    }

    /// Builds from `source` into `sink`, and commits.
    fn build_into<S: ByteSource + Sync, K: ByteSink>(
        &self,
        source: S,
        sink: K,
        progress: &dyn Progress,
    ) -> Result<(K, Report), BuildError> {
        let reader = TextReader::open_with_chunk(source, &self.text_options(), self.chunk).unwrap();
        let memory = Memory::from(Arc::new(Budget::new(self.budget)));
        let mut options = BuildOptions::new(self.graph, NonZeroUsize::new(self.threads).unwrap());
        if self.ids == IdType::String {
            // Text that looks like integers is kept as text, as `--ids string` does.
            options.ids = options.ids.with_kind(KeyKind::Bytes);
        }
        let unfinished = build_tuned(
            &reader,
            &options,
            &memory,
            &MemoryScratch,
            sink,
            progress,
            self.tuning,
        )?;
        unfinished.commit(Provenance {
            engine: "everygraph test".into(),
            ..Provenance::default()
        })
    }

    /// The bytes of the graph built from `text`, and its report.
    fn bytes(&self, text: &[u8]) -> Result<(Vec<u8>, Report), BuildError> {
        let (sink, report) = self.build_into(
            MemorySource::new(text),
            MemorySource::new(Vec::new()),
            &Silent,
        )?;
        Ok((sink.into_inner(), report))
    }
}

/// A graph read back: each node's key as text, and its list of (neighbour, stored
/// events).
pub(super) struct Stored {
    pub(super) keys: Vec<String>,
    pub(super) lists: Vec<Vec<(u32, u64)>>,
    events_section: bool,
    pub(super) parameters: BTreeMap<String, String>,
}

pub(super) fn read_back(bytes: &[u8]) -> Stored {
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
            OwnedKey::UInt(k) => k.to_string(),
            OwnedKey::Text(k) => k,
            other => panic!("keys come back as {other:?}"),
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

/// Graph order: sorted integers, or text in the order the rows first name it.
fn graph_order(g: &Graph, ids: IdType) -> Vec<String> {
    let mut seen = Vec::new();
    for (&u, &v) in g.src.iter().zip(&g.dst) {
        seen.extend([u, v]);
    }
    if ids == IdType::String {
        let mut known = std::collections::HashSet::new();
        seen.retain(|v| known.insert(*v));
    } else {
        seen.sort_unstable();
        seen.dedup();
    }
    seen.iter().map(u32::to_string).collect()
}

/// Builds `g` and compares the stored graph and the report with the oracle's
/// projection of the same rows.
fn compare_with_oracle(g: &Graph, run: &Run<'_>) {
    let graph = run.graph;
    let mut g = g.clone();
    g.semantics = oracle_semantics(graph);
    let projection = Projection::new(&g);
    let (bytes, report) = run
        .bytes(&text_of(&g))
        .unwrap_or_else(|e| panic!("{}: {e}", g.name));
    // The file passes the deep check of `everygraph verify`.
    let verified = everygraph_format::verify::verify(
        &MemorySource::new(&bytes[..]),
        everygraph_format::verify::Depth::Deep,
        &Memory::unlimited(),
    )
    .unwrap();
    assert!(
        verified.problems.is_empty(),
        "{}: {:?}",
        g.name,
        verified.problems
    );
    let stored = read_back(&bytes);
    assert_eq!(stored.keys, graph_order(&g, run.ids), "{}", g.name);

    let events = graph.duplicates == Duplicates::Events;
    for (id, key) in stored.keys.iter().enumerate() {
        let v: usize = key.parse().unwrap();
        let mut expected: Vec<(String, u64)> = projection
            .out
            .range(v)
            .map(|e| {
                let m = if events {
                    projection.out.multiplicity[e]
                } else {
                    1
                };
                (projection.out.targets[e].to_string(), m)
            })
            .collect();
        if graph.self_loops == SelfLoops::Keep && projection.loops[v] > 0 {
            let m = if events { projection.loops[v] } else { 1 };
            expected.push((key.clone(), m));
        }
        expected.sort_unstable();
        let mut got: Vec<(String, u64)> = stored.lists[id]
            .iter()
            .map(|&(t, m)| (stored.keys[usize::try_from(t).unwrap()].clone(), m))
            .collect();
        // Lists are in increasing internal id; compare them as sets of keys.
        assert!(stored.lists[id].windows(2).all(|w| w[0].0 < w[1].0));
        got.sort_unstable();
        assert_eq!(got, expected, "{}: node {key}", g.name);
    }

    // The report: the oracle's, but for nodes that appear in no row.
    let absent = u64::from(projection.n) - u64::try_from(stored.keys.len()).unwrap();
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

pub(super) fn all_options(directed: bool) -> Vec<GraphOptions> {
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
            compare_with_oracle(&g, &Run::new(graph));
        }
    }
}

/// The same with text identifiers: graph order by first appearance, look-ups through
/// the hash function.
#[test]
fn the_ci_corpus_is_stored_the_same_with_text_identifiers() {
    for spec in specs(Tier::Ci).into_iter().step_by(2) {
        let g = generate(&spec);
        for graph in all_options(g.directed) {
            let run = Run {
                ids: IdType::String,
                ..Run::new(graph)
            };
            compare_with_oracle(&g, &run);
        }
    }
}

/// The same with tiny buckets, segments and budget: many rounds, ranges across
/// segment ends, and spills.
#[test]
fn the_ci_corpus_is_stored_the_same_with_spills() {
    let mut spilled = 0;
    for (i, spec) in specs(Tier::Ci).into_iter().step_by(3).enumerate() {
        let g = generate(&spec);
        for graph in all_options(g.directed) {
            let run = Run {
                budget: 3 << 20,
                tuning: &SPILLING,
                ids: if i % 2 == 0 {
                    IdType::Integer
                } else {
                    IdType::String
                },
                ..Run::new(graph)
            };
            compare_with_oracle(&g, &run);
            let (_, report) = run.bytes(&text_of(&g)).unwrap();
            spilled += u64::from(report.regime == Regime::OnDisk);
        }
    }
    assert!(spilled > 0, "no graph spilled");
}

/// A graph with loops, repeated pairs in both directions, and a few hubs; every id
/// has five digits, so that `shift` changes the ids and not the length.
fn mixed_text(rows: u64, shift: u64) -> Vec<u8> {
    let mut text = Vec::new();
    let mut x = 0x2545_F491_4F6C_DD1D_u64;
    for i in 0..rows {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let u = if i % 7 == 0 { x % 5 } else { x % 3000 };
        let v = (x >> 20) % 3000;
        let (u, v) = if i % 11 == 0 { (v, u) } else { (u, v) };
        let line = format!("{}\t{}\n", 10_000 + shift + u * 3, 10_000 + shift + v * 3);
        text.extend_from_slice(line.as_bytes());
        if i % 13 == 0 {
            text.extend_from_slice(line.as_bytes());
        }
    }
    text
}

/// A digest of builds of [`mixed_text`] under every option, with integer and text
/// identifiers, each in memory at a large budget and with spills at a small one, the
/// build's thread option set to the pool size. Printed for
/// [`output_does_not_depend_on_threads_or_budget`], which runs it in child processes
/// with different thread counts.
#[test]
#[ignore = "run by output_does_not_depend_on_threads_or_budget in a child process"]
fn digest_probe() {
    let threads = std::env::var("RAYON_NUM_THREADS").unwrap_or_default();
    let text = mixed_text(20_000, 0);
    let mut digest = 0_u64;
    for ids in [IdType::Integer, IdType::String] {
        for directed in [false, true] {
            let mut small = None;
            for graph in all_options(directed) {
                let large = Run {
                    ids,
                    chunk: 4096,
                    threads: threads.parse().unwrap(),
                    ..Run::new(graph)
                };
                let (in_memory, report) = large.bytes(&text).unwrap();
                assert_eq!(report.regime, Regime::InMemory);
                assert_eq!(report.spilled_bytes, 0);
                let budget = *small.get_or_insert_with(|| spilling_budget(&large, &text));
                let run = Run {
                    budget,
                    tuning: &SMALL,
                    ..large
                };
                let (spilled, report) = run.bytes(&text).unwrap();
                assert_eq!(report.regime, Regime::OnDisk, "{graph:?}");
                assert!(report.spilled_bytes > 0);
                assert!(
                    in_memory == spilled,
                    "{graph:?}: the budget changed the bytes"
                );
                digest = (digest ^ xxhash_rust::xxh3::xxh3_64(&in_memory))
                    .wrapping_mul(0x9E37_79B9_7F4A_7C15);
            }
        }
    }
    #[expect(
        clippy::print_stdout,
        reason = "the parent test reads the digest from the output"
    )]
    {
        println!("digest threads={threads} value={digest:016x}");
    }
}

/// The largest budget, in steps of 128 KiB down from 4 MiB, at which `large` spills
/// with [`SMALL`].
fn spilling_budget(large: &Run<'_>, text: &[u8]) -> u64 {
    for budget in (1..=32).rev().map(|k| k << 17) {
        let small = Run {
            budget,
            tuning: &SMALL,
            ..*large
        };
        match small.bytes(text) {
            Ok((_, report)) if report.regime == Regime::OnDisk => return budget,
            Ok(_) => {}
            Err(e) => panic!("no budget spills before a refusal ({budget}: {e})"),
        }
    }
    panic!("no budget spills");
}

/// The output is byte-identical at 1, 3 and 8 threads (the pool and the build's own
/// thread option, which sizes its buffers) and at two budgets, one of which spills.
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
/// or another error; a refused build leaves no commit in its sink. Integer keys at
/// budgets below their dictionary's bitmap are refused, never given the hashed form.
#[test]
fn every_budget_gives_the_same_bytes_or_a_refusal() {
    let text = mixed_text(3_000, 0);
    for ids in [IdType::Integer, IdType::String] {
        let large = Run {
            ids,
            chunk: 1024,
            threads: 4,
            ..Run::new(GraphOptions::default())
        };
        let (reference, _) = large.bytes(&text).unwrap();
        let (mut refused, mut spilled) = (0, 0);
        for i in 0..44_u64 {
            let budget = 60_000 * (1 << (i / 4)) / 4 * (4 + i % 4) / 4;
            let run = Run {
                budget,
                tuning: &SMALL,
                ..large
            };
            let mut sink = MemorySource::new(Vec::new());
            match run.build_into(MemorySource::new(&text[..]), &mut sink, &Silent) {
                Ok((_, report)) => {
                    spilled += u64::from(report.regime == Regime::OnDisk);
                    assert!(sink.into_inner() == reference, "{ids:?}, budget {budget}");
                }
                Err(BuildError::Budget(refusal)) => {
                    refused += 1;
                    assert!(Container::open(sink).is_err(), "budget {budget}");
                    // A refused bitmap names its whole bound, so that the budget it
                    // suggests is enough: at least its table, never one segment.
                    if refusal.purpose == "id dictionary dense bitmap" {
                        assert!(refusal.needed > 1 << 20, "{refusal:?}");
                    }
                }
                Err(e) => panic!("{ids:?}, budget {budget}: {e}"),
            }
        }
        assert!(refused > 0, "{ids:?}: nothing refused");
        if ids == IdType::String {
            assert!(spilled > 0, "{ids:?}: nothing spilled");
        }
    }
}

/// A source that gives other bytes once `switched` is set: a file overwritten in
/// place, or cut short (reads past its new end fail as a truncated file's do).
struct Changing {
    before: Vec<u8>,
    after: Vec<u8>,
    switched: AtomicBool,
}

impl ByteSource for Changing {
    fn len(&self) -> u64 {
        u64::try_from(self.before.len()).unwrap()
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), ReadError> {
        let bytes = if self.switched.load(Ordering::Relaxed) {
            &self.after
        } else {
            &self.before
        };
        let start = usize::try_from(offset).unwrap();
        match bytes.get(start..start + buf.len()) {
            Some(src) => {
                buf.copy_from_slice(src);
                Ok(())
            }
            None => Err(ReadError::Backend(Box::new(std::io::Error::from(
                std::io::ErrorKind::UnexpectedEof,
            )))),
        }
    }
}

/// Switches the source when `phase` begins, or at the first block read.
struct SwitchAt<'s> {
    source: &'s Changing,
    phase: Phase,
}

impl Progress for SwitchAt<'_> {
    fn begin(&self, phase: Phase, _total: Option<u64>) {
        if phase == self.phase && phase != Phase::Reading {
            self.source.switched.store(true, Ordering::Relaxed);
        }
    }

    fn advance(&self, phase: Phase, _done: u64) {
        if phase == self.phase {
            self.source.switched.store(true, Ordering::Relaxed);
        }
    }
}

/// An input overwritten or cut short between passes is refused with `InputChanged`,
/// with integer and text identifiers, in memory and with spills, whether it changes
/// while the dictionary reads it or after; nothing is committed.
#[test]
fn an_input_that_changes_while_it_is_read_is_refused() {
    let before = mixed_text(4_000, 0);
    let shifted = mixed_text(4_000, 1);
    assert_eq!(before.len(), shifted.len());
    let cut = before[..before.len() / 2].to_vec();
    for after in [&shifted, &cut] {
        for ids in [IdType::Integer, IdType::String] {
            for phase in [Phase::Reading, Phase::Identifiers] {
                for tuning in [&TUNING, &SMALL] {
                    let source = Changing {
                        before: before.clone(),
                        after: after.clone(),
                        switched: AtomicBool::new(false),
                    };
                    let run = Run {
                        ids,
                        chunk: 4096,
                        budget: 3 << 20,
                        tuning,
                        ..Run::new(GraphOptions::default())
                    };
                    let progress = SwitchAt {
                        source: &source,
                        phase,
                    };
                    let mut sink = MemorySource::new(Vec::new());
                    let result = run.build_into(&source, &mut sink, &progress);
                    let case = format!(
                        "{ids:?}, switched at {phase:?}, cut {}",
                        after.len() < before.len()
                    );
                    assert!(
                        matches!(result, Err(BuildError::InputChanged)),
                        "{case}: {result:?}"
                    );
                    assert!(Container::open(sink).is_err(), "{case}");
                }
            }
        }
    }
}

/// A sink that fails every write after the first `ok` bytes.
#[derive(Debug)]
pub(super) struct FailingSink {
    pub(super) inner: MemorySource<Vec<u8>>,
    pub(super) ok: u64,
    pub(super) written: u64,
}

impl ByteSource for FailingSink {
    fn len(&self) -> u64 {
        self.inner.len()
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), ReadError> {
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
    let text = mixed_text(2_000, 0);
    let run = Run {
        chunk: 1024,
        threads: 4,
        ..Run::new(GraphOptions::default())
    };
    let build_into = |sink: &mut FailingSink| {
        run.build_into(MemorySource::new(&text[..]), sink, &Silent)
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

/// A node with more items than a segment of the bucket columns holds cannot be lent as
/// one slice: the build refuses it before any work, instead of aborting.
#[test]
fn a_list_longer_than_a_segment_is_refused_before_any_work() {
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
    let run = Run {
        tuning: &tuning,
        ..Run::new(graph)
    };
    let mut sink = MemorySource::new(Vec::new());
    let result = run.build_into(MemorySource::new(&text[..]), &mut sink, &Silent);
    assert!(
        matches!(
            result,
            Err(BuildError::Memory(MemoryError::AllocationFailed { .. }))
        ),
        "{result:?}"
    );
    assert_eq!(sink.len(), 0, "the output was touched");
    // With segments of 2^7 rows it fits.
    let tuning = Tuning {
        segment_bits: 7,
        ..tuning
    };
    let run = Run {
        tuning: &tuning,
        ..run
    };
    assert!(run.bytes(&text).is_ok());
}

/// Invalid rows and identifiers that do not fit their kind are refused before anything
/// is written.
#[test]
fn invalid_input_is_refused() {
    let run = Run::new(GraphOptions::default());
    let mut sink = MemorySource::new(Vec::new());
    let result = run.build_into(MemorySource::new(&b"1\t2\nx\t3\n"[..]), &mut sink, &Silent);
    assert!(
        matches!(result, Err(BuildError::Text(TextError::InvalidRows(_)))),
        "{result:?}"
    );
    assert_eq!(sink.len(), 0);
}

/// An empty input is a graph with no nodes.
#[test]
fn an_empty_input_is_an_empty_graph() {
    let (bytes, report) = Run::new(GraphOptions::default()).bytes(b"").unwrap();
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

/// The plan's estimate follows the build's rules: more threads need a larger smallest
/// budget, and a budget at which the estimate spills half the items does spill.
#[test]
fn the_estimate_spills_no_sooner_than_the_build() {
    let text = mixed_text(400_000, 0);
    let rows = 400_000 + 400_000_u64.div_ceil(13);
    let block = 1 << 16;
    let blocks = u64::try_from(text.len().div_ceil(block)).unwrap();
    let sample = Sample::new(rows, rows / blocks + 512, 1 << 16, blocks);
    let threads = |n| NonZeroUsize::new(n).unwrap();
    let one = estimate(&sample, false, threads(1), 1 << 30);
    let eight = estimate(&sample, false, threads(8), 1 << 30);
    assert!(eight.smallest_budget > one.smallest_budget);
    assert_eq!(one.regime, Regime::InMemory);
    assert_eq!(one.scratch_bytes, 0);
    let mut checked = 0;
    for budget in (12..64).map(|k| k << 18) {
        let planned = estimate(&sample, false, threads(1), budget);
        let clear = planned.scratch_bytes * 2 >= planned.working_bytes;
        if !clear || budget < planned.smallest_budget {
            continue;
        }
        let run = Run {
            budget,
            chunk: block,
            threads: 1,
            tuning: &TUNING,
            ..Run::new(GraphOptions::default())
        };
        match run.bytes(&text) {
            Ok((_, report)) => {
                assert_eq!(report.regime, Regime::OnDisk, "budget {budget}");
                checked += 1;
            }
            Err(BuildError::Budget(_)) => {}
            Err(e) => panic!("budget {budget}: {e}"),
        }
    }
    assert!(checked > 0);
}

/// Integer keys spread over `0..2^32` take the hashed form at a small budget, with the
/// bytes of a large one: the bitmap stops at the first key that no dense dictionary of
/// this input could hold, instead of growing to 512 MiB and being refused.
#[test]
fn sparse_integer_keys_take_the_hashed_form_at_any_budget() {
    let mut text = Vec::new();
    let mut x = 0x2545_F491_4F6C_DD1D_u64;
    for _ in 0..20_000 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let (u, v) = (x >> 32, x & u64::from(u32::MAX));
        text.extend_from_slice(format!("{u}\t{v}\n").as_bytes());
    }
    let large = Run {
        budget: 8 << 30,
        chunk: 4096,
        ..Run::new(GraphOptions::default())
    };
    let small = Run {
        budget: 8 << 20,
        ..large
    };
    let (expected, _) = large.bytes(&text).unwrap();
    let (got, _) = small.bytes(&text).unwrap();
    assert!(got == expected, "the budget changed the bytes");
    let file = Container::open(MemorySource::new(&got[..])).unwrap();
    assert!(file.get("ids.parts").is_some(), "not the hashed form");
    assert!(file.get("ids.dense").is_none());
}
