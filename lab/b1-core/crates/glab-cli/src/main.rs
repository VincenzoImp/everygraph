use glab_core::extmem::{self, EdgeStream, IoMode};
use glab_core::kernels::{self, Real};
use glab_core::{edge_chunks, ingest, Csr, LoadMode};
use rayon::prelude::*;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering::Relaxed;
use std::time::Instant;

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

fn loadavg() -> f64 {
    std::fs::read_to_string("/proc/loadavg").ok().and_then(|s| s.split_whitespace().next().and_then(|x| x.parse().ok())).unwrap_or(-1.0)
}

fn emit(mut v: Value, a: &HashMap<String, String>) {
    v["loadavg_1m_at_end"] = json!(loadavg());
    if let Some(l) = a.get("label") {
        v["label"] = json!(l);
    }
    let s = serde_json::to_string(&v).unwrap();
    println!("{s}");
    if let Some(p) = a.get("json") {
        use std::io::Write;
        let mut f = std::fs::File::options().create(true).append(true).open(p).expect("json out");
        writeln!(f, "{s}").unwrap();
    }
}

fn gb(x: u64) -> f64 {
    x as f64 / (1u64 << 30) as f64
}

fn main() {
    let argv: Vec<String> = std::env::args().collect();
    if argv.len() < 2 {
        eprintln!("usage: glab <ingest|scan|run|semiext|evict> [--key value ...]");
        std::process::exit(2);
    }
    let a = args_map(&argv[2..]);
    if a.get("thp").map(|s| s == "off").unwrap_or(false) {
        // disable transparent huge pages for the whole process (probe d)
        unsafe { libc::prctl(libc::PR_SET_THP_DISABLE, 1, 0, 0, 0) };
    }
    let threads: usize = a.get("threads").map(|s| s.parse().unwrap()).unwrap_or_else(|| std::thread::available_parallelism().map(|x| x.get()).unwrap_or(1));
    rayon::ThreadPoolBuilder::new().num_threads(threads).build_global().unwrap();
    match argv[1].as_str() {
        "ingest" => cmd_ingest(&a, threads),
        "scan" => cmd_scan(&a, threads),
        "run" => cmd_run(&a, threads),
        "semiext" => cmd_semiext(&a, threads),
        "verify" => cmd_verify(&a),
        "evict" => {
            let dir = PathBuf::from(&a["graph"]);
            for f in ["targets.u32", "offsets.u64", "ids.u64"] {
                let _ = glab_core::evict_file(&dir.join(f));
            }
        }
        x => {
            eprintln!("unknown command {x}");
            std::process::exit(2);
        }
    }
}

fn cmd_ingest(a: &HashMap<String, String>, threads: usize) {
    let inp = &a["input"];
    let input = if inp == "-" {
        ingest::Input::Stdin
    } else if a.contains_key("gz-inproc") {
        ingest::Input::GzInProc(PathBuf::from(inp))
    } else {
        ingest::Input::File(PathBuf::from(inp))
    };
    let out_dir = PathBuf::from(&a["out"]);
    let cfg = ingest::Config {
        input,
        tmp_dir: a.get("tmp").map(PathBuf::from).unwrap_or_else(|| out_dir.join("tmp")),
        out_dir,
        mem_budget: (a.get("mem-gb").map(|s| s.parse::<f64>().unwrap()).unwrap_or(8.0) * (1u64 << 30) as f64) as u64,
        threads,
        fsync: a.contains_key("fsync"),
        parse_only: a.contains_key("parse-only"),
        bucket_mb: a.get("bucket-mb").map(|s| s.parse().unwrap()).unwrap_or(4),
        out_mmap: a.contains_key("out-mmap"),
        direct_in: a.contains_key("direct-in"),
        direct_tmp: a.contains_key("direct-tmp"),
        prealloc: a.contains_key("prealloc"),
    };
    match ingest::run(&cfg) {
        Ok(mut v) => {
            v["cmd"] = json!("ingest");
            v["input"] = json!(inp);
            emit(v, a);
        }
        Err(e) => {
            eprintln!("ingest failed: {e}");
            std::process::exit(1);
        }
    }
}

