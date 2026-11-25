//! m113 — experiment #118 (m1-13): the default storage end to end.
//!
//!   m113 encode  --graph CSR_DIR --out STORE_DIR --name N --threads T [--fsync] --json F
//!   m113 verify  --graph CSR_DIR --store STORE_DIR --threads T --json F
//!   m113 scan    --graph CSR_DIR --store STORE_DIR --config C --threads T --json F
//!   m113 open    --graph CSR_DIR --store STORE_DIR --threads T --what raw|comp|decoded --mem M [--cold] --json F
//!   m113 kernel  --graph CSR_DIR --store STORE_DIR --config C --mem M --algos a,b,c --threads T --reps R --json F
//!   m113 pq      --graph CSR_DIR --store STORE_DIR [--in-graph D --in-store D] --config C --mem M --json F
//!   m113 leiden run --graph CSR_DIR --store STORE_DIR --config C --mem M ... (lab/b2-leiden flags)
//!
//! Configurations (C): raw      = uncompressed arrays in memory (A, the lab baseline)
//!                     comp     = compressed store, lists decoded on the fly (B)
//!                     comp-block = B, but accessed at block granularity only (ablation)
//!                     decoded  = compressed store decoded once into uncompressed arrays (C)
//! Every command appends one JSON object per measurement to the --json file.
mod adj;
mod bfs_ds;
mod ef;
mod kernels;
mod leiden;
mod lutil;
mod pq;
mod store;
mod svb;

use adj::*;
use rayon::prelude::*;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering::Relaxed;
use std::time::Instant;
use store::{BlockView, Comp, DECODER};

fn args_map(args: &[String]) -> HashMap<String, String> {
    let mut m = HashMap::new();
    let mut i = 0;
    while i < args.len() {
        let k = args[i].trim_start_matches("--").to_string();
        if i + 1 < args.len() && !args[i + 1].starts_with("--") {
            m.insert(k, args[i + 1].clone());
            i += 2;
        } else {
            m.insert(k, "1".to_string());
            i += 1;
        }
    }
    m
}

fn emit(mut v: Value, a: &HashMap<String, String>) {
    v["loadavg_1m_end"] = json!(loadavg());
    v["decoder"] = json!(store::decoder_name());
    v["target_cpu"] = json!("native");
    let b = store::BATCH.load(Relaxed);
    if v["config"] == "comp-block" || v["config"] == "comp" {
        v["scan_batch_entries"] = if b == usize::MAX { json!("whole block") } else { json!(b) };
        v["degree_index"] = json!(store::DEG_INDEX.load(Relaxed));
    }
    if let Ok(b) = std::env::var("M113_BUSY") {
        v["busy_cpus_before"] = json!(b.parse::<f64>().unwrap_or(-1.0));
    }
    if let Ok(b) = std::env::var("M113_MEMAVAIL") {
        v["mem_available_gb_before"] = json!(b.parse::<f64>().unwrap_or(-1.0));
    }
    for k in ["label", "name"] {
        if let Some(l) = a.get(k) {
            v[k] = json!(l);
        }
    }
    let s = serde_json::to_string(&v).unwrap();
    println!("{s}");
    if let Some(p) = a.get("json") {
        use std::io::Write;
        let mut f = std::fs::File::options().create(true).append(true).open(p).expect("json out");
        writeln!(f, "{s}").unwrap();
    }
}

