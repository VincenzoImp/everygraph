//! Q3 + Q4: edge property columns, filtered views (bitset / roaring / predicate / materialized),
//! edge-id indirection vs duplication, temporal layouts.
//! usage: props --graph DIR --work DIR --threads T --out FILE
use f1q::*;
use rayon::prelude::*;
use roaring::RoaringBitmap;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicU64, Ordering};

const T0: u32 = 1_577_836_800; // 2020-01-01
const SPAN: u32 = 4 * 365 * 86400; // 4 years

#[inline]
fn edge_hash(u: u32, v: u32) -> u64 {
    let (a, b) = if u < v { (u, v) } else { (v, u) };
    splitmix(((a as u64) << 32 | b as u64) ^ 0x1234_5678_9abc)
}
#[inline]
fn gen_ts(h: u64) -> u32 {
    T0 + ((h & 0xffff_ffff) * SPAN as u64 >> 32) as u32
}
#[inline]
fn gen_w(h: u64) -> f32 {
    // exponential, mean 5 ("ETH")
    let u = ((h >> 32) as f64 + 0.5) / 4294967296.0;
    (-u.ln() * 5.0) as f32
}
#[inline]
fn gen_ty(h: u64) -> u8 {
    let x = splitmix(h) as u32;
    (x.leading_zeros().min(7)) as u8
}

struct SP<T>(*mut T);
unsafe impl<T> Sync for SP<T> {}
unsafe impl<T> Send for SP<T> {}
impl<T> SP<T> {
    #[inline]
    unsafe fn w(&self, i: usize, x: T) {
        unsafe { *self.0.add(i) = x }
    }
}

#[inline]
fn bit(m: &[u64], i: usize) -> bool {
    m[i >> 6] >> (i & 63) & 1 == 1
}

fn time_it<R>(reps: usize, mut f: impl FnMut() -> R) -> (f64, f64, R) {
    let mut ts = vec![];
    let mut r = None;
    for _ in 0..reps {
        let t = now();
        r = Some(f());
        ts.push(secs(t));
    }
    ts.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (ts[0], ts[ts.len() / 2], r.unwrap())
}

/// generic adjacency access: (offsets, targets)
struct G<'a> {
    off: &'a [u64],
    tgt: &'a [u32],
}
impl<'a> G<'a> {
    fn n(&self) -> usize {
        self.off.len() - 1
    }
}

/// parallel top-down BFS; ok(entry, src, dst)
fn bfs<F: Fn(usize, u32, u32) -> bool + Sync>(g: &G, src: u32, ok: F) -> usize {
    let n = g.n();
    let vis: Vec<AtomicU64> = (0..n.div_ceil(64)).map(|_| AtomicU64::new(0)).collect();
    vis[src as usize >> 6].fetch_or(1 << (src & 63), Ordering::Relaxed);
    let mut front = vec![src];
    let mut reached = 1;
    while !front.is_empty() {
        let next: Vec<u32> = front
            .par_iter()
            .with_max_len(64)
            .fold(Vec::new, |mut acc, &u| {
                let (a, b) = (g.off[u as usize] as usize, g.off[u as usize + 1] as usize);
                for i in a..b {
                    let v = g.tgt[i];
                    if !ok(i, u, v) {
                        continue;
                    }
                    let (w, m) = (v as usize >> 6, 1u64 << (v & 63));
                    if vis[w].load(Ordering::Relaxed) & m == 0 && vis[w].fetch_or(m, Ordering::Relaxed) & m == 0 {
                        acc.push(v);
                    }
                }
                acc
            })
            .reduce(Vec::new, |mut a, mut b| {
                if a.len() < b.len() {
                    std::mem::swap(&mut a, &mut b);
                }
                a.extend_from_slice(&b);
                a
            });
        reached += next.len();
        front = next;
    }
    reached
}

/// filtered degree pass
fn fdeg<F: Fn(usize, u32, u32) -> bool + Sync>(g: &G, ok: &F) -> Vec<u32> {
    (0..g.n() as u32)
        .into_par_iter()
        .with_min_len(256)
        .map(|v| {
            let (a, b) = (g.off[v as usize] as usize, g.off[v as usize + 1] as usize);
            (a..b).filter(|&i| ok(i, v, g.tgt[i])).count() as u32
        })
        .collect()
}

