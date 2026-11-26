// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Everygraph's kernels as bench/specs/references.toml compares them with the reference
//! tools: the product functions, called as
//! `crates/everygraph-kernels/examples/edge_map_bench.rs --source raw` calls them, plus
//! what that example cannot do (searches from a list of sources, the result written out)
//! and a load that does not hold the graph twice.
//!
//! ```text
//! eg-driver --graph DIR --kernel pr64|cc|kcore|bfs|sssp --threads T
//!     [--reps R] [--sources FILE] [--dump FILE]
//! ```
//!
//! - `pr64`: PageRank in f64, damping 0.85, 20 iterations, tol 0 (strength included);
//! - `cc`: components with hybrid sampling (`edge_map_bench --kernel cc`);
//! - `kcore`: core numbers (Liu et al. SIGMOD 2025);
//! - `bfs`: direction-optimizing BFS with parents (the output of GAP's and GBBS's BFS),
//!   from each source of the file;
//! - `sssp`: Δ-stepping from each source, the bucket width of the data rule, on the
//!   costs of `edge_map_bench --costs uniform` (the weights `run.py convert wsg` gives
//!   GAP).
//!
//! Prints one JSON line: `load_seconds` (reading the arrays into the heap),
//! `seconds_all`, and `seconds`: the median of `--reps` runs for the whole-graph
//! kernels (`seconds_min` beside it), the mean over the sources for the searches.
//! `hash` is the FNV-1a hash of the last result, as `edge_map_bench` computes it for
//! `pr64`, `cc` and `kcore`. `--dump` writes the last result, or the first source's, as
//! little-endian words: ranks and SSSP distances as f64, labels, core numbers and BFS
//! distances as u32.

#![forbid(unsafe_code)]
#![allow(clippy::print_stdout, clippy::cast_precision_loss)]

use std::collections::HashMap;
use std::io::Read;
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::time::Instant;

use everygraph_core::{Memory, par};
use everygraph_kernels::components::{WccOptions, wcc};
use everygraph_kernels::edge_map::{Csr, Plan, Settings, Source};
use everygraph_kernels::kcore::kcore;
use everygraph_kernels::pagerank::{Graph, PageRank, pagerank, strength};
use everygraph_kernels::sssp::{Sssp, delta, sssp};
use everygraph_kernels::traversal::{Bfs, bfs};

/// Reads little-endian words in blocks, so the peak is the array and one block.
fn read_words<const W: usize, T>(path: &Path, from: fn([u8; W]) -> T) -> Vec<T> {
    let fail = |e: std::io::Error| -> ! { panic!("{}: {e}", path.display()) };
    let mut file = std::fs::File::open(path).unwrap_or_else(|e| fail(e));
    let len = file.metadata().unwrap_or_else(|e| fail(e)).len();
    let mut out = Vec::with_capacity(usize::try_from(len).expect("a file in memory") / W);
    let mut block = vec![0_u8; W << 20];
    loop {
        let mut got = 0;
        while got < block.len() {
            match file.read(&mut block[got..]).unwrap_or_else(|e| fail(e)) {
                0 => break,
                k => got += k,
            }
        }
        out.extend(
            block[..got]
                .chunks_exact(W)
                .map(|c| from(c.try_into().expect("W bytes"))),
        );
        if got < block.len() {
            return out;
        }
    }
}

const FNV_START: u64 = 0xCBF2_9CE4_8422_2325;

/// A result: its words, for the hash, and their width in bytes, for the dump.
struct Output {
    words: Vec<u64>,
    width: usize,
}

impl Output {
    fn u32s(cells: &[AtomicU32]) -> Self {
        let words = cells.iter().map(|c| u64::from(c.load(Relaxed))).collect();
        Self { words, width: 4 }
    }

    fn hash(&self) -> u64 {
        self.words.iter().fold(FNV_START, |h, &w| {
            (h ^ w).wrapping_mul(0x0000_0100_0000_01B3)
        })
    }

    fn dump(&self, path: &str) {
        let bytes: Vec<u8> = self
            .words
            .iter()
            .flat_map(|w| w.to_le_bytes()[..self.width].to_vec())
            .collect();
        std::fs::write(path, bytes).unwrap_or_else(|e| panic!("{path}: {e}"));
    }
}

/// `cost(e, false)` of `edge_map_bench`.
fn cost(e: u64) -> f64 {
    let mut z = e.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (1 + z % 255) as f64
}

