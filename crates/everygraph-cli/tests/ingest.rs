// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `everygraph ingest` as a user runs it: the file it makes, the same graph at any
//! thread count and budget, refusals before any work, and hard stops that leave no
//! damaged graph behind (`docs/ux-contract.md` P-1, P-5, P-16, C-1, C-9, § 6.4).

#![forbid(unsafe_code)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use everygraph_format::container::{Container, Kind};
use everygraph_io::FileSource;

mod common;
use common::schema;

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_everygraph"))
        .current_dir(dir)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

/// A fresh directory for one test.
fn dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("ingest")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Rows with loops, repeats in both directions and a few hubs.
fn edges(rows: u64) -> String {
    let mut text = String::new();
    let mut x = 0x9E37_79B9_7F4A_7C15_u64;
    for i in 0..rows {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let u = if i % 5 == 0 { x % 7 } else { x % 20_000 };
        let v = (x >> 24) % 20_000;
        text.push_str(&format!("{u} {v}\n"));
        if i % 17 == 0 {
            text.push_str(&format!("{v} {u}\n"));
        }
    }
    text
}

/// The sections that make the graph, by name, with their kind and hash: everything
/// but the provenance, which holds times.
fn graph_sections(path: &Path) -> Vec<(String, u32, [u8; 32])> {
    let file = Container::open(FileSource::open(path).unwrap()).unwrap();
    file.verify_all().unwrap();
    file.entries()
        .iter()
        .filter(|e| e.kind != Kind::PROVENANCE)
        .map(|e| (e.name.to_string(), e.kind.code(), e.hash))
        .collect()
}

