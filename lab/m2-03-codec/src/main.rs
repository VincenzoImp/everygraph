// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Issue #30 (m2-03): measurements of the production adjacency codec
//! (`crates/everygraph-format`), and the per-list vs block framing A/B.
//!
//! Every command prints one JSON line on standard output.
//!
//! - `encode --graph <glab-csr-v0 dir> [--out <dir>] [--verify]`: encode, write
//!   `index.bin` and `data.bin` if asked, report bits per entry (index and data), the sizes of
//!   the record variants the issue and the lab describe, and encode time.
//! - `scan --store <dir> --threads T --batch B --reps R`: warm sequential decode of every
//!   list, each entry summed; best and median of R passes after one untimed pass.
//! - `random --store <dir> --queries Q --reps R`: warm decode of Q uniformly random
//!   lists, mean time per list (one timer around the batch).
//! - `framing --graph <dir> --rounds R --iters I`: single-thread pull `PageRank` (f32, I
//!   iterations) and a single-thread scan, interleaved round by round over raw arrays,
//!   the production per-list records (batches of 8,192 entries and whole blocks) and
//!   the block-framed variant of `block.rs`; outputs are checked equal.

mod block;

use std::path::{Path, PathBuf};
use std::time::Instant;

use everygraph_core::NodeId;
use everygraph_format::adjacency::{Adjacency, AdjacencyIndex, Batch, Decoder, encode};
use rayon::prelude::*;
use serde_json::json;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).map(|i| args[i + 1].clone())
}

fn num(args: &[String], name: &str, default: usize) -> usize {
    arg(args, name).map_or(default, |v| v.parse().expect("a number"))
}

fn map(path: &Path) -> memmap2::Mmap {
    let f = std::fs::File::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    // SAFETY (lab): read-only files nobody writes while we run.
    unsafe { memmap2::Mmap::map(&f).expect("mmap") }
}

fn cast<T>(m: &memmap2::Mmap) -> &[T] {
    // SAFETY (lab): mmap is page-aligned; the files hold little-endian T.
    unsafe { std::slice::from_raw_parts(m.as_ptr().cast::<T>(), m.len() / size_of::<T>()) }
}

/// A glab-csr-v0 graph copied into the heap (configuration A of #118).
fn load_csr(dir: &Path) -> (Vec<u64>, Vec<u32>) {
    let o = map(&dir.join("offsets.u64"));
    let t = map(&dir.join("targets.u32"));
    (cast::<u64>(&o).to_vec(), cast::<u32>(&t).to_vec())
}

fn median(xs: &mut [f64]) -> f64 {
    xs.sort_by(f64::total_cmp);
    xs[xs.len() / 2]
}

fn varint_len(x: u64) -> u64 {
    (64 - x.max(1).leading_zeros()).div_ceil(7) as u64
}

fn blen(w: u32) -> u64 {
    match w {
        0..=0xff => 1,
        0x100..=0xffff => 2,
        0x1_0000..=0xff_ffff => 3,
        _ => 4,
    }
}

