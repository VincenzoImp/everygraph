// SPDX-License-Identifier: Apache-2.0 OR MIT

//! WCC against the reference outputs of LDBC Graphalytics (`<graph>-WCC`: each vertex
//! and the smallest vertex id of its component). Runs when `EVERYGRAPH_LDBC_GRAPHS`
//! names the directory of the unpacked graphs (`$EVERYGRAPH_DATA/ldbc-graphalytics/graphs`):
//! the four test graphs always, `dota-league` and `graph500-22` in a release build. Vertex
//! ids are renumbered in increasing order, so the smallest id of a component is its
//! smallest node.

#![forbid(unsafe_code)]
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::print_stderr,
    clippy::cast_possible_truncation
)]

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::sync::atomic::AtomicU32;

use everygraph_core::Memory;
use everygraph_kernels::components::{WccOptions, wcc};
use everygraph_kernels::edge_map::{Csr, Plan, Settings};

fn rows(path: &Path) -> impl Iterator<Item = Vec<u64>> {
    let file = std::fs::File::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    BufReader::new(file).lines().map(|line| {
        line.unwrap()
            .split_whitespace()
            .take(2)
            .map(|w| w.parse().unwrap())
            .collect()
    })
}

fn check(dir: &Path, name: &str) {
    let at = |suffix: &str| dir.join(name).join(format!("{name}{suffix}"));
    let mut ids: Vec<u64> = rows(&at(".v")).map(|r| r[0]).collect();
    ids.sort_unstable();
    let node: HashMap<u64, u32> = ids
        .iter()
        .enumerate()
        .map(|(i, &id)| (id, i as u32))
        .collect();
    let mut lists: Vec<Vec<u32>> = ids.iter().map(|_| Vec::new()).collect();
    for r in rows(&at(".e")) {
        let (u, v) = (node[&r[0]], node[&r[1]]);
        lists[u as usize].push(v);
        lists[v as usize].push(u);
    }
    let (mut offsets, mut targets) = (Vec::from([0_u64]), Vec::new());
    for mut l in lists {
        l.sort_unstable();
        l.dedup();
        targets.extend(l);
        offsets.push(targets.len() as u64);
    }
    let csr = Csr::new(&offsets, &targets).unwrap();
    let memory = Memory::unlimited();
    for sample in [true, false] {
        let options = WccOptions {
            sample,
            ..WccOptions::default()
        };
        let labels = Plan::with(&csr, &memory, Settings::default(), |plan| {
            let (l, _) = wcc(&plan, &memory, options).unwrap();
            l.into_iter().map(AtomicU32::into_inner).collect::<Vec<_>>()
        });
        let mut checked = 0;
        for r in rows(&at("-WCC")) {
            let v = node[&r[0]] as usize;
            assert_eq!(ids[labels[v] as usize], r[1], "{name}: vertex {}", r[0]);
            checked += 1;
        }
        assert_eq!(checked, ids.len(), "{name}: one reference row per vertex");
    }
    eprintln!(
        "{name}: {} vertices, {} entries, labels equal",
        ids.len(),
        targets.len()
    );
}

#[test]
fn wcc_matches_ldbc_graphalytics() {
    let Some(dir) = std::env::var_os("EVERYGRAPH_LDBC_GRAPHS") else {
        eprintln!("EVERYGRAPH_LDBC_GRAPHS is not set: nothing checked");
        return;
    };
    let dir = Path::new(&dir);
    for name in [
        "example-directed",
        "example-undirected",
        "test-wcc-directed",
        "test-wcc-undirected",
    ] {
        check(dir, name);
    }
    if !cfg!(debug_assertions) {
        check(dir, "dota-league");
        check(dir, "graph500-22");
    }
}
