//! m1-05 (#19, hierarchy shaping): two streaming passes over a level-0 store that the shaping
//! driver (`lab/m1-05-hierarchy-shaping/shape.py`) needs. Both read the store mapped, block by
//! block, and keep only O(n) arrays in memory.
//!
//!   m102 cutgraph --store S --out DIR [--labels L.u32] [--selfloop] [--drop F --seed X] --threads T
//!       The subgraph of the edges whose two ends carry the same label (u32::MAX = inactive node:
//!       all its edges are dropped), node ids unchanged. `--selfloop` adds to every active node a
//!       self-loop of weight (degree - kept degree), so that every node keeps its degree in the
//!       full graph (a Leiden run on the cut graph then optimizes the full graph's modularity
//!       inside each region). `--out-stats P` writes P.internal.u64: per label, the undirected
//!       edges kept (the region's edge count in the cut graph). `--drop F --seed X` first removes each undirected edge with
//!       probability F, decided by a hash of (min, max, seed): the perturbation of the stability
//!       test. The output is a weighted store when self-loops are written, else unweighted.
//!   m102 regionstats --store S --memb A.u32[,B.u32...] --out PA[,PB...] --threads T
//!       For each membership: PA.internal.u64 (undirected edges with both ends in the region) and
//!       PA.volume.u64 (sum of degrees), one entry per region id, and a JSON line with the
//!       modularity (gamma 1) and the edge count.
use crate::adj::*;
use crate::store::{encode_record, Comp, StoreWriter};
use rayon::prelude::*;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::Instant;

const NONE: u32 = u32::MAX;

fn get(a: &[String], k: &str) -> Option<String> {
    a.iter().position(|x| x == k).and_then(|i| a.get(i + 1).cloned())
}