/// I/O + newline-scan ceiling: parallel pread of the file and memchr count of '\n'.
fn cmd_scan(a: &HashMap<String, String>, threads: usize) {
    use std::os::unix::fs::FileExt;
    let f = std::fs::File::open(&a["input"]).unwrap();
    let len = f.metadata().unwrap().len() as usize;
    const C: usize = 4 << 20;
    let t = Instant::now();
    let lines: usize = (0..len.div_ceil(C))
        .into_par_iter()
        .map_init(
            || vec![0u8; C],
            |buf, i| {
                let o = i * C;
                let e = (o + C).min(len);
                f.read_exact_at(&mut buf[..e - o], o as u64).unwrap();
                memchr_count(&buf[..e - o])
            },
        )
        .sum();
    let s = t.elapsed().as_secs_f64();
    emit(json!({"cmd": "scan", "threads": threads, "bytes": len, "lines": lines, "seconds": s, "gb_per_s": len as f64 / 1e9 / s}), a);
}
fn memchr_count(b: &[u8]) -> usize {
    b.iter().filter(|&&c| c == b'\n').count()
}

fn load_mode(a: &HashMap<String, String>) -> LoadMode {
    match a.get("load").map(|s| s.as_str()).unwrap_or("mmap") {
        "mmap" => LoadMode::Mmap,
        "heap" => LoadMode::Heap,
        "heap-huge" => LoadMode::HeapHuge,
        "heap-nohuge" => LoadMode::HeapNoHuge,
        x => panic!("bad --load {x}"),
    }
}

