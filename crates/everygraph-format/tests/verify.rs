// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tests of `verify` (issue #41): every single-bit flip of a graph file is found, and so
//! is every structural fault that the hashes cannot see because the writer hashed it.
//!
//! Not run under Miri: they hash and decode a file tens of thousands of times.

#![forbid(unsafe_code)]
#![cfg(not(miri))]

use std::collections::BTreeSet;

use everygraph_core::{ByteSource, MemorySource, ReadError};
use everygraph_format::adjacency::encode;
use everygraph_format::container::{Kind, Provenance, Writer};
use everygraph_format::verify::{Depth, Problem, Report, verify};
use proptest::prelude::*;

type Mem = MemorySource<Vec<u8>>;

/// The provenance an ingest would write for an adjacency.
fn built_with(params: &[(&str, &str)]) -> Provenance {
    Provenance {
        engine: "everygraph test".into(),
        operation: "ingest".into(),
        parameters: params
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
        ..Provenance::default()
    }
}

/// Adds adjacency `name` holding `lists`, made as `provenance` says.
fn add_adjacency(w: &mut Writer<Mem>, name: &str, lists: &[Vec<u32>], provenance: &str) {
    let mut offsets = vec![0u64];
    let mut targets = Vec::new();
    for list in lists {
        targets.extend_from_slice(list);
        offsets.push(targets.len() as u64);
    }
    let e = encode(&offsets, &targets).unwrap();
    let index = format!("{name}.index");
    w.add_section(&index, Kind::ADJACENCY_INDEX, &e.index, Some(provenance))
        .unwrap();
    w.add_section(
        &format!("{name}.data"),
        Kind::ADJACENCY_DATA,
        &e.data,
        Some(provenance),
    )
    .unwrap();
}

/// A file holding one adjacency, `adjacency.out`, declared with `params`.
fn graph(lists: &[Vec<u32>], params: &[(&str, &str)]) -> Vec<u8> {
    let mut w = Writer::create(Mem::default()).unwrap();
    w.add_provenance("provenance/ingest", &built_with(params))
        .unwrap();
    add_adjacency(&mut w, "adjacency.out", lists, "provenance/ingest");
    w.commit().unwrap();
    w.into_sink().into_inner()
}

/// The triangle 0–1–2 with a pendant node 3, undirected, without loops.
fn triangle() -> Vec<Vec<u32>> {
    vec![vec![1, 2, 3], vec![0, 2], vec![0, 1], vec![0]]
}

const UNDIRECTED: &[(&str, &str)] = &[("directed", "false"), ("self_loops", "drop")];

fn check(file: &[u8], depth: Depth) -> Report {
    verify(&MemorySource::new(file), depth).unwrap()
}

fn problems(file: &[u8]) -> Vec<Problem> {
    check(file, Depth::Deep).problems
}

#[test]
fn a_sound_file_is_intact_at_both_depths() {
    let file = graph(&triangle(), UNDIRECTED);
    for depth in [Depth::Fast, Depth::Deep] {
        let report = check(&file, depth);
        assert!(report.is_intact(), "{depth:?}: {:?}", report.problems);
        assert_eq!(report.generation, Some(1));
        assert_eq!((report.sections, report.sections_hashed), (3, 3));
        assert!(report.unchecked.is_empty(), "{:?}", report.unchecked);
    }
    let deep = check(&file, Depth::Deep);
    assert_eq!(deep.adjacencies.len(), 1);
    assert_eq!(
        (deep.adjacencies[0].nodes, deep.adjacencies[0].entries),
        (4, 8)
    );
}

