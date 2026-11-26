// SPDX-License-Identifier: Apache-2.0 OR MIT
//
// The loops are those of github.com/frankmcsherry/COST, src/bin/union_find.rs and
// src/bin/pagerank.rs (MIT licence, Copyright (c) 2015 Frank McSherry), over the edges
// in CSR order, which is COST's `vertex` mode. Changes: safe indexing instead of
// `get_unchecked`; PageRank in f64 (the protocol's precision; COST uses f32) and started
// from rank 1 instead of 0, so that its 20 iterations are the 20 of every other tool
// (COST's first iteration only sets the start); the union-find also returns each node's
// root, for validation.

//! A competent single thread (McSherry et al., HotOS'15), the COST line:
//!
//! ```text
//! cost-line --graph DIR --kernel uf|pr [--reps R] [--dump FILE]
//! ```
//!
//! Prints one JSON line: `load_seconds` (reading the arrays into the heap),
//! `seconds_all`, `seconds` (the median) and `seconds_min`. `--dump` writes the last
//! result: the root of every node (u32) or the ranks divided by n (f64).

#![forbid(unsafe_code)]
#![allow(clippy::print_stdout, clippy::cast_precision_loss)]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::time::Instant;

fn read_words<const W: usize, T>(path: &Path, from: fn([u8; W]) -> T) -> Vec<T> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|mut f| f.read_to_end(&mut bytes))
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    bytes
        .chunks_exact(W)
        .map(|c| from(c.try_into().expect("W bytes")))
        .collect()
}

/// Calls `action(x, y)` for every entry, in CSR order.
fn map_edges(offsets: &[u64], targets: &[u32], mut action: impl FnMut(u32, u32)) {
    for (x, w) in offsets.windows(2).enumerate() {
        let x = u32::try_from(x).expect("a node id");
        for &y in &targets[w[0] as usize..w[1] as usize] {
            action(x, y);
        }
    }
}

fn union_find(offsets: &[u64], targets: &[u32]) -> Vec<u32> {
    let nodes = offsets.len() - 1;
    let mut roots: Vec<u32> = (0..u32::try_from(nodes).expect("node ids")).collect();
    let mut ranks = vec![0_u8; nodes];
    map_edges(offsets, targets, |mut x, mut y| {
        while x != roots[x as usize] {
            x = roots[x as usize];
        }
        while y != roots[y as usize] {
            y = roots[y as usize];
        }
        if x != y {
            match ranks[x as usize].cmp(&ranks[y as usize]) {
                std::cmp::Ordering::Less => roots[x as usize] = y,
                std::cmp::Ordering::Greater => roots[y as usize] = x,
                std::cmp::Ordering::Equal => {
                    roots[y as usize] = x;
                    ranks[x as usize] += 1;
                }
            }
        }
    });
    roots
}

fn pagerank(offsets: &[u64], targets: &[u32], alpha: f64) -> Vec<f64> {
    let nodes = offsets.len() - 1;
    let mut src = vec![0_f64; nodes];
    let mut dst = vec![1_f64; nodes];
    let mut deg = vec![0_f64; nodes];
    map_edges(offsets, targets, |x, _| deg[x as usize] += 1.0);
    for _iteration in 0..20 {
        for node in 0..nodes {
            src[node] = alpha * dst[node] / deg[node];
            dst[node] = 1.0 - alpha;
        }
        map_edges(offsets, targets, |x, y| dst[y as usize] += src[x as usize]);
    }
    dst
}

fn write(path: &str, words: impl Iterator<Item = [u8; 8]>, width: usize) {
    let mut out = std::io::BufWriter::new(std::fs::File::create(path).expect("dump file"));
    for w in words {
        out.write_all(&w[..width]).expect("dump");
    }
}

fn main() {
    let a: HashMap<String, String> = std::env::args()
        .skip(1)
        .collect::<Vec<_>>()
        .chunks(2)
        .map(|kv| {
            (
                kv[0].trim_start_matches("--").to_owned(),
                kv.get(1).cloned().unwrap_or_default(),
            )
        })
        .collect();
    let graph = Path::new(&a["graph"]);
    let kernel = a["kernel"].as_str();
    let reps: usize = a.get("reps").map_or(5, |r| r.parse().expect("--reps"));
    let t = Instant::now();
    let offsets = read_words(&graph.join("offsets.u64"), u64::from_le_bytes);
    let targets = read_words(&graph.join("targets.u32"), u32::from_le_bytes);
    let load = t.elapsed().as_secs_f64();
    let mut secs = Vec::new();
    let mut roots = Vec::new();
    let mut ranks = Vec::new();
    for _ in 0..reps {
        let t = Instant::now();
        match kernel {
            "uf" => roots = union_find(&offsets, &targets),
            "pr" => ranks = pagerank(&offsets, &targets, 0.85),
            other => panic!("unknown --kernel {other}"),
        }
        secs.push(t.elapsed().as_secs_f64());
    }
    if let Some(path) = a.get("dump") {
        if kernel == "uf" {
            let find = |mut x: u32| {
                while x != roots[x as usize] {
                    x = roots[x as usize];
                }
                u64::from(x).to_le_bytes()
            };
            write(path, (0..roots.len() as u32).map(find), 4);
        } else {
            let n = ranks.len() as f64;
            write(path, ranks.iter().map(|r| (r / n).to_le_bytes()), 8);
        }
    }
    let mut sorted = secs.clone();
    sorted.sort_by(f64::total_cmp);
    let all: Vec<String> = secs.iter().map(f64::to_string).collect();
    println!(
        "{{\"tool\": \"cost\", \"kernel\": \"{kernel}\", \"graph\": \"{}\", \"threads\": 1, \
         \"load_seconds\": {load}, \"seconds\": {}, \"seconds_min\": {}, \"seconds_all\": [{}]}}",
        graph.file_name().unwrap_or_default().to_string_lossy(),
        sorted[sorted.len() / 2],
        sorted[0],
        all.join(", ")
    );
}