fn cmd_run(a: &HashMap<String, String>, threads: usize) {
    let dir = PathBuf::from(&a["graph"]);
    let algo = a.get("algo").cloned().unwrap_or("pr64".into());
    let reps: usize = a.get("reps").map(|s| s.parse().unwrap()).unwrap_or(1);
    let iters: usize = a.get("iters").map(|s| s.parse().unwrap()).unwrap_or(20);
    let mode = load_mode(a);
    let t0 = Instant::now();
    let mut g = Csr::load(&dir, mode).expect("load graph");
    let load_s = t0.elapsed().as_secs_f64();
    let order = a.get("order").cloned().unwrap_or("orig".into());
    let mut reorder_s = 0.0;
    if order != "orig" {
        let t = Instant::now();
        let ord = match order.as_str() {
            "degree" => kernels::degree_desc_order(&g),
            "bfs" => kernels::bfs_order(&g, g.max_degree_node()),
            "random" => kernels::random_order(g.n),
            x => panic!("bad --order {x}"),
        };
        g = glab_core::reorder(&g, &ord);
        reorder_s = t.elapsed().as_secs_f64();
    }
    let chunk_edges: u64 = a.get("chunk-edges").map(|s| s.parse().unwrap()).unwrap_or(1 << 16);
    let chunks = edge_chunks(&g.offsets, chunk_edges);
    let ids64 = a.get("ids").map(|s| s == "u64").unwrap_or(false);
    let t64: Vec<u64> = if ids64 { g.targets.par_iter().map(|&x| x as u64).collect() } else { Vec::new() };
    let mut secs: Vec<f64> = Vec::new();
    let mut result = json!({});
    for _ in 0..reps {
        let t = Instant::now();
        match algo.as_str() {
            "pr64" | "pr32" => {
                let summary = if algo == "pr64" {
                    let r: Vec<f64> = if ids64 { kernels::pagerank(&g.offsets, &t64, 0.85, iters, &chunks) } else { kernels::pagerank(&g.offsets, &g.targets, 0.85, iters, &chunks) };
                    secs.push(t.elapsed().as_secs_f64());
                    kernels::pr_summary(&r)
                } else {
                    let r: Vec<f32> = if ids64 { kernels::pagerank(&g.offsets, &t64, 0.85, iters, &chunks) } else { kernels::pagerank(&g.offsets, &g.targets, 0.85, iters, &chunks) };
                    secs.push(t.elapsed().as_secs_f64());
                    kernels::pr_summary(&r)
                };
                result = json!({"iters": iters, "pr": summary});
            }
            "wcc" => {
                let (comp, frac) = kernels::wcc_afforest(&g, &chunks);
                secs.push(t.elapsed().as_secs_f64());
                let nc = kernels::count_components(&comp);
                let giant = largest_component(&comp);
                result = json!({"components": nc, "largest_component": giant, "fraction_of_entries_examined": frac, "label_checksum": label_checksum(&comp)});
            }
            "wcc-stream" => {
                let comp = kernels::wcc_stream(&g, &chunks);
                secs.push(t.elapsed().as_secs_f64());
                result = json!({"components": kernels::count_components(&comp), "largest_component": largest_component(&comp), "label_checksum": label_checksum(&comp)});
            }
            "bfs" => {
                let src = g.max_degree_node();
                let r = kernels::bfs_do(&g, src, &chunks);
                secs.push(t.elapsed().as_secs_f64());
                let dsum: u64 = r.depth.par_iter().map(|d| d.load(Relaxed)).filter(|&d| d != u32::MAX).map(|d| d as u64).sum();
                result = json!({"source": src, "source_degree": g.deg(src), "reached": r.reached, "max_depth": r.levels,
                    "top_down_steps": r.td_steps, "bottom_up_steps": r.bu_steps, "entries_examined": r.edges_examined,
                    "fraction_examined": r.edges_examined as f64 / g.entries() as f64, "depth_sum": dsum,
                    "mteps_of_all_entries": 0});
            }
            "kcore" => {
                let (core, maxc, rounds) = kernels::kcore(&g);
                secs.push(t.elapsed().as_secs_f64());
                let csum: u64 = core.par_iter().map(|&c| c as u64).sum();
                result = json!({"max_core": maxc, "peel_rounds": rounds, "coreness_sum": csum});
            }
            "tc" => {
                let (tri, build) = kernels::triangles(&g);
                secs.push(t.elapsed().as_secs_f64());
                result = json!({"triangles": tri, "forward_build_seconds": build});
            }
            x => panic!("bad --algo {x}"),
        }
    }
    let best = secs.iter().cloned().fold(f64::INFINITY, f64::min);
    emit(
        json!({
            "cmd": "run", "graph": dir.file_name().unwrap().to_string_lossy(), "algo": algo, "threads": threads,
            "load": format!("{mode:?}"), "thp": a.get("thp").cloned().unwrap_or("default".into()), "order": order, "ids": if ids64 { "u64" } else { "u32" },
            "n": g.n, "entries": g.entries(), "load_seconds": load_s, "reorder_seconds": reorder_s,
            "seconds_all": secs, "seconds_best": best,
            "entries_per_s_best": g.entries() as f64 * (if algo.starts_with("pr") { iters as f64 } else { 1.0 }) / best,
            "peak_rss_gb": gb(glab_core::peak_rss()), "rss_anon_gb": gb(glab_core::rss_anon()), "result": result,
        }),
        a,
    );
}

fn largest_component(comp: &[std::sync::atomic::AtomicU32]) -> usize {
    // labels are roots after compress; count the most common via the root of node with max count sample
    let n = comp.len();
    let cnt: Vec<std::sync::atomic::AtomicU32> = (0..n).into_par_iter().map(|_| std::sync::atomic::AtomicU32::new(0)).collect();
    comp.par_iter().for_each(|c| {
        cnt[c.load(Relaxed) as usize].fetch_add(1, Relaxed);
    });
    cnt.par_iter().map(|c| c.load(Relaxed) as usize).max().unwrap_or(0)
}
fn label_checksum(comp: &[std::sync::atomic::AtomicU32]) -> u64 {
    comp.par_iter().enumerate().map(|(i, c)| (c.load(Relaxed) as u64).wrapping_mul(i as u64 | 1).wrapping_mul(0x9E3779B97F4A7C15)).reduce(|| 0, |x, y| x.wrapping_add(y))
}