/// Acceptance: "every single-bit flip injected into a test file is detected". The file
/// is written once, so every byte belongs to a header block, a section, the table of
/// contents or their padding, and each must be checked. A flip in the first 128 bytes
/// of header block 1, which the file does not read from, cannot be told from a commit
/// cut short: it is a note; every other flip is a problem.
#[test]
fn every_single_bit_flip_is_detected() {
    let file = graph(&triangle(), UNDIRECTED);
    assert!(check(&file, Depth::Deep).is_intact());
    let bits = file.len() * 8;
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get().min(8));
    let missed: Vec<usize> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|t| {
                let file = &file;
                scope.spawn(move || {
                    let mut damaged = file.clone();
                    let mut missed = Vec::new();
                    for bit in (t..bits).step_by(threads) {
                        damaged[bit / 8] ^= 1 << (bit % 8);
                        let report = check(&damaged, Depth::Deep);
                        let unused_head = (4096 * 8..4224 * 8).contains(&bit);
                        let found = if unused_head {
                            report.is_intact() && report.notes.len() == 1
                        } else {
                            !report.is_intact()
                        };
                        if !found {
                            missed.push(bit);
                        }
                        damaged[bit / 8] ^= 1 << (bit % 8);
                    }
                    missed
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().unwrap())
            .collect()
    });
    assert!(
        missed.is_empty(),
        "{} of {bits} flips not detected, the first at bits {:?}",
        missed.len(),
        &missed[..missed.len().min(10)]
    );
}

#[test]
fn the_fast_check_reads_only_small_sections() {
    let big = vec![7u8; 2 << 20];
    let mut w = Writer::create(Mem::default()).unwrap();
    w.add_section("results/big", Kind::new(0x8000_0001).unwrap(), &big, None)
        .unwrap();
    w.commit().unwrap();
    let mut file = w.into_sink().into_inner();
    file[8192 + 1000] ^= 1;
    let fast = check(&file, Depth::Fast);
    assert!(fast.is_intact());
    assert_eq!(fast.sections_hashed, 0);
    let deep = check(&file, Depth::Deep);
    assert_eq!(deep.problems.len(), 1);
    assert_eq!(deep.problems[0].section.as_deref(), Some("results/big"));
    assert_eq!(deep.problems[0].what, "its bytes do not match its hash");
}

/// Faults the writer hashed as if they were right: only the structure finds them.
#[test]
fn structural_faults_behind_valid_hashes_are_found() {
    let found = |file: &[u8]| -> Vec<String> {
        problems(file)
            .into_iter()
            .map(|p| format!("{}: {}", p.section.unwrap_or_default(), p.what))
            .collect()
    };
    let one_way = vec![vec![1], vec![], vec![]];
    assert_eq!(
        found(&graph(&one_way, UNDIRECTED)),
        ["adjacency.out.data: the lists are not symmetric, but the graph is undirected"]
    );
    assert!(
        found(&graph(
            &one_way,
            &[("directed", "true"), ("self_loops", "drop")]
        ))
        .is_empty()
    );
    let looped = vec![vec![0, 1], vec![0]];
    assert_eq!(
        found(&graph(&looped, UNDIRECTED)),
        ["adjacency.out.data: 1 self-loops, but the graph was built with self-loops dropped"]
    );
    assert!(
        found(&graph(
            &looped,
            &[("directed", "false"), ("self_loops", "keep")]
        ))
        .is_empty()
    );
    let outside = vec![vec![5], vec![]];
    assert_eq!(
        found(&graph(&outside, &[])),
        ["adjacency.out.data: node 0: a neighbour is not below the number of nodes"]
    );
}

#[test]
fn what_the_provenance_does_not_declare_is_reported_as_unchecked() {
    let report = check(&graph(&triangle(), &[]), Depth::Deep);
    assert!(report.is_intact());
    assert_eq!(
        report.unchecked,
        [
            "adjacency.out: self-loops (its provenance does not say whether they were kept)",
            "adjacency.out: symmetry (its provenance does not say whether the graph is directed)",
        ]
    );
}

/// A directed graph: the out-adjacency and the in-adjacency built from `arcs`.
fn directed(nodes: usize, arcs: &[(u32, u32)], in_arcs: &[(u32, u32)]) -> Vec<u8> {
    let lists = |arcs: &[(u32, u32)]| {
        let mut lists = vec![BTreeSet::new(); nodes];
        for &(u, v) in arcs {
            lists[u as usize].insert(v);
        }
        lists
            .into_iter()
            .map(|l| l.into_iter().collect())
            .collect::<Vec<Vec<u32>>>()
    };
    let reversed: Vec<(u32, u32)> = in_arcs.iter().map(|&(u, v)| (v, u)).collect();
    let mut w = Writer::create(Mem::default()).unwrap();
    let params = [("directed", "true"), ("self_loops", "drop")];
    w.add_provenance("provenance/ingest", &built_with(&params))
        .unwrap();
    add_adjacency(&mut w, "adjacency.out", &lists(arcs), "provenance/ingest");
    add_adjacency(
        &mut w,
        "adjacency.in",
        &lists(&reversed),
        "provenance/ingest",
    );
    w.commit().unwrap();
    w.into_sink().into_inner()
}

