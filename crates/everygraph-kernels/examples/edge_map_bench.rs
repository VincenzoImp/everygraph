// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The benchmark of the edge-map framework and its kernels (#44, #47–#50,
//! `bench/specs/edge-map.toml`, `bench/specs/components-kcore.toml`): the kernels on a
//! CSR directory of the dataset registry
//! (`offsets.u64`, `targets.u32`), over one source, timed inside the process like the
//! lab's `glab-cli run`. SSSP runs on synthetic costs of the entries (`cost`), or with
//! `--costs file` on the graph's own (`costs.f64`, one per entry), with the bucket width
//! of `sssp::delta` times `--delta-scale`.
//!
//! ```text
//! edge_map_bench encode --graph DIR --out STORE
//! edge_map_bench run --graph DIR [--store STORE] --source raw|compressed|mmap|stream
//!     --kernel pr32|pr64|eig|wcc|cc|cc-one-pass|kcore|bfs|bfs-parents|msbfs|sssp|decode
//!     --threads T [--reps R]
//!     [--iters I] [--chunk ENTRIES] [--batch ENTRIES] [--costs uniform|skewed|file]
//!     [--delta-scale X]
//! ```
//!
//! `run` prints one JSON line: the times of every repetition, the best, and a hash of
//! the result, which is the same for every source and thread count. `wcc` is the lab's
//! Afforest (first neighbours only, `proof`), `cc` the components of #48 with hybrid
//! sampling, `cc-one-pass` the same without sampling. Sources: `raw`,
//! arrays read into the heap; `compressed`, the store's data part read into the heap;
//! `mmap`, the data part memory-mapped (warm after the first repetition); `stream`,
//! the raw targets read through the page cache with positional reads.

#![forbid(unsafe_code)]
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation
)]

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::time::Instant;

use everygraph_core::{Memory, par};
use everygraph_format::adjacency::{Adjacency, AdjacencyIndex, encode};
use everygraph_io::{FileSource, MmapSource};
use everygraph_kernels::components::{WccOptions, wcc};
use everygraph_kernels::edge_map::{Compressed, Csr, Plan, Settings, Source, Stream};
use everygraph_kernels::kcore::kcore;
use everygraph_kernels::pagerank::{Eigenvector, Graph, PageRank, Real, eigenvector, strength};
use everygraph_kernels::proof;
use everygraph_kernels::sssp::{Sssp, delta, sssp};
use everygraph_kernels::traversal::{Bfs, UNREACHED, bfs, multi_bfs};

type Args = HashMap<String, String>;

fn args() -> (String, Args) {
    let mut it = std::env::args().skip(1);
    let cmd = it.next().unwrap_or_default();
    let mut a = HashMap::new();
    while let Some(k) = it.next() {
        let v = it.next().unwrap_or_default();
        a.insert(k.trim_start_matches("--").to_owned(), v);
    }
    (cmd, a)
}

fn read_words<const W: usize, T>(path: &Path, from: fn([u8; W]) -> T) -> Vec<T> {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    bytes
        .chunks_exact(W)
        .map(|c| from(c.try_into().expect("W bytes")))
        .collect()
}

fn fnv(words: impl Iterator<Item = u64>) -> u64 {
    words.fold(0xCBF2_9CE4_8422_2325, |h, w| {
        (h ^ w).wrapping_mul(0x0000_0100_0000_01B3)
    })
}

/// Times `reps` runs of `kernel` on `source`; returns the times and the last result's
/// hash and summary.
fn time<S: Source>(source: &S, a: &Args, settings: Settings) -> (Vec<f64>, u64, String) {
    let memory = Memory::unlimited();
    Plan::with(source, &memory, settings, |plan| {
        time_plan(&plan, a, &memory)
    })
}