fn read_offsets(dir: &Path) -> Vec<u64> {
    let b = std::fs::read(dir.join("offsets.u64")).expect("offsets");
    b.chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect()
}

fn cmd_semiext(a: &HashMap<String, String>, threads: usize) {
    let dir = PathBuf::from(&a["graph"]);
    let algo = a.get("algo").cloned().unwrap_or("pr32".into());
    let iters: usize = a.get("iters").map(|s| s.parse().unwrap()).unwrap_or(20);
    let io = a.get("io").cloned().unwrap_or("direct".into());
    let chunk_mb: u64 = a.get("chunk-mb").map(|s| s.parse().unwrap()).unwrap_or(8);
    let t0 = Instant::now();
    let offsets = read_offsets(&dir);
    let n = offsets.len() - 1;
    let entries = offsets[n];
    let chunks = edge_chunks(&offsets, chunk_mb * (1 << 20) / 4);
    let load_s = t0.elapsed().as_secs_f64();
    let t = Instant::now();
    let mut iter_secs: Vec<f64> = vec![];
    let mut bytes_read = 0u64;
    let mut io_thread_s = 0f64;
    let result;
    if io == "mmap-hint" || io == "mmap-pageout" {
        let f = std::fs::File::open(dir.join("targets.u32")).unwrap();
        let m = unsafe { memmap2_map(&f) };
        let tg: &[u32] = unsafe { std::slice::from_raw_parts(m.0 as *const u32, entries as usize) };
        let pageout = io == "mmap-pageout";
        result = match algo.as_str() {
            "pr32" => {
                let (r, s) = extmem::pagerank_mmap_hinted::<f32>(&offsets, tg, 0.85, iters, &chunks, pageout);
                iter_secs = s;
                json!({"iters": iters, "pr": kernels::pr_summary(&r)})
            }
            "pr64" => {
                let (r, s) = extmem::pagerank_mmap_hinted::<f64>(&offsets, tg, 0.85, iters, &chunks, pageout);
                iter_secs = s;
                json!({"iters": iters, "pr": kernels::pr_summary(&r)})
            }
            x => panic!("bad algo {x} for mmap-hint"),
        };
    } else {
        let mode = if io == "direct" { IoMode::Direct } else { IoMode::Buffered };
        let es = EdgeStream::open(&dir.join("targets.u32"), mode, &offsets, &chunks).expect("open targets");
        result = match algo.as_str() {
            "pr32" => {
                let (r, s) = extmem::pagerank_semiext::<f32>(&offsets, &es, 0.85, iters, &chunks);
                iter_secs = s;
                json!({"iters": iters, "pr": kernels::pr_summary(&r)})
            }
            "pr64" => {
                let (r, s) = extmem::pagerank_semiext::<f64>(&offsets, &es, 0.85, iters, &chunks);
                iter_secs = s;
                json!({"iters": iters, "pr": kernels::pr_summary(&r)})
            }
            "wcc" => {
                let comp = extmem::wcc_semiext(&offsets, &es, &chunks);
                json!({"components": kernels::count_components(&comp), "largest_component": largest_component(&comp), "label_checksum": label_checksum(&comp)})
            }
            x => panic!("bad algo {x}"),
        };
        bytes_read = es.bytes_read.load(Relaxed);
        io_thread_s = es.io_nanos.load(Relaxed) as f64 / 1e9;
    }
    let secs = t.elapsed().as_secs_f64();
    emit(
        json!({
            "cmd": "semiext", "graph": dir.file_name().unwrap().to_string_lossy(), "algo": algo, "io": io, "threads": threads,
            "n": n, "entries": entries, "chunk_mb": chunk_mb, "load_offsets_seconds": load_s, "seconds": secs,
            "iter_seconds": iter_secs, "bytes_read_gb": bytes_read as f64 / 1e9,
            "read_gb_per_s_wall": bytes_read as f64 / 1e9 / secs, "io_thread_seconds": io_thread_s,
            "peak_rss_gb": gb(glab_core::peak_rss()), "rss_anon_gb": gb(glab_core::rss_anon()), "result": result,
        }),
        a,
    );
}

