// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The `everygraph-bench` command end to end: `fetch`, `list`, `verify` and `path` on the
//! generated datasets (no network), and one real download that is ignored by default.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let p =
            std::env::temp_dir().join(format!("everygraph-bench-cli-{}-{tag}", std::process::id()));
        let _ignored = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).expect("scratch dir");
        Self(p)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ignored = std::fs::remove_dir_all(&self.0);
    }
}

fn bench(data: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_everygraph-bench"))
        .arg("--data")
        .arg(data)
        .args(args)
        .env_remove("EVERYGRAPH_DATA")
        .output()
        .expect("everygraph-bench runs")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn status_of<'a>(listing: &'a str, name: &str) -> &'a str {
    listing
        .lines()
        .find(|l| l.split_whitespace().next() == Some(name))
        .unwrap_or_else(|| panic!("{name} not listed in:\n{listing}"))
}

#[test]
fn fetch_list_verify_and_path_work_on_a_generated_dataset() {
    let dir = Scratch::new("synth");
    let before = bench(&dir.0, &["list"]);
    assert!(before.status.success(), "{}", stderr(&before));
    assert!(status_of(&stdout(&before), "synth-grid-30x30").ends_with("  missing"));

    let fetched = bench(&dir.0, &["fetch", "synth-grid-30x30", "synth-barbell-10"]);
    assert!(fetched.status.success(), "{}", stderr(&fetched));

    let after = bench(&dir.0, &["list"]);
    assert!(status_of(&stdout(&after), "synth-grid-30x30").ends_with("  verified"));
    assert!(status_of(&stdout(&after), "synth-path-100").ends_with("  missing"));

    let verified = bench(&dir.0, &["verify", "synth-grid-30x30"]);
    assert!(verified.status.success(), "{}", stderr(&verified));

    let path = bench(&dir.0, &["path", "synth-grid-30x30"]);
    let printed = PathBuf::from(stdout(&path).trim());
    assert_eq!(
        printed,
        dir.0.join("synth-grid-30x30").join("grid-30x30.txt")
    );
    let text = std::fs::read_to_string(&printed).expect("the file is there");
    assert_eq!(text.lines().filter(|l| !l.starts_with('#')).count(), 1740);

    // Placed files are read-only.
    let meta = std::fs::metadata(&printed).expect("meta");
    assert!(meta.permissions().readonly());

    // A changed file fails `verify` and is no longer listed as verified.
    std::fs::remove_file(&printed).expect("remove");
    std::fs::write(&printed, "0\t1\n").expect("a different file in its place");
    let failed = bench(&dir.0, &["verify", "synth-grid-30x30"]);
    assert_eq!(failed.status.code(), Some(1));
    assert!(
        !status_of(&stdout(&bench(&dir.0, &["list"])), "synth-grid-30x30").ends_with("  verified")
    );

    // `fetch` says what to do about it, and `--replace` does it.
    let stuck = bench(&dir.0, &["fetch", "synth-grid-30x30"]);
    assert_eq!(stuck.status.code(), Some(1));
    assert!(
        stderr(&stuck).contains("and run fetch again"),
        "{}",
        stderr(&stuck)
    );
    let replaced = bench(&dir.0, &["fetch", "synth-grid-30x30", "--replace"]);
    assert!(replaced.status.success(), "{}", stderr(&replaced));
    assert!(
        status_of(&stdout(&bench(&dir.0, &["list"])), "synth-grid-30x30").ends_with("  verified")
    );
}

#[test]
fn verify_of_a_missing_dataset_fails() {
    let dir = Scratch::new("missing");
    let out = bench(&dir.0, &["verify", "synth-path-100"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stdout(&out).contains("missing"), "{}", stdout(&out));
}

#[test]
fn usage_errors_exit_with_status_2() {
    let dir = Scratch::new("usage");
    for args in [
        &[][..],
        &["frobnicate"][..],
        &["fetch"][..],
        &["fetch", "synth-path-100", "--bogus"][..],
        &["list", "--connections", "3"][..],
        &["fetch", "synth-path-100", "--connections", "0"][..],
        // An empty data root would mean the working directory.
        &["--data=", "list"][..],
        &["--data", "", "list"][..],
    ] {
        let out = bench(&dir.0, args);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {}", stderr(&out));
    }
    let unknown = bench(&dir.0, &["fetch", "no-such-dataset"]);
    assert_eq!(unknown.status.code(), Some(1));
}

#[test]
fn list_markdown_prints_the_generated_tables() {
    let dir = Scratch::new("markdown");
    let out = bench(&dir.0, &["list", "--markdown"]);
    assert!(out.status.success());
    let text = stdout(&out);
    assert!(text.starts_with("<!-- BEGIN GENERATED"));
    assert!(text.contains("| [`roadnet-ca`]"));
}

/// Downloads the smallest real dataset from SNAP. Needs the network; run with
/// `cargo test -p everygraph-bench --test cli -- --ignored`.
#[test]
#[ignore = "downloads ego-facebook (219 kB) from snap.stanford.edu"]
fn fetch_downloads_and_verifies_a_real_dataset() {
    let dir = Scratch::new("real");
    let out = bench(&dir.0, &["fetch", "ego-facebook"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stderr(&out).contains("verified, from https://snap.stanford.edu/"));
    let listing = stdout(&bench(&dir.0, &["list"]));
    assert!(status_of(&listing, "ego-facebook").ends_with("  verified"));
    let again = bench(&dir.0, &["fetch", "ego-facebook"]);
    assert!(
        stderr(&again).contains("already verified"),
        "{}",
        stderr(&again)
    );
}
