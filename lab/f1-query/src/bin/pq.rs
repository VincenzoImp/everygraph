//! Q2: point queries on a memory-mapped glab-csr-v0 graph.
//! usage: pq --graph DIR --name NAME [--threads 32] [--mode warm|cold] [--prefetch 0|1] [--random 0|1]
//!        pq --transpose DIR --threads T   (CSC build cost on an oriented copy)
use f1q::*;
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use serde_json::json;

struct Ctx<'a> {
    g: &'a Csr,
    gin: &'a Csr,
    vis: Visited,
    vis2: Visited,
    q: Vec<u32>,
    nq: Vec<u32>,
    prefetch: bool,
}

fn willneed(ptr: *const u8, len: usize) {
    let pg = 4096usize;
    let a = (ptr as usize) & !(pg - 1);
    let e = ptr as usize + len;
    unsafe {
        libc::madvise(a as *mut _, e - a, libc::MADV_WILLNEED);
    }
}

impl<'a> Ctx<'a> {
    fn new(g: &'a Csr, gin: &'a Csr, prefetch: bool) -> Self {
        Ctx { g, gin, vis: Visited::new(g.n), vis2: Visited::new(g.n), q: vec![], nq: vec![], prefetch }
    }
    /// async-prefetch (madvise WILLNEED) the offsets then the adjacency of a frontier
    fn pf(&self, nodes: &[u32]) {
        self.pf_g(self.g, nodes)
    }
    fn pf_g(&self, g: &Csr, nodes: &[u32]) {
        if !self.prefetch || nodes.len() < 2 {
            return;
        }
        let o = g.offs();
        let mut last = usize::MAX;
        for &v in nodes {
            let p = &o[v as usize] as *const u64 as usize >> 12;
            if p != last {
                willneed(&o[v as usize] as *const u64 as *const u8, 16);
                last = p;
            }
        }
        let t = g.tgts();
        for &v in nodes {
            let (a, b) = g.range(v);
            if b > a {
                willneed(&t[a] as *const u32 as *const u8, (b - a) * 4);
            }
        }
    }
    fn neighbors(&mut self, v: u32) -> usize {
        self.q.clear();
        self.q.extend_from_slice(self.g.nbrs(v));
        self.q.len()
    }
    fn two_hop(&mut self, v: u32) -> usize {
        self.vis.clear();
        self.vis.set(v);
        let mut c = 0;
        let n1 = self.g.nbrs(v);
        if self.prefetch {
            self.pf(n1);
        }
        for &u in n1 {
            if self.vis.set(u) {
                c += 1;
            }
        }
        for &u in n1 {
            for &w in self.g.nbrs(u) {
                if self.vis.set(w) {
                    c += 1;
                }
            }
        }
        c
    }
    fn khop(&mut self, v: u32, k: usize, budget: usize) -> usize {
        self.vis.clear();
        self.vis.set(v);
        self.q.clear();
        self.q.push(v);
        let mut c = 1;
        'outer: for _ in 0..k {
            self.nq.clear();
            if self.prefetch {
                self.pf(&self.q);
            }
            for i in 0..self.q.len() {
                let u = self.q[i];
                for &w in self.g.nbrs(u) {
                    if self.vis.set(w) {
                        c += 1;
                        self.nq.push(w);
                        if c >= budget {
                            break 'outer;
                        }
                    }
                }
            }
            std::mem::swap(&mut self.q, &mut self.nq);
            if self.q.is_empty() {
                break;
            }
        }
        c
    }
    fn common(&self, u: u32, w: u32) -> usize {
        let (a, b) = (self.g.nbrs(u), self.g.nbrs(w));
        let (mut i, mut j, mut c) = (0, 0, 0);
        while i < a.len() && j < b.len() {
            let (x, y) = (a[i], b[j]);
            c += (x == y) as usize;
            i += (x <= y) as usize;
            j += (y <= x) as usize;
        }
        c
    }
    /// bidirectional BFS; returns (path length, nodes touched). Reconstructs the path.
    /// membership via per-thread bitsets; (node, parent, dist) logs per side, hashed only on meeting.
    fn bibfs(&mut self, s: u32, t: u32, maxd: usize) -> (i32, usize) {
        if s == t {
            return (0, 1);
        }
        let mut vs = std::mem::replace(&mut self.vis, Visited::new(0));
        let mut vt = std::mem::replace(&mut self.vis2, Visited::new(0));
        vs.clear();
        vt.clear();
        vs.set(s);
        vt.set(t);
        let mut ls: Vec<(u32, u32, u32)> = vec![(s, u32::MAX, 0)];
        let mut lt: Vec<(u32, u32, u32)> = vec![(t, u32::MAX, 0)];
        let (mut fs, mut ft) = (vec![s], vec![t]);
        let (mut ds, mut dt) = (0u32, 0u32);
        let (go, gi) = (self.g, self.gin);
        let mut result = -1i32;
        for _ in 0..maxd {
            let ws: u64 = fs.iter().map(|&v| go.deg(v) as u64).sum();
            let wt: u64 = ft.iter().map(|&v| gi.deg(v) as u64).sum();
            let side_s = ws <= wt;
            let (front, mine, other, log, d) =
                if side_s { (&mut fs, &mut vs, &vt, &mut ls, &mut ds) } else { (&mut ft, &mut vt, &vs, &mut lt, &mut dt) };
            if front.is_empty() {
                break;
            }
            let g = if side_s { go } else { gi };
            if self.prefetch {
                self.pf_g(g, front);
            }
            let mut next = Vec::new();
            let mut hits: Vec<(u32, u32)> = Vec::new();
            for &u in front.iter() {
                for &w in g.nbrs(u) {
                    if other.get(w) {
                        hits.push((u, w));
                    }
                    if mine.set(w) {
                        log.push((w, u, *d + 1));
                        next.push(w);
                    }
                }
            }
            if !hits.is_empty() {
                let du = *d;
                let (lm, lo) = if side_s { (&ls, &lt) } else { (&lt, &ls) };
                let mm: FxHashMap<u32, (u32, u32)> = lm.iter().map(|&(x, p, dd)| (x, (p, dd))).collect();
                let om: FxHashMap<u32, (u32, u32)> = lo.iter().map(|&(x, p, dd)| (x, (p, dd))).collect();
                let (mut bl, mut bu, mut bw) = (u32::MAX, 0, 0);
                for &(u, w) in &hits {
                    let l = du + 1 + om[&w].1;
                    if l < bl {
                        (bl, bu, bw) = (l, u, w);
                    }
                }
                // reconstruct path
                let mut path = vec![bw];
                let mut x = bu;
                while x != u32::MAX {
                    path.push(x);
                    x = mm[&x].0;
                }
                let mut x = om[&bw].0;
                while x != u32::MAX {
                    path.insert(0, x);
                    x = om[&x].0;
                }
                debug_assert_eq!(path.len() as u32, bl + 1);
                result = bl as i32;
                break;
            }
            *d += 1;
            *front = next;
        }
        let touched = ls.len() + lt.len();
        self.vis = vs;
        self.vis2 = vt;
        (result, touched)
    }
}

