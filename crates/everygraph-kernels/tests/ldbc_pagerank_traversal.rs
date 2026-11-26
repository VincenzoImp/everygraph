// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `PageRank`, BFS and SSSP against the reference outputs of the LDBC Graphalytics test
//! graphs (`<graph>-PR`, `-BFS`, `-SSSP`). Runs when `EVERYGRAPH_LDBC_GRAPHS` names the
//! directory of the unpacked graphs (`$EVERYGRAPH_DATA/ldbc-graphalytics/graphs`).
//! Vertex ids are renumbered in increasing order. `PageRank` runs the specification's
//! fixed iterations (`tol = 0`) and must meet its rule, `|r − s| ≤ 1e-4·|r|`; BFS must be
//! equal, and SSSP equal to the printed decimals (within 1e-12, relative).

#![forbid(unsafe_code)]
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::print_stderr,
    clippy::cast_possible_truncation
)]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::Ordering::Relaxed;

use everygraph_core::Memory;
use everygraph_kernels::edge_map::{Csr, Plan, Settings};
use everygraph_kernels::pagerank::{Graph, PageRank, pagerank, strength};
use everygraph_kernels::sssp::{Sssp, sssp};
use everygraph_kernels::traversal::{Bfs, UNREACHED, bfs};

/// LDBC's distance of an unreached vertex in BFS output.
const LDBC_UNREACHED: &str = "9223372036854775807";

/// Sorted lists with one weight per entry, from `(source, target, weight)` arcs.
fn lists(n: usize, mut arcs: Vec<(u32, u32, f64)>) -> (Vec<u64>, Vec<u32>, Vec<f64>) {
    arcs.sort_by_key(|a| (a.0, a.1));
    let mut offsets = vec![0_u64; n + 1];
    for a in &arcs {
        offsets[a.0 as usize + 1] += 1;
    }
    for v in 0..n {
        offsets[v + 1] += offsets[v];
    }
    let targets = arcs.iter().map(|a| a.1).collect();
    (offsets, targets, arcs.iter().map(|a| a.2).collect())
}

fn check(dir: &Path, name: &str) {
    let file = |suffix: &str| {
        let path = dir.join(name).join(format!("{name}{suffix}"));
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    };
    let props: BTreeMap<String, String> = file(".properties")
        .lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| {
            let k = k.trim().trim_start_matches(&format!("graph.{name}."));
            (k.to_owned(), v.trim().to_owned())
        })
        .collect();
    let directed = props["directed"] == "true";
    let mut ids: Vec<u64> = file(".v")
        .lines()
        .map(|l| l.trim().parse().unwrap())
        .collect();
    ids.sort_unstable();
    let node = |id: &str| ids.binary_search(&id.parse().unwrap()).unwrap() as u32;
    let mut arcs = Vec::new();
    for line in file(".e").lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        let (u, v) = (node(f[0]), node(f[1]));
        let w = f.get(2).map_or(1.0, |w| w.parse().unwrap());
        arcs.push((u, v, w));
        if !directed {
            arcs.push((v, u, w));
        }
    }
    let reversed = arcs.iter().map(|&(u, v, w)| (v, u, w)).collect();
    let (out, inn) = (lists(ids.len(), arcs), lists(ids.len(), reversed));
    let expected = |algorithm: &str| -> Vec<String> {
        let by_id: BTreeMap<u64, String> = file(&format!("-{algorithm}"))
            .lines()
            .filter_map(|l| l.split_once(' '))
            .map(|(id, x)| (id.parse().unwrap(), x.trim().to_owned()))
            .collect();
        ids.iter().map(|id| by_id[id].clone()).collect()
    };
    let memory = Memory::unlimited();
    let (oc, ic) = (
        Csr::new(&out.0, &out.1).unwrap(),
        Csr::new(&inn.0, &inn.1).unwrap(),
    );
    Plan::with_pair(&oc, &ic, &memory, Settings::default(), |po, pi| {
        if let Some(iterations) = props.get("pr.num-iterations") {
            let s = strength(&po, None, directed, &memory).unwrap();
            let g = Graph {
                lists: &pi,
                weights: None,
                directed,
            };
            let p = PageRank {
                damping: props["pr.damping-factor"].parse().unwrap(),
                tol: 0.0,
                max_iter: iterations.parse().unwrap(),
                personalize: None,
            };
            let r = pagerank::<_, f64>(&g, s.as_slice().unwrap(), &p, &memory).unwrap();
            let worst = r
                .values
                .iter()
                .zip(expected("PR"))
                .map(|(&x, s)| (x - s.parse::<f64>().unwrap()).abs() / x)
                .fold(0.0, f64::max);
            eprintln!("{name}: PageRank within {worst:e} of the reference, relative");
            assert!(worst <= 1e-4, "{name}");
        }
        let source = |key: &str| props.get(key).map(|s| node(s));
        if let Some(s) = source("bfs.source-vertex") {
            let hops = bfs(&po, Some(&pi), s, &Bfs::default(), &memory).unwrap();
            for (v, want) in expected("BFS").iter().enumerate() {
                let d = hops.distance[v].load(Relaxed);
                let got = if d == UNREACHED {
                    LDBC_UNREACHED.to_owned()
                } else {
                    d.to_string()
                };
                assert_eq!(&got, want, "{name}: BFS, vertex {}", ids[v]);
            }
        }
        if let Some(s) = source("sssp.source-vertex") {
            let d = sssp(&po, &out.2, s, &Sssp::default(), &memory).unwrap();
            for (v, want) in expected("SSSP").iter().enumerate() {
                let got = d.get(v as u32).unwrap_or(f64::INFINITY);
                let want = if want == "infinity" {
                    f64::INFINITY
                } else {
                    want.parse().unwrap()
                };
                // The reference prints a rounded decimal.
                let close = got == want || (got - want).abs() <= 1e-12 * want;
                assert!(close, "{name}: SSSP, vertex {}: {got} vs {want}", ids[v]);
            }
        }
    })
    .unwrap();
}

#[test]
fn ldbc_test_graphs() {
    let Some(dir) = std::env::var_os("EVERYGRAPH_LDBC_GRAPHS") else {
        eprintln!("EVERYGRAPH_LDBC_GRAPHS is not set: skipped");
        return;
    };
    let dir = Path::new(&dir);
    for name in [
        "example-directed",
        "example-undirected",
        "test-pr-directed",
        "test-pr-undirected",
        "test-bfs-directed",
        "test-bfs-undirected",
        "test-sssp-directed",
        "test-sssp-undirected",
    ] {
        check(dir, name);
    }
}