struct RawMap(*const u8, #[allow(dead_code)] usize);
unsafe fn memmap2_map(f: &std::fs::File) -> RawMap {
    use std::os::unix::io::AsRawFd;
    let len = f.metadata().unwrap().len() as usize;
    let p = libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ, libc::MAP_SHARED, f.as_raw_fd(), 0);
    assert!(p != libc::MAP_FAILED);
    RawMap(p as *const u8, len)
}
#[allow(dead_code)]
fn _unused<F: Real>() {}

/// Structural validation of a glab-csr-v0 directory: offsets monotone, targets in range, strictly
/// increasing per node (sorted + deduplicated), no self loops, symmetric (order-independent hash of
/// (u,v) pairs must equal that of (v,u) pairs), ids strictly increasing.
fn cmd_verify(a: &HashMap<String, String>) {
    let dir = PathBuf::from(&a["graph"]);
    let g = Csr::load(&dir, LoadMode::Mmap).expect("load");
    let n = g.n;
    let h = |u: u64, v: u64| -> u64 {
        let mut x = (u << 32 | v).wrapping_mul(0x9E3779B97F4A7C15);
        x ^= x >> 29;
        x = x.wrapping_mul(0xBF58476D1CE4E5B9);
        x ^ (x >> 32)
    };
    let chunks = edge_chunks(&g.offsets, 1 << 18);
    let (bad, fwd, rev, xor_t) = chunks
        .par_iter()
        .map(|&(a0, b0)| {
            let (mut bad, mut fwd, mut rev, mut x) = (0u64, 0u64, 0u64, 0u64);
            for u in a0..b0 {
                if g.offsets[u + 1] < g.offsets[u] {
                    bad += 1;
                    continue;
                }
                let mut prev: i64 = -1;
                for &v in g.nbrs(u) {
                    if v as usize >= n || v as usize == u || (v as i64) <= prev {
                        bad += 1;
                    }
                    prev = v as i64;
                    fwd = fwd.wrapping_add(h(u as u64, v as u64));
                    rev = rev.wrapping_add(h(v as u64, u as u64));
                    x = x.wrapping_add((v as u64 + 1).wrapping_mul(g.offsets[u] | 1));
                }
            }
            (bad, fwd, rev, x)
        })
        .reduce(|| (0, 0, 0, 0), |p, q| (p.0 + q.0, p.1.wrapping_add(q.1), p.2.wrapping_add(q.2), p.3.wrapping_add(q.3)));
    let ids = std::fs::read(dir.join("ids.u64")).expect("ids");
    let ids_ok = ids.len() == n * 8 && ids.par_chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect::<Vec<_>>().windows(2).all(|w| w[0] < w[1]);
    let maxdeg = g.deg(g.max_degree_node());
    let ok = bad == 0 && fwd == rev && ids_ok && g.offsets[0] == 0;
    emit(json!({"cmd": "verify", "graph": dir.to_string_lossy(), "n": n, "entries": g.entries(), "m_undirected": g.entries() / 2,
        "violations": bad, "symmetric": fwd == rev, "ids_strictly_increasing": ids_ok, "max_degree": maxdeg,
        "pair_hash": format!("{fwd:016x}"), "content_hash": format!("{xor_t:016x}"), "ok": ok}), a);
    if !ok {
        std::process::exit(1);
    }
}