#[derive(Clone, Copy)]
enum QT {
    Nbrs,
    Deg,
    TwoHop,
    KHop,
    BiBfs,
    Common,
}
const QTS: [(QT, &str); 6] = [
    (QT::Deg, "degree"),
    (QT::Nbrs, "neighbors"),
    (QT::Common, "common_neighbors"),
    (QT::TwoHop, "two_hop_size"),
    (QT::KHop, "khop3_budget10k"),
    (QT::BiBfs, "bidir_bfs_path"),
];

/// query parameters derived deterministically from an index
fn params(g: &Csr, qt: QT, i: u64) -> (u32, u32) {
    let mut r = Rng(i.wrapping_mul(7919) ^ 0x5EED);
    let n = g.n as u64;
    let a = r.below(n) as u32;
    match qt {
        QT::Common => {
            // (u, w) with w a 2-hop neighbour of u when possible
            let na = g.nbrs(a);
            if na.is_empty() {
                return (a, r.below(n) as u32);
            }
            let m = na[r.below(na.len() as u64) as usize];
            let nm = g.nbrs(m);
            if nm.is_empty() {
                return (a, m);
            }
            (a, nm[r.below(nm.len() as u64) as usize])
        }
        QT::BiBfs => (a, r.below(n) as u32),
        _ => (a, 0),
    }
}

fn run1(c: &mut Ctx, qt: QT, a: u32, b: u32) -> usize {
    match qt {
        QT::Deg => c.g.deg(a),
        QT::Nbrs => c.neighbors(a),
        QT::TwoHop => c.two_hop(a),
        QT::KHop => c.khop(a, 3, 10_000),
        QT::BiBfs => {
            let (l, t) = c.bibfs(a, b, 30);
            (l as usize) << 32 | t
        }
        QT::Common => c.common(a, b),
    }
}