fn encode_cmd(args: &[String]) {
    let graph = PathBuf::from(arg(args, "--graph").expect("--graph"));
    let out = arg(args, "--out").map(PathBuf::from);
    let o = map(&graph.join("offsets.u64"));
    let t = map(&graph.join("targets.u32"));
    let (offsets, targets) = (cast::<u64>(&o), cast::<u32>(&t));
    let t0 = Instant::now();
    let e = encode(offsets, targets).expect("valid CSR");
    let encode_s = t0.elapsed().as_secs_f64();
    if let Some(out) = &out {
        std::fs::create_dir_all(out).unwrap();
        std::fs::write(out.join("index.bin"), &e.index).unwrap();
        std::fs::write(out.join("data.bin"), &e.data).unwrap();
    }
    let entries = targets.len() as f64;
    let n = offsets.len() - 1;
    // Record variants, from the same lists: the lab's (a degree LEB128 before every
    // record, empty lists included) and the issue's text (first neighbour as its own
    // LEB128 of the 64-bit zigzag, the gaps alone in Stream VByte).
    let (mut lab_extra, mut issue_delta) = (0u64, 0i64);
    for v in 0..n {
        let l = &targets[offsets[v] as usize..offsets[v + 1] as usize];
        lab_extra += varint_len(l.len() as u64);
        if let Some(&x) = l.first() {
            let d = i64::from(x) - v as i64;
            let zz64 = ((d << 1) ^ (d >> 63)) as u64;
            let zz32 = ((x.wrapping_sub(v as u32)) << 1) ^ 0u32.wrapping_sub(x.wrapping_sub(v as u32) >> 31);
            let svb_now = (l.len() as u64).div_ceil(4) + blen(zz32);
            let issue = varint_len(zz64) + (l.len() as u64 - 1).div_ceil(4);
            issue_delta += issue as i64 - svb_now as i64;
        }
    }
    let index = AdjacencyIndex::parse(&e.index).expect("index");
    let mut verified = serde_json::Value::Null;
    if args.iter().any(|a| a == "--verify") {
        // Every block decoded on its own and compared with the input, in parallel: no
        // second copy of the graph in memory (Friendster's would not fit the limit).
        let adj = Adjacency::new(&index, &e.data).unwrap();
        let t0 = Instant::now();
        let equal = (0..index.blocks()).into_par_iter().all(|b| {
            let nodes = index.block_nodes(b);
            let mut batch = Batch::default();
            adj.decode_nodes(nodes.clone(), usize::MAX, &mut batch).is_ok_and(|end| {
                let (lo, hi) = (offsets[nodes.start as usize], offsets[nodes.end as usize]);
                end == nodes.end && batch.targets() == &targets[lo as usize..hi as usize]
            })
        });
        let s = t0.elapsed().as_secs_f64();
        assert!(equal, "a decoded block differs from the input");
        verified = json!({"decode_blocks_s": s, "equal": true, "decoder": adj.decoder().name()});
    }
    let bits = |b: usize| b as f64 * 8.0 / entries;
    println!(
        "{}",
        json!({
            "cmd": "encode", "graph": graph, "nodes": n, "entries": targets.len(),
            "index_bytes": e.index.len(), "data_bytes": e.data.len(), "blocks": index.blocks(),
            "bits_per_entry": bits(e.index.len() + e.data.len()),
            "bits_index": bits(e.index.len()), "bits_data": bits(e.data.len()),
            "raw_bits_per_entry": (8.0 * (n as f64 + 1.0) + 4.0 * entries) * 8.0 / entries,
            "variant_lab_records_bits_data": bits(e.data.len() + lab_extra as usize),
            "variant_issue_first_varint_bits_data": (e.data.len() as f64 + issue_delta as f64) * 8.0 / entries,
            "encode_s": encode_s, "threads": rayon::current_num_threads(),
            "verify": verified,
        })
    );
}

struct Store {
    index: AdjacencyIndex,
    data: Vec<u8>,
}

fn load_store(args: &[String]) -> Store {
    let dir = PathBuf::from(arg(args, "--store").expect("--store"));
    let index = AdjacencyIndex::parse(&std::fs::read(dir.join("index.bin")).unwrap()).unwrap();
    let data = std::fs::read(dir.join("data.bin")).unwrap();
    Store { index, data }
}

fn decoder(args: &[String]) -> Decoder {
    match arg(args, "--decoder").as_deref() {
        Some("scalar") => Decoder::SCALAR,
        _ => Decoder::best(),
    }
}

/// One pass over every list of the nodes of `blocks`, in batches of `batch` entries.
fn scan_blocks(adj: Adjacency<'_>, blocks: std::ops::Range<usize>, batch_entries: usize, batch: &mut Batch) -> u64 {
    let mut sum = 0u64;
    for b in blocks {
        let nodes = adj.index().block_nodes(b);
        let mut at = nodes.start;
        while at < nodes.end {
            at = adj.decode_nodes(at..nodes.end, batch_entries, batch).unwrap();
            for &x in batch.targets() {
                sum = sum.wrapping_add(u64::from(x));
            }
        }
    }
    sum
}

fn scan_cmd(args: &[String]) {
    let s = load_store(args);
    let adj = Adjacency::new(&s.index, &s.data).unwrap().with_decoder(decoder(args));
    let threads = num(args, "--threads", 1);
    let batch = num(args, "--batch", 8192);
    let reps = num(args, "--reps", 5);
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
    let blocks = s.index.blocks();
    let pass = || -> u64 {
        if threads == 1 {
            scan_blocks(adj, 0..blocks, batch, &mut Batch::default())
        } else {
            pool.install(|| {
                (0..blocks)
                    .into_par_iter()
                    .map_init(Batch::default, |bt, b| scan_blocks(adj, b..b + 1, batch, bt))
                    .reduce(|| 0, u64::wrapping_add)
            })
        }
    };
    let check = pass();
    let mut times = Vec::new();
    for _ in 0..reps {
        let t0 = Instant::now();
        assert_eq!(pass(), check);
        times.push(t0.elapsed().as_secs_f64());
    }
    let best = times.iter().copied().fold(f64::INFINITY, f64::min);
    let e = s.index.entries() as f64;
    println!(
        "{}",
        json!({
            "cmd": "scan", "decoder": adj.decoder().name(), "threads": threads, "batch_entries": batch,
            "entries": s.index.entries(), "seconds": times.clone(), "best_s": best,
            "g_entries_per_s_best": e / best / 1e9, "g_entries_per_s_median": e / median(&mut times) / 1e9,
        })
    );
}

