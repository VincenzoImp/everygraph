// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `everygraph verify` as a user runs it: the binary, its streams and its exit statuses
//! (`docs/api-sketch.md` § 5, `docs/ux-contract.md` § 6.4). Output is piped here, so it
//! is in the ASCII rendering.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use everygraph_core::{Budget, ByteSource, MemorySource};
use everygraph_format::adjacency::encode;
use everygraph_format::container::{Kind, Provenance, Writer};
use everygraph_ingest::ids::{
    BuildOptions, ColumnType, DictionaryError, Key, KeySource, MemoryScratch, build_dictionary,
};

fn everygraph(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_everygraph"))
        .args(args)
        .output()
        .expect("the everygraph binary runs")
}

fn stdout(o: &Output) -> String {
    String::from_utf8(o.stdout.clone()).unwrap()
}

fn stderr(o: &Output) -> String {
    String::from_utf8(o.stderr.clone()).unwrap()
}

/// The keys `"k0"`, `"k1"`, …: the identifier column of a test graph.
struct Names(usize);

impl KeySource for Names {
    fn column(&self) -> ColumnType {
        ColumnType::Utf8
    }

    fn chunks(&self) -> usize {
        1
    }

    fn read_chunk(
        &self,
        _: usize,
        f: &mut dyn FnMut(u64, Key<'_>) -> Result<(), DictionaryError>,
    ) -> Result<(), DictionaryError> {
        for i in 0..self.0 {
            f(i as u64, Key::from(format!("k{i}").as_str()))?;
        }
        Ok(())
    }
}

/// A graph file: the undirected path 0–1–2–3, and a dictionary of `keys` keys.
fn graph_file(dir: &Path, keys: usize) -> PathBuf {
    let e = encode(&[0, 1, 3, 5, 6], &[1, 0, 2, 1, 3, 2]).unwrap();
    let mut w = Writer::create(MemorySource::new(Vec::new())).unwrap();
    let record = Provenance {
        engine: "everygraph test".into(),
        operation: "ingest".into(),
        parameters: [("directed", "false"), ("self_loops", "drop")]
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .into_iter()
            .collect(),
        ..Provenance::default()
    };
    w.add_provenance("provenance/ingest", &record).unwrap();
    for (name, kind, bytes) in [
        ("adjacency.out.index", Kind::ADJACENCY_INDEX, &e.index),
        ("adjacency.out.data", Kind::ADJACENCY_DATA, &e.data),
    ] {
        w.add_section(name, kind, bytes, Some("provenance/ingest"))
            .unwrap();
    }
    let budget = Budget::unlimited();
    let built = build_dictionary(
        &Names(keys),
        &BuildOptions::default(),
        &budget,
        &MemoryScratch,
    )
    .unwrap();
    for (name, section) in built.dictionary.sections() {
        let mut bytes = vec![0; usize::try_from(section.len()).unwrap()];
        section.read_at(0, &mut bytes).unwrap();
        w.add_section(name, Kind::ID_DICTIONARY, &bytes, None)
            .unwrap();
    }
    w.commit().unwrap();
    let path = dir.join(format!("graph-{keys}.eg"));
    std::fs::write(&path, w.into_sink().into_inner()).unwrap();
    path
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("everygraph-cli-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn an_intact_file_passes_both_checks_with_its_report_on_standard_output() {
    let dir = scratch("intact");
    let path = graph_file(&dir, 4);
    let file = path.to_str().unwrap();
    for args in [vec!["verify", file], vec!["verify", file, "--deep"]] {
        let o = everygraph(&args);
        assert_eq!(o.status.code(), Some(0), "{}{}", stdout(&o), stderr(&o));
        assert!(
            stdout(&o).starts_with(&format!("ok {file} is intact.\n")),
            "{}",
            stdout(&o)
        );
        assert!(o.stderr.is_empty());
    }
    let o = everygraph(&["verify", "--deep", "--json", file]);
    assert_eq!(o.status.code(), Some(0));
    let doc = stdout(&o);
    assert!(doc.starts_with(r#"{"schema":1,"verify":"#), "{doc}");
    assert!(
        doc.contains(r#""deep":true,"intact":true,"generation":1,"#),
        "{doc}"
    );
    assert!(
        doc.ends_with("\"problems\":[],\"notes\":[],\"unchecked\":[]}\n"),
        "{doc}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn damage_exits_with_status_5_and_names_the_section() {
    let dir = scratch("damaged");
    let path = graph_file(&dir, 4);
    let mut bytes = std::fs::read(&path).unwrap();
    // The first section after the header blocks is the provenance record.
    bytes[8192 + 20] ^= 0x10;
    std::fs::write(&path, &bytes).unwrap();
    let o = everygraph(&["verify", path.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(5));
    let text = stdout(&o);
    assert!(text.contains("is damaged: 1 problem.\n"), "{text}");
    assert!(
        text.contains("  provenance/ingest: its bytes do not match its hash\n"),
        "{text}"
    );
    assert!(text.ends_with("\nNothing was changed.\n"), "{text}");
    let o = everygraph(&["verify", "--json", path.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(5));
    assert!(
        stdout(&o).contains(
            r#""problems":[{"section":"provenance/ingest","problem":"its bytes do not match its hash"}]"#
        ),
        "{}",
        stdout(&o)
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_deep_check_compares_the_dictionary_with_the_graph() {
    let dir = scratch("dictionary");
    let path = graph_file(&dir, 3);
    let file = path.to_str().unwrap();
    assert_eq!(everygraph(&["verify", file]).status.code(), Some(0));
    let o = everygraph(&["verify", file, "--deep"]);
    assert_eq!(o.status.code(), Some(5));
    assert!(
        stdout(&o).contains("ids.header: the dictionary has 3 keys, but adjacency.out has 4 nodes"),
        "{}",
        stdout(&o)
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_file_that_cannot_be_read_is_an_io_error_with_status_1() {
    let o = everygraph(&["verify", "/nonexistent/graph.eg"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(o.stdout.is_empty());
    assert!(
        stderr(&o).starts_with("error: Cannot read /nonexistent/graph.eg: "),
        "{}",
        stderr(&o)
    );
    let o = everygraph(&["verify", "--json", "/nonexistent/graph.eg"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stdout(&o).starts_with(r#"{"schema":1,"error":"IoError","#));
    assert!(stdout(&o).contains(r#""path":"/nonexistent/graph.eg""#));
}

#[test]
fn usage_errors_exit_with_status_2_and_help_is_a_result() {
    for args in [
        &[][..],
        &["verify"],
        &["verify", "a.eg", "b.eg"],
        &["verify", "--deeep", "a.eg"],
        &["vrify"],
    ] {
        let o = everygraph(args);
        assert_eq!(o.status.code(), Some(2), "{args:?}");
        assert!(o.stdout.is_empty(), "{args:?}");
    }
    let o = everygraph(&["verify", "--deeep", "a.eg"]);
    assert!(
        stderr(&o).starts_with("error: Unknown option --deeep.\n"),
        "{}",
        stderr(&o)
    );
    let o = everygraph(&["verify", "--json", "--deeep"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(
        stdout(&o)
            .starts_with(r#"{"schema":1,"error":"UsageError","message":"Unknown option --deeep.""#)
    );
    for args in [&["--help"][..], &["verify", "--help"]] {
        let o = everygraph(args);
        assert_eq!(o.status.code(), Some(0));
        assert!(stdout(&o).contains("verify FILE"));
        assert!(stdout(&o).lines().all(|l| l.chars().count() <= 80));
        assert!(
            stdout(&o).is_ascii(),
            "piped output is ASCII (ux-contract S-9)"
        );
    }
    let o = everygraph(&["--version"]);
    assert_eq!(
        stdout(&o),
        format!("everygraph {}\n", env!("CARGO_PKG_VERSION"))
    );
}
