// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The orderings. Each one is deterministic: the same graph, method and seed give the
//! same permutation at any thread count.
//!
//! - `random`: ids sorted by a seeded hash (the literature's worst case).
//! - `bfs`: breadth-first from the largest-degree node, lists in id order; then the
//!   unreached nodes, each starting a new search, in id order.
//! - `dbg`: degree-based grouping (Faldu, Diamond and Grot, IISWC 2019): 8 groups whose
//!   bounds double from half the mean degree, the highest-degree group first, input
//!   order kept inside a group.
//! - `flat`: by level-0 Leiden community, then id.
//! - `hier`: by the whole path of the Leiden hierarchy, top level first, then id: a
//!   depth-first walk of the dendrogram, children in community-id order.
//! - `hier-deg`: the same path, then degree (largest first), then id.
//! - `hier-bfs`: the same path, then the `bfs` position.
//! - `bp`: recursive graph bisection ([`crate::bp`]).

use std::path::Path;
use std::time::Instant;

use rayon::prelude::*;

use crate::{Args, Graph, bp, get, load_as, perm_of_order, read_words, write_u32};

pub fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// Old ids in the order of a seeded hash.
pub fn random_order(n: usize, seed: u64) -> Vec<u32> {
    let mut keyed: Vec<(u64, u32)> = (0..n as u32)
        .into_par_iter()
        .map(|v| (splitmix(seed ^ splitmix(u64::from(v))), v))
        .collect();
    keyed.par_sort_unstable();
    keyed.into_iter().map(|(_, v)| v).collect()
}

pub fn bfs_order(g: &Graph) -> Vec<u32> {
    let n = g.n();
    let hub = (0..n as u32).max_by_key(|&v| (g.degree(v), std::cmp::Reverse(v))).unwrap_or(0);
    let mut seen = vec![false; n];
    let mut order = Vec::with_capacity(n);
    for root in std::iter::once(hub).chain(0..n as u32) {
        if seen[root as usize] {
            continue;
        }
        seen[root as usize] = true;
        let mut head = order.len();
        order.push(root);
        while head < order.len() {
            let u = order[head];
            head += 1;
            for &v in g.list(u) {
                if !std::mem::replace(&mut seen[v as usize], true) {
                    order.push(v);
                }
            }
        }
    }
    order
}

fn dbg_order(g: &Graph) -> Vec<u32> {
    let n = g.n();
    let mean = g.targets.len() as f64 / n as f64;
    let group = |v: u32| -> u8 {
        let t = f64::from(g.degree(v)) / mean;
        let mut b = 0.5;
        let mut k = 0;
        while k < 7 && t >= b {
            b *= 2.0;
            k += 1;
        }
        7 - k
    };
    let mut order: Vec<u32> = (0..n as u32).collect();
    order.par_sort_by_key(|&v| group(v));
    order
}

/// Community of every node at every level, level 0 first.
fn hier_levels(dir: &Path, n: usize) -> Vec<Vec<u32>> {
    let mut levels = Vec::new();
    let mut c: Vec<u32> = read_words(&dir.join("level_0.u32"));
    assert_eq!(c.len(), n, "level_0.u32: not n entries");
    loop {
        let next = dir.join(format!("level_{}.u32", levels.len() + 1));
        let up: Option<Vec<u32>> = next.exists().then(|| read_words(&next));
        levels.push(c.clone());
        match up {
            Some(up) => c = c.par_iter().map(|&x| up[x as usize]).collect(),
            None => break,
        }
    }
    levels
}

/// Sorts `order` (already in tie-break order) by the hierarchy path, top level first:
/// stable sorts from the bottom level up.
fn by_path(mut order: Vec<u32>, levels: &[Vec<u32>]) -> Vec<u32> {
    for l in levels {
        order.par_sort_by_key(|&v| l[v as usize]);
    }
    order
}

pub fn main(a: &Args, threads: usize) {
    let method = a["method"].as_str();
    let seed: u64 = get(a, "seed", 1);
    let t_load = Instant::now();
    let g = load_as(Path::new(&a["graph"]), get(a, "mmap", 0) == 1);
    let n = g.n();
    let load_s = t_load.elapsed().as_secs_f64();
    let hier = || hier_levels(Path::new(a.get("hier").expect("--hier DIR")), n);
    let t = Instant::now();
    let mut extra = String::new();
    let order = match method {
        "random" => random_order(n, seed),
        "bfs" => bfs_order(&g),
        "dbg" => dbg_order(&g),
        "flat" => {
            let l = hier();
            by_path((0..n as u32).collect(), &l[..1])
        }
        "hier" => by_path((0..n as u32).collect(), &hier()),
        "hier-deg" => {
            let mut o: Vec<u32> = (0..n as u32).collect();
            o.par_sort_by_key(|&v| std::cmp::Reverse(g.degree(v)));
            by_path(o, &hier())
        }
        "hier-bfs" => by_path(bfs_order(&g), &hier()),
        "bp" => {
            let cfg = bp::Config {
                seed,
                iters: get(a, "iters", 20),
                depth: get(a, "depth", bp::default_depth(n)),
                random_start: a.get("start").is_none_or(|s| s == "random"),
                tol_step: get(a, "tol-step", 1.0),
                self_term: get(a, "self-term", 0) == 1,
            };
            let (order, stats) = bp::order(&g, &cfg);
            extra = format!(
                ", \"bp\": {{\"seed\": {seed}, \"iters_cap\": {}, \"tol_step\": {}, \"self_term\": {}, \"depth\": {}, \"start\": \"{}\", {stats}}}",
                cfg.iters,
                cfg.tol_step,
                cfg.self_term,
                cfg.depth,
                if cfg.random_start { "random" } else { "input" }
            );
            order
        }
        other => panic!("unknown --method {other}"),
    };
    let order_s = t.elapsed().as_secs_f64();
    let perm = perm_of_order(&order);
    write_u32(Path::new(&a["out"]), &perm);
    println!(
        "{{\"graph\": \"{}\", \"method\": \"{method}\", \"threads\": {threads}, \"n\": {n}, \
         \"entries\": {}, \"load_s\": {load_s:.3}, \"order_s\": {order_s:.3}{extra}}}",
        Path::new(&a["graph"]).file_name().unwrap().to_string_lossy(),
        g.targets.len(),
    );
}