fn main() {
    let argv: Vec<String> = std::env::args().collect();
    if argv.len() < 2 {
        eprintln!("usage: m113 <encode|verify|scan|open|kernel|pq|leiden> [--key value ...]");
        std::process::exit(2);
    }
    if argv.iter().any(|a| a == "--decoder") {
        let i = argv.iter().position(|a| a == "--decoder").unwrap();
        DECODER.store(match argv[i + 1].as_str() { "lab" => 0, "fused" => 1, "pad" => 2, x => panic!("bad --decoder {x}") }, Relaxed);
    }
    if let Some(i) = argv.iter().position(|a| a == "--batch") {
        store::BATCH.store(argv[i + 1].parse().unwrap(), Relaxed);
    }
    if argv.iter().any(|a| a == "--deg-index") {
        store::DEG_INDEX.store(true, Relaxed);
    }
    if argv[1] == "leiden" {
        return leiden::main_leiden(argv[1..].to_vec());
    }
    let a = args_map(&argv[2..]);
    let threads: usize = a.get("threads").map(|s| s.parse().unwrap()).unwrap_or(16);
    rayon::ThreadPoolBuilder::new().num_threads(threads).build_global().unwrap();
    match argv[1].as_str() {
        "encode" => cmd_encode(&a, threads),
        "verify" => cmd_verify(&a, threads),
        "scan" => cmd_scan(&a, threads),
        "open" => cmd_open(&a, threads),
        "kernel" => cmd_kernel(&a, threads),
        "pq" => cmd_pq(&a, threads),
        "evict" => {
            for (k, files) in [("graph", &["targets.u32", "offsets.u64"][..]), ("store", &["adj.bin", "boff.ef", "blocks.u32"][..])] {
                if let Some(d) = a.get(k) {
                    for f in files {
                        let _ = evict_file(&Path::new(d).join(f));
                    }
                }
            }
        }
        x => {
            eprintln!("unknown command {x}");
            std::process::exit(2);
        }
    }
}

fn mem_of(a: &HashMap<String, String>) -> Mem {
    Mem::parse(a.get("mem").map(|s| s.as_str()).unwrap_or("heap"))
}

fn cmd_encode(a: &HashMap<String, String>, threads: usize) {
    let dir = PathBuf::from(&a["graph"]);
    let out = PathBuf::from(&a["out"]);
    let la = loadavg();
    let g = Raw::load(&dir, Mem::Heap).expect("load raw");
    let reps: usize = a.get("reps").map(|s| s.parse().unwrap()).unwrap_or(1);
    let mut best: Option<(Comp, store::EncodeStats, f64)> = None;
    let mut all = Vec::new();
    for _ in 0..reps {
        let t = Instant::now();
        let (c, st) = Comp::from_raw(&g);
        let s = t.elapsed().as_secs_f64();
        all.push(s);
        if best.as_ref().map(|b| s < b.2).unwrap_or(true) {
            best = Some((c, st, s));
        }
    }
    let (c, st, enc_s) = best.unwrap();
    let t = Instant::now();
    let tmp = out.with_extension(format!("tmp{}", std::process::id()));
    c.write(&tmp, a.contains_key("fsync")).expect("write store");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::rename(&tmp, &out).expect("rename");
    let write_s = t.elapsed().as_secs_f64();
    let m = g.entries() as f64;
    let (data, efb, blk) = (c.data_len as f64, c.boff.stored_bytes() as f64, (c.chunks.len() + 1) as f64 * 4.0);
    emit(
        json!({
            "cmd": "encode", "graph": dir.file_name().unwrap().to_string_lossy(), "threads": threads, "n": g.n, "entries": g.entries(),
            "blocks": c.chunks.len(),
            "bits_per_edge": (data + efb + blk) * 8.0 / m, "bits_per_edge_data": data * 8.0 / m,
            "bits_per_edge_offsets_ef": efb * 8.0 / m, "bits_per_edge_block_table": blk * 8.0 / m,
            "bits_per_edge_raw_u32_u64": (m * 4.0 + (g.n + 1) as f64 * 8.0) * 8.0 / m,
            "data_bytes": c.data_len, "offsets_ef_bytes": c.boff.stored_bytes(), "offsets_ef_ram_bytes": c.boff.ram_bytes(),
            "encode_seconds_best": enc_s, "encode_seconds_all": all, "encode_edges_per_s": m / enc_s,
            "encode_phase_seconds": {"blocks": st.encode_s, "concat": st.concat_s, "elias_fano": st.ef_s},
            "write_seconds": write_s, "fsync": a.contains_key("fsync"),
            "included": "encode = in-memory raw CSR -> compressed bytes + Elias-Fano, excludes loading the raw CSR; write = files to scratch",
            "loadavg_1m_start": la,
        }),
        a,
    );
}

