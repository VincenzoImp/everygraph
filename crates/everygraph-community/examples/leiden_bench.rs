// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The benchmark of production Leiden (#57, `bench/specs/leiden.toml`) on a CSR directory
//! of the dataset registry (`offsets.u64`, `targets.u32`), over one source, timed inside
//! the process like the lab's `b2` (the time excludes loading and the evaluation).
//!
//! ```text
//! leiden_bench encode --graph DIR --out STORE
//! leiden_bench run --graph DIR [--store STORE] [--source raw|compressed|mmap|stream|eg]
//!     [--threads T] [--reps R] [--mode det|fast] [--iterations K|converge]
//!     [--seed S] [--refine greedy|random|louvain] [--theta X] [--prolong 1]
//!     [--split last|every]
//!     [--chunk ENTRIES] [--budget-gib G] [--check 1] [--hier FILE]
//! ```
//!
//! `run` prints one JSON line: the times of every repetition, the modularity, the
//! number of communities, the hash of the levels (the same for every source, thread
//! count and chunking in the deterministic mode), the statistics of each level, and with
//! `--check 1` the modularity evaluated again on the result and the number of
//! disconnected communities. `--budget-gib` bounds what Leiden reserves (not the graph,
//! which is loaded first). `--hier FILE` then stores the hierarchy (#62) in a new
//! container `FILE` and reports its time, its bytes, the budget's peak while Leiden ran
//! and while the hierarchy was written, and the time to read the table of each level.
//! With `--source eg`, Leiden reads the `adjacency.out` of the graph file `--hier FILE`
//! (built by `everygraph ingest`, undirected), and the hierarchy is stored in it.

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
use std::num::{NonZeroU32, NonZeroUsize};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use everygraph_community::hierarchy::Hierarchy;
use everygraph_community::{
    Communities, Iterations, Mode, Options, Refinement, SplitAt, leiden, modularity,
};
use everygraph_core::{Budget, ByteSource, Memory, MemorySource, par};
use everygraph_format::adjacency::{Adjacency, AdjacencyIndex, encode};
use everygraph_format::container::{Container, Provenance, Writer};
use everygraph_io::{FileSink, FileSource, MmapSource};
use everygraph_kernels::edge_map::{Compressed, Csr, Settings, Source, Stream};

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

fn fnv(h: u64, words: impl Iterator<Item = u32>) -> u64 {
    words.fold(h, |h, w| {
        (h ^ u64::from(w)).wrapping_mul(0x0000_0100_0000_01B3)
    })
}

fn options(a: &Args) -> Options {
    let mut o = Options::default();
    if a.get("mode").is_some_and(|m| m == "fast") {
        o.mode = Mode::Fast;
    }
    match a.get("iterations").map(String::as_str) {
        None => {}
        Some("converge") => {
            o.iterations = Iterations::Converge {
                max: NonZeroU32::new(50).expect("non-zero"),
            };
        }
        Some(k) => o.iterations = Iterations::Fixed(k.parse().expect("--iterations")),
    }
    if let Some(s) = a.get("seed") {
        o.seed = s.parse().expect("--seed");
    }
    match a.get("refine").map(String::as_str) {
        Some("random") => {
            let theta = a.get("theta").map_or(0.01, |t| t.parse().expect("--theta"));
            o.refinement = Refinement::Randomized { theta };
        }
        Some("louvain") => o.refinement = Refinement::Louvain,
        _ => {}
    }
    o.prolong = a.get("prolong").is_some_and(|p| p == "1");
    if a.get("split").is_some_and(|s| s == "every") {
        o.split = SplitAt::EveryLevel;
    }
    if let Some(g) = a.get("gamma") {
        o.resolution = g.parse().expect("--gamma");
    }
    o
}

/// Communities with more than one component, by a union-find over the edges inside.
fn disconnected<S: Source>(source: &S, membership: &[u32]) -> usize {
    let n = source.nodes() as usize;
    let mut parent: Vec<u32> = (0..n as u32).collect();
    fn find(p: &mut [u32], mut x: u32) -> u32 {
        while p[x as usize] != x {
            p[x as usize] = p[p[x as usize] as usize];
            x = p[x as usize];
        }
        x
    }
    let memory = Memory::unlimited();
    let mut reader = source.reader(&memory, 1 << 16, 1).expect("a reader");
    for u in 0..n as u32 {
        let (_, list) = source.list(&mut reader, u).expect("a list");
        let list = list.to_vec();
        for v in list {
            if membership[u as usize] == membership[v as usize] {
                let (a, b) = (find(&mut parent, u), find(&mut parent, v));
                if a != b {
                    parent[a.max(b) as usize] = a.min(b);
                }
            }
        }
    }
    let mut roots: HashMap<u32, u32> = HashMap::new();
    for v in 0..n as u32 {
        if find(&mut parent, v) == v {
            *roots.entry(membership[v as usize]).or_default() += 1;
        }
    }
    roots.values().filter(|&&r| r > 1).count()
}