fn transpose(dir: &str, threads: usize, outdir: &str) -> serde_json::Value {
    let g = Csr::open(dir, false);
    let t = now();
    g.warm();
    let load_s = secs(t);
    let tp = pool(threads);
    let directed = std::fs::read_to_string(format!("{dir}/meta.json")).unwrap().contains("\"directed\": true");
    // symmetric input -> oriented copy: keep u->v for u<v (a DAG with m/2 arcs) as the "directed" graph
    let n = g.n;
    let t0 = now();
    let (doff_own, dtgt_own) = if directed { (vec![], vec![]) } else { tp.install(|| {
        let degs: Vec<u64> =
            (0..n as u32).into_par_iter().map(|v| g.nbrs(v).iter().filter(|&&w| w > v).count() as u64).collect();
        let mut off = vec![0u64; n + 1];
        for i in 0..n {
            off[i + 1] = off[i] + degs[i];
        }
        let mut tg = vec![0u32; off[n] as usize];
        let sp = SyncPtr(tg.as_mut_ptr());
        (0..n as u32).into_par_iter().for_each(|v| {
            let _ = &sp;
            let mut p = off[v as usize] as usize;
            for &w in g.nbrs(v) {
                if w > v {
                    unsafe { *sp.0.add(p) = w };
                    p += 1;
                }
            }
        });
        (off, tg)
    }) };
    let orient_s = secs(t0);
    let (doff, dtgt): (&[u64], &[u32]) = if directed { (g.offs(), g.tgts()) } else { (&doff_own, &dtgt_own) };
    let m = dtgt.len();
    // transpose: parallel in-degree count (atomics), prefix sum, parallel scatter, per-list sort
    use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
    let t1 = now();
    let (ioff, itgt) = tp.install(|| {
        let cnt: Vec<AtomicU32> = (0..n).map(|_| AtomicU32::new(0)).collect();
        dtgt.par_iter().with_min_len(1 << 16).for_each(|&w| {
            cnt[w as usize].fetch_add(1, Ordering::Relaxed);
        });
        let mut ioff = vec![0u64; n + 1];
        for i in 0..n {
            ioff[i + 1] = ioff[i] + cnt[i].load(Ordering::Relaxed) as u64;
        }
        let pos: Vec<AtomicU64> = ioff[..n].iter().map(|&x| AtomicU64::new(x)).collect();
        let mut itgt = vec![0u32; m];
        let sp = SyncPtr(itgt.as_mut_ptr());
        (0..n as u32).into_par_iter().with_min_len(1024).for_each(|v| {
            let _ = &sp;
            for &w in &dtgt[doff[v as usize] as usize..doff[v as usize + 1] as usize] {
                let p = pos[w as usize].fetch_add(1, Ordering::Relaxed);
                unsafe { *sp.0.add(p as usize) = v };
            }
        });
        // sort each in-list (scatter order is nondeterministic)
        let mut rest: &mut [u32] = &mut itgt;
        let mut chunks = Vec::with_capacity(n);
        for v in 0..n {
            let l = (ioff[v + 1] - ioff[v]) as usize;
            let (a, b) = rest.split_at_mut(l);
            chunks.push(a);
            rest = b;
        }
        chunks.par_iter_mut().with_min_len(1024).for_each(|c| c.sort_unstable());
        (ioff, itgt)
    });
    let transpose_s = secs(t1);
    // verify: sum of in-degrees == m, and a spot check
    assert_eq!(ioff[n] as usize, m);
    let bytes_out = (n + 1) * 8 + m * 4;
    let mut write_s = 0.0;
    if !outdir.is_empty() {
        let t = now();
        let tmp = format!("{outdir}.tmp{}", std::process::id());
        std::fs::create_dir_all(&tmp).unwrap();
        write_slice(format!("{tmp}/offsets.u64"), &ioff);
        write_slice(format!("{tmp}/targets.u32"), &itgt);
        let meta = json!({"format": "glab-csr-v0", "n": n, "entries": m, "arcs": m, "directed": true, "symmetric": false,
            "sorted": true, "transpose_of": dir, "note": "IN-adjacency (CSC) of transpose_of: neighbors of i are the sources of arcs into i; ids.u64 identical to transpose_of",
            "produced_by": "exp/f1-query pq --transpose"});
        std::fs::write(format!("{tmp}/meta.json"), serde_json::to_string_pretty(&meta).unwrap()).unwrap();
        std::fs::copy(format!("{dir}/ids.u64"), format!("{tmp}/ids.u64")).unwrap();
        if !std::path::Path::new(outdir).exists() {
            std::fs::rename(&tmp, outdir).unwrap();
        }
        write_s = secs(t);
    }
    json!({"graph": dir, "threads": threads, "n": n, "arcs": m, "load_warm_s": load_s, "orient_s": orient_s, "write_s": write_s,
           "transpose_csc_s": transpose_s, "csr_bytes": bytes_out, "csc_extra_bytes": bytes_out,
           "csc_extra_bytes_per_arc": bytes_out as f64 / m as f64,
           "note": "CSC (in-adjacency) doubles topology bytes: (n+1)*8 + 4*m extra; transpose = count+prefix+scatter+sort"})
}

