// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `run`, `info`, `export` and `compact` as a user runs them: results checked against
//! an independent computation and under a relabelling of the input, stored in the
//! graph file and served from it, written to every format and read back, every JSON
//! document checked against its closed schema, exit statuses (`docs/ux-contract.md`
//! § 6.4), plain progress in a terminal (S-10), and the golden transcripts.

#![forbid(unsafe_code)]
#![allow(clippy::print_stderr, reason = "a test says what it could not check")]

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

use common::{dir, document, run, schema, stderr, stdout};
use serde_json::Value;

/// An undirected graph: the triangle a b c with i hanging from c, the path d e f, and
/// h, whose only row is a loop (dropped, so h has no edge).
const EDGES: &str = "a b\nb c\nc a\nc i\nd e\ne f\nh h\n";

/// The same graph with its rows in another order and every name changed.
const RELABELLED: &str = "xf xe\nxh xh\nxi xc\nxe xd\nxa xc\nxc xb\nxb xa\n";

fn graph(group: &str, name: &str) -> std::path::PathBuf {
    let dir = dir(group, name);
    std::fs::write(dir.join("g.txt"), EDGES).unwrap();
    let out = run(&dir, &["ingest", "g.txt", "--undirected"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    dir
}

/// Every row of `run NAME g.eg` as `identifier → values`, from its result document.
fn rows(dir: &Path, args: &[&str]) -> BTreeMap<String, Vec<Value>> {
    let mut all = vec!["run"];
    all.extend_from_slice(args);
    all.extend_from_slice(&["--json", "--top", "1000"]);
    let out = run(dir, &all);
    assert_eq!(out.status.code(), Some(0), "{args:?}: {}", stderr(&out));
    let doc = document(&out);
    schema::result(&doc).unwrap_or_else(|e| panic!("{e}: {doc}"));
    doc["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            let r = r.as_array().unwrap();
            (r[0].as_str().unwrap().to_owned(), r[1..].to_vec())
        })
        .collect()
}

fn first(rows: BTreeMap<String, Vec<Value>>) -> BTreeMap<String, Value> {
    rows.into_iter().map(|(k, v)| (k, v[0].clone())).collect()
}

/// `PageRank` by power iteration on the undirected `edges`, dangling rank spread
/// evenly: the oracle of `run pagerank`.
fn pagerank(edges: &[(&str, &str)], nodes: &[&str]) -> BTreeMap<String, f64> {
    let n = nodes.len() as f64;
    let mut adj: BTreeMap<&str, Vec<&str>> = nodes.iter().map(|&v| (v, Vec::new())).collect();
    for &(u, v) in edges {
        adj.get_mut(u).unwrap().push(v);
        adj.get_mut(v).unwrap().push(u);
    }
    let mut x: BTreeMap<&str, f64> = nodes.iter().map(|&v| (v, 1.0 / n)).collect();
    for _ in 0..1000 {
        let dangling: f64 = nodes
            .iter()
            .filter(|v| adj[*v].is_empty())
            .map(|v| x[v])
            .sum();
        let next: BTreeMap<&str, f64> = nodes
            .iter()
            .map(|&v| {
                let pulled: f64 = adj[v].iter().map(|u| x[u] / adj[u].len() as f64).sum();
                (v, 0.15 / n + 0.85 * (pulled + dangling / n))
            })
            .collect();
        x = next;
    }
    x.into_iter().map(|(k, v)| (k.to_owned(), v)).collect()
}

#[test]
fn whole_graph_results_equal_an_independent_computation() {
    let dir = graph("commands", "oracle");
    let component = first(rows(&dir, &["components", "g.eg"]));
    // Numbered by decreasing size: {a b c i}, {d e f}, {h}.
    for (v, c) in [
        ("a", 0),
        ("b", 0),
        ("c", 0),
        ("i", 0),
        ("d", 1),
        ("e", 1),
        ("f", 1),
        ("h", 2),
    ] {
        assert_eq!(component[v], c, "{v}");
    }
    let core = first(rows(&dir, &["kcore", "g.eg"]));
    for (v, k) in [
        ("a", 2),
        ("b", 2),
        ("c", 2),
        ("i", 1),
        ("d", 1),
        ("e", 1),
        ("f", 1),
        ("h", 0),
    ] {
        assert_eq!(core[v], k, "{v}");
    }
    let degree = first(rows(&dir, &["degrees", "g.eg"]));
    for (v, d) in [
        ("a", 2),
        ("b", 2),
        ("c", 3),
        ("i", 1),
        ("d", 1),
        ("e", 2),
        ("f", 1),
        ("h", 0),
    ] {
        assert_eq!(degree[v], d, "{v}");
    }
    let bfs = rows(&dir, &["bfs", "g.eg", "a"]);
    let expect = [
        ("a", Value::from(0), Value::Null),
        ("b", 1.into(), "a".into()),
        ("c", 1.into(), "a".into()),
        ("i", 2.into(), "c".into()),
        ("d", Value::Null, Value::Null),
        ("h", Value::Null, Value::Null),
    ];
    for (v, d, p) in expect {
        assert_eq!(bfs[v], [d, p], "{v}");
    }
    let edges = [
        ("a", "b"),
        ("b", "c"),
        ("c", "a"),
        ("c", "i"),
        ("d", "e"),
        ("e", "f"),
    ];
    let nodes = ["a", "b", "c", "d", "e", "f", "h", "i"];
    let oracle = pagerank(&edges, &nodes);
    let got = first(rows(
        &dir,
        &["pagerank", "g.eg", "--tol", "0", "--max-iter", "1000"],
    ));
    for v in nodes {
        let x = got[v].as_f64().unwrap();
        assert!(
            (x - oracle[v]).abs() < 1e-12,
            "{v}: {x} against {}",
            oracle[v]
        );
    }
    let sum: f64 = got.values().map(|x| x.as_f64().unwrap()).sum();
    assert!((sum - 1.0).abs() < 1e-12);
}

#[test]
fn results_do_not_depend_on_the_order_or_the_names_of_the_input() {
    let dir = graph("commands", "relabel");
    std::fs::write(dir.join("r.txt"), RELABELLED).unwrap();
    let out = run(&dir, &["ingest", "r.txt", "--undirected"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    for name in ["pagerank", "components", "kcore", "degrees"] {
        let a = rows(&dir, &[name, "g.eg"]);
        let b = rows(&dir, &[name, "r.eg"]);
        for (v, values) in &a {
            let x = &b[&format!("x{v}")];
            // The first value: a rank follows it, which ties may order differently.
            match name {
                "pagerank" => {
                    let (p, q) = (values[0].as_f64().unwrap(), x[0].as_f64().unwrap());
                    assert!((p - q).abs() < 1e-12, "{name} {v}: {p} {q}");
                }
                // Component numbers are by size; sizes are distinct here.
                _ => assert_eq!(values[0], x[0], "{name} {v}"),
            }
        }
    }
}

#[test]
fn a_result_is_kept_in_the_graph_file_and_served_from_it() {
    let dir = graph("commands", "cache");
    let out = run(&dir, &["run", "pagerank", "g.eg", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let computed = document(&out);
    schema::result(&computed).unwrap();
    assert_eq!(computed["cached"], false);
    assert_eq!(computed["stored_in"], "g.eg");
    assert_eq!(computed["meta"]["converged"], true);
    let out = run(&dir, &["run", "pagerank", "g.eg", "--json"]);
    let served = document(&out);
    assert_eq!(served["cached"], true);
    assert_eq!(served["meta"]["cached"], true);
    assert_eq!(served["rows"], computed["rows"]);
    assert!(
        stderr(&out).contains("  pagerank "),
        "a hit shows its phase as kept (G-3)"
    );
    assert!(
        stderr(&out).trim_end().ends_with("kept"),
        "{}",
        stderr(&out)
    );
    // Another parameter is another result; bfs is never kept (§ 2.8).
    let out = run(
        &dir,
        &["run", "pagerank", "g.eg", "--damping", "0.5", "--json"],
    );
    assert_eq!(document(&out)["cached"], false);
    let out = run(&dir, &["run", "bfs", "g.eg", "a", "--json"]);
    assert_eq!(document(&out)["stored_in"], Value::Null);

    let out = run(&dir, &["info", "g.eg", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let info = document(&out);
    schema::info(&info).unwrap_or_else(|e| panic!("{e}: {info}"));
    let results = info["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    assert!(
        results
            .iter()
            .all(|r| r["operation"] == "pagerank" && r["stale"].is_null())
    );
    let dampings: BTreeSet<&str> = results
        .iter()
        .map(|r| r["parameters"]["damping"].as_str().unwrap())
        .collect();
    assert_eq!(dampings, BTreeSet::from(["0.5", "0.85"]));
    assert_eq!(info["graph"]["nodes"], 8);
    assert_eq!(info["graph"]["edges"], 6);
    assert_eq!(info["graph"]["directed"], false);
    assert_eq!(info["build"]["dropped"]["self_loops"], 1);
    assert!(info["reclaimable_bytes"].as_u64().unwrap() > 0);
    let text = stdout(&run(&dir, &["info", "g.eg"]));
    assert!(text.contains("  computed so far\n    pagerank"), "{text}");
    assert!(text.contains("next: everygraph compact g.eg"), "{text}");
}

#[test]
fn compact_reclaims_what_replaced_commits_left_and_keeps_every_result() {
    let dir = graph("commands", "compact");
    for name in ["pagerank", "components", "kcore"] {
        assert_eq!(run(&dir, &["run", name, "g.eg"]).status.code(), Some(0));
    }
    let info = |file: &str| document(&run(&dir, &["info", file, "--json"]));
    let before = info("g.eg");
    assert!(before["reclaimable_bytes"].as_u64().unwrap() > 0);
    let original = std::fs::read(dir.join("g.eg")).unwrap();

    // With --out, the file is not changed.
    let out = run(&dir, &["compact", "g.eg", "--out", "copy.eg", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    schema::file(&document(&out)).unwrap();
    assert_eq!(std::fs::read(dir.join("g.eg")).unwrap(), original);
    let out = run(&dir, &["compact", "g.eg", "--out", "copy.eg"]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "an existing NEW needs --overwrite"
    );

    let out = run(&dir, &["compact", "g.eg"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(!dir.join("g.eg.partial").exists());
    for file in ["g.eg", "copy.eg"] {
        let after = info(file);
        assert_eq!(after["reclaimable_bytes"], 0, "{file}");
        assert_eq!(after["results"], before["results"], "{file}");
        let out = run(&dir, &["verify", file, "--deep"]);
        assert_eq!(out.status.code(), Some(0), "{}", stdout(&out));
    }
    let out = run(&dir, &["run", "kcore", "g.eg", "--json"]);
    assert_eq!(document(&out)["cached"], true);
}

#[test]
fn a_whole_result_is_written_in_every_format() {
    let dir = graph("commands", "formats");
    let expected = rows(&dir, &["bfs", "g.eg", "a"]);
    for name in ["bfs.csv", "bfs.json", "bfs.parquet", "bfs.arrow"] {
        let out = run(&dir, &["run", "bfs", "g.eg", "a", "--out", name]);
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        assert!(
            out.stdout.is_empty(),
            "with --out, the table goes to the file"
        );
        assert!(
            stderr(&out).starts_with(&format!("ok {name} | ")),
            "{}",
            stderr(&out)
        );
    }
    let csv = std::fs::read_to_string(dir.join("bfs.csv")).unwrap();
    let mut lines = csv.lines();
    assert_eq!(lines.next(), Some("node,distance,parent"));
    let from_csv: BTreeMap<String, Vec<Value>> = lines
        .map(|l| {
            let f: Vec<&str> = l.split(',').collect();
            let cell = |s: &str| match s {
                "" => Value::Null,
                s => s
                    .parse::<u64>()
                    .map_or_else(|_| Value::from(s), Value::from),
            };
            (f[0].to_owned(), vec![cell(f[1]), cell(f[2])])
        })
        .collect();
    assert_eq!(from_csv, expected);
    let json: Value =
        serde_json::from_slice(&std::fs::read(dir.join("bfs.json")).unwrap()).unwrap();
    let from_json: BTreeMap<String, Vec<Value>> = json
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["node"].as_str().unwrap().to_owned(),
                vec![r["distance"].clone(), r["parent"].clone()],
            )
        })
        .collect();
    assert_eq!(from_json, expected);
    for (name, batches) in [
        ("bfs.parquet", read_parquet(&dir.join("bfs.parquet"))),
        ("bfs.arrow", read_arrow(&dir.join("bfs.arrow"))),
    ] {
        assert_eq!(batches, expected, "{name}");
    }

    let out = run(&dir, &["run", "bfs", "g.eg", "a", "--out", "bfs.csv"]);
    assert_eq!(out.status.code(), Some(1));
    let out = run(
        &dir,
        &["run", "bfs", "g.eg", "a", "--out", "bfs.csv", "--overwrite"],
    );
    assert_eq!(out.status.code(), Some(0));
    let out = run(
        &dir,
        &["run", "bfs", "g.eg", "a", "--out", "bfs.txt", "--json"],
    );
    assert_eq!(
        out.status.code(),
        Some(2),
        "an extension that names no format"
    );
    schema::error(&document(&out), "error").unwrap();
    assert!(!dir.join("bfs.txt").exists() && !dir.join("bfs.txt.partial").exists());
}

fn to_rows(batch: &arrow_array::RecordBatch, into: &mut BTreeMap<String, Vec<Value>>) {
    use arrow_array::Array;
    use arrow_array::cast::AsArray;
    use arrow_array::types::UInt32Type;
    let node = batch.column(0).as_string::<i32>();
    let distance = batch.column(1).as_primitive::<UInt32Type>();
    let parent = batch.column(2).as_string::<i32>();
    for i in 0..batch.num_rows() {
        let d = if distance.is_null(i) {
            Value::Null
        } else {
            distance.value(i).into()
        };
        let p = if parent.is_null(i) {
            Value::Null
        } else {
            parent.value(i).into()
        };
        into.insert(node.value(i).to_owned(), vec![d, p]);
    }
}

fn read_parquet(path: &Path) -> BTreeMap<String, Vec<Value>> {
    let file = std::fs::File::open(path).unwrap();
    let reader =
        parquet::arrow::arrow_reader::ParquetRecordBatchReader::try_new(file, 1024).unwrap();
    let mut rows = BTreeMap::new();
    for batch in reader {
        to_rows(&batch.unwrap(), &mut rows);
    }
    rows
}

fn read_arrow(path: &Path) -> BTreeMap<String, Vec<Value>> {
    let file = std::fs::File::open(path).unwrap();
    let reader = arrow_ipc::reader::FileReader::try_new(file, None).unwrap();
    let mut rows = BTreeMap::new();
    for batch in reader {
        to_rows(&batch.unwrap(), &mut rows);
    }
    rows
}

#[test]
fn export_writes_each_pair_once_and_says_it_is_not_anonymized() {
    let dir = graph("commands", "export");
    let out = run(
        &dir,
        &[
            "export",
            "g.eg",
            "--out",
            "edges.csv",
            "--topology-only",
            "--json",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let doc = document(&out);
    schema::file(&doc).unwrap();
    assert_eq!(doc["made"]["kind"], "file");
    assert_eq!(doc["counts"]["rows"], 6);
    assert!(
        stderr(&out).contains("The output is not anonymized"),
        "{}",
        stderr(&out)
    );
    let csv = std::fs::read_to_string(dir.join("edges.csv")).unwrap();
    let pairs: BTreeSet<(String, String)> = csv
        .lines()
        .skip(1)
        .map(|l| {
            let (u, v) = l.split_once(',').unwrap();
            (u.min(v).to_owned(), u.max(v).to_owned())
        })
        .collect();
    let input: BTreeSet<(String, String)> = EDGES
        .lines()
        .filter_map(|l| l.split_once(' '))
        .filter(|(u, v)| u != v)
        .map(|(u, v)| (u.min(v).to_owned(), u.max(v).to_owned()))
        .collect();
    assert_eq!(pairs, input);
    assert_eq!(csv.lines().count(), 7, "a header, then each pair once");
    // Without --topology-only there is no notice; the export needs --out.
    let out = run(&dir, &["export", "g.eg", "--out", "e.parquet"]);
    assert!(!stderr(&out).contains("anonymized"));
    assert_eq!(run(&dir, &["export", "g.eg"]).status.code(), Some(2));
}

#[test]
fn every_failure_has_its_exit_status_and_one_document() {
    let dir = graph("commands", "statuses");
    std::fs::write(dir.join("d.csv"), "from,to\na,b\nb,c\n").unwrap();
    assert_eq!(run(&dir, &["ingest", "d.csv"]).status.code(), Some(0));
    std::fs::write(dir.join("not.eg"), "not a graph").unwrap();
    std::fs::write(dir.join("taken.csv"), "mine").unwrap();
    let cases: &[(&[&str], u8, &str)] = &[
        (&["run", "bfs", "g.eg", "zz"], 6, "UnknownNode"),
        (&["run", "pagerank", "d.eg"], 2, "Unsupported"),
        (&["run", "kcore", "d.eg"], 2, "Unsupported"),
        (&["run", "triangles", "g.eg"], 2, "Unsupported"),
        (&["run", "components", "g.eg", "--strong"], 2, "Unsupported"),
        (
            &["run", "pagerank", "g.eg", "--max-hops", "2"],
            2,
            "UsageError",
        ),
        (
            &["run", "pagerank", "g.eg", "--damping", "2"],
            2,
            "UsageError",
        ),
        (&["run", "frobnicate", "g.eg"], 2, "UsageError"),
        (&["run", "bfs", "g.eg"], 2, "UsageError"),
        (&["run", "degrees", "missing.eg"], 1, "IoError"),
        (&["run", "degrees", "not.eg"], 5, "CorruptFile"),
        (&["info", "missing.eg"], 1, "IoError"),
        (&["info", "not.eg"], 5, "CorruptFile"),
        (&["info"], 2, "UsageError"),
        (&["compact", "missing.eg"], 1, "IoError"),
        (&["export", "g.eg", "--out", "taken.csv"], 1, "OutputExists"),
        (
            &["run", "degrees", "g.eg", "--memory", "900TB"],
            2,
            "UsageError",
        ),
        (
            &["run", "degrees", "g.eg", "--progress", "fancy"],
            2,
            "UsageError",
        ),
        (&["run", "degrees", "g.eg", "--json=yes"], 2, "UsageError"),
    ];
    // The machine's memory is known where the probe is (Linux): a budget above it is a
    // usage error (P-16).
    let too_much: &[(&[&str], u8, &str)] = &[(
        &["run", "degrees", "g.eg", "--memory", "900TB"],
        2,
        "UsageError",
    )];
    let cases = if cfg!(target_os = "linux") {
        [cases, too_much].concat()
    } else {
        cases.to_vec()
    };
    for &(args, status, class) in &cases {
        let mut with_json = args.to_vec();
        with_json.push("--json");
        let out = run(&dir, &with_json);
        assert_eq!(
            out.status.code(),
            Some(i32::from(status)),
            "{args:?}: {}",
            stderr(&out)
        );
        let doc = document(&out);
        schema::error(&doc, "error").unwrap_or_else(|e| panic!("{args:?}: {e}: {doc}"));
        assert_eq!(doc["error"], class, "{args:?}");
        let out = run(&dir, args);
        assert_eq!(out.status.code(), Some(i32::from(status)));
        assert!(
            out.stdout.is_empty(),
            "{args:?}: an error is on standard error"
        );
        let text = stderr(&out);
        assert!(text.starts_with("error: "), "{args:?}: {text}");
        assert!(
            text.contains("  Ways forward:\n"),
            "{args:?}: every error has one (T-5)"
        );
        assert!(text.lines().all(|l| l.chars().count() <= 80), "{text}");
    }
}

#[test]
fn the_closed_checks_refuse_a_misspelt_key() {
    let dir = graph("commands", "closed");
    let result = document(&run(&dir, &["run", "degrees", "g.eg", "--json"]));
    let info = document(&run(&dir, &["info", "g.eg", "--json"]));
    let file = document(&run(&dir, &["export", "g.eg", "--out", "e.csv", "--json"]));
    for (doc, check) in [
        (&result, schema::result as fn(&Value) -> Result<(), String>),
        (&info, schema::info),
        (&file, schema::file),
    ] {
        check(doc).unwrap();
        let mut bad = doc.clone();
        bad["rwos"] = 1.into();
        assert!(check(&bad).is_err());
    }
    let mut bad = result.clone();
    bad["rows_returned"] = 99.into();
    assert!(schema::result(&bad).is_err());
    let mut bad = info.clone();
    bad["results"] = serde_json::json!([{"operation": "x"}]);
    assert!(schema::info(&bad).is_err());
}

/// Runs `args` in a pseudo-terminal through util-linux `script`, with `env`; returns
/// what the terminal showed, or `None` where `script` is not there.
fn in_terminal(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Option<String> {
    let command = std::iter::once(env!("CARGO_BIN_EXE_everygraph"))
        .chain(args.iter().copied())
        .collect::<Vec<_>>()
        .join(" ");
    let out = Command::new("script")
        .current_dir(dir)
        .args(["-q", "-e", "-c", &command, "/dev/null"])
        .envs(env.iter().copied())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

#[test]
fn progress_is_plain_in_a_terminal_under_term_dumb_and_with_progress_plain() {
    let dir = dir("commands", "terminal");
    std::fs::write(dir.join("g.txt"), EDGES).unwrap();
    let ingest = ["ingest", "g.txt", "--undirected", "--overwrite"];
    let utf8 = [("LANG", "C.UTF-8"), ("TERM", "xterm-256color")];
    let Some(bars) = in_terminal(&dir, &ingest, &utf8) else {
        eprintln!("no `script` here: the terminal test did not run");
        return;
    };
    assert!(bars.contains('█'), "a terminal shows bars: {bars}");
    let mut plain_args = ingest.to_vec();
    plain_args.push("--progress=plain");
    let dumb = [("LANG", "C.UTF-8"), ("TERM", "dumb")];
    for (args, env) in [(&ingest[..], &dumb[..]), (&plain_args[..], &utf8[..])] {
        let shown = in_terminal(&dir, args, env).unwrap();
        let progress: Vec<&str> = shown
            .lines()
            .filter(|l| {
                ["  reading", "  identifiers", "  building", "  checking"]
                    .iter()
                    .any(|p| l.starts_with(p))
            })
            .collect();
        assert_eq!(progress.len(), 4, "{shown}");
        for line in progress {
            assert!(line.is_ascii() && !line.contains('#'), "{line:?}");
            assert!(
                !line.trim_end_matches('\r').contains('\r'),
                "nothing is redrawn: {line:?}"
            );
            assert!(!line.contains('\u{1b}'), "{line:?}");
        }
    }
}

/// `text` with what changes from run to run masked: times, dates and ages.
fn masked(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let mut words: Vec<String> = Vec::new();
        let tokens: Vec<&str> = line.split(' ').collect();
        let mut i = 0;
        while i < tokens.len() {
            let t = tokens[i];
            let unit = tokens.get(i + 1).copied().unwrap_or("");
            if t.parse::<f64>().is_ok() && ["ms", "s", "min", "h"].contains(&unit) {
                words.push("<time>".into());
                i += 2;
                while tokens.get(i).is_some_and(|t| t.parse::<f64>().is_ok())
                    && ["s", "min"].contains(&tokens.get(i + 1).copied().unwrap_or(""))
                {
                    i += 2;
                }
                continue;
            }
            if t.len() == 10 && t.as_bytes()[4] == b'-' && t.as_bytes()[7] == b'-' {
                words.push("<date>".into());
            } else if t == "just" && unit == "now" {
                words.push("<age>".into());
                i += 1;
            } else {
                words.push(t.to_owned());
            }
            i += 1;
        }
        out.push_str(words.join(" ").trim_end());
        out.push('\n');
    }
    out
}

/// Compares `md` with the stored transcripts `name`, which `EVERYGRAPH_BLESS=1`
/// writes again, and checks them against the terminal rules.
fn golden(name: &str, md: &str) {
    let golden = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name);
    if std::env::var_os("EVERYGRAPH_BLESS").is_some() {
        std::fs::create_dir_all(golden.parent().unwrap()).unwrap();
        std::fs::write(&golden, md).unwrap();
    }
    // A checkout may have written the file with CRLF line ends (Windows).
    let stored = std::fs::read_to_string(&golden)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert_eq!(
        md, stored,
        "run with EVERYGRAPH_BLESS=1 to write the new transcripts"
    );
    let checker =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/tools/check_transcripts.py");
    if let Ok(out) = Command::new("python3").arg(&checker).arg(&golden).output() {
        assert!(out.status.success(), "{}{}", stdout(&out), stderr(&out));
    }
}

const PREAMBLE: &str = "# Golden transcripts of the command\n\nWritten by `tests/commands.rs` \
     (`EVERYGRAPH_BLESS=1` writes them again); checked by\n\
     `docs/tools/check_transcripts.py`. Times, dates and ages are masked.\n";

#[test]
fn golden_transcripts() {
    let dir = graph("commands", "golden");
    let sessions: &[&[&str]] = &[
        &["run", "components", "g.eg"],
        &["run", "pagerank", "g.eg", "--top", "3"],
        &["run", "pagerank", "g.eg", "--top", "3"],
        &["run", "kcore", "g.eg", "--out", "core.csv"],
        &["run", "bfs", "g.eg", "a", "--top", "4"],
        &["run", "bfs", "g.eg", "zz"],
        &["info", "g.eg"],
        &["export", "g.eg", "--out", "edges.csv", "--topology-only"],
        &["compact", "g.eg"],
        &["run", "pagerank", "g.eg", "--max-hops", "3"],
    ];
    let mut md = format!("{PREAMBLE}Piped: standard output, then standard error.\n");
    for args in sessions {
        let out = run(&dir, args);
        md.push_str(&format!("\n```console\n$ everygraph {}\n", args.join(" ")));
        md.push_str(&masked(&stdout(&out)));
        md.push_str(&masked(&stderr(&out)));
        md.push_str(&format!("$ echo $?\n{}\n```\n", out.status.code().unwrap()));
    }
    golden("piped.md", &md);

    // On a terminal, with TERM=dumb: no colour, Unicode, aligned tables (§ 5.3).
    let terminal: &[&[&str]] = &[
        &["run", "pagerank", "g.eg", "--top", "3"],
        &["run", "bfs", "g.eg", "a"],
        &["info", "g.eg"],
    ];
    let env = [("LANG", "C.UTF-8"), ("TERM", "dumb")];
    let mut md = format!("{PREAMBLE}On a terminal under `TERM=dumb`: no colour.\n");
    for args in terminal {
        let Some(shown) = in_terminal(&dir, args, &env) else {
            eprintln!("no `script` here: the terminal transcripts were not checked");
            return;
        };
        md.push_str(&format!("\n```console\n$ everygraph {}\n", args.join(" ")));
        md.push_str(&masked(&shown.replace('\r', "")));
        md.push_str("```\n");
    }
    golden("terminal.md", &md);
}