fn xorshift(x: &mut u64) -> u64 {
    *x ^= *x << 13;
    *x ^= *x >> 7;
    *x ^= *x << 17;
    *x
}

fn random_cmd(args: &[String]) {
    let s = load_store(args);
    let adj = Adjacency::new(&s.index, &s.data).unwrap().with_decoder(decoder(args));
    let q = num(args, "--queries", 1_000_000);
    let reps = num(args, "--reps", 5);
    let n = u64::from(s.index.nodes());
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let nodes: Vec<NodeId> = (0..q).map(|_| NodeId::new((xorshift(&mut seed) % n) as u32)).collect();
    let mut buf = Vec::new();
    let mut pass = || {
        let mut sum = 0u64;
        for &v in &nodes {
            let l = adj.neighbors(v, &mut buf).unwrap();
            sum = sum.wrapping_add(l.len() as u64 + l.first().map_or(0, |&x| u64::from(x)));
        }
        sum
    };
    let check = pass();
    let mut ns = Vec::new();
    for _ in 0..reps {
        let t0 = Instant::now();
        assert_eq!(pass(), check);
        ns.push(t0.elapsed().as_secs_f64() * 1e9 / q as f64);
    }
    let best = ns.iter().copied().fold(f64::INFINITY, f64::min);
    println!(
        "{}",
        json!({
            "cmd": "random", "decoder": adj.decoder().name(), "queries": q, "ns_per_list": ns.clone(),
            "ns_per_list_best": best, "ns_per_list_median": median(&mut ns), "threads": 1,
        })
    );
}

/// Pull `PageRank`, f32, over lists handed out by `lists(f)`, which calls
/// `f(v, neighbours)` for every node in order.
fn pagerank(n: usize, degree: &[u32], iters: usize, mut lists: impl FnMut(&mut dyn FnMut(usize, &[u32]))) -> Vec<f32> {
    let damping = 0.85f32;
    let mut rank = vec![1.0 / n as f32; n];
    let mut contrib = vec![0f32; n];
    for _ in 0..iters {
        for v in 0..n {
            contrib[v] = if degree[v] > 0 { rank[v] / degree[v] as f32 } else { 0.0 };
        }
        let base = (1.0 - damping) / n as f32;
        lists(&mut |v, l| {
            let mut s = 0f32;
            for &u in l {
                s += contrib[u as usize];
            }
            rank[v] = base + damping * s;
        });
    }
    rank
}

/// The 1-minute load average: the A/B rows are interleaved, not taken on an idle machine.
fn load() -> f64 {
    std::fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|s| s.split_whitespace().next()?.parse().ok())
        .unwrap_or(f64::NAN)
}