fn read_u32(p: &str) -> Vec<u32> {
    let b = std::fs::read(p).unwrap_or_else(|e| panic!("{p}: {e}"));
    assert_eq!(b.len() % 4, 0);
    b.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

fn write_u64(p: &str, v: &[u64]) {
    let b: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
    std::fs::write(p, b).unwrap_or_else(|e| panic!("{p}: {e}"));
}

#[inline]
fn mix(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Is the undirected edge {u, v} removed by the perturbation?
#[inline]
fn dropped(u: u32, v: u32, thr: u64, seed: u64) -> bool {
    if thr == 0 {
        return false;
    }
    let (a, b) = if u < v { (u, v) } else { (v, u) };
    mix(seed ^ mix(((a as u64) << 32) | b as u64)) < thr
}

pub fn cutgraph(a: &[String]) {
    let t = Instant::now();
    let c = Comp::open(Path::new(&get(a, "--store").expect("--store")), Mem::MmapLazy).expect("open store");
    assert!(!c.weighted, "cutgraph reads an unweighted level-0 store");
    let out = PathBuf::from(get(a, "--out").expect("--out"));
    let labels: Option<Vec<u32>> = get(a, "--labels").map(|p| read_u32(&p));
    if let Some(l) = &labels {
        assert_eq!(l.len(), c.n, "--labels: one u32 per node");
    }
    let selfloop = a.iter().any(|x| x == "--selfloop");
    let frac: f64 = get(a, "--drop").map(|s| s.parse().unwrap()).unwrap_or(0.0);
    let seed: u64 = get(a, "--seed").map(|s| s.parse().unwrap()).unwrap_or(1);
    let thr = (frac * 18446744073709551615.0) as u64;
    let tmp = out.with_extension(format!("tmp{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let mut w = StoreWriter::create(&tmp.join("adj.bin"), c.n, selfloop).unwrap();
    let win = rayon::current_num_threads() * 8;
    let mut i = 0;
    let kept_edges = AtomicU64::new(0);
    let nl = labels.as_ref().map(|l| l.iter().filter(|&&x| x != NONE).max().map(|&m| m as usize + 1).unwrap_or(0)).unwrap_or(1);
    let per_label: Vec<AtomicU64> = (0..nl).map(|_| AtomicU64::new(0)).collect();
    let loop_weight = AtomicU64::new(0);
    while i < c.chunks.len() {
        let e = (i + win).min(c.chunks.len());
        let parts: Vec<(Vec<u8>, Vec<u32>, Vec<u32>)> = c.chunks[i..e]
            .par_iter()
            .map(|&(lo, hi)| {
                let mut cur = c.cur();
                let (mut out, mut rel, mut deg) = (Vec::new(), Vec::with_capacity(hi - lo), Vec::with_capacity(hi - lo));
                let (mut tg, mut wt, mut tmp) = (Vec::new(), Vec::new(), Vec::new());
                let (mut kept, mut lw) = (0u64, 0u64);
                for v in lo..hi {
                    rel.push(out.len() as u32);
                    tg.clear();
                    wt.clear();
                    let lv = labels.as_ref().map(|l| l[v]).unwrap_or(0);
                    let mut full = 0u64;
                    for &x in c.nbrs(v, &mut cur) {
                        if dropped(v as u32, x, thr, seed) {
                            continue;
                        }
                        full += 1;
                        if lv != NONE && labels.as_ref().map(|l| l[x as usize] == lv).unwrap_or(true) {
                            tg.push(x);
                        }
                    }
                    kept += tg.len() as u64;
                    if lv != NONE && !tg.is_empty() {
                        per_label[lv as usize].fetch_add(tg.len() as u64, Relaxed);
                    }
                    let sl = full - tg.len() as u64;
                    if selfloop && lv != NONE && sl > 0 {
                        let p = tg.partition_point(|&x| (x as usize) < v);
                        tg.insert(p, v as u32);
                        wt.resize(tg.len(), 1);
                        wt[p] = u32::try_from(sl).expect("self-loop weight above u32");
                        lw += sl;
                    } else if selfloop {
                        wt.resize(tg.len(), 1);
                    }
                    deg.push(tg.len() as u32);
                    encode_record(v, &tg, if selfloop { Some(&wt) } else { None }, &mut out, &mut tmp);
                }
                kept_edges.fetch_add(kept, Relaxed);
                loop_weight.fetch_add(lw, Relaxed);
                (out, rel, deg)
            })
            .collect();
        let offs: Vec<u64> = c.chunks[i..e].iter().zip(&parts).map(|(&(lo, hi), p)| w.place(lo, hi, p.0.len(), &p.1, &p.2)).collect();
        parts.par_iter().zip(&offs).for_each(|(p, &o)| w.write_at(o, &p.0));
        i = e;
    }
    let s = w.finish_dir(&tmp, false).expect("finish");
    if let Some(p) = get(a, "--out-stats") {
        let v: Vec<u64> = per_label.into_iter().map(|x| x.into_inner() / 2).collect();
        write_u64(&format!("{p}.internal.u64"), &v);
    }
    let _ = std::fs::remove_dir_all(&out);
    std::fs::rename(&tmp, &out).expect("rename");
    println!(
        "{}",
        json!({"cmd": "cutgraph", "store": out.display().to_string(), "n": s.n, "entries": s.entries, "kept_directed_entries": kept_edges.load(Relaxed),
               "selfloop_weight": loop_weight.load(Relaxed), "drop": frac, "seed": seed, "weighted": selfloop, "data_bytes": s.data_len,
               "seconds": t.elapsed().as_secs_f64(), "peak_rss_bytes": peak_rss()})
    );
}

pub fn regionstats(a: &[String]) {
    let t = Instant::now();
    let c = Comp::open(Path::new(&get(a, "--store").expect("--store")), Mem::MmapLazy).expect("open store");
    assert!(!c.weighted);
    let membs: Vec<String> = get(a, "--memb").expect("--memb").split(',').map(String::from).collect();
    let outs: Vec<String> = get(a, "--out").expect("--out").split(',').map(String::from).collect();
    assert_eq!(membs.len(), outs.len());
    let m2 = c.entries as f64;
    for (mp, op) in membs.iter().zip(&outs) {
        let tm = Instant::now();
        let memb = read_u32(mp);
        assert_eq!(memb.len(), c.n);
        let nr = memb.par_iter().copied().max().unwrap_or(0) as usize + 1;
        let internal: Vec<AtomicU64> = (0..nr).map(|_| AtomicU64::new(0)).collect();
        let volume: Vec<AtomicU64> = (0..nr).map(|_| AtomicU64::new(0)).collect();
        c.chunks.par_iter().for_each(|&(lo, hi)| {
            let mut cur = c.cur();
            // runs of one label (members are often contiguous) are summed before the atomic add
            let (mut lab, mut ai, mut av) = (NONE, 0u64, 0u64);
            for v in lo..hi {
                let l = memb[v];
                let ls = c.nbrs(v, &mut cur);
                let inn = ls.iter().filter(|&&x| memb[x as usize] == l).count() as u64;
                if l != lab {
                    if lab != NONE {
                        internal[lab as usize].fetch_add(ai, Relaxed);
                        volume[lab as usize].fetch_add(av, Relaxed);
                    }
                    (lab, ai, av) = (l, 0, 0);
                }
                ai += inn;
                av += ls.len() as u64;
            }
            if lab != NONE {
                internal[lab as usize].fetch_add(ai, Relaxed);
                volume[lab as usize].fetch_add(av, Relaxed);
            }
        });
        // every internal edge was counted from both ends
        let internal: Vec<u64> = internal.into_iter().map(|x| x.into_inner() / 2).collect();
        let volume: Vec<u64> = volume.into_iter().map(|x| x.into_inner()).collect();
        let q: f64 = internal.iter().zip(&volume).map(|(&i, &v)| 2.0 * i as f64 / m2 - (v as f64 / m2).powi(2)).sum();
        write_u64(&format!("{op}.internal.u64"), &internal);
        write_u64(&format!("{op}.volume.u64"), &volume);
        println!(
            "{}",
            json!({"cmd": "regionstats", "memb": mp, "out": op, "regions": nr, "edges": c.entries / 2, "modularity": q, "seconds": tm.elapsed().as_secs_f64()})
        );
    }
    eprintln!("regionstats: {:.2} s, peak rss {} MB", t.elapsed().as_secs_f64(), peak_rss() >> 20);
}
