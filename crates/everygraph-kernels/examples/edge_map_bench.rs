// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The benchmark of the edge-map framework (#44, `bench/specs/edge-map.toml`): the proof
//! kernels on a CSR directory of the dataset registry (`offsets.u64`, `targets.u32`),
//! over one source, timed inside the process like the lab's `glab-cli run`.
//!
//! ```text
//! edge_map_bench encode --graph DIR --out STORE
//! edge_map_bench run --graph DIR [--store STORE] --source raw|compressed|mmap|stream
//!     --kernel pr32|pr64|wcc|bfs --threads T [--reps R] [--iters I]
//!     [--chunk ENTRIES] [--batch ENTRIES]
//! ```
//!
//! `run` prints one JSON line: the times of every repetition, the best, and a hash of
//! the result, which is the same for every source and thread count. Sources: `raw`,
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
use std::sync::atomic::AtomicU32;
use std::time::Instant;

use everygraph_core::{Memory, par};
use everygraph_format::adjacency::{Adjacency, AdjacencyIndex, encode};
use everygraph_io::{FileSource, MmapSource};
use everygraph_kernels::edge_map::{Compressed, Csr, Plan, Settings, Source, Stream};
use everygraph_kernels::proof::{self, UNREACHED};

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
    let mut secs = Vec::new();
    let mut last = (0, String::new());
    for _ in 0..reps {
        let t = Instant::now();
        last = match kernel {
            "pr32" | "pr64" => {
                let (hash, sum) = if kernel == "pr32" {
                    let r = proof::pagerank::<_, f32>(plan, memory, 0.85, iters).expect("pagerank");
                    secs.push(t.elapsed().as_secs_f64());
                    let sum: f64 = r.iter().map(|&x| f64::from(x)).sum();
                    (fnv(r.iter().map(|x| u64::from(x.to_bits()))), sum)
                } else {
                    let r = proof::pagerank::<_, f64>(plan, memory, 0.85, iters).expect("pagerank");
                    secs.push(t.elapsed().as_secs_f64());
                    (fnv(r.iter().map(|x| x.to_bits())), r.iter().sum())
                };
                (hash, format!("{{\"iters\": {iters}, \"sum\": {sum}}}"))
            }
            "wcc" => {
                let (labels, _held) = proof::wcc(plan, memory).expect("wcc");
                secs.push(t.elapsed().as_secs_f64());
                let labels: Vec<u32> = labels.into_iter().map(AtomicU32::into_inner).collect();
                let components = labels.iter().zip(0..).filter(|&(&l, v)| l == v).count();
                (
                    fnv(labels.iter().map(|&l| u64::from(l))),
                    format!("{{\"components\": {components}}}"),
                )
            }
            "bfs" => {
                let (depth, _held) = proof::bfs(plan, memory, hub, true).expect("bfs");
                secs.push(t.elapsed().as_secs_f64());
                let depth: Vec<u32> = depth.into_iter().map(AtomicU32::into_inner).collect();
                let reached = depth.iter().filter(|&&d| d != UNREACHED).count();
                let sum: u64 = depth
                    .iter()
                    .filter(|&&d| d != UNREACHED)
                    .map(|&d| u64::from(d))
                    .sum();
                let summary =
                    format!("{{\"source\": {hub}, \"reached\": {reached}, \"depth_sum\": {sum}}}");
                (fnv(depth.iter().map(|&d| u64::from(d))), summary)
            }
            other => panic!("unknown --kernel {other}"),
        };
    }
    (secs, last.0, last.1)
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