fn time_plan<S: Source>(
    plan: &Plan<'_, '_, S>,
    a: &Args,
    memory: &Memory,
) -> (Vec<f64>, u64, String) {
    let source = plan.source();
    let reps: usize = a.get("reps").map_or(3, |s| s.parse().expect("--reps"));
    let iters: u32 = a.get("iters").map_or(20, |s| s.parse().expect("--iters"));
    let kernel = a.get("kernel").map_or("pr32", String::as_str);
    let n = source.nodes();
    let hub = (0..n)
        .max_by_key(|&v| (source.degree(v), std::cmp::Reverse(v)))
        .unwrap_or(0);
    let costs: Vec<f64> = match kernel {
        "sssp" => match a.get("costs").map(String::as_str) {
            Some("file") => {
                let path = Path::new(&a["graph"]).join("costs.f64");
                read_words(&path, f64::from_le_bytes)
            }
            costs => {
                let skewed = costs == Some("skewed");
                (0..source.entries()).map(|e| cost(e, skewed)).collect()
            }
        },
        _ => Vec::new(),
    };
    let mut secs = Vec::new();
    let mut last = (0, String::new());
    for _ in 0..reps {
        let t = Instant::now();
        last = match kernel {
            "pr32" => {
                let r = pagerank::<_, f32>(plan, memory, iters);
                secs.push(t.elapsed().as_secs_f64());
                let sum: f64 = r.iter().map(|&x| f64::from(x)).sum();
                let hash = fnv(r.iter().map(|x| u64::from(x.to_bits())));
                (hash, format!("{{\"iters\": {iters}, \"sum\": {sum}}}"))
            }
            "pr64" => {
                let r = pagerank::<_, f64>(plan, memory, iters);
                secs.push(t.elapsed().as_secs_f64());
                let sum: f64 = r.iter().sum();
                let hash = fnv(r.iter().map(|x| x.to_bits()));
                (hash, format!("{{\"iters\": {iters}, \"sum\": {sum}}}"))
            }
            "eig" => {
                let p = Eigenvector {
                    tol: 0.0,
                    max_iter: iters,
                };
                let r = eigenvector(&graph(plan), &p, memory).expect("eigenvector");
                secs.push(t.elapsed().as_secs_f64());
                let hash = fnv(r.values.iter().map(|x| x.to_bits()));
                (hash, format!("{{\"iters\": {iters}}}"))
            }
            "wcc" | "cc" | "cc-one-pass" => {
                let (labels, _held) = match kernel {
                    "wcc" => proof::wcc(plan, memory),
                    _ => {
                        let sample = kernel == "cc";
                        let options = WccOptions {
                            sample,
                            ..WccOptions::default()
                        };
                        wcc(plan, memory, options)
                    }
                }
                .expect("wcc");
                secs.push(t.elapsed().as_secs_f64());
                let labels: Vec<u32> = labels.into_iter().map(AtomicU32::into_inner).collect();
                let components = labels.iter().zip(0..).filter(|&(&l, v)| l == v).count();
                (
                    fnv(labels.iter().map(|&l| u64::from(l))),
                    format!("{{\"components\": {components}}}"),
                )
            }
            "bfs" | "bfs-parents" => {
                let p = Bfs {
                    parents: kernel == "bfs-parents",
                    ..Bfs::default()
                };
                let hops = bfs(plan, Some(plan), hub, &p, memory).expect("bfs");
                secs.push(t.elapsed().as_secs_f64());
                let depth: Vec<u32> = hops
                    .distance
                    .into_iter()
                    .map(AtomicU32::into_inner)
                    .collect();
                let reached = depth.iter().filter(|&&d| d != UNREACHED).count();
                let sum: u64 = depth
                    .iter()
                    .filter(|&&d| d != UNREACHED)
                    .map(|&d| u64::from(d))
                    .sum();
                let parents = hops
                    .parent
                    .map_or(0, |p| fnv(p.into_iter().map(|x| u64::from(x.into_inner()))));
                let summary = format!(
                    "{{\"source\": {hub}, \"reached\": {reached}, \"depth_sum\": {sum}, \
                     \"levels\": {}, \"parents_hash\": \"{parents:016x}\"}}",
                    hops.levels.len()
                );
                (fnv(depth.iter().map(|&d| u64::from(d))), summary)
            }
            "msbfs" => {
                let sources: Vec<u32> = (0..64_u64)
                    .map(|k| u32::try_from(k * u64::from(n) / 64).expect("a node"))
                    .collect();
                let r = multi_bfs(plan, &sources, None, memory).expect("multi-source bfs");
                secs.push(t.elapsed().as_secs_f64());
                let hash = fnv(r.distance_sum.iter().copied());
                (
                    hash,
                    format!("{{\"sources\": 64, \"levels\": {}}}", r.levels.len()),
                )
            }
            "sssp" => {
                let scale: f64 = a
                    .get("delta-scale")
                    .map_or(1.0, |d| d.parse().expect("--delta-scale"));
                let p = Sssp {
                    delta: Some(delta(plan, &costs) * scale),
                };
                let before = plan.work().entries;
                let d = sssp(plan, &costs, hub, &p, memory).expect("sssp");
                secs.push(t.elapsed().as_secs_f64());
                let relaxed = plan.work().entries - before;
                let reached = (0..n).filter(|&v| d.get(v).is_some()).count();
                let hash = fnv(d.bits.iter().map(|b| b.load(Relaxed)));
                let summary = format!(
                    "{{\"source\": {hub}, \"reached\": {reached}, \"delta\": {}, \"costs\": \"{}\", \
                     \"entries_relaxed\": {relaxed}}}",
                    d.delta,
                    a.get("costs").map_or("uniform", String::as_str)
                );
                (hash, summary)
            }
            "kcore" => {
                let (core, _held) = kcore(plan, memory).expect("kcore");
                secs.push(t.elapsed().as_secs_f64());
                let core: Vec<u32> = core.into_iter().map(AtomicU32::into_inner).collect();
                let max = core.iter().copied().max().unwrap_or(0);
                let sum: u64 = core.iter().map(|&c| u64::from(c)).sum();
                (
                    fnv(core.iter().map(|&c| u64::from(c))),
                    format!("{{\"max_core\": {max}, \"coreness_sum\": {sum}}}"),
                )
            }
            other => panic!("unknown --kernel {other}"),
        };
    }
    (secs, last.0, last.1)
}

