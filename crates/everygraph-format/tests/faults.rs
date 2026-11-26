// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Error paths of the container, swept (issue #190): the same work is run once for
//! every operation `k` of a clean run, with operation `k` failing — a write or a length
//! change with `ENOSPC`, a read with `EIO` or a short read, a sync with `EIO` that leaves
//! the new bytes (ext4, XFS) or the old ones (btrfs).
//!
//! For each `k`: the work ends in a typed error, never in success (an error dropped on
//! the way is the bug class of Gunawi et al., FAST'08); the file, as the process sees it
//! and as a crash would leave it, reads as the last commit that returned or as the one
//! that failed, whole; and a new writer can go on from it. A reader that fails returns
//! an error, or the true answer, or says that it fell back. Each test prints how many
//! distinct error paths it reached.
//!
//! Not run under Miri: each test runs the work hundreds of times.

#![forbid(unsafe_code)]
#![cfg(not(miri))]

mod support;

use std::collections::{BTreeMap, BTreeSet};

use everygraph_core::{ByteSink, ByteSource, Memory, MemorySource, ReadError};
use everygraph_format::container::{Container, ContainerError, Kind, Provenance, Writer, compact};
use everygraph_format::verify::{Depth, verify};
use everygraph_io::faults::{AfterFailedSync, Faulty, Op, Plan};
use support::bytes;

type Mem = MemorySource<Vec<u8>>;
/// The sections of a commit, by name.
type State = BTreeMap<String, Vec<u8>>;

const PRIVATE: Kind = Kind::PRIVATE;

fn record() -> Provenance {
    Provenance {
        engine: "everygraph test".into(),
        operation: "ingest".into(),
        ..Provenance::default()
    }
}

fn state<S: ByteSource>(file: &Container<S>) -> State {
    file.entries()
        .iter()
        .filter(|e| e.kind != Kind::PROVENANCE)
        .map(|e| {
            let mut buf = vec![0; usize::try_from(e.len).unwrap()];
            file.section(&e.name).unwrap().read_at(0, &mut buf).unwrap();
            (e.name.to_string(), buf)
        })
        .collect()
}

/// The state `bytes` read as, `None` for a file that is not a container yet; and that
/// it is intact (a header block left torn by the failure is a note, not a problem).
fn reads_as(bytes: &[u8]) -> Option<State> {
    let source = MemorySource::new(bytes);
    let file = match Container::open(&source) {
        Ok(file) => file,
        Err(ContainerError::NotAContainer | ContainerError::NotCommitted) => return None,
        Err(e) => panic!("a file left by a failure does not open: {e}"),
    };
    let report = verify(&source, Depth::Deep, &Memory::unlimited()).unwrap();
    assert!(report.is_intact(), "{:?}", report.problems);
    Some(state(&file))
}

type Failed = (&'static str, ContainerError);

/// Runs `steps` on `w`. After a failure the writer must refuse to go on (it is
/// poisoned): a further commit writes nothing and says so.
fn on<S: ByteSink>(
    w: &mut Writer<S>,
    steps: impl FnOnce(&mut Writer<S>) -> Result<(), Failed>,
) -> Result<(), Failed> {
    let outcome = steps(w);
    if outcome.is_err() {
        assert!(
            matches!(w.commit(), Err(ContainerError::Poisoned)),
            "a writer went on after a failure"
        );
    }
    outcome
}

/// Two generations by two writers: create and commit, then update, remove, add and
/// commit. Each commit that returns pushes the state it made.
fn work(sink: &mut Faulty<Mem>, done: &mut Vec<State>) -> Result<(), Failed> {
    let mut w = Writer::create(&mut *sink).map_err(|e| ("create", e))?;
    on(&mut w, |w| {
        w.add_provenance("provenance/ingest", &record())
            .map_err(|e| ("add_provenance", e))?;
        w.add_section("a", PRIVATE, &bytes(1, 5000), Some("provenance/ingest"))
            .map_err(|e| ("add_section", e))?;
        let mut big = w
            .begin_section("b", PRIVATE, None)
            .map_err(|e| ("begin_section", e))?;
        for piece in 0..3 {
            big.write(&bytes(piece, 3000)).map_err(|e| ("write", e))?;
        }
        big.finish().map_err(|e| ("finish", e))?;
        w.commit().map(drop).map_err(|e| ("commit", e))
    })?;
    done.push(State::from([
        ("a".into(), bytes(1, 5000)),
        (
            "b".into(),
            [bytes(0, 3000), bytes(1, 3000), bytes(2, 3000)].concat(),
        ),
    ]));
    drop(w);
    let mut w = Writer::update(&mut *sink).map_err(|e| ("update", e))?;
    on(&mut w, |w| {
        w.remove("b").map_err(|e| ("remove", e))?;
        w.add_section("c", PRIVATE, &bytes(3, 100), None)
            .map_err(|e| ("add_section", e))?;
        w.commit().map(drop).map_err(|e| ("commit", e))
    })?;
    done.push(State::from([
        ("a".into(), bytes(1, 5000)),
        ("c".into(), bytes(3, 100)),
    ]));
    Ok(())
}

/// What a commit in progress at a failure would have made.
fn next_state(done: &[State]) -> State {
    let mut all = Vec::new();
    work(&mut Faulty::new(Mem::default(), Plan::NONE), &mut all).unwrap();
    all[done.len()].clone()
}

#[expect(
    clippy::print_stderr,
    clippy::disallowed_macros,
    reason = "the count of error paths is the test's report"
)]
fn report(name: &str, sweeps: usize, paths: &BTreeSet<String>) {
    eprintln!(
        "{name}: {sweeps} runs, {} distinct error paths: {paths:?}",
        paths.len()
    );
}

