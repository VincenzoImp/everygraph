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

#[path = "support/bytes.rs"]
mod support;

use std::fs;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use everygraph_core::ByteSource;
use everygraph_format::container::{Container, Kind, Writer};
use everygraph_io::{FileSink, FileSource, MmapSource, persist};
use support::bytes;

/// Set in the child process of the kill test, to the file it writes.
const CHILD: &str = "EVERYGRAPH_TEST_KILL_CHILD";
/// The name of the kill test, which the child runs again in child mode.
const KILL_TEST: &str =
    "a_writer_killed_at_any_moment_leaves_a_valid_file_with_every_acknowledged_commit";

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("everygraph-io-container")
        .join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

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
    // While the file is mapped, this process cannot commit to it (the guard is Unix
    // only, `src/claim.rs`).
    if cfg!(unix) {
        let refused = FileSink::open(&path).unwrap_err();
        assert_eq!(refused.kind(), ErrorKind::ResourceBusy);
    }
    drop(mapped);
    FileSink::open(&path).unwrap();
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
