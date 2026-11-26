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
//!     --kernel pr32|pr64|eig|wcc|cc|cc-one-pass|kcore|bfs|bfs-parents|msbfs|sssp|scc|tarjan
//!     --threads T [--reps R]
//!     [--iters I] [--chunk ENTRIES] [--batch ENTRIES] [--costs uniform|skewed|file]
//!     [--delta-scale X] [--in same|DIR] [--in-store same|STORE]
//! ```
//!
//! `scc` is the strong components of #223 on the out-lists of `--graph` and the in-lists
//! of `--in` (a CSR directory; `same` for a symmetric graph), or of `--in-store` for the
//! compressed sources. `tarjan` is its sequential baseline, Tarjan's algorithm on the
//! raw out-lists in the heap, labelled the same way, so the two hashes are equal.
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
use everygraph_kernels::scc::scc;
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

/// A little-endian array file of `W`-byte words, read in pieces of 64 MiB, so that the
/// bytes and the words are never both whole in memory.
fn read_words<const W: usize, T>(path: &Path, from: fn([u8; W]) -> T) -> Vec<T> {
    use std::io::Read;
    let fail = |e: std::io::Error| -> ! { panic!("{}: {e}", path.display()) };
    let mut file = std::fs::File::open(path).unwrap_or_else(|e| fail(e));
    let len = file.metadata().unwrap_or_else(|e| fail(e)).len() as usize / W;
    let mut out = Vec::with_capacity(len);
    let mut piece = vec![0_u8; W << 24];
    while out.len() < len {
        let bytes = (len - out.len()).min(1 << 24) * W;
        file.read_exact(&mut piece[..bytes])
            .unwrap_or_else(|e| fail(e));
        out.extend(
            piece[..bytes]
                .chunks_exact(W)
                .map(|c| from(c.try_into().expect("W bytes"))),
        );
    }
    out
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

/// Times `reps` runs of `scc` on the out-lists `out` and in-lists `inn`.
fn time_pair<S: Source>(out: &S, inn: &S, a: &Args, settings: Settings) -> (Vec<f64>, u64, String) {
    let memory = Memory::unlimited();
    let reps: usize = a.get("reps").map_or(3, |s| s.parse().expect("--reps"));
    Plan::with_pair(out, inn, &memory, settings, |po, pi| {
        let mut secs = Vec::new();
        let mut labels = Vec::new();
        for _ in 0..reps {
            let t = Instant::now();
            let (l, _held) = scc(&po, &pi, &memory).expect("scc");
            secs.push(t.elapsed().as_secs_f64());
            labels = l.into_iter().map(AtomicU32::into_inner).collect();
        }
        let r = reps.max(1) as u64;
        let (wo, wi) = (po.work(), pi.work());
        let work = format!(
            "\"out_lists\": {}, \"out_entries\": {}, \"in_lists\": {}, \"in_entries\": {}",
            wo.lists / r,
            wo.entries / r,
            wi.lists / r,
            wi.entries / r
        );
        let (hash, summary) = strong_summary(&labels, &work);
        (secs, hash, summary)
    })
    .expect("two adjacencies of one graph")
}

/// The hash of strong-component labels, and their number and largest size.
fn strong_summary(labels: &[u32], more: &str) -> (u64, String) {
    let mut size = vec![0_u32; labels.len()];
    for &l in labels {
        size[l as usize] += 1;
    }
    let components = size.iter().filter(|&&s| s > 0).count();
    let largest = size.iter().copied().max().unwrap_or(0);
    let sep = if more.is_empty() { "" } else { ", " };
    (
        fnv(labels.iter().map(|&l| u64::from(l))),
        format!("{{\"components\": {components}, \"largest\": {largest}{sep}{more}}}"),
    )
}

/// Strong components by Tarjan's algorithm, iterative and sequential: the baseline of
/// `scc`, labelled by the smallest node of each component. A node is on the stack
/// while it has an index and no label.
fn tarjan(offsets: &[u64], targets: &[u32]) -> Vec<u32> {
    const NONE: u32 = u32::MAX;
    let n = offsets.len() - 1;
    let (mut index, mut low, mut label) = (vec![NONE; n], vec![0_u32; n], vec![NONE; n]);
    let (mut stack, mut call) = (Vec::<u32>::new(), Vec::<(u32, u64)>::new());
    let mut next = 0_u32;
    for root in 0..n {
        if index[root] != NONE {
            continue;
        }
        index[root] = next;
        low[root] = next;
        next += 1;
        stack.push(root as u32);
        call.push((root as u32, offsets[root]));
        while let Some(top) = call.last_mut() {
            let v = top.0 as usize;
            if top.1 < offsets[v + 1] {
                let w = targets[top.1 as usize] as usize;
                top.1 += 1;
                if index[w] == NONE {
                    index[w] = next;
                    low[w] = next;
                    next += 1;
                    stack.push(w as u32);
                    call.push((w as u32, offsets[w]));
                } else if label[w] == NONE {
                    low[v] = low[v].min(index[w]);
                }
                continue;
            }
            call.pop();
            if let Some(&(p, _)) = call.last() {
                low[p as usize] = low[p as usize].min(low[v]);
            }
            if low[v] == index[v] {
                let at = stack
                    .iter()
                    .rposition(|&u| u as usize == v)
                    .expect("on the stack");
                let smallest = stack[at..].iter().copied().min().expect("v itself");
                for &u in &stack[at..] {
                    label[u as usize] = smallest;
                }
                stack.truncate(at);
            }
        }
    }
    label
}

/// `scc` or `tarjan` on the graph of `a`: the load time, and the times and summary.
fn strong(
    a: &Args,
    graph: &Path,
    source: &str,
    offsets: &[u64],
    settings: Settings,
    t: Instant,
) -> (f64, (Vec<f64>, u64, String)) {
    let same = |key: &str| a.get(key).is_none_or(|v| v == "same");
    let targets = |dir: &Path| read_words(&dir.join("targets.u32"), u32::from_le_bytes);
    if a.get("kernel").is_some_and(|k| k == "tarjan") {
        assert_eq!(source, "raw", "tarjan reads raw lists in the heap");
        let targets = targets(graph);
        let load = t.elapsed().as_secs_f64();
        let reps: usize = a.get("reps").map_or(3, |s| s.parse().expect("--reps"));
        let mut secs = Vec::new();
        let mut labels = Vec::new();
        for _ in 0..reps {
            let t = Instant::now();
            labels = tarjan(offsets, &targets);
            secs.push(t.elapsed().as_secs_f64());
        }
        let (hash, summary) = strong_summary(&labels, "");
        return (load, (secs, hash, summary));
    }
    let in_dir = (!same("in")).then(|| PathBuf::from(&a["in"]));
    let in_offsets = in_dir
        .as_ref()
        .map(|d| read_words(&d.join("offsets.u64"), u64::from_le_bytes));
    let in_offsets = in_offsets.as_deref().unwrap_or(offsets);
    match source {
        "raw" => {
            let out = targets(graph);
            let inn = in_dir.as_deref().map(targets);
            let inn = inn.as_deref().unwrap_or(&out);
            let (out, inn) = (
                Csr::new(offsets, &out).expect("a CSR"),
                Csr::new(in_offsets, inn).expect("a CSR"),
            );
            (
                t.elapsed().as_secs_f64(),
                time_pair(&out, &inn, a, settings),
            )
        }
        "compressed" | "mmap" => {
            let open = |store: &Path| {
                let (index, data) = store_paths(store);
                let index = std::fs::read(&index)
                    .unwrap_or_else(|e| panic!("{}: {e} (run `encode` first)", index.display()));
                (AdjacencyIndex::parse(&index).expect("an index"), data)
            };
            let out = open(Path::new(&a["store"]));
            let inn = (!same("in-store")).then(|| open(Path::new(&a["in-store"])));
            let inn = inn.as_ref().unwrap_or(&out);
            if source == "mmap" {
                let maps = [&out.1, &inn.1].map(|d| MmapSource::open(d).expect("map"));
                let o = Compressed::mapped(&out.0, &maps[0]).expect("data part");
                let i = Compressed::mapped(&inn.0, &maps[1]).expect("data part");
                return (t.elapsed().as_secs_f64(), time_pair(&o, &i, a, settings));
            }
            let heap = [&out.1, &inn.1].map(|d| std::fs::read(d).expect("data"));
            let o = Compressed::new(Adjacency::new(&out.0, &heap[0]).expect("data part"));
            let i = Compressed::new(Adjacency::new(&inn.0, &heap[1]).expect("data part"));
            (t.elapsed().as_secs_f64(), time_pair(&o, &i, a, settings))
        }
        "stream" => {
            let in_dir = in_dir.as_deref().unwrap_or(graph);
            let files =
                [graph, in_dir].map(|d| FileSource::open(d.join("targets.u32")).expect("targets"));
            let out = Stream::new(offsets, &files[0]).expect("a CSR");
            let inn = Stream::new(in_offsets, &files[1]).expect("a CSR");
            (
                t.elapsed().as_secs_f64(),
                time_pair(&out, &inn, a, settings),
            )
        }
        other => panic!("unknown --source {other}"),
    }
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
    let kernel = a.get("kernel").map_or("pr32", String::as_str);
    if kernel == "scc" || kernel == "tarjan" {
        let result =
            par::with_threads(threads, || strong(a, &graph, source, &offsets, settings, t));
        print_run(
            a,
            &graph,
            source,
            threads,
            settings,
            result.expect("a thread pool"),
        );
        return;
    }
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
    print_run(a, &graph, source, threads, settings, result);
}

/// The JSON line of one `run`.
fn print_run(
    a: &Args,
    graph: &Path,
    source: &str,
    threads: NonZeroUsize,
    settings: Settings,
    (load, (secs, hash, summary)): (f64, (Vec<f64>, u64, String)),
) {
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