/// Stores the hierarchy of `c` in a new container at `path` and reads it back.
fn hierarchy<S: Source>(
    source: &S,
    path: &Path,
    o: &Options,
    c: &Communities,
    settings: Settings,
) -> String {
    let budget = Arc::new(Budget::unlimited());
    let memory = Memory::new(budget.clone());
    let t = Instant::now();
    // Into the graph file itself when Leiden read it (`--source eg`), else a new file.
    let mut w = if path.extension().is_some_and(|e| e == "eg") && path.exists() {
        Writer::update(FileSink::open(path).expect("the graph file")).expect("a container")
    } else {
        Writer::create(FileSink::create(path).expect("the hierarchy file")).expect("a container")
    };
    let key = everygraph_community::hierarchy::key(w.graph_identity(), o).expect("a key");
    let about = Provenance {
        engine: "leiden_bench".into(),
        ..Provenance::default()
    };
    everygraph_community::hierarchy::store(&mut w, &key, &about, source, &memory, settings, c)
        .expect("the hierarchy is stored");
    w.commit().expect("commit");
    drop(w);
    let write = t.elapsed().as_secs_f64();
    let map = MmapSource::open(path).expect("map");
    let file = Container::open(&map).expect("a container");
    let bytes = file.results().expect("results")[0].bytes;
    let h = Hierarchy::find(&file, &key).expect("read").expect("found");
    let reads: Vec<String> = (1..=h.levels())
        .map(|level| {
            let t = Instant::now();
            let table = h.table(level, &memory).expect("a table");
            format!("[{}, {}]", table.nodes.len(), t.elapsed().as_secs_f64())
        })
        .collect();
    format!(
        ", \"hier_write_seconds\": {write}, \"hier_bytes\": {bytes}, \"hier_peak_reserved\": {}, \
         \"hier_table_read_seconds\": [{}]",
        budget.peak(),
        reads.join(", ")
    )
}

