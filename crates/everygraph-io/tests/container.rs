// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The `.eg` container on real files: written through [`FileSink`] under a temporary
//! name and [`persist`]ed, read back through every native source, and a writer killed
//! with `SIGKILL` at arbitrary moments of its commits (`docs/spec/eg-container.md` § 8).
//!
//! The kill test is evidence about **process death only**: the page cache survives
//! `SIGKILL`, so every write the process made is on the file afterwards. What a power
//! cut can leave (lost, reordered, torn writes) is tested step by step in
//! `everygraph-format`'s `tests/container.rs`.

#![forbid(unsafe_code)]

#[path = "support/scratch.rs"]
mod scratch_dir;
#[path = "support/bytes.rs"]
mod support;

use std::fs;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use everygraph_core::{ByteSource, WriteError};
use everygraph_format::container::{
    Container, ContainerError, Kind, Provenance, ResultKey, Writer,
};
use everygraph_io::{FileSink, FileSource, MmapSource, persist};
use scratch_dir::scratch;
use support::bytes;

/// Set in the child process of the kill test, to the file it writes.
const CHILD: &str = "EVERYGRAPH_TEST_KILL_CHILD";
/// The name of the kill test, which the child runs again in child mode.
const KILL_TEST: &str =
    "a_writer_killed_at_any_moment_leaves_a_valid_file_with_every_acknowledged_commit";

/// The length of section `s{i}` in the kill test: up to 300 KB, so that a kill lands
/// in section writes as well as in syncs.
fn len_of(i: u64) -> usize {
    usize::try_from(i * 7919 % 300_000).unwrap() + 1
}

fn kind() -> Kind {
    Kind::new(0x8000_0000).unwrap()
}

/// Writes a new graph file at `path` with one section, `base`: under a temporary name,
/// committed, then persisted.
fn create_graph(path: &Path) {
    let partial = path.with_extension("eg.partial");
    let mut w = Writer::create(FileSink::create(&partial).unwrap()).unwrap();
    w.add_section("base", kind(), &bytes(0, 10_000), None)
        .unwrap();
    w.commit().unwrap();
    drop(w);
    persist(&partial, path).unwrap();
}

/// Checks that `path` holds a valid container whose sections are `base` and `s1 …
/// s{g-1}` with the contents the writer gave them; returns the generation.
fn check_file(path: &Path) -> u64 {
    let file = Container::open(FileSource::open(path).unwrap()).unwrap();
    assert_eq!(file.recovered_from(), None);
    file.verify_all().unwrap();
    let generation = file.generation();
    assert_eq!(file.entries().len(), usize::try_from(generation).unwrap());
    let read = |name: &str| {
        let section = file.section(name).unwrap();
        let mut buf = vec![0; usize::try_from(section.len()).unwrap()];
        section.read_at(0, &mut buf).unwrap();
        buf
    };
    assert_eq!(read("base"), bytes(0, 10_000));
    for i in 1..generation {
        assert_eq!(read(&format!("s{i}")), bytes(i, len_of(i)), "section s{i}");
    }
    generation
}

#[test]
fn a_file_sink_writes_in_place_extends_with_zeros_and_truncates() {
    use everygraph_core::ByteSink;

    let dir = scratch("sink");
    let path = dir.join("f");
    let mut sink = FileSink::create(&path).unwrap();
    sink.write_at(5, b"abc").unwrap();
    assert_eq!(sink.len(), 8);
    sink.write_at(10, b"").unwrap();
    assert_eq!(sink.len(), 10);
    sink.write_at(0, b"xy").unwrap();
    let mut buf = [9; 10];
    sink.read_at(0, &mut buf).unwrap();
    assert_eq!(&buf, b"xy\0\0\0abc\0\0");
    assert!(sink.read_at(5, &mut [0; 6]).is_err());
    sink.set_len(6).unwrap();
    sink.sync().unwrap();
    drop(sink);
    assert_eq!(fs::read(&path).unwrap(), b"xy\0\0\0a");
    // Opening keeps the bytes.
    let sink = FileSink::open(&path).unwrap();
    assert_eq!(sink.len(), 6);
    drop(sink.into_file());
    assert_eq!(fs::read(&path).unwrap(), b"xy\0\0\0a");
    fs::remove_dir_all(dir).unwrap();
}