#[test]
fn every_failing_write_sync_or_read_of_a_writer_is_reported_and_leaves_a_whole_file() {
    let mut clean = Faulty::new(Mem::default(), Plan::NONE);
    work(&mut clean, &mut Vec::new()).unwrap();
    let ops = clean.ops();
    let mut paths = BTreeSet::new();
    for after_failed_sync in [AfterFailedSync::KeepNew, AfterFailedSync::Revert] {
        for k in 0..ops {
            let plan = Plan {
                after_failed_sync,
                ..Plan::at(k)
            };
            let mut sink = Faulty::new(Mem::default(), plan);
            let mut done = Vec::new();
            let outcome = work(&mut sink, &mut done);
            let op = sink.failed().expect("operation k was reached");
            let Err((call, error)) = outcome else {
                // Opening reads each header block again after a failed read of both
                // (spec § 7), so one failed read may be absorbed; nothing else may.
                assert_eq!(op, Op::Read, "operation {k} failed and the work succeeded");
                let file = sink.into_inner().into_inner();
                assert_eq!(reads_as(&file), done.last().cloned(), "k = {k}");
                paths.insert("Read absorbed".to_owned());
                continue;
            };
            assert!(
                matches!(error, ContainerError::Write(_) | ContainerError::Read(_)),
                "k = {k}: {call} returned {error}"
            );
            assert_eq!(sink.ops(), k + 1, "k = {k}: operations after the failure");
            paths.insert(format!("{op:?} in {call}"));
            let last = done.last().cloned();
            let allowed = [last.clone(), Some(next_state(&done))];
            let durable = reads_as(sink.durable());
            assert!(
                allowed.contains(&durable),
                "k = {k}: a crash now leaves {durable:?}"
            );
            let bytes = sink.into_inner().into_inner();
            let seen = reads_as(&bytes);
            match (after_failed_sync, op) {
                // A sync that failed and reverted leaves exactly the last commit.
                (AfterFailedSync::Revert, Op::Sync) => assert_eq!(seen, last, "k = {k}"),
                _ => assert!(
                    allowed.contains(&seen),
                    "k = {k}: the file reads as {seen:?}"
                ),
            }
            // A new writer goes on from what is there.
            let mut again = MemorySource::new(bytes);
            let mut w = match seen {
                None => Writer::create(&mut again).unwrap(),
                Some(_) => Writer::update(&mut again).unwrap(),
            };
            w.add_section("d", PRIVATE, b"after", None).unwrap();
            w.commit().unwrap();
            let mut expected = seen.unwrap_or_default();
            expected.insert("d".into(), b"after".to_vec());
            assert_eq!(
                reads_as(again.as_slice().unwrap()),
                Some(expected),
                "k = {k}"
            );
        }
    }
    report("writer", 2 * usize::try_from(ops).unwrap(), &paths);
    assert!(
        paths.contains("Sync in commit"),
        "a failed sync was never reached"
    );
}