#[test]
fn an_in_adjacency_must_be_the_transpose_of_its_out_adjacency() {
    let arcs = [(0, 1), (1, 2), (2, 0), (0, 2)];
    assert!(problems(&directed(3, &arcs, &arcs)).is_empty());
    for wrong in [&arcs[..3], &[(0, 1), (1, 2), (2, 0), (2, 1)][..]] {
        let found = problems(&directed(3, &arcs, wrong));
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].section.as_deref(), Some("adjacency.in.data"));
    }
}

#[test]
fn adjacency_sections_must_come_in_pairs_with_the_right_names() {
    let mut w = Writer::create(Mem::default()).unwrap();
    let e = encode(&[0, 0], &[]).unwrap();
    w.add_section("lonely.index", Kind::ADJACENCY_INDEX, &e.index, None)
        .unwrap();
    w.add_section("a.data.x", Kind::ADJACENCY_DATA, &e.data, None)
        .unwrap();
    w.add_section("b.events", Kind::new(0x8000_0002).unwrap(), &[], None)
        .unwrap();
    w.add_section("lonely.events", Kind::ADJACENCY_EVENTS, &[], None)
        .unwrap();
    w.commit().unwrap();
    let found: Vec<String> = problems(&w.into_sink().into_inner())
        .into_iter()
        .map(|p| format!("{}: {}", p.section.unwrap(), p.what))
        .collect();
    assert_eq!(
        found,
        [
            "a.data.x: an adjacency section whose name does not end in .data",
            "b.events: it is named like an events section but is not one",
            "lonely.index: the adjacency has no lonely.data section",
        ]
    );
}

#[test]
fn a_file_that_fell_back_to_an_older_commit_is_not_intact() {
    let mut w = Writer::create(Mem::default()).unwrap();
    w.add_section("a", Kind::new(0x8000_0001).unwrap(), b"one", None)
        .unwrap();
    w.commit().unwrap();
    w.add_section("b", Kind::new(0x8000_0001).unwrap(), b"two", None)
        .unwrap();
    w.commit().unwrap();
    let mut file = w.into_sink().into_inner();
    // The newest table of contents is the last item; damage its first byte.
    let toc = file.len() - 4096;
    file[toc] ^= 1;
    let report = check(&file, Depth::Fast);
    assert_eq!(report.generation, Some(1));
    assert_eq!(
        report.problems[0].what,
        "the newest commit (generation 2) cannot be used; the file reads as generation 1"
    );
}

#[test]
fn what_is_not_a_graph_file_is_a_problem_and_a_failing_source_an_error() {
    let report = check(b"source,target\n1,2\n", Depth::Fast);
    assert_eq!(report.generation, None);
    assert_eq!(report.problems.len(), 1);

    struct Failing;
    impl ByteSource for Failing {
        fn len(&self) -> u64 {
            1 << 20
        }
        fn read_at(&self, _: u64, _: &mut [u8]) -> Result<(), ReadError> {
            Err(ReadError::Backend("injected EIO".into()))
        }
    }
    assert!(matches!(
        verify(&Failing, Depth::Fast),
        Err(ReadError::Backend(_))
    ));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// The pair-hash symmetry check against the definition: an undirected graph is
    /// sound exactly when every arc has its reverse.
    #[test]
    fn symmetry_is_found_exactly_when_it_holds(
        arcs in proptest::collection::btree_set((0u32..12, 0u32..12), 0..40),
        symmetrize in any::<bool>(),
    ) {
        let mut set: BTreeSet<(u32, u32)> = arcs.into_iter().filter(|(u, v)| u != v).collect();
        if symmetrize {
            set = set.iter().flat_map(|&(u, v)| [(u, v), (v, u)]).collect();
        }
        let mut lists = vec![Vec::new(); 12];
        for &(u, v) in &set {
            lists[u as usize].push(v);
        }
        let symmetric = set.iter().all(|&(u, v)| set.contains(&(v, u)));
        prop_assert_eq!(problems(&graph(&lists, UNDIRECTED)).is_empty(), symmetric);
    }
}
