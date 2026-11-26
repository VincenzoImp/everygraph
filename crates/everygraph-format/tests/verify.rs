// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tests of `verify` (issue #41): every single-bit flip of a graph file is found, and so
//! is every structural fault that the hashes cannot see because the writer hashed it.
//!
//! Not run under Miri: they hash and decode a file tens of thousands of times.

#![forbid(unsafe_code)]
#![cfg(not(miri))]

use std::collections::BTreeSet;

use everygraph_core::{ByteSource, Memory, MemorySource, ReadError};
use everygraph_format::adjacency::{IndexBuilder, append_record, encode, encode_events};
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
    verify(&MemorySource::new(file), depth, &Memory::unlimited()).unwrap()
}

/// A source that is not in memory: every byte comes through `read_at`.
struct NotMapped<'a>(&'a [u8]);

impl ByteSource for NotMapped<'_> {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), ReadError> {
        MemorySource::new(self.0).read_at(offset, buf)
    }
}

/// The problems a deep check finds, the same whether the file is mapped or read.
fn problems(file: &[u8]) -> Vec<Problem> {
    let report = check(file, Depth::Deep);
    assert_eq!(
        verify(&NotMapped(file), Depth::Deep, &Memory::unlimited()).unwrap(),
        report
    );
    report.problems
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

/// Flips every bit of `file` in turn and returns the bits whose deep report `expected`
/// rejects.
#[expect(
    clippy::disallowed_methods,
    reason = "a test may use threads; the crate's code goes through par, sequential here"
)]
fn flips(file: &[u8], expected: impl Fn(usize, &Report) -> bool + Sync) -> Vec<usize> {
    let bits = file.len() * 8;
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get().min(8));
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|t| {
                let expected = &expected;
                scope.spawn(move || {
                    let mut damaged = file.to_vec();
                    let mut missed = Vec::new();
                    for bit in (t..bits).step_by(threads) {
                        damaged[bit / 8] ^= 1 << (bit % 8);
                        if !expected(bit, &check(&damaged, Depth::Deep)) {
                            missed.push(bit);
                        }
                        damaged[bit / 8] ^= 1 << (bit % 8);
                    }
                    missed
                })
            })
            .collect();
        let mut missed: Vec<usize> = workers
            .into_iter()
            .flat_map(|w| w.join().unwrap())
            .collect();
        missed.sort_unstable();
        missed
    })
}

/// Bits of the 128 used bytes of header block `b`.
fn head(b: usize) -> std::ops::Range<usize> {
    b * 4096 * 8..(b * 4096 + 128) * 8
}

/// Acceptance: "every single-bit flip injected into a test file is detected". The file
/// is written once, so every byte belongs to a header block, a section, the table of
/// contents or their padding, and each must be checked. A flip in the used bytes of
/// header block 1, which the file does not read from, cannot be told from a commit cut
/// short: it is a note; every other flip is a problem.
#[test]
fn every_single_bit_flip_is_detected() {
    let file = graph(&triangle(), UNDIRECTED);
    assert!(check(&file, Depth::Deep).is_intact());
    let missed = flips(&file, |bit, report| {
        if head(1).contains(&bit) {
            report.is_intact() && report.notes.len() == 1
        } else {
            !report.is_intact()
        }
    });
    assert!(
        missed.is_empty(),
        "{} flips missed: {missed:?}",
        missed.len()
    );
}