fn time<S: Source>(source: &S, a: &Args, settings: Settings) -> String {
    let budget = Arc::new(match a.get("budget-gib") {
        Some(g) => {
            let gib: u64 = g.parse().expect("--budget-gib");
            Budget::new(gib << 30)
        }
        None => Budget::unlimited(),
    });
    let memory = Memory::new(budget.clone());
    let reps: usize = a.get("reps").map_or(1, |s| s.parse().expect("--reps"));
    let o = options(a);
    let mut secs = Vec::new();
    let mut last: Option<Communities> = None;
    for _ in 0..reps {
        drop(last.take());
        let t = Instant::now();
        let c = leiden(source, &memory, settings, &o).expect("leiden");
        secs.push(t.elapsed().as_secs_f64());
        last = Some(c);
    }
    let c = last.expect("one repetition");
    let levels = c
        .levels
        .iter()
        .fold(0xCBF2_9CE4_8422_2325, |h, l| fnv(h, l.iter().copied()));
    let flat = fnv(0xCBF2_9CE4_8422_2325, c.membership.iter().copied());
    let sizes: Vec<String> = c.levels.iter().map(|l| l.len().to_string()).collect();
    let stats: Vec<String> = c
        .stats
        .iter()
        .map(|s| {
            format!(
                "{{\"iteration\": {}, \"level\": {}, \"nodes\": {}, \"entries\": {}, \"sweeps\": {}, \
                 \"moves\": {}, \"parallel_rounds\": {}, \"serial_rounds\": {}, \"communities\": {}, \
                 \"heavy\": {}}}",
                s.iteration,
                s.level,
                s.nodes,
                s.entries,
                s.sweeps,
                s.moves,
                s.parallel_rounds,
                s.serial_rounds,
                s.communities,
                s.heavy
            )
        })
        .collect();
    let check = if a.get("check").is_some_and(|c| c == "1") {
        let q = modularity(source, &memory, settings, &c.membership, o.resolution).expect("q");
        format!(
            ", \"check_modularity\": {q}, \"disconnected\": {}",
            disconnected(source, &c.membership)
        )
    } else {
        String::new()
    };
    let hier = a.get("hier").map_or_else(String::new, |path| {
        format!(
            ", \"leiden_peak_reserved\": {}{}",
            budget.peak(),
            hierarchy(source, Path::new(path), &o, &c, settings)
        )
    });
    let all: Vec<String> = secs.iter().map(f64::to_string).collect();
    let best = secs.iter().copied().fold(f64::INFINITY, f64::min);
    format!(
        "\"mode\": \"{:?}\", \"iterations\": \"{:?}\", \"iterations_run\": {}, \"seed\": {}, \
         \"refinement\": \"{:?}\", \"prolong\": {}, \"split\": \"{:?}\", \
         \"seconds_all\": [{}], \
         \"seconds_best\": {best}, \"modularity\": {}, \"communities\": {}, \"level_sizes\": [{}], \
         \"hash_levels\": \"{levels:016x}\", \"hash_levels_low53\": {}, \"hash_membership\": \
         \"{flat:016x}\", \"stats\": [{}]{check}{hier}",
        c.mode,
        o.iterations,
        c.iterations,
        o.seed,
        o.refinement,
        o.prolong,
        o.split,
        all.join(", "),
        c.modularity,
        c.count,
        sizes.join(", "),
        levels & ((1 << 53) - 1),
        stats.join(", "),
    )
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
        ..Settings::default()
    };
    let t = Instant::now();
    let offsets = read_words(&graph.join("offsets.u64"), u64::from_le_bytes);
    let (load, result) = par::with_threads(threads, || match source {
        "raw" => {
            let targets = read_words(&graph.join("targets.u32"), u32::from_le_bytes);
            let csr = Csr::new(&offsets, &targets).expect("a CSR");
            (t.elapsed().as_secs_f64(), time(&csr, a, settings))
        }
        "compressed" | "mmap" => {
            let (index, data) = store_paths(Path::new(&a["store"]));
            let index = std::fs::read(&index)
                .unwrap_or_else(|e| panic!("{}: {e} (run `encode` first)", index.display()));
            let index = AdjacencyIndex::parse(&index).expect("an index");
            let map = MmapSource::open(&data).expect("map");
            let heap = if source == "mmap" {
                Vec::new()
            } else {
                std::fs::read(&data).expect("data")
            };
            let adj = if source == "mmap" {
                Compressed::mapped(&index, &map).expect("data part")
            } else {
                Compressed::new(Adjacency::new(&index, &heap).expect("data part"))
            };
            (t.elapsed().as_secs_f64(), time(&adj, a, settings))
        }
        "eg" => {
            // The undirected `adjacency.out` of a graph file, read into memory: the file
            // cannot be mapped while the hierarchy is committed to it.
            let bytes = std::fs::read(&a["hier"]).expect("the graph file");
            let held = MemorySource::new(&bytes[..]);
            let file = Container::open(&held).expect("a container");
            let part = |name: &str| {
                let e = file.get(name).expect("an undirected adjacency");
                file.source()
                    .slice(e.offset..e.offset + e.len)
                    .expect("mapped")
            };
            let index = AdjacencyIndex::parse(part("adjacency.out.index")).expect("an index");
            let adj = Compressed::new(
                Adjacency::new(&index, part("adjacency.out.data")).expect("data part"),
            );
            (t.elapsed().as_secs_f64(), time(&adj, a, settings))
        }
        "stream" => {
            let file = FileSource::open(graph.join("targets.u32")).expect("targets");
            let stream = Stream::new(&offsets, &file).expect("a CSR");
            (t.elapsed().as_secs_f64(), time(&stream, a, settings))
        }
        other => panic!("unknown --source {other}"),
    })
    .expect("a thread pool");
    println!(
        "{{\"graph\": \"{}\", \"source\": \"{source}\", \"threads\": {threads}, \"chunk\": {}, \
         \"load_seconds\": {load}, {result}}}",
        graph.file_name().unwrap_or_default().to_string_lossy(),
        settings.chunk_entries,
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
            eprintln!("usage: leiden_bench encode|run --graph DIR ... (see the source)");
            return ExitCode::from(2);
        }
    }
    ExitCode::SUCCESS
}
