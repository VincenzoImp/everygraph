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
    plan_schema::error(&document, "error").unwrap_or_else(|e| panic!("{e}: {document}"));
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
    plan_schema::plan(&plan).unwrap_or_else(|e| panic!("{e}: {plan}"));
    let mut misspelt = plan.clone();
    misspelt["machine"]["memroy_bytes"] = 1.into();
    assert!(plan_schema::plan(&misspelt).is_err());
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
    plan_schema::plan(&plan).unwrap_or_else(|e| panic!("{e}: {plan}"));
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

/// The keys and enumerations of `urn:everygraph:schema:plan:1` and of the error
/// document it refers to (`docs/ux-contract.md` § 3.6, § 6.1), checked as closed:
/// a missing required key, an unknown key or a value outside an enumeration is an
/// error naming its path.
mod plan_schema {
    use serde_json::{Map, Value};

    type Check = Result<(), String>;

    fn object<'v>(
        v: &'v Value,
        at: &str,
        required: &[&str],
        optional: &[&str],
    ) -> Result<&'v Map<String, Value>, String> {
        let map = v.as_object().ok_or(format!("{at}: not an object"))?;
        for key in required {
            if !map.contains_key(*key) {
                return Err(format!("{at}: {key} missing"));
            }
        }
        for key in map.keys() {
            if !required.contains(&key.as_str()) && !optional.contains(&key.as_str()) {
                return Err(format!("{at}: unknown key {key}"));
            }
        }
        Ok(map)
    }

    fn one_of(v: &Value, at: &str, values: &[&str]) -> Check {
        match v.as_str() {
            Some(s) if values.contains(&s) => Ok(()),
            _ => Err(format!("{at}: {v} is not one of {values:?}")),
        }
    }

    fn count(v: &Value, at: &str) -> Check {
        v.as_u64()
            .map(|_| ())
            .ok_or(format!("{at}: {v} is not a count"))
    }

    fn string(v: &Value, at: &str) -> Check {
        v.as_str()
            .map(|_| ())
            .ok_or(format!("{at}: {v} is not a string"))
    }

    fn boolean(v: &Value, at: &str) -> Check {
        v.as_bool()
            .map(|_| ())
            .ok_or(format!("{at}: {v} is not a boolean"))
    }

    fn estimate(v: &Value, at: &str) -> Check {
        let m = object(v, at, &["value", "low", "high", "basis"], &[])?;
        for key in ["value", "low", "high"] {
            if !m[key].as_f64().is_some_and(|x| x >= 0.0) {
                return Err(format!("{at}.{key}: not a number at least 0"));
            }
        }
        one_of(
            &m["basis"],
            &format!("{at}.basis"),
            &["exact", "sampled", "model"],
        )
    }

    const ORIGINS: &[&str] = &["argument", "graph", "environment", "default"];
    const REGIMES: &[&str] = &["in_memory", "mapped", "streamed", "on_disk"];

    /// Checks a plan document.
    pub(super) fn plan(v: &Value) -> Check {
        let m = object(
            v,
            "plan",
            &[
                "schema",
                "operation",
                "parameters",
                "machine",
                "budget",
                "regime",
                "phases",
                "estimate",
                "scratch",
                "output",
                "fits",
                "refusal",
            ],
            &["call", "input", "graph", "detected", "steps"],
        )?;
        if m["schema"] != 1 {
            return Err("plan.schema: not 1".into());
        }
        string(&m["operation"], "plan.operation")?;
        if !m["parameters"].is_object() {
            return Err("plan.parameters: not an object".into());
        }
        if let Some(input) = m.get("input").filter(|i| !i.is_null()) {
            let i = object(
                input,
                "plan.input",
                &["path", "bytes", "format", "rows"],
                &["compression"],
            )?;
            string(&i["path"], "plan.input.path")?;
            count(&i["bytes"], "plan.input.bytes")?;
            string(&i["format"], "plan.input.format")?;
            estimate(&i["rows"], "plan.input.rows")?;
            if let Some(c) = i.get("compression") {
                one_of(c, "plan.input.compression", &["none", "gzip", "zstd"])?;
            }
        }
        if m.get("graph").is_some_and(|g| !g.is_null()) {
            return Err("plan.graph: an ingest has no graph yet".into());
        }
        if let Some(detected) = m.get("detected").filter(|d| !d.is_null()) {
            let d = object(detected, "plan.detected", &["ids", "edges", "columns"], &[])?;
            let ids = object(
                &d["ids"],
                "plan.detected.ids",
                &["kind", "distinct"],
                &["width_bytes", "noun"],
            )?;
            one_of(
                &ids["kind"],
                "plan.detected.ids.kind",
                &["integer", "hex", "text"],
            )?;
            estimate(&ids["distinct"], "plan.detected.ids.distinct")?;
            let e = object(
                &d["edges"],
                "plan.detected.edges",
                &["directed", "directed_from", "duplicates", "self_loops"],
                &[],
            )?;
            boolean(&e["directed"], "plan.detected.edges.directed")?;
            one_of(
                &e["directed_from"],
                "plan.detected.edges.directed_from",
                &["caller", "column names", "default"],
            )?;
            one_of(
                &e["duplicates"],
                "plan.detected.edges.duplicates",
                &["kept as events", "collapsed", "summed"],
            )?;
            one_of(
                &e["self_loops"],
                "plan.detected.edges.self_loops",
                &["dropped", "kept"],
            )?;
            for (k, c) in d["columns"]
                .as_array()
                .ok_or("plan.detected.columns: not an array")?
                .iter()
                .enumerate()
            {
                let at = format!("plan.detected.columns[{k}]");
                let c = object(c, &at, &["name", "role"], &["type"])?;
                string(&c["name"], &at)?;
                one_of(
                    &c["role"],
                    &at,
                    &["source", "target", "weight", "time", "ignored"],
                )?;
            }
        }
        let machine = object(
            &m["machine"],
            "plan.machine",
            &[
                "cores",
                "hardware_threads",
                "memory_bytes",
                "memory_free_bytes",
                "memory_limit_bytes",
                "storage",
            ],
            &[],
        )?;
        for key in [
            "cores",
            "hardware_threads",
            "memory_bytes",
            "memory_free_bytes",
        ] {
            count(&machine[key], &format!("plan.machine.{key}"))?;
        }
        if !machine["memory_limit_bytes"].is_null() {
            count(
                &machine["memory_limit_bytes"],
                "plan.machine.memory_limit_bytes",
            )?;
        }
        let storage = object(
            &machine["storage"],
            "plan.machine.storage",
            &["kind", "free_bytes"],
            &[],
        )?;
        string(&storage["kind"], "plan.machine.storage.kind")?;
        count(&storage["free_bytes"], "plan.machine.storage.free_bytes")?;
        let budget = object(
            &m["budget"],
            "plan.budget",
            &[
                "memory_bytes",
                "memory_from",
                "above_free",
                "threads",
                "threads_from",
            ],
            &[],
        )?;
        count(&budget["memory_bytes"], "plan.budget.memory_bytes")?;
        one_of(&budget["memory_from"], "plan.budget.memory_from", ORIGINS)?;
        boolean(&budget["above_free"], "plan.budget.above_free")?;
        count(&budget["threads"], "plan.budget.threads")?;
        one_of(&budget["threads_from"], "plan.budget.threads_from", ORIGINS)?;
        let regime = object(
            &m["regime"],
            "plan.regime",
            &["name", "reason", "working_bytes", "ratio"],
            &[],
        )?;
        one_of(&regime["name"], "plan.regime.name", REGIMES)?;
        string(&regime["reason"], "plan.regime.reason")?;
        count(&regime["working_bytes"], "plan.regime.working_bytes")?;
        if !regime["ratio"].as_f64().is_some_and(|r| r >= 0.0) {
            return Err("plan.regime.ratio: not a number at least 0".into());
        }
        let phases = m["phases"]
            .as_array()
            .filter(|p| !p.is_empty())
            .ok_or("plan.phases: not a non-empty array")?;
        for (k, p) in phases.iter().enumerate() {
            let at = format!("plan.phases[{k}]");
            let p = object(p, &at, &["name", "kept", "seconds"], &["regime"])?;
            let name = p["name"].as_str().unwrap_or_default();
            if name.is_empty() || name.len() > 11 || !name.bytes().all(|b| b.is_ascii_lowercase()) {
                return Err(format!("{at}.name: {name:?} is not ^[a-z]{{1,11}}$"));
            }
            boolean(&p["kept"], &at)?;
            estimate(&p["seconds"], &format!("{at}.seconds"))?;
            if let Some(r) = p.get("regime") {
                one_of(r, &format!("{at}.regime"), REGIMES)?;
            }
        }
        let e = object(
            &m["estimate"],
            "plan.estimate",
            &[
                "seconds",
                "peak_memory_bytes",
                "scratch_bytes",
                "output_bytes",
            ],
            &[],
        )?;
        for (key, value) in e {
            estimate(value, &format!("plan.estimate.{key}"))?;
        }
        let scratch = object(
            &m["scratch"],
            "plan.scratch",
            &["path", "from", "memory_backed", "free_bytes"],
            &[],
        )?;
        string(&scratch["path"], "plan.scratch.path")?;
        one_of(&scratch["from"], "plan.scratch.from", ORIGINS)?;
        boolean(&scratch["memory_backed"], "plan.scratch.memory_backed")?;
        count(&scratch["free_bytes"], "plan.scratch.free_bytes")?;
        let output = object(
            &m["output"],
            "plan.output",
            &["path", "kind"],
            &["free_bytes"],
        )?;
        if !output["path"].is_null() {
            string(&output["path"], "plan.output.path")?;
        }
        one_of(
            &output["kind"],
            "plan.output.kind",
            &["graph", "map", "result", "file", "none"],
        )?;
        if let Some(free) = output.get("free_bytes") {
            count(free, "plan.output.free_bytes")?;
        }
        boolean(&m["fits"], "plan.fits")?;
        if !m["refusal"].is_null() {
            error(&m["refusal"], "plan.refusal")?;
        }
        Ok(())
    }

    /// Checks an error document (§ 6.1).
    pub(super) fn error(v: &Value, at: &str) -> Check {
        let m = object(
            v,
            at,
            &["schema", "error", "message", "ways_forward", "changed"],
            &[
                "why",
                "step",
                "needs_bytes",
                "budget_bytes",
                "smallest_budget_bytes",
                "available_bytes",
                "machine_memory_bytes",
                "arithmetic",
                "path",
                "holds",
                "section",
                "column",
                "node",
                "operation",
                "reason",
                "expression",
                "position",
                "problems",
                "invalid_rows",
                "rows_checked",
                "kept",
            ],
        )?;
        one_of(
            &m["error"],
            &format!("{at}.error"),
            &[
                "InsufficientBudget",
                "InsufficientDisk",
                "InvalidInput",
                "InvalidExpression",
                "UnknownNode",
                "Unsupported",
                "OutputExists",
                "CorruptFile",
                "IoError",
                "UsageError",
                "EverygraphError",
            ],
        )?;
        string(&m["message"], &format!("{at}.message"))?;
        boolean(&m["changed"], &format!("{at}.changed"))?;
        for (k, w) in m["ways_forward"]
            .as_array()
            .ok_or(format!("{at}.ways_forward: not an array"))?
            .iter()
            .enumerate()
        {
            let wat = format!("{at}.ways_forward[{k}]");
            let w = object(w, &wat, &["command", "python", "note"], &["needs_bytes"])?;
            for key in ["command", "python", "note"] {
                string(&w[key], &format!("{wat}.{key}"))?;
            }
        }
        if m["error"] == "InsufficientBudget" {
            for key in [
                "step",
                "needs_bytes",
                "budget_bytes",
                "smallest_budget_bytes",
            ] {
                if !m.contains_key(key) {
                    return Err(format!("{at}: {key} missing for InsufficientBudget"));
                }
            }
        }
        Ok(())
    }
}

#[test]
fn the_schema_check_refuses_what_the_schema_refuses() {
    let good: serde_json::Value = serde_json::from_str(
        r#"{"schema":1,"error":"UsageError","message":"m","ways_forward":[{"command":"c","python":"","note":""}],"changed":false}"#,
    )
    .unwrap();
    assert!(plan_schema::error(&good, "e").is_ok());
    for bad in [
        r#"{"schema":1,"error":"UsageError","message":"m","ways_forward":[],"changed":false,"mesage":"x"}"#,
        r#"{"schema":1,"error":"Oops","message":"m","ways_forward":[],"changed":false}"#,
        r#"{"schema":1,"error":"UsageError","message":"m","ways_forward":[{"command":"c","note":""}],"changed":false}"#,
        r#"{"schema":1,"error":"InsufficientBudget","message":"m","ways_forward":[],"changed":false}"#,
    ] {
        let bad: serde_json::Value = serde_json::from_str(bad).unwrap();
        assert!(plan_schema::error(&bad, "e").is_err(), "{bad}");
    }
}
