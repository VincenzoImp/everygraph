// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `fc`: the counters of experiment #266 (m3-27). Runs one product kernel on a CSR
//! directory or an encoded store with the kernels' feature `frontier-counters`, and
//! writes what the frontier driver and the decoder did.
//!
//! ```text
//! fc encode --graph DIR --out STORE
//! fc run --kernel kcore|sssp|sssp-product|bfs|wcc|leiden --graph DIR [--store STORE]
//!     [--source raw|compressed] --threads T --calls FILE.jsonl
//! ```
//!
//! `run` writes one line per frontier call to `--calls` and prints one JSON line: the
//! result hash, the k-core restarts and the marks (decoded, handed and examined entries
//! after each step of BFS and WCC). `sssp` is the lab copy (`sssp.rs`) with costs
//! `cost(e)`, integers 1..255 hashed from the entry index as `edge_map_bench`'s
//! `uniform`; `sssp-product` is the product kernel on the same costs as a column.
//! `leiden` is production Leiden with its default options (deterministic mode).

#![allow(unsafe_code, clippy::cast_possible_truncation, clippy::cast_precision_loss)]

mod sssp;

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering::Relaxed};

use everygraph_community::{Options, leiden};
use everygraph_core::{ByteSource, Memory, par};
use everygraph_format::adjacency::{AdjacencyIndex, encode};
use everygraph_io::MmapSource;
use everygraph_kernels::components::{WccOptions, wcc};
use everygraph_kernels::edge_map::{Compressed, Csr, Plan, Settings, Source, counters};
use everygraph_kernels::kcore::kcore;
use everygraph_kernels::traversal::{Bfs, bfs};

type Args = HashMap<String, String>;

fn args() -> (String, Args) {
    let mut it = std::env::args().skip(1);
    let cmd = it.next().unwrap_or_default();
    let mut a = HashMap::new();
    while let Some(k) = it.next() {
        a.insert(k.trim_start_matches("--").to_owned(), it.next().unwrap_or_default());
    }
    (cmd, a)
}

fn offsets(dir: &Path) -> Vec<u64> {
    let bytes = std::fs::read(dir.join("offsets.u64")).expect("offsets.u64");
    bytes.chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().expect("8"))).collect()
}

/// `targets.u32`, memory-mapped: the heap copy of twitter-sym's would need 2× its 9.6 GB.
fn targets(map: &MmapSource) -> &[u32] {
    let bytes = map.as_slice().expect("a mapped file");
    // SAFETY: a map starts on a page boundary, and any 4 bytes are a valid u32.
    let (head, words, tail) = unsafe { bytes.align_to::<u32>() };
    assert!(head.is_empty() && tail.is_empty(), "targets.u32: not whole u32s");
    words
}

fn fnv(words: impl Iterator<Item = u64>) -> u64 {
    words.fold(0xCBF2_9CE4_8422_2325, |h, w| (h ^ w).wrapping_mul(0x0000_0100_0000_01B3))
}

/// `edge_map_bench`'s `uniform` cost of entry `e`: an integer from 1 to 255.
fn cost(e: u64) -> f64 {
    let mut z = e.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (1 + z % 255) as f64
}

fn hub<S: Source>(s: &S) -> u32 {
    (0..s.nodes()).max_by_key(|&v| (s.degree(v), std::cmp::Reverse(v))).unwrap_or(0)
}

/// Runs the kernel; returns its result hash.
fn kernel<S: Source>(plan: &Plan<'_, '_, S>, kernel: &str, memory: &Memory) -> u64 {
    let s = plan.source();
    let atomics32 = |v: Vec<AtomicU32>| fnv(v.into_iter().map(|x| u64::from(x.into_inner())));
    let atomics64 = |v: &[AtomicU64]| fnv(v.iter().map(|x| x.load(Relaxed)));
    match kernel {
        "kcore" => atomics32(kcore(plan, memory).expect("kcore").0),
        "wcc" => atomics32(wcc(plan, memory, WccOptions::default()).expect("wcc").0),
        "bfs" => atomics32(bfs(plan, Some(plan), hub(s), &Bfs::default(), memory).expect("bfs").distance),
        "sssp" => {
            let width = sssp::delta(plan, cost);
            atomics64(&sssp::sssp(plan, cost, hub(s), width, memory).expect("sssp"))
        }
        "sssp-product" => {
            use everygraph_kernels::sssp::{Sssp, delta, sssp};
            let costs: Vec<f64> = (0..s.entries()).map(cost).collect();
            let p = Sssp { delta: Some(delta(plan, &costs)) };
            assert_eq!(p.delta, Some(sssp::delta(plan, cost)), "the copy's delta");
            counters::tag("sssp.step", 0);
            atomics64(&sssp(plan, &costs, hub(s), &p, memory).expect("sssp").bits)
        }
        other => panic!("unknown --kernel {other}"),
    }
}