/// Exactness: full decode (configuration C path) equals the raw arrays; sequential, block and
/// random-access paths agree with them too. Both decoders.
fn cmd_verify(a: &HashMap<String, String>, threads: usize) {
    let g = Raw::load(Path::new(&a["graph"]), Mem::Mmap).expect("load raw");
    let c = Comp::open(Path::new(&a["store"]), Mem::Mmap).expect("open store");
    assert_eq!((c.n, c.entries), (g.n, g.entries()));
    assert_eq!(c.chunks, g.chunks, "blocks must equal the lab work partition");
    let mut res = json!({"cmd": "verify", "graph": a["graph"], "threads": threads, "n": g.n, "entries": g.entries()});
    for dec in [2u8, 1, 0] {
        DECODER.store(dec, Relaxed);
        let (r, _, _) = c.decode_to_raw();
        let ok_full = r.offsets[..] == g.offsets[..] && r.targets.par_chunks(1 << 20).zip(g.targets.par_chunks(1 << 20)).all(|(x, y)| x == y);
        drop(r);
        let ok_seq = c.chunks.par_iter().all(|&(lo, hi)| {
            let mut cur = c.cur();
            let mut ok = true;
            c.for_range(lo, hi, &mut cur, |v, l| ok &= l == g.list(v));
            c.for_range_prefix(lo, hi, 2, &mut cur, |v, l, d| ok &= d == g.deg(v) && l == &g.list(v)[..2.min(d)]);
            c.for_range_deg(lo, hi, |v, d| ok &= d == g.deg(v));
            BlockView(&c).for_range(lo, hi, &mut cur, |v, l| ok &= l == g.list(v));
            ok
        });
        let ok_rand = (0..2_000_000usize).into_par_iter().with_min_len(4096).all(|i| {
            let v = (pq::Rng(i as u64).next() % g.n as u64) as usize;
            let mut cur = c.cur();
            let t = g.list(v).get(g.deg(v) / 3).copied();
            c.nbrs(v, &mut cur) == g.list(v) && c.deg(v) == g.deg(v) && c.any(v, &mut cur, |u| Some(u) == t) == g.any(v, &mut (), |u| Some(u) == t)
        });
        // list-by-list sequential path, and degrees through the optional entry-offset index
        store::BATCH.store(1, Relaxed);
        let ok_seq = ok_seq
            && c.chunks.par_iter().all(|&(lo, hi)| {
                let mut cur = c.cur();
                let mut ok = true;
                c.for_range(lo, hi, &mut cur, |v, l| ok &= l == g.list(v));
                ok
            });
        store::BATCH.store(8192, Relaxed);
        let k = store::decoder_name();
        res[k] = json!({"full_decode_identical": ok_full, "sequential_paths_identical": ok_seq, "random_access_identical_2M_samples": ok_rand});
        assert!(ok_full && ok_seq && ok_rand, "verification failed ({k})");
    }
    DECODER.store(2, Relaxed);
    let mut c = c;
    c.build_degree_index();
    let ok_deg = c.chunks.par_iter().all(|&(lo, hi)| {
        let mut ok = true;
        c.for_range_deg(lo, hi, |v, d| ok &= d == g.deg(v) && c.deg(v) == d);
        ok
    });
    assert!(ok_deg, "degree index mismatch");
    let e = c.eoff.as_ref().unwrap();
    res["degree_index"] = json!({"identical": ok_deg, "stored_bytes": e.stored_bytes(), "ram_bytes": e.ram_bytes(), "bits_per_edge": e.stored_bytes() as f64 * 8.0 / g.entries() as f64});
    res["ok"] = json!(true);
    emit(res, a);
}

fn scan_sum<G: Adj>(g: &G) -> u64 {
    g.chunks()
        .par_iter()
        .with_max_len(1)
        .map(|&(lo, hi)| {
            let mut cur = g.cur();
            let mut s = 0u64;
            g.for_range(lo, hi, &mut cur, |_, l| {
                for &x in l {
                    s = s.wrapping_add(x as u64);
                }
            });
            s
        })
        .reduce(|| 0, |x, y| x.wrapping_add(y))
}