fn framing_cmd(args: &[String]) {
    let load_start = load();
    let graph = PathBuf::from(arg(args, "--graph").expect("--graph"));
    let rounds = num(args, "--rounds", 5);
    let iters = num(args, "--iters", 5);
    let (offsets, targets) = load_csr(&graph);
    let n = offsets.len() - 1;
    let degree: Vec<u32> = offsets.windows(2).map(|w| (w[1] - w[0]) as u32).collect();
    let e = encode(&offsets, &targets).unwrap();
    let index = AdjacencyIndex::parse(&e.index).unwrap();
    let adj = Adjacency::new(&index, &e.data).unwrap();
    let starts: Vec<u32> = (0..index.blocks()).map(|b| index.block_nodes(b).start).chain([n as u32]).collect();
    let blk = block::Blocked::encode(&offsets, &targets, starts);

    let mut batch = Batch::default();
    let (mut buf, mut bits, mut nodes) = (Vec::new(), Vec::new(), Vec::new());
    let blocks = index.blocks();
    // Lists of every variant, handed to a kernel body.
    let mut per_list = |batch_entries: usize, f: &mut dyn FnMut(usize, &[u32])| {
        for b in 0..blocks {
            let r = index.block_nodes(b);
            let mut at = r.start;
            while at < r.end {
                at = adj.decode_nodes(at..r.end, batch_entries, &mut batch).unwrap();
                for (v, l) in batch.iter() {
                    f(v.index(), l);
                }
            }
        }
    };
    let mut blocked = |f: &mut dyn FnMut(usize, &[u32])| {
        for b in 0..blocks {
            blk.decode_block(b, &mut buf, &mut bits, &mut nodes);
            let (lo, hi) = (blk.starts[b] as usize, blk.starts[b + 1] as usize);
            let e0 = blk.offsets[lo];
            for v in lo..hi {
                f(v, &buf[(blk.offsets[v] - e0) as usize..(blk.offsets[v + 1] - e0) as usize]);
            }
        }
    };
    let raw = |f: &mut dyn FnMut(usize, &[u32])| {
        for v in 0..n {
            f(v, &targets[offsets[v] as usize..offsets[v + 1] as usize]);
        }
    };
    let variants = ["raw", "per-list-8192", "per-list-block", "block-framed"];
    let mut pr: Vec<Vec<f64>> = vec![Vec::new(); 4];
    let mut sc: Vec<Vec<f64>> = vec![Vec::new(); 4];
    let mut reference: Option<Vec<f32>> = None;
    let mut sums = Vec::new();
    for round in 0..rounds + 1 {
        for j in 0..4 {
            let k = (j + round) % 4; // rotate the order every round
            let mut run = |f: &mut dyn FnMut(usize, &[u32])| match k {
                0 => raw(f),
                1 => per_list(8192, f),
                2 => per_list(usize::MAX, f),
                _ => blocked(f),
            };
            let t0 = Instant::now();
            let r = pagerank(n, &degree, iters, &mut run);
            let pr_s = t0.elapsed().as_secs_f64();
            match &reference {
                None => reference = Some(r),
                Some(x) => assert!(x == &r, "PageRank of {} differs", variants[k]),
            }
            let t0 = Instant::now();
            let mut sum = 0u64;
            run(&mut |_, l| {
                for &x in l {
                    sum = sum.wrapping_add(u64::from(x));
                }
            });
            let scan_s = t0.elapsed().as_secs_f64();
            sums.push(sum);
            if round > 0 {
                pr[k].push(pr_s);
                sc[k].push(scan_s);
            }
        }
    }
    assert!(sums.windows(2).all(|w| w[0] == w[1]), "scan sums differ");
    // Random access, interleaved too.
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let q = 1_000_000;
    let qs: Vec<u32> = (0..q).map(|_| (xorshift(&mut seed) % n as u64) as u32).collect();
    let (mut ra, mut rb) = (Vec::new(), Vec::new());
    let mut lbuf = Vec::new();
    for _ in 0..rounds {
        let t0 = Instant::now();
        let mut s1 = 0u64;
        for &v in &qs {
            s1 += adj.neighbors(NodeId::new(v), &mut lbuf).unwrap().len() as u64;
        }
        ra.push(t0.elapsed().as_secs_f64() * 1e9 / q as f64);
        let t0 = Instant::now();
        let mut s2 = 0u64;
        for &v in &qs {
            s2 += blk.list(v as usize, &mut lbuf) as u64;
        }
        rb.push(t0.elapsed().as_secs_f64() * 1e9 / q as f64);
        assert_eq!(s1, s2);
    }
    let med: Vec<f64> = pr.iter_mut().map(|x| median(x)).collect();
    let meds: Vec<f64> = sc.iter_mut().map(|x| median(x)).collect();
    println!(
        "{}",
        json!({
            "cmd": "framing", "graph": graph, "load_start": load_start, "load_end": load(), "threads": 1, "iters": iters, "rounds": rounds,
            "variants": variants, "decoder": adj.decoder().name(),
            "pagerank_s": pr, "pagerank_median_s": med,
            "pagerank_ratio_to_raw": med.iter().map(|m| m / med[0]).collect::<Vec<_>>(),
            "scan_s": sc, "scan_median_s": meds,
            "scan_ratio_to_raw": meds.iter().map(|m| m / meds[0]).collect::<Vec<_>>(),
            "random_ns_per_list": {"per-list": median(&mut ra), "block-framed": median(&mut rb)},
            "data_bytes": {"per-list": e.data.len(), "block-framed": blk.bytes()},
        })
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("encode") => encode_cmd(&args),
        Some("scan") => scan_cmd(&args),
        Some("random") => random_cmd(&args),
        Some("framing") => framing_cmd(&args),
        _ => eprintln!("usage: m203 encode|scan|random|framing ... (see the source)"),
    }
}