/// one pull PageRank iteration on the view; deg = filtered degree
fn pr_iter<F: Fn(usize, u32, u32) -> bool + Sync>(g: &G, deg: &[u32], pr: &[f32], ok: F) -> f64 {
    let n = g.n();
    let contrib: Vec<f32> =
        pr.par_iter().zip(deg.par_iter()).map(|(&p, &d)| if d > 0 { p / d as f32 } else { 0.0 }).collect();
    let base = 0.15 / n as f32;
    let new: Vec<f32> = (0..n as u32)
        .into_par_iter()
        .with_min_len(256)
        .map(|v| {
            let (a, b) = (g.off[v as usize] as usize, g.off[v as usize + 1] as usize);
            let mut s = 0f32;
            for i in a..b {
                let u = g.tgt[i];
                if ok(i, v, u) {
                    s += contrib[u as usize];
                }
            }
            base + 0.85 * s
        })
        .collect();
    new.par_iter().map(|&x| x as f64).sum()
}

/// PR iteration where the view is given by a roaring bitmap over entries (iterate set bits per node)
fn pr_iter_roaring(g: &G, deg: &[u32], pr: &[f32], rb: &RoaringBitmap) -> f64 {
    let n = g.n();
    let contrib: Vec<f32> =
        pr.par_iter().zip(deg.par_iter()).map(|(&p, &d)| if d > 0 { p / d as f32 } else { 0.0 }).collect();
    let base = 0.15 / n as f32;
    let new: Vec<f32> = (0..n as u32)
        .into_par_iter()
        .with_min_len(256)
        .map(|v| {
            let (a, b) = (g.off[v as usize] as u32, g.off[v as usize + 1] as u32);
            let mut s = 0f32;
            if b > a {
                for i in rb.range(a..b) {
                    s += contrib[g.tgt[i as usize] as usize];
                }
            }
            base + 0.85 * s
        })
        .collect();
    new.par_iter().map(|&x| x as f64).sum()
}

fn bfs_roaring(g: &G, src: u32, rb: &RoaringBitmap) -> usize {
    let n = g.n();
    let vis: Vec<AtomicU64> = (0..n.div_ceil(64)).map(|_| AtomicU64::new(0)).collect();
    vis[src as usize >> 6].fetch_or(1 << (src & 63), Ordering::Relaxed);
    let mut front = vec![src];
    let mut reached = 1;
    while !front.is_empty() {
        let next: Vec<u32> = front
            .par_iter()
            .with_max_len(64)
            .fold(Vec::new, |mut acc, &u| {
                let (a, b) = (g.off[u as usize] as u32, g.off[u as usize + 1] as u32);
                if b > a {
                    for i in rb.range(a..b) {
                        let v = g.tgt[i as usize];
                        let (w, m) = (v as usize >> 6, 1u64 << (v & 63));
                        if vis[w].load(Ordering::Relaxed) & m == 0 && vis[w].fetch_or(m, Ordering::Relaxed) & m == 0 {
                            acc.push(v);
                        }
                    }
                }
                acc
            })
            .reduce(Vec::new, |mut a, b| {
                a.extend_from_slice(&b);
                a
            });
        reached += next.len();
        front = next;
    }
    reached
}

fn bitset_from<F: Fn(usize) -> bool + Sync>(len: usize, f: F) -> Vec<u64> {
    (0..len.div_ceil(64))
        .into_par_iter()
        .with_min_len(1024)
        .map(|w| {
            let mut x = 0u64;
            let base = w * 64;
            let end = (base + 64).min(len);
            for i in base..end {
                x |= (f(i) as u64) << (i - base);
            }
            x
        })
        .collect()
}

fn roaring_from_bitset(bs: &[u64]) -> RoaringBitmap {
    // parallel over 2^20-bit chunks, then union (disjoint containers)
    let words_per = (1 << 20) / 64;
    let parts: Vec<RoaringBitmap> = bs
        .par_chunks(words_per)
        .enumerate()
        .map(|(c, ws)| {
            let bytes = unsafe { std::slice::from_raw_parts(ws.as_ptr() as *const u8, ws.len() * 8) };
            RoaringBitmap::from_lsb0_bytes((c * (1 << 20)) as u32, bytes)
        })
        .collect();
    let mut r = RoaringBitmap::new();
    for p in parts {
        r |= p;
    }
    r
}