fn scan_bench<G: Adj>(g: &G, a: &HashMap<String, String>, threads: usize, config: &str) {
    let reps: usize = a.get("reps").map(|s| s.parse().unwrap()).unwrap_or(3);
    let la = loadavg();
    let _ = scan_sum(g);
    let mut all = Vec::new();
    let mut cs = 0;
    for _ in 0..reps {
        let t = Instant::now();
        cs = scan_sum(g);
        all.push(t.elapsed().as_secs_f64());
    }
    let best = all.iter().cloned().fold(f64::INFINITY, f64::min);
    // random access to whole lists, single thread (as lab C: 200k random nodes, list materialized)
    let nodes: Vec<usize> = (0..200_000u64).map(|i| (pq::Rng(i ^ 0xABCD).next() % g.n() as u64) as usize).collect();
    let mut cur = g.cur();
    let mut tot = 0u64;
    let mut chk = 0u64;
    let mut sink: Vec<u32> = Vec::new();
    let t = Instant::now();
    for &v in &nodes {
        let l = g.nbrs(v, &mut cur);
        sink.clear();
        sink.extend_from_slice(l);
        tot += l.len() as u64;
        chk = chk.wrapping_add(sink.last().copied().unwrap_or(0) as u64);
    }
    let rs = t.elapsed().as_secs_f64();
    let t = Instant::now();
    let mut dsum = 0u64;
    for &v in &nodes {
        dsum += g.deg(v) as u64;
    }
    let ds = t.elapsed().as_secs_f64();
    assert_eq!(dsum, tot);
    std::hint::black_box(chk);
    emit(
        json!({
            "cmd": "scan", "graph": Path::new(&a["graph"]).file_name().unwrap().to_string_lossy(), "config": config, "mem": mem_of(a).name(), "threads": threads,
            "entries": g.entries(), "checksum": cs, "seconds_all": all, "seconds_best": best, "edges_per_s": g.entries() as f64 / best,
            "random_list_ns": rs * 1e9 / nodes.len() as f64, "random_list_ns_per_edge": rs * 1e9 / tot.max(1) as f64,
            "random_degree_ns": ds * 1e9 / nodes.len() as f64,
            "included": "warm; scan = sum of all targets over the work chunks; random list = 200k random nodes, 1 thread, list copied out",
            "loadavg_1m_start": la,
        }),
        a,
    );
}

fn cmd_scan(a: &HashMap<String, String>, threads: usize) {
    let config = a.get("config").cloned().unwrap_or("raw".into());
    let mem = mem_of(a);
    match config.as_str() {
        "raw" => scan_bench(&Raw::load(Path::new(&a["graph"]), mem).expect("raw"), a, threads, &config),
        "comp" => scan_bench(&Comp::open(Path::new(&a["store"]), mem).expect("store"), a, threads, &config),
        "comp-block" => scan_bench(&BlockView(&Comp::open(Path::new(&a["store"]), mem).expect("store")), a, threads, &config),
        x => panic!("bad --config {x}"),
    }
}