/// One run of a whole-graph kernel.
fn whole<S: Source>(plan: &Plan<'_, '_, S>, kernel: &str, memory: &Memory) -> Output {
    match kernel {
        "pr64" => {
            let s = strength(plan, None, false, memory).expect("strength");
            let p = PageRank {
                tol: 0.0,
                max_iter: 20,
                ..PageRank::default()
            };
            let g = Graph {
                lists: plan,
                weights: None,
                directed: false,
            };
            let r = pagerank::<_, f64>(&g, s.as_slice().expect("one segment"), &p, memory)
                .expect("pagerank");
            let words = r.values.iter().map(|x| x.to_bits()).collect();
            Output { words, width: 8 }
        }
        "cc" => {
            let options = WccOptions {
                sample: true,
                ..WccOptions::default()
            };
            Output::u32s(&wcc(plan, memory, options).expect("wcc").0)
        }
        "kcore" => Output::u32s(&kcore(plan, memory).expect("kcore").0),
        other => panic!("unknown --kernel {other}"),
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
    let threads =
        NonZeroUsize::new(a["threads"].parse().expect("--threads")).expect("--threads > 0");
    let reps: usize = a.get("reps").map_or(5, |r| r.parse().expect("--reps"));
    let search = matches!(kernel, "bfs" | "sssp");
    let sources: Vec<u32> = if search {
        std::fs::read_to_string(&a["sources"])
            .expect("--sources FILE")
            .split_whitespace()
            .map(|s| s.parse().expect("a node id"))
            .collect()
    } else {
        Vec::new()
    };

    let t = Instant::now();
    let offsets = read_words(&graph.join("offsets.u64"), u64::from_le_bytes);
    let targets = read_words(&graph.join("targets.u32"), u32::from_le_bytes);
    let csr = Csr::new(&offsets, &targets).expect("a CSR");
    let load = t.elapsed().as_secs_f64();
    let costs: Vec<f64> = if kernel == "sssp" {
        (0..targets.len() as u64).map(cost).collect()
    } else {
        Vec::new()
    };

    let memory = Memory::unlimited();
    let (secs, out) = par::with_threads(threads, || {
        Plan::with(&csr, &memory, Settings::default(), |plan| {
            let mut secs = Vec::new();
            let mut out = None;
            if !search {
                for _ in 0..reps {
                    let t = Instant::now();
                    let o = whole(&plan, kernel, &memory);
                    secs.push(t.elapsed().as_secs_f64());
                    out = Some(o);
                }
                return (secs, out);
            }
            let width = (kernel == "sssp").then(|| delta(&plan, &costs));
            for &s in &sources {
                let t = Instant::now();
                let o = if kernel == "bfs" {
                    let p = Bfs {
                        parents: true,
                        ..Bfs::default()
                    };
                    let hops = bfs(&plan, Some(&plan), s, &p, &memory).expect("bfs");
                    secs.push(t.elapsed().as_secs_f64());
                    Output::u32s(&hops.distance)
                } else {
                    let d = sssp(&plan, &costs, s, &Sssp { delta: width }, &memory).expect("sssp");
                    secs.push(t.elapsed().as_secs_f64());
                    let words = d.bits.iter().map(|b| b.load(Relaxed)).collect();
                    Output { words, width: 8 }
                };
                out.get_or_insert(o);
            }
            (secs, out)
        })
    })
    .expect("a thread pool");
    let out = out.expect("at least one run");
    if let Some(path) = a.get("dump") {
        out.dump(path);
    }
    let mut sorted = secs.clone();
    sorted.sort_by(f64::total_cmp);
    let seconds = if search {
        secs.iter().sum::<f64>() / secs.len() as f64
    } else {
        sorted[sorted.len() / 2]
    };
    let all: Vec<String> = secs.iter().map(f64::to_string).collect();
    println!(
        "{{\"tool\": \"everygraph\", \"kernel\": \"{kernel}\", \"graph\": \"{}\", \"threads\": {threads}, \
         \"sources\": {}, \"load_seconds\": {load}, \"seconds\": {seconds}, \"seconds_min\": {}, \
         \"seconds_all\": [{}], \"hash\": \"{:016x}\"}}",
        graph.file_name().unwrap_or_default().to_string_lossy(),
        sources.len(),
        sorted[0],
        all.join(", "),
        out.hash()
    );
}