// The in-process guard identifies files by device and inode: Unix only (`src/claim.rs`).
#[cfg(unix)]
#[test]
fn a_file_sink_holds_its_file_alone_in_the_process() {
    let dir = scratch("alone");
    let path = dir.join("f");
    let link = dir.join("hard-link");
    let sink = FileSink::create(&path).unwrap();
    fs::hard_link(&path, &link).unwrap();
    for target in [&path, &link] {
        assert_eq!(
            FileSink::open(target).unwrap_err().kind(),
            ErrorKind::ResourceBusy
        );
        assert_eq!(
            FileSink::create(target).unwrap_err().kind(),
            ErrorKind::ResourceBusy
        );
        assert_eq!(
            everygraph_io::AppendSink::create(target)
                .unwrap_err()
                .kind(),
            ErrorKind::ResourceBusy
        );
    }
    drop(sink);
    // And the other way round: no FileSink on a file another sink writes.
    let append = everygraph_io::AppendSink::create(&path).unwrap();
    assert_eq!(
        FileSink::open(&link).unwrap_err().kind(),
        ErrorKind::ResourceBusy
    );
    drop(append);
    FileSink::open(&link).unwrap();
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn persist_replaces_the_destination_whole() {
    let dir = scratch("persist");
    let (partial, destination) = (dir.join("g.eg.partial"), dir.join("g.eg"));
    fs::write(&destination, b"old").unwrap();
    fs::write(&partial, b"new graph").unwrap();
    persist(&partial, &destination).unwrap();
    assert_eq!(fs::read(&destination).unwrap(), b"new graph");
    assert!(!partial.exists());
    // A missing source changes nothing.
    assert!(persist(&partial, &destination).is_err());
    assert_eq!(fs::read(&destination).unwrap(), b"new graph");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_container_written_to_a_file_reads_back_through_every_source() {
    let dir = scratch("round-trip");
    let path = dir.join("g.eg");
    create_graph(&path);
    // An update through a new sink on the same file.
    let mut w = Writer::update(FileSink::open(&path).unwrap()).unwrap();
    w.add_section("s1", kind(), &bytes(1, len_of(1)), None)
        .unwrap();
    w.commit().unwrap();
    drop(w);
    assert_eq!(check_file(&path), 2);
    let mapped = Container::open(MmapSource::open(&path).unwrap()).unwrap();
    mapped.verify_all().unwrap();
    let section = mapped.section("base").unwrap();
    assert_eq!(section.as_slice().unwrap(), &bytes(0, 10_000)[..]);
    // While the whole file is mapped, this process cannot commit to it: the header
    // write is refused, and the file reads as before (the guard is Unix only,
    // `src/claim.rs`).
    if cfg!(unix) {
        let mut w = Writer::update(FileSink::open(&path).unwrap()).unwrap();
        w.add_section("s2", kind(), &bytes(2, len_of(2)), None)
            .unwrap();
        match w.commit() {
            Err(ContainerError::Write(WriteError::Backend(e))) => assert_eq!(
                e.downcast_ref::<std::io::Error>().unwrap().kind(),
                ErrorKind::ResourceBusy
            ),
            other => panic!("the commit beside a whole-file map: {other:?}"),
        }
        drop(w);
        assert_eq!(check_file(&path), 2);
    }
    drop(mapped);
    fs::remove_dir_all(dir).unwrap();
}

/// Maps the committed sections of the graph file at `path`, as a reader should
/// (`MmapSource`, spec § 11).
fn map_by_range(path: &Path) -> Container<MmapSource> {
    let file = fs::File::open(path).unwrap();
    let probe = Container::open(FileSource::from_file(file.try_clone().unwrap()).unwrap()).unwrap();
    let map = MmapSource::from_file_range(&file, probe.stable_range()).unwrap();
    probe.with_source(map).unwrap()
}

/// Commits one section `s{i}` to the graph at `path`.
fn commit_one(path: &Path, i: u64) {
    let mut w = Writer::update(FileSink::open(path).unwrap()).unwrap();
    w.add_section(&format!("s{i}"), kind(), &bytes(i, len_of(i)), None)
        .unwrap();
    w.commit().unwrap();
}

/// The recipe of spec § 11 reads the commit it probed, whatever is committed between
/// the probe and the map: one commit could make the second header read fall back
/// (the file then looks damaged), two could put the table past the map (review of
/// PR #216).
#[test]
fn a_range_map_reads_the_commit_it_probed_even_after_more_commits() {
    for between in [1, 2] {
        let dir = scratch(&format!("probe-{between}"));
        let path = dir.join("g.eg");
        create_graph(&path);
        let file = fs::File::open(&path).unwrap();
        let probe =
            Container::open(FileSource::from_file(file.try_clone().unwrap()).unwrap()).unwrap();
        let map = MmapSource::from_file_range(&file, probe.stable_range()).unwrap();
        for i in 1..=between {
            commit_one(&path, i);
        }
        let graph = probe.with_source(map).unwrap();
        assert_eq!(graph.generation(), 1);
        assert_eq!(graph.recovered_from(), None);
        graph.verify_all().unwrap();
        let base = graph.section("base").unwrap();
        assert!(base.as_slice().expect("mapped") == bytes(0, 10_000));
        drop(graph);
        assert_eq!(check_file(&path), 1 + between);
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn a_process_commits_a_result_to_a_graph_it_holds_mapped() {
    let dir = scratch("mapped-commit");
    let path = dir.join("g.eg");
    create_graph(&path);
    let graph = map_by_range(&path);
    let before = graph.source().mapped().to_vec();
    // The result: the sum of each 1,000 bytes of `base`, read through the map.
    let base = graph.section("base").unwrap();
    let sums: Vec<u64> = base
        .as_slice()
        .expect("mapped")
        .chunks(1000)
        .map(|c| c.iter().map(|&b| u64::from(b)).sum())
        .collect();
    let key = ResultKey::new(graph.graph_identity(), "block-sums", 1, true)
        .unwrap()
        .with("block", "1000")
        .unwrap();
    let about = Provenance {
        engine: "test".into(),
        ..Provenance::default()
    };
    let mut w = Writer::update(FileSink::open(&path).unwrap()).unwrap();
    w.add_result(&key, &about, &sums).unwrap();
    w.commit().unwrap();
    drop(w);
    assert!(
        graph.source().mapped() == before,
        "the mapped bytes changed"
    );
    assert_eq!(graph.generation(), 1);
    let again = map_by_range(&path);
    assert_eq!(again.generation(), 2);
    let stored = again.find_result(&key).unwrap().expect("the stored result");
    let mut got = vec![0_u64; usize::try_from(stored.len()).unwrap()];
    stored.read(0, &mut got).unwrap();
    assert_eq!(got, sums);
    drop(graph);
    drop(again);
    fs::remove_dir_all(dir).unwrap();
}

/// Waits for a line that ends with `want` from a child (the test harness may have
/// started the line).
fn wait_for(lines: &mut impl Iterator<Item = std::io::Result<String>>, want: &str) {
    while !lines
        .next()
        .expect("the child ended early")
        .unwrap()
        .ends_with(want)
    {}
}

/// Runs test `name` again in a child process, with `var` set to `path`.
fn spawn_child(name: &str, var: &str, path: &Path) -> Child {
    Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture", "--test-threads=1"])
        .env(var, path)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap()
}

/// Set in the child of the lock test, to the file it holds.
const HOLD: &str = "EVERYGRAPH_TEST_HOLD_CHILD";
const LOCK_TEST: &str = "a_second_writer_is_refused_while_the_first_lives_and_not_after_kill_9";

#[test]
fn a_second_writer_is_refused_while_the_first_lives_and_not_after_kill_9() {
    if let Some(path) = std::env::var_os(HOLD) {
        let _sink = FileSink::open(Path::new(&path)).unwrap();
        let mut out = std::io::stdout().lock();
        writeln!(out, "holding").unwrap();
        out.flush().unwrap();
        // Killed by the parent long before.
        std::thread::sleep(Duration::from_secs(60));
        return;
    }
    let dir = scratch("lock");
    let path = dir.join("g.eg");
    create_graph(&path);
    let mut child = spawn_child(LOCK_TEST, HOLD, &path);
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    wait_for(&mut lines, "holding");
    let refused = FileSink::open(&path).unwrap_err();
    assert_eq!(refused.kind(), ErrorKind::ResourceBusy);
    let message = refused.to_string();
    assert!(message.contains("g.eg"), "{message}");
    // Linux names the holder, unless the file system's device in `/proc/locks` is not
    // the file's (btrfs, overlayfs).
    assert!(
        message.contains(&format!("process {} is writing", child.id()))
            || message.contains("another process is writing")
            || message.contains("another program has this file open"),
        "{message}"
    );
    let refused = FileSink::create(&path).unwrap_err();
    assert_eq!(refused.kind(), ErrorKind::ResourceBusy);
    // Nothing was truncated, and readers are not stopped.
    assert_eq!(check_file(&path), 1);
    // `Child::kill` is SIGKILL on Unix, sent to this child's PID.
    child.kill().unwrap();
    child.wait().unwrap();
    let mut w = Writer::update(FileSink::open(&path).unwrap()).unwrap();
    w.add_section("s1", kind(), &bytes(1, len_of(1)), None)
        .unwrap();
    w.commit().unwrap();
    drop(w);
    assert_eq!(check_file(&path), 2);
    fs::remove_dir_all(dir).unwrap();
}

/// Set in the child of the range-map test, to the file it commits to.
const COMMIT: &str = "EVERYGRAPH_TEST_COMMIT_CHILD";
const RANGE_TEST: &str = "another_process_commits_100_times_to_a_file_this_one_maps_by_range";

/// The child of the range-map test: 100 commits, each through a sink of its own; every
/// tenth is preceded by a section that is never committed, so that the next commit
/// cuts off a tail.
fn child_committer(path: &Path) {
    for i in 1..=100 {
        let mut w = Writer::update(FileSink::open(path).unwrap()).unwrap();
        if i % 10 == 0 {
            w.add_section("abandoned", kind(), &bytes(999, 50_000), None)
                .unwrap();
            drop(w);
            w = Writer::update(FileSink::open(path).unwrap()).unwrap();
        }
        w.add_section(&format!("s{i}"), kind(), &bytes(i, len_of(i)), None)
            .unwrap();
        w.commit().unwrap();
    }
}

#[test]
fn another_process_commits_100_times_to_a_file_this_one_maps_by_range() {
    if let Some(path) = std::env::var_os(COMMIT) {
        child_committer(Path::new(&path));
        return;
    }
    let dir = scratch("range-map");
    let path = dir.join("g.eg");
    create_graph(&path);
    let graph = map_by_range(&path);
    let map = graph.source();
    let before = map.mapped().to_vec();
    let base = graph.section("base").unwrap();
    let mut child = spawn_child(RANGE_TEST, COMMIT, &path);
    let mut reads = 0;
    while child.try_wait().unwrap().is_none() {
        // A SIGBUS here would end the test process.
        assert!(map.mapped() == before, "the mapped bytes changed");
        assert!(base.as_slice().expect("mapped") == bytes(0, 10_000));
        reads += 1;
    }
    assert!(child.wait().unwrap().success());
    assert!(map.mapped() == before, "the mapped bytes changed");
    assert!(reads > 0);
    drop(graph);
    assert_eq!(check_file(&path), 101);
    fs::remove_dir_all(dir).unwrap();
}

/// The child of the kill test: commits one section after another to `path`, printing
/// each generation once its commit has returned, until it is killed.
fn child_writer(path: &Path) {
    let mut w = Writer::update(FileSink::open(path).unwrap()).unwrap();
    let mut out = std::io::stdout().lock();
    for i in w.generation()..100_000 {
        let data = bytes(i, len_of(i));
        let mut section = w.begin_section(&format!("s{i}"), kind(), None).unwrap();
        for piece in data.chunks(4096) {
            section.write(piece).unwrap();
        }
        section.finish().unwrap();
        let generation = w.commit().unwrap();
        writeln!(out, "committed {generation}").unwrap();
        out.flush().unwrap();
    }
}

#[test]
fn a_writer_killed_at_any_moment_leaves_a_valid_file_with_every_acknowledged_commit() {
    // Run again by itself with `CHILD` set, this test is the writer that gets killed.
    if let Some(path) = std::env::var_os(CHILD) {
        child_writer(Path::new(&path));
        return;
    }
    let dir = scratch("kill");
    let rounds = 20;
    for round in 0..rounds {
        let path = dir.join(format!("g{round}.eg"));
        create_graph(&path);

        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", KILL_TEST, "--nocapture", "--test-threads=1"])
            .env(CHILD, &path)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        // Wait for a few acknowledged commits, then kill at a moment that drifts
        // through the steps of the next ones.
        let wait_for = 2 + round % 5;
        let mut acknowledged = 1;
        while acknowledged < wait_for {
            let line = lines.next().expect("the child ended early").unwrap();
            if let Some(g) = line.strip_prefix("committed ") {
                acknowledged = g.parse().unwrap();
            }
        }
        std::thread::sleep(Duration::from_micros(round * 613 % 6000));
        // `Child::kill` is SIGKILL on Unix, sent to this child's PID.
        child.kill().unwrap();
        child.wait().unwrap();
        for line in lines.map_while(Result::ok) {
            if let Some(g) = line.strip_prefix("committed ") {
                acknowledged = g.parse().unwrap();
            }
        }

        let generation = check_file(&path);
        assert!(
            generation >= acknowledged,
            "round {round}: commit {acknowledged} was acknowledged, the file holds {generation}"
        );
        assert!(generation <= acknowledged + 1, "round {round}");
        // The file can be written again after the kill.
        let mut w = Writer::update(FileSink::open(&path).unwrap()).unwrap();
        let i = w.generation();
        w.add_section(&format!("s{i}"), kind(), &bytes(i, len_of(i)), None)
            .unwrap();
        w.commit().unwrap();
        drop(w);
        assert_eq!(check_file(&path), generation + 1);
    }
    fs::remove_dir_all(dir).unwrap();
}