/// Cost of opening: raw arrays from disk, the compressed store, or the store decoded into raw arrays.
fn cmd_open(a: &HashMap<String, String>, threads: usize) {
    let what = a.get("what").cloned().unwrap_or("decoded".into());
    let mem = mem_of(a);
    let cold = a.contains_key("cold");
    let gdir = PathBuf::from(&a["graph"]);
    let sdir = a.get("store").map(PathBuf::from);
    let la = loadavg();
    let mut res = json!({"cmd": "open", "graph": gdir.file_name().unwrap().to_string_lossy(), "what": what, "mem": mem.name(), "threads": threads, "cold": cold, "loadavg_1m_start": la});
    let files: Vec<PathBuf> = match what.as_str() {
        "raw" => vec![gdir.join("targets.u32"), gdir.join("offsets.u64")],
        _ => {
            let s = sdir.clone().expect("--store");
            vec![s.join("adj.bin"), s.join("boff.ef"), s.join("blocks.u32")]
        }
    };
    if cold {
        for f in &files {
            evict_file(f).expect("evict");
        }
        res["resident_fraction_after_evict"] = json!(resident_fraction(&files[0]));
    }
    res["file_bytes"] = json!(files.iter().map(|f| std::fs::metadata(f).map(|m| m.len()).unwrap_or(0)).sum::<u64>());
    let t = Instant::now();
    match what.as_str() {
        "raw" => {
            let g = Raw::load(&gdir, mem).expect("raw");
            res["open_seconds"] = json!(t.elapsed().as_secs_f64());
            res["touch_checksum"] = json!(if mem == Mem::MmapLazy { scan_sum(&g) } else { 0 });
            res["total_seconds"] = json!(t.elapsed().as_secs_f64());
            res["entries"] = json!(g.entries());
        }
        "comp" => {
            let c = Comp::open(&sdir.unwrap(), mem).expect("store");
            res["open_seconds"] = json!(t.elapsed().as_secs_f64());
            res["total_seconds"] = json!(t.elapsed().as_secs_f64());
            res["entries"] = json!(c.entries);
            res["offsets_ef_ram_bytes"] = json!(c.boff.ram_bytes());
        }
        "decoded" => {
            let c = Comp::open(&sdir.unwrap(), mem).expect("store");
            let open_s = t.elapsed().as_secs_f64();
            let (g, deg_s, dec_s) = c.decode_to_raw();
            res["open_seconds"] = json!(open_s);
            res["degree_pass_seconds"] = json!(deg_s);
            res["decode_pass_seconds"] = json!(dec_s);
            res["decode_seconds"] = json!(deg_s + dec_s);
            res["total_seconds"] = json!(t.elapsed().as_secs_f64());
            res["decode_edges_per_s"] = json!(g.entries() as f64 / (deg_s + dec_s));
            res["entries"] = json!(g.entries());
            res["raw_bytes_in_ram"] = json!(g.entries() * 4 + (g.n + 1) * 8);
            res["rss_anon_gb_with_both"] = json!(gb(rss_anon()));
            res["rss_file_gb_with_both"] = json!(gb(rss_file()));
            drop(c);
            res["rss_anon_gb_after_dropping_store"] = json!(gb(rss_anon()));
            std::hint::black_box(&g);
        }
        x => panic!("bad --what {x}"),
    }
    res["peak_rss_gb"] = json!(gb(peak_rss()));
    res["included"] = json!("open = read/map the files (+ rebuild Elias-Fano select samples); decode = degree pass + prefix sum + parallel decode into anonymous memory; cold = files evicted from the page cache first");
    emit(res, a);
}

fn label_checksum(comp: &[std::sync::atomic::AtomicU32]) -> u64 {
    comp.par_iter().enumerate().map(|(i, c)| (c.load(Relaxed) as u64).wrapping_mul(i as u64 | 1).wrapping_mul(0x9E3779B97F4A7C15)).reduce(|| 0, |x, y| x.wrapping_add(y))
}

