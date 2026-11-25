#![allow(dead_code)]
//! m102 — experiment #16 (m1-02): out-of-core Leiden.
//!
//!   m102 encode --graph CSR_DIR --out STORE_DIR --threads T [--fsync]
//!       glab-csr-v0 -> the compressed store of lab/m1-13 (DESIGN D4), streamed: the raw arrays
//!       are mapped without pre-fault and blocks are written in node order, so it runs in a few
//!       GiB whatever the graph size. Byte-identical to m1-13's in-memory encoder (`verify`).
//!   m102 verify --graph CSR_DIR --store STORE_DIR --threads T
//!       every list decodes to the raw list; on graphs under 2^31 entries also byte-identity of
//!       adj.bin with m1-13's encoder.
//!   m102 leiden run --store STORE_DIR --config comp --mem mmap|mmap-lazy|heap [--ooc --spill DIR
//!       [--budget-gb G | --budget-frac F]] [--check-agg] (lab/b2-leiden flags)
//!       --ooc: sequential aggregation, quotient graphs spilled (src/seqagg.rs); the budget of
//!       the memory accountant is F (default 0.80) x the cgroup's memory.max - 64 MiB.
//!   m102 evict --store STORE_DIR
mod acct;
mod adj;
mod ef;
mod leiden;
mod lutil;
mod store;
mod svb;

use adj::*;
use rayon::prelude::*;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::Instant;
use store::{encode_record, Comp, StoreWriter};

fn get(args: &[String], k: &str) -> Option<String> {
    args.iter().position(|a| a == k).and_then(|i| args.get(i + 1).cloned())
}

fn main() {
    let argv: Vec<String> = std::env::args().collect();
    if argv.len() < 2 {
        eprintln!("usage: m102 <encode|verify|leiden|evict> [--key value ...]");
        std::process::exit(2);
    }
    if argv[1] == "leiden" {
        return leiden::main_leiden(argv[1..].to_vec());
    }
    let a = &argv[2..];
    let threads: usize = get(a, "--threads").map(|s| s.parse().unwrap()).unwrap_or(16);
    rayon::ThreadPoolBuilder::new().num_threads(threads).build_global().unwrap();
    match argv[1].as_str() {
        "encode" => encode(a, threads),
        "verify" => verify(a),
        "evict" => {
            let d = get(a, "--store").expect("--store");
            for f in ["adj.bin", "boff.ef", "blocks.u32"] {
                let _ = evict_file(&Path::new(&d).join(f));
            }
        }
        x => {
            eprintln!("unknown command {x}");
            std::process::exit(2);
        }
    }
}

fn encode(a: &[String], threads: usize) {
    let dir = PathBuf::from(get(a, "--graph").expect("--graph"));
    let out = PathBuf::from(get(a, "--out").expect("--out"));
    let fsync = a.iter().any(|x| x == "--fsync");
    let t = Instant::now();
    let g = Raw::load(&dir, Mem::MmapLazy).expect("load raw");
    let tmp = out.with_extension(format!("tmp{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let mut w = StoreWriter::create(&tmp.join("adj.bin"), g.n, false).unwrap();
    // windows of blocks encoded in parallel, appended in order
    let win = threads * 8;
    let mut i = 0;
    while i < g.chunks.len() {
        let e = (i + win).min(g.chunks.len());
        let parts: Vec<(Vec<u8>, Vec<u32>, Vec<u32>)> = g.chunks[i..e]
            .par_iter()
            .map(|&(lo, hi)| {
                let mut out = Vec::new();
                let mut rel = Vec::with_capacity(hi - lo);
                let mut deg = Vec::with_capacity(hi - lo);
                let mut tmp = Vec::new();
                for v in lo..hi {
                    rel.push(out.len() as u32);
                    let l = g.list(v);
                    deg.push(l.len() as u32);
                    encode_record(v, l, None, &mut out, &mut tmp);
                }
                (out, rel, deg)
            })
            .collect();
        let offs: Vec<u64> = g.chunks[i..e].iter().zip(&parts).map(|(&(lo, hi), p)| w.place(lo, hi, p.0.len(), &p.1, &p.2)).collect();
        parts.par_iter().zip(&offs).for_each(|(p, &o)| w.write_at(o, &p.0));
        i = e;
    }
    let c = w.finish_dir(&tmp, fsync).expect("finish");
    // the block table must be the raw graph's (same work partition as m1-13)
    assert_eq!(c.chunks, g.chunks, "block table differs from the raw graph's edge chunks");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::rename(&tmp, &out).expect("rename");
    println!(
        "{}",
        json!({"cmd": "encode", "graph": dir.display().to_string(), "store": out.display().to_string(), "n": c.n, "entries": c.entries,
               "data_bytes": c.data_len, "bits_per_edge_data": c.data_len as f64 * 8.0 / c.entries as f64, "max_deg": c.max_deg,
               "seconds": t.elapsed().as_secs_f64(), "fsync": fsync, "peak_rss_bytes": peak_rss()})
    );
}

fn verify(a: &[String]) {
    let g = Raw::load(Path::new(&get(a, "--graph").expect("--graph")), Mem::MmapLazy).expect("load raw");
    let c = Comp::open(Path::new(&get(a, "--store").expect("--store")), Mem::MmapLazy).expect("open store");
    assert_eq!((c.n, c.entries), (g.n, g.targets.len()));
    let bad: usize = g
        .chunks
        .par_iter()
        .map(|&(lo, hi)| {
            let mut cur = c.cur();
            (lo..hi).filter(|&v| c.nbrs(v, &mut cur) != g.list(v)).count()
        })
        .sum();
    assert_eq!(bad, 0, "lists differ");
    let mut bytes_identical = serde_json::Value::Null;
    if g.targets.len() < (1 << 31) {
        let (m, _) = Comp::from_raw(&g);
        bytes_identical = json!(m.data_len == c.data_len && m.data[..m.data_len] == c.data[..c.data_len]);
        assert_eq!(bytes_identical, json!(true), "adj.bin differs from m1-13's encoder");
    }
    println!("{}", json!({"cmd": "verify", "n": g.n, "entries": g.targets.len(), "lists_equal": true, "bytes_identical_to_m113": bytes_identical}));
}

// The Adj trait is used by verify (nbrs/cur).
#[allow(unused_imports)]
use adj::Adj as _;
