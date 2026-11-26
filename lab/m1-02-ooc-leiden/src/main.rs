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
mod bcache;
mod ef;
mod leiden;
mod lutil;
mod shapeio;
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
        "shuffle" => shuffle(a),
        // m1-05 (#19): helpers of the hierarchy-shaping driver
        "cutgraph" => shapeio::cutgraph(a),
        "regionstats" => shapeio::regionstats(a),
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

/// Survey idea L1: a seeded random relabelling. New id of u = rank of splitmix64(seed ^ u).
/// Writes a glab-csr-v0 directory (offsets.u64, targets.u32, sorted lists) and perm.u32
/// (perm[u] = new id). In memory: for lj and orkut only.
fn shuffle(a: &[String]) {
    let g = Raw::load(Path::new(&get(a, "--graph").expect("--graph")), Mem::Heap).expect("load raw");
    let out = PathBuf::from(get(a, "--out").expect("--out"));
    let seed: u64 = get(a, "--seed").map(|s| s.parse().unwrap()).unwrap_or(1);
    let n = g.n;
    let mix = |x: u64| {
        let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let mut keys: Vec<(u64, u32)> = (0..n as u32).map(|u| (mix(seed ^ u as u64), u)).collect();
    keys.par_sort_unstable();
    let mut perm = vec![0u32; n];
    let mut inv = vec![0u32; n];
    for (x, &(_, u)) in keys.iter().enumerate() {
        perm[u as usize] = x as u32;
        inv[x] = u;
    }
    drop(keys);
    let mut off = vec![0u64; n + 1];
    for x in 0..n {
        off[x + 1] = off[x] + g.deg(inv[x] as usize) as u64;
    }
    let mut tgt = vec![0u32; g.targets.len()];
    let tp = SyncPtr(tgt.as_mut_ptr());
    (0..n).into_par_iter().for_each(|x| {
        let s = unsafe { tp.slice_mut(off[x] as usize, off[x + 1] as usize) };
        for (i, &v) in g.list(inv[x] as usize).iter().enumerate() {
            s[i] = perm[v as usize];
        }
        s.sort_unstable();
    });
    std::fs::create_dir_all(&out).unwrap();
    let w = |name: &str, b: &[u8]| std::fs::write(out.join(name), b).unwrap();
    let bytes = |p: *const u8, len: usize| unsafe { std::slice::from_raw_parts(p, len) };
    w("offsets.u64", bytes(off.as_ptr() as *const u8, off.len() * 8));
    w("targets.u32", bytes(tgt.as_ptr() as *const u8, tgt.len() * 4));
    w("perm.u32", bytes(perm.as_ptr() as *const u8, perm.len() * 4));
    println!("{}", json!({"cmd": "shuffle", "n": n, "entries": tgt.len(), "seed": seed, "out": out.display().to_string()}));
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