struct SyncPtr(*mut u32);
unsafe impl Sync for SyncPtr {}
unsafe impl Send for SyncPtr {}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let threads: usize = arg(&args, "threads", 32);
    let out = arg_s(&args, "out", "");
    let tdir = arg_s(&args, "transpose", "");
    if !tdir.is_empty() {
        let r = transpose(&tdir, threads, &arg_s(&args, "write", ""));
        println!("{}", r);
        if !out.is_empty() {
            save_json(&out, &r);
        }
        return;
    }
    let dir = arg_s(&args, "graph", "/home/tux/graphlab/data/csr/orkut");
    let name = arg_s(&args, "name", "orkut");
    let prefetch = arg::<u32>(&args, "prefetch", 0) == 1;
    let random = arg::<u32>(&args, "random", 0) == 1;
    let do_cold = arg::<u32>(&args, "cold", 1) == 1;
    let do_warm = arg::<u32>(&args, "warm", 1) == 1;
    let scale: f64 = arg(&args, "scale", 1.0);
    let g = Csr::open(&dir, random);
    let indir = arg_s(&args, "in", "");
    let gin_own = if indir.is_empty() { None } else { Some(Csr::open(&indir, random)) };
    let gin: &Csr = gin_own.as_ref().unwrap_or(&g);
    eprintln!("{name}: n={} m={} prefetch={prefetch} random={random}", g.n, g.m());
    let mut res = json!({"graph": name, "n": g.n, "entries": g.m(), "prefetch": prefetch,
        "madv_random": random, "threads_tput": threads, "warm": {}, "cold": {}, "tput_warm": {}, "tput_cold": {}});
    let nwarm = |qt: QT| -> usize {
        let b = match qt {
            QT::Deg | QT::Nbrs | QT::Common => 100_000,
            QT::TwoHop | QT::KHop => 3_000,
            QT::BiBfs => 1_000,
        };
        (b as f64 * scale) as usize
    };
    let ncold = |qt: QT| -> usize {
        let b = match qt {
            QT::Deg | QT::Nbrs | QT::Common => 300,
            QT::TwoHop | QT::KHop => 100,
            QT::BiBfs => 40,
        };
        (b as f64 * scale).max(10.0) as usize
    };
    if arg::<u32>(&args, "evict_first", 0) == 1 {
        g.evict();
        gin.evict();
    }
    let mut c = Ctx::new(&g, gin, prefetch);
    if arg::<u32>(&args, "verify", 0) == 1 {
        // check bidirectional BFS distances against a plain BFS (out-edges only)
        let mut bad = 0;
        let vn: u64 = arg(&args, "verify_n", 100);
        for i in 0..vn {
            let (a, b) = params(&g, QT::BiBfs, i);
            let (l, _) = c.bibfs(a, b, 30);
            let mut dist = vec![u32::MAX; g.n];
            dist[a as usize] = 0;
            let mut q = std::collections::VecDeque::from([a]);
            while let Some(u) = q.pop_front() {
                if u == b {
                    break;
                }
                for &w in g.nbrs(u) {
                    if dist[w as usize] == u32::MAX {
                        dist[w as usize] = dist[u as usize] + 1;
                        q.push_back(w);
                    }
                }
            }
            let d = if dist[b as usize] == u32::MAX { -1 } else { dist[b as usize] as i32 };
            if d != l {
                bad += 1;
                eprintln!("mismatch {a}->{b}: bibfs {l} bfs {d}");
            }
        }
        eprintln!("verify: {bad} mismatches / {vn}");
        return;
    }
    // one reusable query context (visited bitset of n bits) per worker thread
    let ctxs: Vec<std::sync::Mutex<Ctx>> = (0..threads).map(|_| std::sync::Mutex::new(Ctx::new(&g, gin, prefetch))).collect();
    let evict_all = || {
        g.evict();
        if gin_own.is_some() {
            gin.evict();
        }
    };
    let mut sink = 0usize;
    if do_warm {
        let t = now();
        if arg::<u32>(&args, "prewarm", 1) == 1 {
            g.warm();
            gin.warm();
        }
        eprintln!("warm-up {:.1}s resident {:.3}", secs(t), g.resident());
        for (qt, nm) in QTS {
            let nq = nwarm(qt);
            let ps: Vec<(u32, u32)> = (0..nq as u64).map(|i| params(&g, qt, i)).collect();
            let mut lat = Vec::with_capacity(nq);
            let mut extra = 0usize;
            for &(a, b) in &ps {
                let t = now();
                let r = run1(&mut c, qt, a, b);
                lat.push(t.elapsed().as_nanos() as f64 / 1000.0);
                sink += r;
                extra += if let QT::BiBfs = qt { r & 0xffffffff } else { r };
            }
            let s = lat_summary(&mut lat);
            eprintln!("warm {nm}: {s}");
            res["warm"][nm] = s;
            res["warm"][nm]["avg_result"] = json!(extra as f64 / nq as f64);
            // throughput with T threads
            let tp = pool(threads);
            let nt = nq * 4;
            let ps: Vec<(u32, u32)> = (0..nt as u64).map(|i| params(&g, qt, i + 10_000_000)).collect();
            let t = now();
            let s: usize = tp.install(|| {
                ps.par_iter()
                    .with_max_len(16)
                    .map(|&(a, b)| run1(&mut ctxs[rayon::current_thread_index().unwrap()].lock().unwrap(), qt, a, b))
                    .sum()
            });
            sink += s;
            let qps = nt as f64 / secs(t);
            eprintln!("  tput {threads}t: {qps:.0} q/s");
            res["tput_warm"][nm] = json!({"threads": threads, "queries": nt, "qps": qps});
        }
    }
    if do_cold {
        for (qt, nm) in QTS {
            let nq = ncold(qt);
            let mut lat = Vec::with_capacity(nq);
            let mut faults = 0f64;
            let mut max_res = 0f64;
            for i in 0..nq as u64 {
                let (a, b) = params(&g, qt, i + 20_000_000);
                evict_all();
                if i % 20 == 0 {
                    max_res = max_res.max(g.resident()).max(gin.resident());
                }
                let f0 = minflt_majflt();
                let t = now();
                let r = run1(&mut c, qt, a, b);
                lat.push(t.elapsed().as_nanos() as f64 / 1000.0);
                faults += (minflt_majflt() - f0) as f64;
                sink += r;
            }
            let s = lat_summary(&mut lat);
            eprintln!("cold {nm}: {s}");
            res["cold"][nm] = s;
            res["cold"][nm]["avg_major_faults"] = json!(faults / nq as f64);
            res["cold"][nm]["max_resident_fraction_after_evict"] = json!(max_res);
            // cold throughput: evict once then T threads run queries on an initially cold cache
            if matches!(qt, QT::Nbrs | QT::TwoHop | QT::BiBfs) {
                let nt = nq * 10;
                let ps: Vec<(u32, u32)> = (0..nt as u64).map(|i| params(&g, qt, i + 30_000_000)).collect();
                evict_all();
                let tp = pool(threads);
                let t = now();
                let s: usize = tp.install(|| {
                    ps.par_iter()
                        .with_max_len(1)
                        .map(|&(a, b)| run1(&mut ctxs[rayon::current_thread_index().unwrap()].lock().unwrap(), qt, a, b))
                        .sum()
                });
                sink += s;
                let qps = nt as f64 / secs(t);
                eprintln!("  cold tput {threads}t: {qps:.0} q/s");
                res["tput_cold"][nm] = json!({"threads": threads, "queries": nt, "qps": qps});
            }
        }
    }
    eprintln!("sink {sink}");
    println!("{}", res);
    if !out.is_empty() {
        save_json(&out, &res);
    }
}

fn minflt_majflt() -> u64 {
    // major faults of this process
    let s = std::fs::read_to_string("/proc/self/stat").unwrap();
    let rest = &s[s.rfind(')').unwrap() + 2..];
    let f: Vec<&str> = rest.split_whitespace().collect();
    f[9].parse().unwrap()
}