/// The same on a file of two commits. A flip in the newest header block (block 1) makes
/// the file read as generation 1: it cannot be told from a second commit cut short
/// while its header block was written, so it is a note, and generation 2 is named as
/// lost when its table of contents is whole (`design/DECISIONS.md`, "A lost newest
/// commit"). A flip in the newest table of contents makes the file read as generation
/// 1 too, with generation 2 named as lost. A flip in the table of generation 1, which
/// no reader of generation 2 reads, changes nothing.
#[test]
fn every_single_bit_flip_of_a_file_of_two_commits_is_detected_or_harmless() {
    let mut w = Writer::update(Mem::new(graph(&triangle(), UNDIRECTED))).unwrap();
    let old_end = w.sink().len();
    w.add_section("results/x", Kind::PRIVATE, b"a result", None)
        .unwrap();
    w.commit().unwrap();
    let file = w.into_sink().into_inner();
    assert_eq!(check(&file, Depth::Deep).generation, Some(2));
    let bits =
        |end: u64| usize::try_from(end - 4096).unwrap() * 8..usize::try_from(end).unwrap() * 8;
    let (old_toc, new_toc) = (bits(old_end), bits(file.len() as u64));
    let missed = flips(&file, |bit, report| {
        let note = report.is_intact() && report.notes.len() == 1;
        let lost =
            report.is_intact() && report.generation == Some(1) && report.lost_generation == Some(2);
        if new_toc.contains(&bit) {
            !report.is_intact() || lost
        } else if old_toc.contains(&bit) {
            report.is_intact() && report.notes.is_empty()
        } else if head(0).contains(&bit) {
            !report.is_intact() || (note && report.generation == Some(2))
        } else if head(1).contains(&bit) {
            // The block's own hash: the rest of the block names generation 2 and its table.
            let lost =
                !(4096 + 96..4096 + 128).contains(&(bit / 8)) || report.lost_generation == Some(2);
            !report.is_intact() || (note && report.generation == Some(1) && lost)
        } else {
            !report.is_intact()
        }
    });
    assert!(
        missed.is_empty(),
        "{} flips missed: {missed:?}",
        missed.len()
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

/// A file holding adjacency `adjacency.out` made of `index` and `data` as given, and
/// the `extra` sections.
fn raw(index: &[u8], data: &[u8], extra: &[(&str, Kind)]) -> Vec<u8> {
    let mut w = Writer::create(Mem::default()).unwrap();
    w.add_section("adjacency.out.index", Kind::ADJACENCY_INDEX, index, None)
        .unwrap();
    w.add_section("adjacency.out.data", Kind::ADJACENCY_DATA, data, None)
        .unwrap();
    for (name, kind) in extra {
        w.add_section(name, *kind, &[], None).unwrap();
    }
    w.commit().unwrap();
    w.into_sink().into_inner()
}

/// Records that the writer hashed but that do not decode as their index says.
#[test]
fn records_that_do_not_decode_or_do_not_increase_are_found() {
    // Node 0 of 3 lists 2 and 2 + 2^24 + 1: words of one byte and of four bytes.
    let e = encode(&[0, 2, 2, 2], &[2, 2 + (1 << 24) + 1]).unwrap();
    assert_eq!(e.data, [0x0c, 0x04, 0x00, 0x00, 0x00, 0x01]);
    let first = |file: &[u8]| problems(file).remove(0).what;
    // The same lengths, but the second gap wraps past 2^32 to neighbour 1.
    let wraps = [0x0c, 0x04, 0xfe, 0xff, 0xff, 0xff];
    assert_eq!(
        first(&raw(&e.index, &wraps, &[])),
        "node 0: its list is not strictly increasing"
    );
    // The control byte asks for a first word of two bytes: one byte more than the record.
    let long = [0x0d, 0x04, 0x00, 0x00, 0x00, 0x01];
    assert_eq!(
        first(&raw(&e.index, &long, &[])),
        "node 0: its record does not decode"
    );
    let events = raw(
        &e.index,
        &e.data,
        &[("adjacency.out.events", Kind::ADJACENCY_EVENTS)],
    );
    assert_eq!(
        first(&events),
        "an events section, but adjacency.out.index does not announce one"
    );
}

/// A source that counts the bytes read through it.
struct Counting<'a> {
    bytes: &'a [u8],
    read: std::sync::atomic::AtomicU64,
}

impl ByteSource for Counting<'_> {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), ReadError> {
        self.read
            .fetch_add(buf.len() as u64, std::sync::atomic::Ordering::Relaxed);
        MemorySource::new(self.bytes).read_at(offset, buf)
    }
}

/// Spec § 7, "No amplification": one large provenance record that a hundred adjacencies
/// refer to is read once, not once for each.
#[test]
fn a_provenance_record_is_read_once_however_many_sections_refer_to_it() {
    let mut record = built_with(UNDIRECTED);
    record
        .parameters
        .extend((0..20_000).map(|i| (format!("key{i:05}"), "v".repeat(40))));
    let mut w = Writer::create(Mem::default()).unwrap();
    w.add_provenance("provenance/big", &record).unwrap();
    for i in 0..100 {
        add_adjacency(&mut w, &format!("a{i:03}"), &[vec![]], "provenance/big");
    }
    w.commit().unwrap();
    let file = w.into_sink().into_inner();
    assert!(file.len() > 1 << 20);
    let source = Counting {
        bytes: &file,
        read: 0.into(),
    };
    let report = verify(&source, Depth::Deep, &Memory::unlimited()).unwrap();
    assert!(
        report.is_intact() && report.unchecked.is_empty(),
        "{report:?}"
    );
    let read = source.read.into_inner();
    assert!(
        read < 3 * file.len() as u64,
        "{read} bytes read of a file of {}",
        file.len()
    );
}