fn run(a: &Args) {
    let graph = PathBuf::from(&a["graph"]);
    let name = graph.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let k = a["kernel"].clone();
    let source = a.get("source").cloned().unwrap_or_else(|| {
        if matches!(k.as_str(), "bfs" | "wcc") { "compressed" } else { "raw" }.to_owned()
    });
    let threads: usize = a.get("threads").map_or(16, |s| s.parse().expect("--threads"));
    let offsets = offsets(&graph);
    let memory = Memory::unlimited();
    let settings = Settings::default();
    let threads = NonZeroUsize::new(threads).expect("--threads > 0");
    let hash = par::with_threads(threads, || {
        let _ = counters::take();
        match source.as_str() {
            "raw" => {
                let map = MmapSource::open(graph.join("targets.u32")).expect("targets.u32");
                let csr = Csr::new(&offsets, targets(&map)).expect("a CSR");
                if k == "leiden" {
                    counters::tag("leiden", 0);
                    let c = leiden(&csr, &memory, settings, &Options::default()).expect("leiden");
                    fnv(c.membership.iter().map(|&x| u64::from(x)))
                } else {
                    Plan::with(&csr, &memory, settings, |p| kernel(&p, &k, &memory))
                }
            }
            "compressed" => {
                let store = PathBuf::from(&a["store"]);
                let index = std::fs::read(store.join("adjacency.index")).expect("index");
                let index = AdjacencyIndex::parse(&index).expect("an index");
                let map = MmapSource::open(store.join("adjacency.data")).expect("data");
                let adj = Compressed::mapped(&index, &map).expect("data part");
                Plan::with(&adj, &memory, settings, |p| kernel(&p, &k, &memory))
            }
            other => panic!("unknown --source {other}"),
        }
    })
    .expect("a thread pool");
    let c = counters::take();
    let mut out = std::io::BufWriter::new(std::fs::File::create(&a["calls"]).expect("--calls"));
    for x in &c.calls {
        writeln!(
            out,
            "{{\"tag\":\"{}\",\"key\":{},\"nodes\":{},\"runs\":{},\"entries\":{},\"max_run\":{},\
             \"max_list\":{},\"max_chain\":{},\"side_lists\":{},\"draws\":{}}}",
            x.tag, x.key, x.nodes, x.runs, x.entries, x.max_run, x.max_list, x.max_chain,
            x.side_lists, x.draws
        )
        .expect("write");
    }
    let mut marks = String::new();
    for (i, m) in c.marks.iter().enumerate() {
        let sep = if i == 0 { "" } else { "," };
        write!(
            marks,
            "{sep}{{\"tag\":\"{}\",\"key\":{},\"decoded\":{},\"handed\":{},\"examined\":{}}}",
            m.tag, m.key, m.decoded, m.handed, m.examined
        )
        .expect("a string");
    }
    let n = offsets.len() - 1;
    let m = offsets[n];
    // Entries in lists of at least 128, 1,024, 10,240 and 65,536 entries.
    let over: Vec<u64> = [128, 1024, 10_240, 65_536]
        .iter()
        .map(|&t| offsets.windows(2).map(|w| w[1] - w[0]).filter(|&d| d >= t).sum())
        .collect();
    // Nodes with at least one and two entries: what WCC's sampling reads, one or two each.
    let deg_ge = |t: u64| offsets.windows(2).filter(|w| w[1] - w[0] >= t).count();
    println!(
        "{{\"graph\":\"{name}\",\"kernel\":\"{k}\",\"source\":\"{source}\",\"threads\":{threads},\
         \"nodes\":{n},\"entries\":{m},\"entries_in_lists_ge\":{{\"128\":{},\"1024\":{},\
         \"10240\":{},\"65536\":{}}},\"nodes_deg_ge1\":{},\"nodes_deg_ge2\":{},\
         \"hash\":\"{hash:016x}\",\"calls\":{},\"restarts\":{},\"marks\":[{marks}]}}",
        over[0], over[1], over[2], over[3], deg_ge(1), deg_ge(2), c.calls.len(), c.restarts
    );
}

fn encode_store(a: &Args) {
    let graph = PathBuf::from(&a["graph"]);
    let out = PathBuf::from(&a["out"]);
    let offsets = offsets(&graph);
    let map = MmapSource::open(graph.join("targets.u32")).expect("targets.u32");
    let encoded = encode(&offsets, targets(&map)).expect("encode");
    std::fs::create_dir_all(&out).expect("store directory");
    std::fs::write(out.join("adjacency.index"), &encoded.index).expect("index");
    std::fs::write(out.join("adjacency.data"), &encoded.data).expect("data");
    println!("{{\"entries\":{},\"data_bytes\":{}}}", offsets[offsets.len() - 1], encoded.data.len());
}

fn main() {
    let (cmd, a) = args();
    match cmd.as_str() {
        "run" => run(&a),
        "encode" => encode_store(&a),
        _ => panic!("usage: fc encode|run ... (see the source)"),
    }
}