/// The committed file of [`work`].
fn finished() -> Vec<u8> {
    let mut sink = Faulty::new(Mem::default(), Plan::NONE);
    work(&mut sink, &mut Vec::new()).unwrap();
    sink.into_inner().into_inner()
}

#[test]
fn a_compaction_that_fails_leaves_no_file_or_the_whole_copy() {
    let original = finished();
    let from = Container::open(MemorySource::new(original.as_slice())).unwrap();
    let mut clean = Faulty::new(Mem::default(), Plan::NONE);
    compact(&from, &mut clean).unwrap();
    let ops = clean.ops();
    let mut paths = BTreeSet::new();
    for k in 0..ops {
        let mut sink = Faulty::new(Mem::default(), Plan::at(k));
        let Err(error) = compact(&from, &mut sink) else {
            panic!("operation {k} failed and compact succeeded");
        };
        assert!(
            matches!(error, ContainerError::Write(_)),
            "k = {k}: {error}"
        );
        paths.insert(format!("{:?}", sink.failed().unwrap()));
        let copy = reads_as(&sink.into_inner().into_inner());
        assert!(
            copy.is_none() || copy == Some(state(&from)),
            "k = {k}: {copy:?}"
        );
    }
    report("compact", usize::try_from(ops).unwrap(), &paths);
}

/// What a reader found in [`finished`].
#[derive(Debug, PartialEq, Eq)]
struct Answer {
    generation: u64,
    sections: State,
    /// The container opened an older commit, and said so.
    fell_back: bool,
    /// What a deep `verify` found: a newer commit it could not read, and problems.
    lost: Option<u64>,
    problems: Vec<String>,
}

/// Everything a reader can be asked about [`finished`]: each section, a provenance
/// record, and a deep `verify`.
fn read(source: &Faulty<Mem>) -> Result<Answer, String> {
    let file = Container::open(source).map_err(|e| format!("open: {e}"))?;
    let mut sections = State::new();
    for e in file.entries().iter().filter(|e| e.kind != Kind::PROVENANCE) {
        let mut buf = vec![0; usize::try_from(e.len).unwrap()];
        file.section(&e.name)
            .unwrap()
            .read_at(0, &mut buf)
            .map_err(|e| format!("section: {e}"))?;
        sections.insert(e.name.to_string(), buf);
    }
    file.provenance("a")
        .map_err(|e| format!("provenance: {e}"))?;
    let report = verify(source, Depth::Deep, &Memory::unlimited())
        .map_err(|e: ReadError| format!("verify: {e}"))?;
    Ok(Answer {
        generation: file.generation(),
        sections,
        fell_back: file.recovered_from().is_some() || file.unreadable_block().is_some(),
        lost: report.lost_generation,
        problems: report.problems.into_iter().map(|p| p.what).collect(),
    })
}

#[test]
fn a_failing_read_is_an_error_a_reported_fall_back_or_the_true_answer() {
    let file = finished();
    let clean = Faulty::new(Mem::new(file.clone()), Plan::NONE);
    let truth = read(&clean).unwrap();
    assert!(!truth.fell_back && truth.lost.is_none() && truth.problems.is_empty());
    let mut states = Vec::new();
    work(&mut Faulty::new(Mem::default(), Plan::NONE), &mut states).unwrap();
    let older = &states[0];
    let ops = clean.ops();
    let mut paths = BTreeSet::new();
    for short_read in [false, true] {
        for k in 0..ops {
            let plan = Plan {
                short_read,
                ..Plan::at(k)
            };
            let path = match read(&Faulty::new(Mem::new(file.clone()), plan)) {
                Err(e) => e.split(':').next().unwrap().to_owned(),
                Ok(answer) if answer.fell_back => {
                    assert_eq!(answer.generation, 1, "k = {k}");
                    assert_eq!(&answer.sections, older, "k = {k}");
                    "open fell back".to_owned()
                }
                Ok(answer) if answer.lost.is_some() || !answer.problems.is_empty() => {
                    assert!(
                        answer.problems.iter().all(|p| p.contains("cannot be read")),
                        "k = {k}: {:?}",
                        answer.problems
                    );
                    "verify fell back".to_owned()
                }
                Ok(answer) => {
                    assert_eq!(
                        answer, truth,
                        "k = {k}: a read failed and the answer changed"
                    );
                    "absorbed".to_owned()
                }
            };
            paths.insert(path);
        }
    }
    report("reader", 2 * usize::try_from(ops).unwrap(), &paths);
}