/// From a source that is not mapped, the data is read a block at a time, each buffer
/// reserved against the budget; what does not fit is unchecked, never allocated.
#[test]
fn a_source_that_is_not_mapped_is_read_in_pieces_within_the_budget() {
    use std::sync::Arc;

    use everygraph_core::Budget;

    let lists: Vec<Vec<u32>> = (0..2000u32)
        .map(|v| (0..2000).filter(|&t| t != v && (t + v) % 7 == 0).collect())
        .collect();
    let file = graph(&lists, UNDIRECTED);
    let data = Writer::update(Mem::new(file.clone()))
        .unwrap()
        .get("adjacency.out.data")
        .unwrap()
        .len;
    let with = |limit: u64, source: &dyn Fn(&Memory) -> Report| {
        let budget = Arc::new(Budget::new(limit));
        let report = source(&Memory::from(Arc::clone(&budget)));
        (report, budget.peak())
    };
    let read = |memory: &Memory| verify(&NotMapped(&file), Depth::Deep, memory).unwrap();
    let mapped = |memory: &Memory| verify(&MemorySource::new(&file), Depth::Deep, memory).unwrap();
    let (report, peak) = with(data / 2, &read);
    assert!(
        report.is_intact() && report.unchecked.is_empty(),
        "{report:?}"
    );
    assert!(peak < data / 2, "peak {peak} of a {data}-byte section");
    let (report, _) = with(16 << 10, &read);
    assert!(report.is_intact());
    assert_eq!(
        report.unchecked,
        ["adjacency.out: its structure (not enough memory for its buffers)"]
    );
    let (report, _) = with(16 << 10, &mapped);
    assert!(
        report.is_intact() && report.unchecked.is_empty(),
        "{report:?}"
    );
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

/// Three commits; then the newest table of contents, or the newest header block, is
/// damaged. Either way the file reads as generation 2, whole, and generation 3 is named.
#[test]
fn a_file_that_reads_as_an_older_commit_names_the_lost_one() {
    let mut w = Writer::create(Mem::default()).unwrap();
    for name in ["a", "b", "c"] {
        w.add_section(name, Kind::PRIVATE, name.as_bytes(), None)
            .unwrap();
        w.commit().unwrap();
    }
    let file = w.into_sink().into_inner();
    let newest_block = 0; // generation 3: blocks alternate 0, 1, 0.
    let toc = file.len() - 4096;
    for at in [toc, newest_block * 4096 + 100] {
        let mut damaged = file.clone();
        damaged[at] ^= 1;
        let report = check(&damaged, Depth::Fast);
        assert!(report.is_intact(), "{:?}", report.problems);
        assert_eq!(
            (report.generation, report.lost_generation),
            (Some(2), Some(3))
        );
    }
    assert_eq!(check(&file, Depth::Fast).lost_generation, None);
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
        verify(&Failing, Depth::Fast, &Memory::unlimited()),
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

/// The index of `lists` and its data, written list by list, with flag bit 1 when
/// `events`.
fn streamed(lists: &[Vec<u32>], events: bool) -> (Vec<u8>, Vec<u8>) {
    let memory = Memory::unlimited();
    let mut index = IndexBuilder::new(&memory, u32::try_from(lists.len()).unwrap()).unwrap();
    let mut data = Vec::new();
    for (v, list) in lists.iter().enumerate() {
        let before = data.len();
        let degree = append_record(u32::try_from(v).unwrap(), list.iter().copied(), &mut data);
        index
            .push(degree.unwrap(), (data.len() - before) as u64)
            .unwrap();
    }
    (index.finish(events).unwrap().0, data)
}

/// An adjacency with repeated pairs: index flag bit 1 and a `.events` section, both or
/// neither (`docs/spec/eg-adjacency.md` § 3, § 5), and the section checked whole.
#[test]
fn an_events_section_is_found_exactly_where_the_index_announces_one() {
    let lists = triangle();
    let entries: u64 = lists.iter().map(|l| l.len() as u64).sum();
    let mut multiplicities = vec![1_u64; usize::try_from(entries).unwrap()];
    multiplicities[0] = 3;
    let events = encode_events(entries, entries + 2, multiplicities).unwrap();
    let with = |events_flag: bool, section: Option<&[u8]>| {
        let (index, data) = streamed(&lists, events_flag);
        let mut w = Writer::create(Mem::default()).unwrap();
        w.add_section("adjacency.out.index", Kind::ADJACENCY_INDEX, &index, None)
            .unwrap();
        w.add_section("adjacency.out.data", Kind::ADJACENCY_DATA, &data, None)
            .unwrap();
        if let Some(section) = section {
            w.add_section(
                "adjacency.out.events",
                Kind::ADJACENCY_EVENTS,
                section,
                None,
            )
            .unwrap();
        }
        w.commit().unwrap();
        w.into_sink().into_inner()
    };
    let first = |file: &[u8]| problems(file).remove(0).what;
    let sound = with(true, Some(&events));
    assert_eq!(problems(&sound), Vec::new());
    let report = check(&sound, Depth::Deep);
    assert_eq!(report.adjacencies[0].events, Some(entries + 2));
    assert_eq!(
        first(&with(false, Some(&events))),
        "an events section, but adjacency.out.index does not announce one"
    );
    assert_eq!(
        first(&with(true, None)),
        "it announces an events section, and adjacency.out.events is missing"
    );
    // A sequence with no set bit, behind a valid hash.
    let mut flat = events.clone();
    flat[32..].fill(0);
    assert!(!problems(&with(true, Some(&flat))).is_empty());
}
