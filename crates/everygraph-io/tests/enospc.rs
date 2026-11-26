// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A real full disk (issue #190): a commit to a graph file on a 256 KiB `tmpfs` fails
//! with `ENOSPC`, and the file still reads as its last commit. The test mounts the
//! `tmpfs` inside an unprivileged user and mount namespace (`unshare`), so it needs no
//! root; where such namespaces are not allowed (GitHub's Ubuntu 24.04 runners restrict
//! them), it says so and passes. The same failure at every step is swept in memory by
//! `everygraph-format`'s `tests/faults.rs`.

#![forbid(unsafe_code)]
#![cfg(target_os = "linux")]

use std::io;
use std::path::Path;
use std::process::Command;

use everygraph_core::{Memory, WriteError};
use everygraph_format::container::{Container, ContainerError, Kind, Writer};
use everygraph_format::verify::{Depth, verify};
use everygraph_io::{FileSink, FileSource};

/// Set in the child process, to the directory where the `tmpfs` is mounted.
const CHILD: &str = "EVERYGRAPH_TEST_TMPFS";
const TEST: &str = "a_full_disk_fails_the_commit_and_keeps_the_last_one";
/// The exit status of the child when the mount is refused.
const NO_MOUNT: i32 = 77;

#[test]
fn a_full_disk_fails_the_commit_and_keeps_the_last_one() {
    if let Some(dir) = std::env::var_os(CHILD) {
        on_a_small_disk(Path::new(&dir));
        return;
    }
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("enospc-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let status = Command::new("unshare")
        .args(["--user", "--map-root-user", "--mount", "sh", "-c"])
        .arg(format!(
            r#"mount -t tmpfs -o size=256k none "$1" || exit {NO_MOUNT}; shift; exec "$@""#
        ))
        .arg("sh")
        .arg(&dir)
        .arg(std::env::current_exe().unwrap())
        .args(["--exact", TEST, "--nocapture"])
        .env(CHILD, &dir)
        .status();
    std::fs::remove_dir_all(&dir).unwrap();
    match status {
        Ok(s) if s.success() => {}
        // libtest's status for a failed test.
        Ok(s) if s.code() == Some(101) => panic!("the test failed on the small disk"),
        refused => {
            #[expect(clippy::print_stderr, reason = "a skipped test says why")]
            {
                eprintln!("skipped: no user namespace with a tmpfs here ({refused:?})");
            }
        }
    }
}

fn on_a_small_disk(dir: &Path) {
    let path = dir.join("graph.eg");
    let kind = Kind::PRIVATE;
    let mut w = Writer::create(FileSink::create(&path).unwrap()).unwrap();
    w.add_section("a", kind, b"the first commit", None).unwrap();
    w.commit().unwrap();
    drop(w);
    let mut w = Writer::update(FileSink::open(&path).unwrap()).unwrap();
    let big = vec![1u8; 1 << 20];
    let error = w
        .add_section("big", kind, &big, None)
        .and_then(|_| w.commit())
        .unwrap_err();
    let ContainerError::Write(WriteError::Backend(cause)) = &error else {
        panic!("not a write error: {error}");
    };
    let kind_of = cause.downcast_ref::<io::Error>().map(io::Error::kind);
    assert_eq!(kind_of, Some(io::ErrorKind::StorageFull), "{error}");
    assert!(matches!(w.commit(), Err(ContainerError::Poisoned)));
    drop(w);
    let source = FileSource::open(&path).unwrap();
    let file = Container::open(&source).unwrap();
    assert_eq!(file.generation(), 1);
    assert!(file.get("big").is_none());
    assert!(
        verify(&source, Depth::Deep, &Memory::unlimited())
            .unwrap()
            .is_intact()
    );
}