/// materialize filtered CSR copy
fn materialize<F: Fn(usize, u32, u32) -> bool + Sync>(g: &G, ok: &F) -> (Vec<u64>, Vec<u32>) {
    let d = fdeg(g, ok);
    let n = g.n();
    let mut off = vec![0u64; n + 1];
    for i in 0..n {
        off[i + 1] = off[i] + d[i] as u64;
    }
    let mut tg = vec![0u32; off[n] as usize];
    let sp = SP(tg.as_mut_ptr());
    (0..n as u32).into_par_iter().with_min_len(256).for_each(|v| {
        let _ = &sp;
        let (a, b) = (g.off[v as usize] as usize, g.off[v as usize + 1] as usize);
        let mut p = off[v as usize] as usize;
        for i in a..b {
            if ok(i, v, g.tgt[i]) {
                unsafe { sp.w(p, g.tgt[i]) };
                p += 1;
            }
        }
    });
    (off, tg)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = arg_s(&args, "graph", "/home/tux/graphlab/data/csr/orkut");
    let work = std::path::PathBuf::from(arg_s(&args, "work", "work/props_orkut"));
    let threads: usize = arg(&args, "threads", 4);
    let reps: usize = arg(&args, "reps", 3);
    let out = arg_s(&args, "out", "");
    std::fs::create_dir_all(&work).unwrap();
    rayon::ThreadPoolBuilder::new().num_threads(threads).build_global().unwrap();
    let csr = Csr::open(&dir, false);
    csr.warm();
    let g = G { off: csr.offs(), tgt: csr.tgts() };
    let n = g.n();
    let e = g.tgt.len();
    let mut res = json!({"graph": dir, "threads": threads, "n": n, "entries": e});
    eprintln!("n={n} entries={e} threads={threads}");

    // ---------- edge ids: canonical undirected edge (u<v) numbered in CSR order of the u-side ----------
    let t = now();
    let upos: Vec<u32> = (0..n as u32)
        .into_par_iter()
        .map(|v| {
            let (a, b) = (g.off[v as usize] as usize, g.off[v as usize + 1] as usize);
            g.tgt[a..b].partition_point(|&w| w < v) as u32
        })
        .collect();
    let mut ustart = vec![0u64; n + 1];
    for v in 0..n {
        let d = g.off[v + 1] - g.off[v];
        ustart[v + 1] = ustart[v] + d - upos[v] as u64;
    }
    let m = ustart[n] as usize;
    let mut eid = vec![0u32; e];
    let sp = SP(eid.as_mut_ptr());
    (0..n as u32).into_par_iter().with_min_len(64).for_each(|v| {
        let _ = &sp;
        let (a, b) = (g.off[v as usize] as usize, g.off[v as usize + 1] as usize);
        let p = upos[v as usize] as usize;
        for i in a..b {
            let w = g.tgt[i];
            let id = if w > v {
                ustart[v as usize] + (i - a - p) as u64
            } else {
                // find v in w's list (it is in w's upper part)
                let (wa, wb) = (g.off[w as usize] as usize, g.off[w as usize + 1] as usize);
                let j = g.tgt[wa..wb].binary_search(&v).unwrap();
                ustart[w as usize] + (j - upos[w as usize] as usize) as u64
            };
            unsafe { sp.w(i, id as u32) };
        }
    });
    let eid_s = secs(t);
    eprintln!("eid build {eid_s:.2}s m={m}");
    res["edge_id_build_s"] = json!(eid_s);
    res["m_undirected"] = json!(m);

    // ---------- synthetic columns per undirected edge, then duplicated per entry ----------
    let t = now();
    let mut ts_e = vec![0u32; m];
    let mut w_e = vec![0f32; m];
    let mut ty_e = vec![0u8; m];
    {
        let (a1, a2, a3) = (SP(ts_e.as_mut_ptr()), SP(w_e.as_mut_ptr()), SP(ty_e.as_mut_ptr()));
        (0..n as u32).into_par_iter().with_min_len(64).for_each(|v| {
            let _ = (&a1, &a2, &a3);
            let (a, b) = (g.off[v as usize] as usize, g.off[v as usize + 1] as usize);
            let p = upos[v as usize] as usize;
            for i in a + p..b {
                let id = ustart[v as usize] as usize + (i - a - p);
                let h = edge_hash(v, g.tgt[i]);
                unsafe {
                    a1.w(id, gen_ts(h));
                    a2.w(id, gen_w(h));
                    a3.w(id, gen_ty(h));
                }
            }
        });
    }
    let gen_s = secs(t);
    let t = now();
    let ts_d: Vec<u32> = eid.par_iter().map(|&x| ts_e[x as usize]).collect();
    let w_d: Vec<f32> = eid.par_iter().map(|&x| w_e[x as usize]).collect();
    let ty_d: Vec<u8> = eid.par_iter().map(|&x| ty_e[x as usize]).collect();
    let dup_s = secs(t);
    let t = now();
    for (nm, b) in [
        ("eid.u32", unsafe { std::slice::from_raw_parts(eid.as_ptr() as *const u8, e * 4) }),
        ("ts_e.u32", unsafe { std::slice::from_raw_parts(ts_e.as_ptr() as *const u8, m * 4) }),
        ("w_e.f32", unsafe { std::slice::from_raw_parts(w_e.as_ptr() as *const u8, m * 4) }),
        ("ty_e.u8", &ty_e[..]),
        ("ts_d.u32", unsafe { std::slice::from_raw_parts(ts_d.as_ptr() as *const u8, e * 4) }),
        ("w_d.f32", unsafe { std::slice::from_raw_parts(w_d.as_ptr() as *const u8, e * 4) }),
        ("ty_d.u8", &ty_d[..]),
    ] {
        write_slice(work.join(nm), b);
    }
    let write_s = secs(t);
    drop((eid, ts_e, w_e, ty_e, ts_d, w_d, ty_d));
    let f_eid = MFile::open(work.join("eid.u32"), false);
    let f_tse = MFile::open(work.join("ts_e.u32"), false);
    let f_we = MFile::open(work.join("w_e.f32"), false);
    let f_tsd = MFile::open(work.join("ts_d.u32"), false);
    let f_wd = MFile::open(work.join("w_d.f32"), false);
    let f_tyd = MFile::open(work.join("ty_d.u8"), false);
    for f in [&f_eid, &f_tse, &f_we, &f_tsd, &f_wd, &f_tyd] {
        f.warm();
    }
    let eid: &[u32] = f_eid.as_slice();
    let ts_e: &[u32] = f_tse.as_slice();
    let w_e: &[f32] = f_we.as_slice();
    let ts_d: &[u32] = f_tsd.as_slice();
    let w_d: &[f32] = f_wd.as_slice();
    let ty_d: &[u8] = f_tyd.as_slice();
    // symmetric check
    for i in (0..e).step_by(e / 997) {
        assert_eq!(ts_d[i], ts_e[eid[i] as usize]);
    }
    res["columns"] = json!({
        "gen_per_edge_s": gen_s, "duplicate_gather_s": dup_s, "write_s": write_s,
        "layout": "raw little-endian columnar files (same bytes as an Arrow primitive values buffer), mmapped",
        "bytes_duplicated_per_entry": {"ts_u32": e*4, "w_f32": e*4, "ty_u8": e, "total": e*9},
        "bytes_indirect": {"eid_u32": e*4, "ts_u32": m*4, "w_f32": m*4, "ty_u8": m, "total": e*4 + m*9},
        "breakeven_bytes_per_edge_of_props": "duplication costs 2*m*P, indirection 2*m*4 + m*P: indirection wins when P > 4 bytes/edge"
    });
    eprintln!("columns gen {gen_s:.2}s dup {dup_s:.2}s write {write_s:.2}s");

    let src = (0..n as u32).max_by_key(|&v| g.off[v as usize + 1] - g.off[v as usize]).unwrap();
    let pr0: Vec<f32> = vec![1.0 / n as f32; n];
    let deg_full: Vec<u32> = (0..n).map(|v| (g.off[v + 1] - g.off[v]) as u32).collect();

    // ---------- baselines ----------
    let (b_min, b_med, reach) = time_it(reps, || bfs(&g, src, |_, _, _| true));
    let (p_min, p_med, _) = time_it(reps, || pr_iter(&g, &deg_full, &pr0, |_, _, _| true));
    res["unfiltered"] = json!({"bfs_s_min": b_min, "bfs_s_med": b_med, "bfs_reached": reach, "pr_iter_s_min": p_min, "pr_iter_s_med": p_med});
    eprintln!("unfiltered bfs {b_min:.3}s pr {p_min:.3}s");

    // ---------- indirection cost for property access: weighted PR iteration ----------
    {
        let ws: Vec<f32> = (0..n as u32)
            .into_par_iter()
            .map(|v| (g.off[v as usize] as usize..g.off[v as usize + 1] as usize).map(|i| w_d[i]).sum())
            .collect();
        let run = |ind: bool| {
            let contrib: Vec<f32> =
                pr0.par_iter().zip(ws.par_iter()).map(|(&p, &d)| if d > 0.0 { p / d } else { 0.0 }).collect();
            let s: f64 = (0..n as u32)
                .into_par_iter()
                .with_min_len(256)
                .map(|v| {
                    let (a, b) = (g.off[v as usize] as usize, g.off[v as usize + 1] as usize);
                    let mut s = 0f32;
                    if ind {
                        for i in a..b {
                            s += contrib[g.tgt[i] as usize] * w_e[eid[i] as usize];
                        }
                    } else {
                        for i in a..b {
                            s += contrib[g.tgt[i] as usize] * w_d[i];
                        }
                    }
                    s as f64
                })
                .sum();
            s
        };
        let (d_min, _, s1) = time_it(reps, || run(false));
        let (i_min, _, s2) = time_it(reps, || run(true));
        assert!((s1 - s2).abs() < 1e-3 * s1.abs().max(1e-9));
        res["weighted_pr_iter"] = json!({"duplicated_s": d_min, "indirect_eid_s": i_min, "slowdown": i_min / d_min});
        eprintln!("weighted PR: dup {d_min:.3}s indirect {i_min:.3}s");
    }

    // ---------- filtered views ----------
    let only = arg_s(&args, "only", "");
    let mut views = vec![];
    let sels: &[f64] = if only == "temporal" { &[] } else { &[0.01f64, 0.10, 0.50, 0.90] };
    for &sel in sels {
        let hi = T0 + (sel * SPAN as f64) as u32;
        let wthr = (-(sel.ln()) * 5.0) as f32;
        let mut r = json!({"selectivity": sel});
        // mask builds
        let (mb_e, _, me) = time_it(reps, || bitset_from(e, |i| ts_d[i] < hi));
        let (mb_w, _, mw) = time_it(reps, || bitset_from(e, |i| w_d[i] > wthr));
        let (mb_edge, _, medge) = time_it(reps, || bitset_from(m, |j| ts_e[j] < hi));
        let (mb_r, _, rb) = time_it(reps, || roaring_from_bitset(&me));
        let (mb_ty, _, _mty) = time_it(reps, || bitset_from(e, |i| ty_d[i] == 0 && ts_d[i] < hi));
        let actual = me.par_iter().map(|w| w.count_ones() as u64).sum::<u64>() as f64 / e as f64;
        let actual_w = mw.par_iter().map(|w| w.count_ones() as u64).sum::<u64>() as f64 / e as f64;
        // node mask with same selectivity (nodes hashed)
        let nthr = (sel * u32::MAX as f64) as u64;
        let (mb_n, _, mn) = time_it(reps, || bitset_from(n, |v| v == src as usize || (splitmix(v as u64 ^ 77) as u32 as u64) < nthr));
        r["actual_sel_time"] = json!(actual);
        r["actual_sel_weight"] = json!(actual_w);
        r["mask_build_s"] = json!({"entry_bitset_time": mb_e, "entry_bitset_weight": mb_w, "edgeid_bitset_time": mb_edge,
            "roaring_from_bitset": mb_r, "entry_bitset_type_and_time": mb_ty, "node_bitset": mb_n});
        r["mask_bytes"] = json!({"entry_bitset": me.len()*8, "edgeid_bitset": medge.len()*8,
            "roaring_entries": rb.serialized_size(), "node_bitset": mn.len()*8});

        // views: closures ok(i, src, dst)
        let ok_b = |i: usize, _: u32, _: u32| bit(&me, i);
        let ok_e = |i: usize, _: u32, _: u32| bit(&medge, eid[i] as usize);
        let ok_p = |i: usize, _: u32, _: u32| ts_d[i] < hi;
        let ok_pi = |i: usize, _: u32, _: u32| ts_e[eid[i] as usize] < hi;
        let ok_n = |_: usize, s: u32, d: u32| bit(&mn, s as usize) && bit(&mn, d as usize);
        let mut kv = json!({});
        macro_rules! run_view {
            ($name:expr, $ok:expr) => {{
                let (dg, _, deg) = time_it(1, || fdeg(&g, &$ok));
                let (bm, _, reach) = time_it(reps, || bfs(&g, src, $ok));
                let (pm, _, _) = time_it(reps, || pr_iter(&g, &deg, &pr0, $ok));
                kv[$name] = json!({"degree_pass_s": dg, "bfs_s": bm, "bfs_reached": reach, "pr_iter_s": pm,
                    "bfs_overhead_vs_unfiltered": bm / b_min, "pr_overhead_vs_unfiltered": pm / p_min});
                eprintln!("sel {sel} {}: deg {dg:.3} bfs {bm:.3} ({reach}) pr {pm:.3}", $name);
                deg
            }};
        }
        let deg_b = run_view!("entry_bitset", ok_b);
        run_view!("edgeid_bitset_via_eid", ok_e);
        run_view!("predicate_dup_column", ok_p);
        run_view!("predicate_via_eid", ok_pi);
        run_view!("node_bitset", ok_n);
        {
            let (bm, _, reach) = time_it(reps, || bfs_roaring(&g, src, &rb));
            let (pm, _, _) = time_it(reps, || pr_iter_roaring(&g, &deg_b, &pr0, &rb));
            kv["roaring_entries"] = json!({"bfs_s": bm, "bfs_reached": reach, "pr_iter_s": pm,
                "bfs_overhead_vs_unfiltered": bm / b_min, "pr_overhead_vs_unfiltered": pm / p_min});
            eprintln!("sel {sel} roaring: bfs {bm:.3} ({reach}) pr {pm:.3}");
        }
        {
            let (ms, _, (moff, mtg)) = time_it(1, || materialize(&g, &ok_b));
            let mg = G { off: &moff, tgt: &mtg };
            let mdeg: Vec<u32> = (0..n).map(|v| (moff[v + 1] - moff[v]) as u32).collect();
            let (bm, _, reach) = time_it(reps, || bfs(&mg, src, |_, _, _| true));
            let (pm, _, _) = time_it(reps, || pr_iter(&mg, &mdeg, &pr0, |_, _, _| true));
            kv["materialized_csr"] = json!({"build_s": ms, "bytes": moff.len()*8 + mtg.len()*4, "bfs_s": bm, "bfs_reached": reach,
                "pr_iter_s": pm, "bfs_overhead_vs_unfiltered": bm / b_min, "pr_overhead_vs_unfiltered": pm / p_min});
            eprintln!("sel {sel} materialized: build {ms:.3} bfs {bm:.3} pr {pm:.3}");
        }
        r["views"] = kv;
        views.push(r);
    }
    res["filtered"] = Value::Array(views);

    // ---------- Q4 temporal layouts ----------
    {
        // layout T: each adjacency list sorted by timestamp (targets + ts carried along)
        let t = now();
        let mut tgt_t = vec![0u32; e];
        let mut ts_t = vec![0u32; e];
        {
            let (s1, s2) = (SP(tgt_t.as_mut_ptr()), SP(ts_t.as_mut_ptr()));
            (0..n as u32).into_par_iter().with_min_len(64).for_each(|v| {
                let _ = (&s1, &s2);
                let (a, b) = (g.off[v as usize] as usize, g.off[v as usize + 1] as usize);
                let mut tmp: Vec<(u32, u32)> = (a..b).map(|i| (ts_d[i], g.tgt[i])).collect();
                tmp.sort_unstable();
                for (k, (ts, w)) in tmp.into_iter().enumerate() {
                    unsafe {
                        s1.w(a + k, w);
                        s2.w(a + k, ts);
                    }
                }
            });
        }
        let build_t = secs(t);
        let gt = G { off: g.off, tgt: &tgt_t };
        let mut tr = json!({"build_time_sorted_layout_s": build_t});
        let mut wins = vec![];
        for &sel in &[0.01f64, 0.10, 0.50] {
            let lo = T0 + (0.3 * SPAN as f64) as u32;
            let hi = lo + (sel * SPAN as f64) as u32;
            // window degree for all nodes
            let (wd_n, _, s_n) = time_it(reps, || {
                fdeg(&g, &|i: usize, _: u32, _: u32| ts_d[i] >= lo && ts_d[i] < hi).iter().map(|&x| x as u64).sum::<u64>()
            });
            let win = |v: usize| {
                let (a, b) = (g.off[v] as usize, g.off[v + 1] as usize);
                let s = &ts_t[a..b];
                (a + s.partition_point(|&x| x < lo), a + s.partition_point(|&x| x < hi))
            };
            let (wd_t, _, s_t) = time_it(reps, || {
                (0..n).into_par_iter().map(|v| { let (x, y) = win(v); (y - x) as u64 }).sum::<u64>()
            });
            assert_eq!(s_n, s_t);
            // PR iteration on the window: N layout = predicate scan; T layout = contiguous sub-range
            let deg_w: Vec<u32> = (0..n).into_par_iter().map(|v| { let (x, y) = win(v); (y - x) as u32 }).collect();
            let (pr_n, _, a1) =
                time_it(reps, || pr_iter(&g, &deg_w, &pr0, |i: usize, _: u32, _: u32| ts_d[i] >= lo && ts_d[i] < hi));
            let (pr_t, _, a2) = time_it(reps, || {
                let contrib: Vec<f32> =
                    pr0.par_iter().zip(deg_w.par_iter()).map(|(&p, &d)| if d > 0 { p / d as f32 } else { 0.0 }).collect();
                let base = 0.15 / n as f32;
                let new: Vec<f32> = (0..n)
                    .into_par_iter()
                    .with_min_len(256)
                    .map(|v| {
                        let (x, y) = win(v);
                        let mut s = 0f32;
                        for &u in &gt.tgt[x..y] {
                            s += contrib[u as usize];
                        }
                        base + 0.85 * s
                    })
                    .collect();
                new.par_iter().map(|&x| x as f64).sum::<f64>()
            });
            assert!((a1 - a2).abs() < 1e-3);
            // roaring on the time-sorted layout: the window is one run per node
            let rb_t = roaring_from_bitset(&bitset_from(e, |i| ts_t[i] >= lo && ts_t[i] < hi));
            let rb_n = roaring_from_bitset(&bitset_from(e, |i| ts_d[i] >= lo && ts_d[i] < hi));
            let mut rb_t2 = rb_t.clone();
            rb_t2.optimize();
            let mut rb_n2 = rb_n.clone();
            rb_n2.optimize();
            wins.push(json!({"selectivity": sel, "window_degree_all_nodes_s": {"nbr_sorted_scan": wd_n, "time_sorted_bsearch": wd_t},
                "pr_iter_window_s": {"nbr_sorted_scan": pr_n, "time_sorted_subrange": pr_t},
                "roaring_window_mask_bytes": {"nbr_sorted": rb_n2.serialized_size(), "time_sorted": rb_t2.serialized_size()}}));
            eprintln!("temporal sel {sel}: wdeg N {wd_n:.3} T {wd_t:.4}; pr N {pr_n:.3} T {pr_t:.3}");
        }
        tr["windows"] = Value::Array(wins);
        // cost of losing neighbour order: has_edge(u,v) by binary search (N) vs linear scan (T)
        let mut rr = Rng(9);
        let pairs: Vec<(u32, u32)> = (0..200_000)
            .map(|_| {
                let u = rr.below(n as u64) as u32;
                let nb = &g.tgt[g.off[u as usize] as usize..g.off[u as usize + 1] as usize];
                let w = if nb.is_empty() { 0 } else { nb[rr.below(nb.len() as u64) as usize] };
                (u, w)
            })
            .collect();
        let t = now();
        let c1: usize = pairs.iter().filter(|&&(u, w)| g.tgt[g.off[u as usize] as usize..g.off[u as usize + 1] as usize].binary_search(&w).is_ok()).count();
        let he_n = secs(t) / pairs.len() as f64 * 1e9;
        let t = now();
        let c2: usize = pairs.iter().filter(|&&(u, w)| gt.tgt[g.off[u as usize] as usize..g.off[u as usize + 1] as usize].contains(&w)).count();
        let he_t = secs(t) / pairs.len() as f64 * 1e9;
        assert_eq!(c1, c2);
        tr["has_edge_ns"] = json!({"nbr_sorted_bsearch": he_n, "time_sorted_linear": he_t});
        eprintln!("has_edge N {he_n:.0}ns T {he_t:.0}ns");
        res["temporal"] = tr;
    }
    println!("{}", res);
    if !out.is_empty() {
        save_json(&out, &res);
    }
}