/// `decode`: the whole adjacency decoded into raw arrays, the planner's decode tier (#45).
fn time_decode(adj: &Adjacency<'_>, a: &Args) -> (Vec<f64>, u64, String) {
    let reps: usize = a.get("reps").map_or(3, |s| s.parse().expect("--reps"));
    let index = adj.index();
    let mut offsets = vec![0_u64; index.nodes() as usize + 1];
    let mut targets = vec![0_u32; index.entries() as usize];
    let mut secs = Vec::new();
    for _ in 0..reps {
        let t = Instant::now();
        adj.decode_all(&mut offsets, &mut targets).expect("decode");
        secs.push(t.elapsed().as_secs_f64());
    }
    let hash = fnv(targets.iter().map(|&x| u64::from(x)));
    (secs, hash, format!("{{\"entries\": {}}}", targets.len()))
}

/// The graph as `PageRank` pulls it: the benchmark graphs are undirected.
fn graph<'p, 's, 'g, S: Source>(plan: &'p Plan<'s, 'g, S>) -> Graph<'p, 's, 'g, S> {
    Graph {
        lists: plan,
        weights: None,
        directed: false,
    }
}

/// `iters` iterations of `PageRank` at damping 0.85, as the lab runs it.
fn pagerank<S: Source, F: Real>(plan: &Plan<'_, '_, S>, memory: &Memory, iters: u32) -> Vec<F> {
    let s = strength(plan, None, false, memory).expect("strength");
    let p = PageRank {
        tol: 0.0,
        max_iter: iters,
        ..PageRank::default()
    };
    let s = s.as_slice().expect("one segment");
    let r = everygraph_kernels::pagerank::pagerank::<_, F>(&graph(plan), s, &p, memory);
    r.expect("pagerank").values.iter().copied().collect()
}

/// A synthetic cost for entry `e`: an integer from 1 to 255, or with `skewed` a power of
/// two from 2^0 to 2^19.
fn cost(e: u64, skewed: bool) -> f64 {
    let mut z = e.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    if skewed {
        f64::from(1_u32 << (z % 20))
    } else {
        (1 + z % 255) as f64
    }
}

fn store_paths(store: &Path) -> (PathBuf, PathBuf) {
    (store.join("adjacency.index"), store.join("adjacency.data"))
}

