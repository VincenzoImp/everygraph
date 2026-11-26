// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The LDBC Graphalytics module: the reader, the match rules and the specification's
//! algorithms. The algorithms are checked against the oracle's references where the two
//! semantics agree, and against hand-worked cases where they do not. No LDBC data is
//! needed: the LDBC test graphs run in `oracle/run-ci.sh`.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use everygraph_oracle::graph::Graph;
use everygraph_oracle::ldbc::read::merge_directions;
use everygraph_oracle::ldbc::run::{Implementation, Symmetric, run, run_spec};
use everygraph_oracle::ldbc::validate::check;
use everygraph_oracle::ldbc::{Algorithm, Dataset, Ids, UNREACHABLE, Values, spec};
use everygraph_oracle::projection::Projection;
use everygraph_oracle::reference;
use everygraph_oracle::strategies::arb_graph;
use proptest::prelude::*;

fn cases() -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(96)
}

/// A data set in a fresh directory: `<name>.properties`, `.v`, `.e` and reference files.
struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(
        tag: &str,
        name: &str,
        directed: bool,
        vertices: &[u64],
        edges: &str,
        props: &str,
    ) -> Self {
        let dir =
            std::env::temp_dir().join(format!("everygraph-ldbc-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let mut p = format!(
            "graph.{name}.vertex-file = {name}.v\ngraph.{name}.edge-file = {name}.e\n\
             graph.{name}.meta.vertices = {}\ngraph.{name}.meta.edges = {}\n\
             graph.{name}.directed = {directed}\n",
            vertices.len(),
            edges.lines().count()
        );
        for line in props.lines() {
            let _ = writeln!(p, "graph.{name}.{line}");
        }
        std::fs::write(dir.join(format!("{name}.properties")), p).expect("write");
        let v: String = vertices.iter().map(|x| format!("{x}\n")).collect();
        std::fs::write(dir.join(format!("{name}.v")), v).expect("write");
        std::fs::write(dir.join(format!("{name}.e")), edges).expect("write");
        Self { dir }
    }

    fn reference(&self, name: &str, a: Algorithm, text: &str) {
        std::fs::write(self.dir.join(format!("{name}-{}", a.suffix())), text).expect("write");
    }

    fn path(&self) -> &Path {
        &self.dir
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn ids(v: &[u64]) -> Ids {
    Ids::new(v.to_vec()).expect("ids")
}

#[test]
fn equivalence_rule_follows_the_specification_example() {
    // Specification v1.0.5, Figure 2.3.
    let i = ids(&[1, 2, 3, 4, 5, 6]);
    let want = Values::Ints(vec![1, 1, 1, 2, 2, 3]);
    let good = Values::Ints(vec![81, 81, 81, 32, 32, 12]);
    let bad = Values::Ints(vec![31, 52, 31, 31, 31, 74]);
    assert!(
        check(Algorithm::Wcc, &good, &want, &i)
            .expect("same kind")
            .matches()
    );
    assert!(
        !check(Algorithm::Wcc, &bad, &want, &i)
            .expect("same kind")
            .matches()
    );
    // Merging two classes is caught from either side.
    let merged = Values::Ints(vec![1, 1, 1, 1, 1, 3]);
    assert_eq!(
        check(Algorithm::Wcc, &merged, &want, &i)
            .expect("same kind")
            .mismatched,
        2
    );
}

#[test]
fn epsilon_rule_follows_the_specification_example() {
    // Figure 2.4 (row 5 is infinity in an SSSP output).
    let i = ids(&[1, 2, 3, 4, 5, 6]);
    let inf = f64::INFINITY;
    let want = Values::Reals(vec![0.0, 0.3, 0.45, 0.23, inf, 0.001]);
    let good = Values::Reals(vec![0.0, 0.30002, 0.45, 0.229_997, inf, 0.001]);
    let bad = Values::Reals(vec![0.000_001, 0.3, 0.46, 0.22, 1.797_69e308, 0.0]);
    assert!(
        check(Algorithm::Sssp, &good, &want, &i)
            .expect("same kind")
            .matches()
    );
    let o = check(Algorithm::Sssp, &bad, &want, &i).expect("same kind");
    assert_eq!(o.mismatched, 5);
    assert!(
        check(Algorithm::Bfs, &good, &want, &i).is_err(),
        "kinds differ"
    );
}

#[test]
fn reader_maps_sparse_ids_in_increasing_order_and_reads_references() {
    let f = Fixture::new(
        "read",
        "g",
        true,
        &[30, 10, 20, 7],
        "10 20 0.5\n20 30 1.5\n30 10 0.25\n",
        "edge-properties.names = weight\nalgorithms = bfs, sssp\nbfs.source-vertex = 10\nsssp.source-vertex = 10",
    );
    f.reference(
        "g",
        Algorithm::Bfs,
        "7 9223372036854775807\n10 0\n20 1\n30 2\n",
    );
    f.reference(
        "g",
        Algorithm::Sssp,
        "7 infinity\n10 0.0\n20 5.000000000000000e-01\n30 2.0\n",
    );
    let ds = Dataset::open(f.path()).expect("open");
    assert!(ds.notes().is_empty());
    let g = ds.load().expect("load");
    assert_eq!(g.ids.ids, vec![7, 10, 20, 30]);
    assert_eq!(g.out.offsets, vec![0, 0, 1, 2, 3]);
    assert_eq!(g.out.targets, vec![2, 3, 1]);
    assert_eq!(g.inn().list(1), &[3]);
    assert_eq!(ds.algorithms(), vec![Algorithm::Bfs, Algorithm::Sssp]);
    let rows = run(&ds, &g, &Algorithm::ALL, &Implementation::ALL, 1000);
    for r in &rows {
        assert_eq!(r.status(), "match", "{}", r.describe());
    }
    assert_eq!(rows.len(), 4, "spec and oracle, BFS and SSSP");
}

#[test]
fn reader_refuses_what_the_specification_excludes() {
    let cases = [
        ("loop", false, "1 2\n2 2\n", "self-loop"),
        ("dup", true, "1 2\n1 2\n", "listed twice"),
        ("both-ways", false, "1 2\n2 1\n", "listed twice"),
        ("unknown", false, "1 9\n", "unknown vertex"),
    ];
    for (tag, directed, edges, msg) in cases {
        let f = Fixture::new(tag, "g", directed, &[1, 2, 3], edges, "");
        let err = Dataset::open(f.path())
            .expect("open")
            .load()
            .expect_err(tag);
        assert!(err.to_string().contains(msg), "{tag}: {err}");
    }
    let f = Fixture::new("bad-ref", "g", false, &[1, 2], "1 2\n", "algorithms = wcc");
    f.reference("g", Algorithm::Wcc, "1 1\n");
    let ds = Dataset::open(f.path()).expect("open");
    assert!(
        ds.reference(Algorithm::Wcc, &ids(&[1, 2])).is_err(),
        "vertex 2 has no value"
    );
}

#[test]
fn edge_file_property_that_names_the_vertex_file_is_reported() {
    let f = Fixture::new("note", "g", false, &[1, 2], "1 2\n", "");
    let p = f.path().join("g.properties");
    let text = std::fs::read_to_string(&p)
        .expect("read")
        .replace("edge-file = g.e", "edge-file = g.v");
    std::fs::write(&p, text).expect("write");
    let ds = Dataset::open(f.path()).expect("open");
    assert_eq!(ds.notes().len(), 1);
    assert_eq!(ds.load().expect("g.e is read").edges(), 1);
}

#[test]
fn cdlp_breaks_ties_by_smallest_label_and_counts_both_directions() {
    // Undirected star: centre 0, leaves 1..3. Iteration 1: each leaf takes label 0; the
    // centre sees labels 1, 2, 3 once each and takes the smallest, 1.
    let f = Fixture::new("star", "s", false, &[0, 1, 2, 3], "0 1\n0 2\n0 3\n", "");
    let g = Dataset::open(f.path()).expect("open").load().expect("load");
    let id: Vec<u32> = (0..4).collect();
    assert_eq!(spec::cdlp(g.out.lists(), None, &id, 1), vec![1, 0, 0, 0]);
    assert_eq!(spec::cdlp(g.out.lists(), None, &id, 2), vec![0, 1, 1, 1]);
    // Directed: 0 has arcs 0 -> 2, 2 -> 0 and 1 -> 0. Label 2 is seen twice, 1 once.
    let f = Fixture::new("dir", "d", true, &[0, 1, 2], "0 2\n2 0\n1 0\n", "");
    let g = Dataset::open(f.path()).expect("open").load().expect("load");
    let inn = g.inn.as_ref().map(|c| c.lists());
    let id: Vec<u32> = (0..3).collect();
    assert_eq!(spec::cdlp(g.out.lists(), inn, &id, 1)[0], 2);
    // A vertex without neighbours keeps its label.
    let f = Fixture::new("iso", "i", false, &[0, 1, 2], "0 1\n", "");
    let g = Dataset::open(f.path()).expect("open").load().expect("load");
    assert_eq!(spec::cdlp(g.out.lists(), None, &[5, 4, 3], 3)[2], 3);
}

#[test]
fn directed_lcc_counts_arcs_between_neighbours() {
    // 0 -> 1, 0 -> 2, 1 -> 2: N(0) = {1, 2}, one arc of two possible: 1/2. With 2 -> 1
    // as well, 1. The oracle's undirected clustering says 1 in both cases.
    for (edges, want) in [("0 1\n0 2\n1 2\n", 0.5), ("0 1\n0 2\n1 2\n2 1\n", 1.0)] {
        let f = Fixture::new("lcc", "t", true, &[0, 1, 2], edges, "");
        let g = Dataset::open(f.path()).expect("open").load().expect("load");
        let (sym, dirs) = merge_directions(g.out.lists(), g.inn());
        assert_eq!(spec::lcc(sym.lists(), Some(&dirs))[0], want, "{edges}");
    }
}

/// The graph without loops and repeated pairs, as LDBC allows it, with weights where the
/// generator gave them (the smallest of a pair's events).
fn simple(g: &Graph) -> Graph {
    let mut pairs: BTreeMap<(u32, u32), f64> = BTreeMap::new();
    let w = |i: usize| g.weight.as_ref().map_or(1.0, |w| w[i]);
    for i in 0..g.rows() {
        let (mut u, mut v) = (g.src[i], g.dst[i]);
        if u == v {
            continue;
        }
        if !g.directed && u > v {
            std::mem::swap(&mut u, &mut v);
        }
        let e = pairs.entry((u, v)).or_insert(f64::INFINITY);
        *e = e.min(w(i));
    }
    let mut h = Graph::with_nodes("simple", g.n, g.directed);
    h.src = pairs.keys().map(|k| k.0).collect();
    h.dst = pairs.keys().map(|k| k.1).collect();
    h.weight = g.weight.as_ref().map(|_| pairs.values().copied().collect());
    h
}

/// Writes `h` as an LDBC data set with ids `3 v + 7`, so that the reader must remap.
fn as_ldbc(h: &Graph, tag: &str) -> Fixture {
    let id = |v: u32| 3 * u64::from(v) + 7;
    let vertices: Vec<u64> = (0..h.n).rev().map(id).collect();
    let mut edges = String::new();
    for i in 0..h.rows() {
        let _ = write!(edges, "{} {}", id(h.src[i]), id(h.dst[i]));
        if let Some(w) = &h.weight {
            let _ = write!(edges, " {:e}", w[i]);
        }
        edges.push('\n');
    }
    let props = format!(
        "{}bfs.source-vertex = 7\nsssp.source-vertex = 7\npr.damping-factor = 0.85\npr.num-iterations = 300\ncdlp.max-iterations = 3",
        if h.weight.is_some() {
            "edge-properties.names = weight\n"
        } else {
            ""
        }
    );
    Fixture::new(tag, "p", h.directed, &vertices, &edges, &props)
}

fn ints(v: Values) -> Vec<u64> {
    match v {
        Values::Ints(x) => x,
        Values::Reals(_) => panic!("integers expected"),
    }
}

fn reals(v: Values) -> Vec<f64> {
    match v {
        Values::Reals(x) => x,
        Values::Ints(_) => panic!("reals expected"),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(cases()))]

    /// Where LDBC and Everygraph define the same thing, the specification's algorithms
    /// equal the oracle's references: BFS, WCC, SSSP, undirected LCC, and PageRank once
    /// 300 iterations have converged to the fixed point.
    #[test]
    fn spec_agrees_with_the_references_where_semantics_agree(g in arb_graph(40)) {
        let h = simple(&g);
        prop_assume!(h.n > 0);
        let f = as_ldbc(&h, &format!("prop-{}", g.provenance["spec"]["seed"]));
        let ds = Dataset::open(f.path()).expect("open");
        let lg = ds.load().expect("load");
        let sym = Symmetric::of(&lg);
        let p = Projection::new(&h);
        let to_id = |v: u32| 3 * u64::from(v) + 7;

        let bfs = ints(run_spec(&ds, &lg, &sym, Algorithm::Bfs).expect("bfs"));
        let want: Vec<u64> = reference::bfs(&p.out, 0).expect("source").into_iter().map(|d| d.unwrap_or(UNREACHABLE)).collect();
        prop_assert_eq!(bfs, want);

        let wcc = ints(run_spec(&ds, &lg, &sym, Algorithm::Wcc).expect("wcc"));
        let want: Vec<u64> = reference::wcc(&p.undirected).into_iter().map(to_id).collect();
        prop_assert_eq!(wcc, want);

        let pr = reals(run_spec(&ds, &lg, &sym, Algorithm::Pr).expect("pr"));
        let want = reference::pagerank(&p, 0.85, false, &[]);
        prop_assert!((pr.iter().sum::<f64>() - 1.0).abs() < 1e-9);
        for (a, b) in pr.iter().zip(&want) {
            prop_assert!((a - b).abs() <= 1e-9 * b.abs(), "{} vs {}", a, b);
        }

        let lcc = reals(run_spec(&ds, &lg, &sym, Algorithm::Lcc).expect("lcc"));
        let und = reference::clustering(&p.undirected);
        for (a, b) in lcc.iter().zip(&und) {
            if h.directed {
                prop_assert!(*a <= b + 1e-12 && *a >= b / 2.0 - 1e-12, "{} vs {}", a, b);
            } else {
                prop_assert!((a - b).abs() <= 1e-12, "{} vs {}", a, b);
            }
        }

        if h.weight.is_some() {
            let d = reals(run_spec(&ds, &lg, &sym, Algorithm::Sssp).expect("sssp"));
            let want = reference::sssp(&p.out, 0).expect("source");
            for (a, b) in d.iter().zip(&want) {
                match b {
                    None => prop_assert!(a.is_infinite()),
                    Some(b) => prop_assert!((a - b).abs() <= 1e-9 * b.abs().max(1.0), "{} vs {}", a, b),
                }
            }
        }

        // CDLP: labels are vertex ids, and every label is one a neighbour or the vertex had.
        let cdlp = ints(run_spec(&ds, &lg, &sym, Algorithm::Cdlp).expect("cdlp"));
        prop_assert!(cdlp.iter().all(|l| lg.ids.index(*l).is_some()));
    }
}