fn run_algos<G: Adj>(g: &G, a: &HashMap<String, String>, threads: usize, config: &str, open: &Value) {
    let reps: usize = a.get("reps").map(|s| s.parse().unwrap()).unwrap_or(3);
    let iters: usize = a.get("iters").map(|s| s.parse().unwrap()).unwrap_or(20);
    let gname = Path::new(&a["graph"]).file_name().unwrap().to_string_lossy().to_string();
    for algo in a.get("algos").cloned().unwrap_or("pr32".into()).split(',') {
        let la = loadavg();
        let mut secs: Vec<f64> = Vec::new();
        let mut result = json!({});
        let nrep = match algo {
            "tc" | "tc-comp" | "tc-naive" => a.get("tc-reps").map(|s| s.parse().unwrap()).unwrap_or(reps),
            _ => reps,
        };
        for _ in 0..nrep {
            let t = Instant::now();
            match algo {
                "pr32" => {
                    let r: Vec<f32> = kernels::pagerank(g, 0.85, iters);
                    secs.push(t.elapsed().as_secs_f64());
                    result = json!({"iters": iters, "pr": kernels::pr_summary(&r)});
                }
                "pr64" => {
                    let r: Vec<f64> = kernels::pagerank(g, 0.85, iters);
                    secs.push(t.elapsed().as_secs_f64());
                    result = json!({"iters": iters, "pr": kernels::pr_summary(&r)});
                }
                "wcc" => {
                    let (comp, frac) = kernels::wcc_afforest(g);
                    secs.push(t.elapsed().as_secs_f64());
                    result = json!({"components": kernels::count_components(&comp), "fraction_of_entries_examined": frac, "label_checksum": label_checksum(&comp)});
                }
                "bfs" => {
                    let src = g.max_degree_node();
                    let r = kernels::bfs_do(g, src);
                    secs.push(t.elapsed().as_secs_f64());
                    let dsum: u64 = r.depth.par_iter().map(|d| d.load(Relaxed)).filter(|&d| d != u32::MAX).map(|d| d as u64).sum();
                    result = json!({"source": src, "reached": r.reached, "max_depth": r.levels, "top_down_steps": r.td_steps, "bottom_up_steps": r.bu_steps,
                        "entries_examined": r.edges_examined, "fraction_examined": r.edges_examined as f64 / g.entries() as f64, "depth_sum": dsum});
                }
                "bfs-ds" => {
                    let (r, src) = bfs_ds::bfs_do_degree_in_state(g);
                    secs.push(t.elapsed().as_secs_f64());
                    let dsum: u64 = r.depth.par_iter().map(|d| d.load(Relaxed)).filter(|&d| d != u32::MAX).map(|d| d as u64).sum();
                    result = json!({"source": src, "reached": r.reached, "max_depth": r.levels, "top_down_steps": r.td_steps, "bottom_up_steps": r.bu_steps,
                        "entries_examined": r.edges_examined, "fraction_examined": r.edges_examined as f64 / g.entries() as f64, "depth_sum": dsum});
                }
                "maxdeg" => {
                    let src = g.max_degree_node();
                    secs.push(t.elapsed().as_secs_f64());
                    result = json!({"source": src});
                }
                "kcore" => {
                    let (core, maxc, rounds) = kernels::kcore(g);
                    secs.push(t.elapsed().as_secs_f64());
                    let csum: u64 = core.par_iter().map(|&c| c as u64).sum();
                    result = json!({"max_core": maxc, "peel_rounds": rounds, "coreness_sum": csum});
                }
                "tc" => {
                    let (tri, build) = kernels::triangles_fwd_raw(g);
                    secs.push(t.elapsed().as_secs_f64());
                    result = json!({"triangles": tri, "forward_build_seconds": build, "variant": "forward adjacency uncompressed (kernel-owned)"});
                }
                "tc-comp" => {
                    let (tri, build, bytes) = kernels::triangles_fwd_comp(g);
                    secs.push(t.elapsed().as_secs_f64());
                    result = json!({"triangles": tri, "forward_build_seconds": build, "forward_bytes": bytes, "forward_bits_per_undirected_edge": bytes as f64 * 16.0 / g.entries() as f64,
                        "variant": "forward adjacency compressed (kernel-owned), lists decoded per intersection"});
                }
                "tc-naive" => {
                    let tri = kernels::triangles_naive(g);
                    secs.push(t.elapsed().as_secs_f64());
                    result = json!({"triangles": tri, "sum_deg_squared": kernels::sum_deg_sq(g), "variant": "no derived adjacency: id-oriented, intersect input lists"});
                }
                "degsq" => {
                    let s = kernels::sum_deg_sq(g);
                    secs.push(t.elapsed().as_secs_f64());
                    result = json!({"sum_deg_squared": s});
                }
                x => panic!("bad algo {x}"),
            }
        }
        let best = secs.iter().cloned().fold(f64::INFINITY, f64::min);
        emit(
            json!({
                "cmd": "kernel", "graph": gname, "config": config, "mem": mem_of(a).name(), "algo": algo, "threads": threads,
                "n": g.n(), "entries": g.entries(), "seconds_all": secs, "seconds_best": best, "reps": nrep,
                "precision": match algo { "pr32" => "f32", "pr64" => "f64", _ => "n/a" },
                "warm": true, "included": "kernel only (its own O(n) state and derived structures included); graph open/decode excluded, reported in `open`",
                "open": open, "loadavg_1m_start": la,
                "peak_rss_gb": gb(peak_rss()), "rss_anon_gb": gb(rss_anon()), "anon_huge_gb": gb(anon_huge()), "result": result,
            }),
            a,
        );
    }
}