fn run(a: &Args) {
    let graph = PathBuf::from(&a["graph"]);
    let source = a.get("source").map_or("raw", String::as_str);
    let threads = NonZeroUsize::new(
        a.get("threads")
            .map_or(1, |s| s.parse().expect("--threads")),
    )
    .expect("--threads > 0");
    let settings = Settings {
        chunk_entries: a
            .get("chunk")
            .map_or(1 << 16, |s| s.parse().expect("--chunk")),
        batch_entries: a.get("batch").map_or(8192, |s| s.parse().expect("--batch")),
        prefetch: false,
    };
    let t = Instant::now();
    let offsets = read_words(&graph.join("offsets.u64"), u64::from_le_bytes);
    let result = par::with_threads(threads, || match source {
        "raw" => {
            let targets = read_words(&graph.join("targets.u32"), u32::from_le_bytes);
            let csr = Csr::new(&offsets, &targets).expect("a CSR");
            let load = t.elapsed().as_secs_f64();
            (load, time(&csr, a, settings))
        }
        "compressed" | "mmap" => {
            let (index, data) = store_paths(Path::new(&a["store"]));
            let index = std::fs::read(&index)
                .unwrap_or_else(|e| panic!("{}: {e} (run `encode` first)", index.display()));
            let index = AdjacencyIndex::parse(&index).expect("an index");
            let map = MmapSource::open(&data).expect("map");
            let mapped = source == "mmap";
            let heap = if mapped {
                Vec::new()
            } else {
                std::fs::read(&data).expect("data")
            };
            if a.get("kernel").map(String::as_str) == Some("decode") {
                let bytes = if mapped { map.mapped() } else { &heap[..] };
                let adj = Adjacency::new(&index, bytes).expect("data part");
                return (t.elapsed().as_secs_f64(), time_decode(&adj, a));
            }
            let adj = if mapped {
                Compressed::mapped(&index, &map).expect("data part")
            } else {
                Compressed::new(Adjacency::new(&index, &heap).expect("data part"))
            };
            let load = t.elapsed().as_secs_f64();
            (load, time(&adj, a, settings))
        }
        "stream" => {
            let file = FileSource::open(graph.join("targets.u32")).expect("targets");
            let stream = Stream::new(&offsets, &file).expect("a CSR");
            let load = t.elapsed().as_secs_f64();
            (load, time(&stream, a, settings))
        }
        other => panic!("unknown --source {other}"),
    })
    .expect("a thread pool");
    let (load, (secs, hash, summary)) = result;
    let best = secs.iter().copied().fold(f64::INFINITY, f64::min);
    let all: Vec<String> = secs.iter().map(f64::to_string).collect();
    println!(
        "{{\"graph\": \"{}\", \"source\": \"{source}\", \"kernel\": \"{}\", \"threads\": {threads}, \
         \"chunk\": {}, \"batch\": {}, \"load_seconds\": {load}, \"seconds_all\": [{}], \
         \"seconds_best\": {best}, \"hash\": \"{hash:016x}\", \"result\": {summary}}}",
        graph.file_name().unwrap_or_default().to_string_lossy(),
        a.get("kernel").map_or("pr32", String::as_str),
        settings.chunk_entries,
        settings.batch_entries,
        all.join(", "),
    );
}

fn encode_store(a: &Args) {
    let graph = PathBuf::from(&a["graph"]);
    let out = PathBuf::from(&a["out"]);
    let offsets = read_words(&graph.join("offsets.u64"), u64::from_le_bytes);
    let targets = read_words(&graph.join("targets.u32"), u32::from_le_bytes);
    let encoded = encode(&offsets, &targets).expect("encode");
    std::fs::create_dir_all(&out).expect("store directory");
    let (index, data) = store_paths(&out);
    std::fs::write(index, &encoded.index).expect("write index");
    std::fs::write(data, &encoded.data).expect("write data");
    println!(
        "{{\"entries\": {}, \"data_bytes\": {}}}",
        targets.len(),
        encoded.data.len()
    );
}

fn main() -> ExitCode {
    let (cmd, a) = args();
    match cmd.as_str() {
        "run" => run(&a),
        "encode" => encode_store(&a),
        _ => {
            eprintln!("usage: edge_map_bench encode|run --graph DIR ... (see the source)");
            return ExitCode::from(2);
        }
    }
    ExitCode::SUCCESS
}