/// Nothing but the input, and the graph when `graph` is given, is in `dir`.
fn only(dir: &Path, expected: &[&str]) {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    let mut expected: Vec<String> = expected.iter().map(|&s| s.to_owned()).collect();
    expected.sort();
    assert_eq!(names, expected);
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn ingest_writes_a_graph_and_says_so() {
    let dir = dir("writes");
    std::fs::write(dir.join("edges.txt"), edges(2_000)).unwrap();
    let out = run(&dir, &["ingest", "edges.txt"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(
        out.stdout.is_empty(),
        "the result goes to standard error (S-1)"
    );
    let text = stderr(&out);
    assert!(text.contains("ok edges.eg | "), "{text}");
    assert!(text.contains("next: everygraph map edges.eg"), "{text}");
    for phase in ["reading", "identifiers", "building", "checking"] {
        assert!(text.contains(&format!("  {phase} ")), "{text}");
    }
    assert!(text.lines().all(|l| l.chars().count() <= 80), "{text}");
    only(&dir, &["edges.eg", "edges.txt"]);
    assert!(!graph_sections(&dir.join("edges.eg")).is_empty());

    // With --json, standard output is one file document (S-2, G-20).
    let out = run(
        &dir,
        &["ingest", "edges.txt", "--json", "--out", "again.eg"],
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let document: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(document["operation"], "ingest");
    assert_eq!(document["made"]["path"], "again.eg");
    let counts = &document["counts"];
    let dropped = &document["dropped"];
    let rows = counts["rows"].as_u64().unwrap();
    let sum = counts["edges"].as_u64().unwrap()
        + dropped["self_loops"].as_u64().unwrap_or(0)
        + dropped["duplicates"].as_u64().unwrap_or(0)
        + counts["merged"].as_u64().unwrap();
    assert_eq!(rows, sum);
}

#[test]
fn the_graph_does_not_depend_on_threads_or_the_budget() {
    let dir = dir("threads");
    // More than three blocks of 4 MiB of text.
    let text = edges(1_400_000);
    assert!(text.len() > 3 * (4 << 20));
    std::fs::write(dir.join("edges.txt"), text).unwrap();
    // At this budget the plan, which spills less readily than the build, spills.
    let small = "50MB";
    let out = run(
        &dir,
        &[
            "ingest",
            "edges.txt",
            "--threads",
            "2",
            "--memory",
            small,
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let plan: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(plan["regime"]["name"], "on_disk", "{plan}");
    let mut seen = Vec::new();
    for (threads, memory) in [("1", "2GB"), ("3", "2GB"), ("8", "2GB"), ("2", small)] {
        let name = format!("t{threads}-{memory}.eg");
        let out = run(
            &dir,
            &[
                "ingest",
                "edges.txt",
                "--threads",
                threads,
                "--memory",
                memory,
                "--out",
                &name,
            ],
        );
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        seen.push(graph_sections(&dir.join(&name)));
    }
    assert!(seen.windows(2).all(|w| w[0] == w[1]));
}

#[test]
fn a_budget_that_cannot_hold_the_build_is_refused_before_any_work() {
    let dir = dir("refused");
    std::fs::write(dir.join("edges.txt"), edges(1_000)).unwrap();
    let out = run(&dir, &["ingest", "edges.txt", "--memory", "1MB", "--json"]);
    assert_eq!(out.status.code(), Some(4), "{}", stderr(&out));
    let text = stderr(&out);
    assert!(text.contains("Not enough memory for this step."), "{text}");
    assert!(text.trim_end().ends_with("Nothing was written."), "{text}");
    let document: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(document["error"], "InsufficientBudget");
    schema::check("error", &document).unwrap_or_else(|e| panic!("{e}: {document}"));
    assert_eq!(document["budget_bytes"], 1_000_000);
    only(&dir, &["edges.txt"]);
}

#[cfg(target_os = "linux")]
#[test]
fn a_budget_above_the_machine_is_a_usage_error() {
    let dir = dir("too-much");
    std::fs::write(dir.join("edges.txt"), edges(10)).unwrap();
    let out = run(&dir, &["ingest", "edges.txt", "--memory", "900TB"]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(stderr(&out).contains("more than this machine has"));
    only(&dir, &["edges.txt"]);
}

#[test]
fn invalid_rows_are_refused_with_their_lines() {
    let dir = dir("invalid");
    std::fs::write(dir.join("edges.txt"), "1 2\n3\n4 5\n").unwrap();
    let out = run(&dir, &["ingest", "edges.txt"]);
    assert_eq!(out.status.code(), Some(3), "{}", stderr(&out));
    let text = stderr(&out);
    assert!(
        text.contains("Cannot read edges.txt: 1 line is not a valid row."),
        "{text}"
    );
    assert!(text.contains("edges.txt:2"), "{text}");
    only(&dir, &["edges.txt"]);
}

#[test]
fn an_existing_file_is_replaced_only_when_asked() {
    let dir = dir("exists");
    std::fs::write(dir.join("edges.txt"), edges(100)).unwrap();
    std::fs::write(dir.join("edges.eg"), "mine").unwrap();
    let out = run(&dir, &["ingest", "edges.txt"]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(stderr(&out).contains("edges.eg exists and was not written by Everygraph."));
    assert_eq!(std::fs::read(dir.join("edges.eg")).unwrap(), b"mine");
    let out = run(&dir, &["ingest", "edges.txt", "--overwrite"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(!graph_sections(&dir.join("edges.eg")).is_empty());
}

#[test]
fn usage_errors_exit_with_two() {
    let dir = dir("usage");
    for args in [
        &["ingest"][..],
        &["ingest", "a.txt", "--duplicates", "some"],
        &["ingest", "a.txt", "--memroy", "8GB"],
        &["ingest", "a.txt", "--weight", "w"],
        &["frobnicate"],
    ] {
        let out = run(&dir, args);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {}", stderr(&out));
    }
}

#[test]
fn a_dry_run_prints_the_plan_and_writes_nothing() {
    let dir = dir("dry-run");
    std::fs::write(dir.join("edges.txt"), edges(100)).unwrap();
    let out = run(&dir, &["ingest", "edges.txt", "--dry-run"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let plan = String::from_utf8_lossy(&out.stdout);
    for line in [
        "  machine ",
        "  budget ",
        "  plan ",
        "  estimate ",
        "Dry run: nothing was written.",
    ] {
        assert!(plan.contains(line), "{plan}");
    }
    only(&dir, &["edges.txt"]);

    // With --json, standard output is the plan document (§ 3.6) and nothing else.
    let out = run(&dir, &["ingest", "edges.txt", "--dry-run", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let plan: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    schema::check("plan", &plan).unwrap_or_else(|e| panic!("{e}: {plan}"));
    let mut misspelt = plan.clone();
    misspelt["machine"]["memroy_bytes"] = 1.into();
    assert!(schema::check("plan", &misspelt).is_err());
    assert_eq!(plan["fits"], true);
    assert!(plan["refusal"].is_null());
    assert_eq!(plan["input"]["rows"]["value"], 100.0 + 6.0);
    assert_eq!(plan["input"]["rows"]["basis"], "exact");

    // A plan that does not fit is returned, with its refusal, and exits with 4.
    let out = run(
        &dir,
        &[
            "ingest",
            "edges.txt",
            "--dry-run",
            "--json",
            "--memory",
            "1MB",
        ],
    );
    assert_eq!(out.status.code(), Some(4), "{}", stderr(&out));
    let plan: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    schema::check("plan", &plan).unwrap_or_else(|e| panic!("{e}: {plan}"));
    assert_eq!(plan["fits"], false);
    assert_eq!(plan["refusal"]["error"], "InsufficientBudget");
    only(&dir, &["edges.txt"]);
}

#[test]
fn a_compressed_input_gives_the_same_graph() {
    let dir = dir("gzip");
    let text = edges(5_000);
    std::fs::write(dir.join("edges.txt"), &text).unwrap();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gz.write_all(text.as_bytes()).unwrap();
    std::fs::write(dir.join("packed.txt.gz"), gz.finish().unwrap()).unwrap();
    for input in ["edges.txt", "packed.txt.gz"] {
        let out = run(&dir, &["ingest", input]);
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    }
    assert_eq!(
        graph_sections(&dir.join("edges.eg")),
        graph_sections(&dir.join("packed.eg"))
    );
    only(
        &dir,
        &["edges.eg", "edges.txt", "packed.eg", "packed.txt.gz"],
    );
}

/// `kill -9` at drifting moments: `NAME.eg` is never there unless whole, and the next
/// run completes with the graph of an uninterrupted one (C-1).
#[cfg(unix)]
#[test]
fn a_killed_ingest_leaves_no_damaged_graph() {
    let dir = dir("killed");
    std::fs::write(dir.join("edges.txt"), edges(150_000)).unwrap();
    let out = run(&dir, &["ingest", "edges.txt", "--out", "reference.eg"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let reference = graph_sections(&dir.join("reference.eg"));
    let mut killed = 0;
    for step in 0..12_u64 {
        let _ = std::fs::remove_file(dir.join("edges.eg"));
        let mut child = Command::new(env!("CARGO_BIN_EXE_everygraph"))
            .current_dir(&dir)
            .args(["ingest", "edges.txt"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(15 * step * step));
        if child.try_wait().unwrap().is_none() {
            child.kill().unwrap();
            killed += 1;
        }
        child.wait().unwrap();
        if dir.join("edges.eg").exists() {
            assert_eq!(
                graph_sections(&dir.join("edges.eg")),
                reference,
                "step {step}"
            );
        }
        let out = run(&dir, &["ingest", "edges.txt", "--overwrite"]);
        assert_eq!(out.status.code(), Some(0), "step {step}: {}", stderr(&out));
        assert_eq!(
            graph_sections(&dir.join("edges.eg")),
            reference,
            "step {step}"
        );
        only(&dir, &["edges.eg", "edges.txt", "reference.eg"]);
    }
    assert!(killed > 0, "every run finished before it was killed");
}