fn cmd_kernel(a: &HashMap<String, String>, threads: usize) {
    let config = a.get("config").cloned().unwrap_or("raw".into());
    let mem = mem_of(a);
    let t = Instant::now();
    match config.as_str() {
        "raw" => {
            let g = Raw::load(Path::new(&a["graph"]), mem).expect("raw");
            let open = json!({"open_seconds": t.elapsed().as_secs_f64()});
            run_algos(&g, a, threads, &config, &open);
        }
        "comp" | "comp-block" => {
            let c = Comp::open(Path::new(&a["store"]), mem).expect("store");
            let open = json!({"open_seconds": t.elapsed().as_secs_f64(), "store_bytes_in_ram": c.data_len + c.boff.ram_bytes()});
            if config == "comp" {
                run_algos(&c, a, threads, &config, &open);
            } else {
                run_algos(&BlockView(&c), a, threads, &config, &open);
            }
        }
        "decoded" => {
            let c = Comp::open(Path::new(&a["store"]), mem).expect("store");
            let open_s = t.elapsed().as_secs_f64();
            let (g, deg_s, dec_s) = c.decode_to_raw();
            drop(c);
            let open = json!({"open_seconds": open_s, "decode_seconds": deg_s + dec_s, "raw_bytes_in_ram": g.entries() * 4 + (g.n + 1) * 8});
            run_algos(&g, a, threads, &config, &open);
        }
        x => panic!("bad --config {x}"),
    }
}

fn pq_run<G: Adj>(g: &G, gin: Option<&G>, a: &HashMap<String, String>, config: &str) {
    let only: Vec<String> = a.get("queries").map(|s| s.split(',').map(|x| x.to_string()).collect()).unwrap_or_default();
    let n_small: usize = a.get("n-small").map(|s| s.parse().unwrap()).unwrap_or(100_000);
    let n_big: usize = a.get("n-big").map(|s| s.parse().unwrap()).unwrap_or(10_000);
    let reps: usize = a.get("reps").map(|s| s.parse().unwrap()).unwrap_or(3);
    let la = loadavg();
    let r = pq::run_all(g, gin.unwrap_or(g), &only, n_small, n_big, reps);
    emit(
        json!({
            "cmd": "pq", "graph": Path::new(&a["graph"]).file_name().unwrap().to_string_lossy(), "directed_with_in_adjacency": gin.is_some(),
            "config": config, "mem": mem_of(a).name(), "threads": 1, "warm": true, "reps": reps,
            "included": "in-process call, single thread, per-query Instant pair (timer cost inside), best repetition by p50; inputs = lab generator",
            "loadavg_1m_start": la, "anon_huge_gb": gb(anon_huge()), "queries": r,
        }),
        a,
    );
}

fn cmd_pq(a: &HashMap<String, String>, _threads: usize) {
    let config = a.get("config").cloned().unwrap_or("raw".into());
    let mem = mem_of(a);
    match config.as_str() {
        "raw" => {
            let g = Raw::load(Path::new(&a["graph"]), mem).expect("raw");
            let gin = a.get("in-graph").map(|d| Raw::load(Path::new(d), mem).expect("raw in"));
            pq_run(&g, gin.as_ref(), a, &config);
        }
        "comp" => {
            let g = Comp::open(Path::new(&a["store"]), mem).expect("store");
            let gin = a.get("in-store").map(|d| Comp::open(Path::new(d), mem).expect("store in"));
            pq_run(&g, gin.as_ref(), a, &config);
        }
        "comp-block" => {
            let c = Comp::open(Path::new(&a["store"]), mem).expect("store");
            pq_run(&BlockView(&c), None, a, &config);
        }
        "decoded" => {
            let dec = |d: &String| Comp::open(Path::new(d), mem).expect("store").decode_to_raw().0;
            let g = dec(&a["store"]);
            let gin = a.get("in-store").map(dec);
            pq_run(&g, gin.as_ref(), a, &config);
        }
        x => panic!("bad --config {x}"),
    }
}
