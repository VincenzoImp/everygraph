//! m1-05 (#19, hierarchy shaping): two streaming passes over a level-0 store that the shaping
//! driver (`lab/m1-05-hierarchy-shaping/shape.py`) needs. Both read the store mapped, block by
//! block, and keep only O(n) arrays in memory.
//!
//!   m102 cutgraph --store S --out DIR [--labels L.u32] [--selfloop] [--drop F --seed X [--drop-keep]]
//!                 [--out-stats P] [--out-deg D.u32] --threads T
//!       The subgraph of the edges whose two ends carry the same label. With `--labels`, the nodes
//!       labelled u32::MAX are inactive and left out, and the active nodes are renumbered in
//!       increasing order of their ids: DIR/map.u32 holds the original id of every output node.
//!       So one round of the driver is a graph of its active regions only: no isolated vertex
//!       enters Leiden's stopping test (a pass stops when the communities exceed 80% of the
//!       vertices), and Leiden scans nothing outside the active regions. (This command still
//!       streams the whole input store every round; a production version filters in the kernel.)
//!       `--selfloop` gives every active node v a diagonal entry A[v][v] = degree - kept degree:
//!       m102 adds a diagonal entry once to v's strength, as with the self-loops of its quotient
//!       graphs, so v keeps its strength in the full graph. In the terms of the graph-semantics
//!       spec (a loop's pair weight counts twice in a degree) it is a loop of pair weight
//!       (degree - kept)/2, possibly a half-integer. It moves with v, so it changes no move gain.
//!       `--out-stats P` writes P.internal.u64: per label, the undirected edges kept.
//!       `--drop F --seed X` first removes each undirected edge with probability F, decided by a
//!       hash of (min, max, seed): the perturbation of the stability test. `--drop-keep` never
//!       removes, for any node, its edge of largest hash, so no node loses all its edges (the largest
//!       hash, not the smallest: the smallest would correlate with being dropped and protect most of
//!       the edges the drop selects; with the largest, about the same 1% is dropped).
//!       `--out-deg D` writes every node's degree after the drop (original ids).
//!       The output is a weighted store when self-loops are written, else unweighted.
//!   m102 regionstats --store S --memb A.u32[,B.u32...] --out PA[,PB...] --threads T
//!       For each membership: PA.internal.u64 (undirected edges with both ends in the region) and
//!       PA.volume.u64 (sum of degrees), one entry per region id, and a JSON line with the
//!       modularity (gamma 1) and the edge count. The input store has no self-loops (lab stores).
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

fn write_u32(p: &str, v: &[u32]) {
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

/// The perturbation: which undirected edges are removed.
struct EdgeDrop {
    thr: u64,
    seed: u64,
    /// --drop-keep: per node, the largest hash among its edges (that edge is never removed)
    keeph: Vec<u64>,
}
impl EdgeDrop {
    #[inline]
    fn hash(&self, u: u32, v: u32) -> u64 {
        let (a, b) = if u < v { (u, v) } else { (v, u) };
        mix(self.seed ^ mix(((a as u64) << 32) | b as u64))
    }
    /// Is the undirected edge {u, v} removed? Symmetric in u and v.
    #[inline]
    fn dropped(&self, u: u32, v: u32) -> bool {
        if self.thr == 0 {
            return false;
        }
        let h = self.hash(u, v);
        h < self.thr && (self.keeph.is_empty() || (h != self.keeph[u as usize] && h != self.keeph[v as usize]))
    }
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
    let mut dr = EdgeDrop { thr: (frac * 18446744073709551615.0) as u64, seed, keeph: Vec::new() };
    if frac > 0.0 && a.iter().any(|x| x == "--drop-keep") {
        let keeph: Vec<AtomicU64> = (0..c.n).map(|_| AtomicU64::new(u64::MAX)).collect();
        c.chunks.par_iter().for_each(|&(lo, hi)| {
            let mut cur = c.cur();
            for v in lo..hi {
                let m = c.nbrs(v, &mut cur).iter().map(|&x| dr.hash(v as u32, x)).max().unwrap_or(u64::MAX);
                keeph[v].store(m, Relaxed);
            }
        });
        dr.keeph = keeph.into_iter().map(|x| x.into_inner()).collect();
    }
    // renumbering: the new id of an active node is the number of active nodes before it
    let newid: Option<Vec<u32>> = labels.as_ref().map(|l| {
        let mut nid = vec![NONE; c.n];
        let mut k = 0u32;
        for v in 0..c.n {
            if l[v] != NONE {
                nid[v] = k;
                k += 1;
            }
        }
        nid
    });
    let n_out = labels.as_ref().map(|l| l.iter().filter(|&&x| x != NONE).count()).unwrap_or(c.n);
    let degs: Vec<AtomicU64> = if get(a, "--out-deg").is_some() { (0..c.n).map(|_| AtomicU64::new(0)).collect() } else { Vec::new() };
    let tmp = out.with_extension(format!("tmp{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let mut w = StoreWriter::create(&tmp.join("adj.bin"), n_out, selfloop).unwrap();
    let win = rayon::current_num_threads() * 8;
    let mut i = 0;
    let kept_edges = AtomicU64::new(0);
    let nl = labels.as_ref().map(|l| l.iter().filter(|&&x| x != NONE).max().map(|&m| m as usize + 1).unwrap_or(0)).unwrap_or(1);
    let per_label: Vec<AtomicU64> = (0..nl).map(|_| AtomicU64::new(0)).collect();
    let loop_weight = AtomicU64::new(0);
    let dropped_entries = AtomicU64::new(0);
    let nmap = |x: u32| -> u32 { newid.as_ref().map(|m| m[x as usize]).unwrap_or(x) };
    while i < c.chunks.len() {
        let e = (i + win).min(c.chunks.len());
        // per source block: (record bytes, rel, deg, first output id, output count)
        let parts: Vec<(Vec<u8>, Vec<u32>, Vec<u32>, usize, usize)> = c.chunks[i..e]
            .par_iter()
            .map(|&(lo, hi)| {
                let mut cur = c.cur();
                let (mut out, mut rel, mut deg) = (Vec::new(), Vec::new(), Vec::new());
                let (mut tg, mut wt, mut tmp) = (Vec::new(), Vec::new(), Vec::new());
                let (mut kept, mut lw, mut dropn) = (0u64, 0u64, 0u64);
                let mut first = usize::MAX;
                for v in lo..hi {
                    let lv = labels.as_ref().map(|l| l[v]).unwrap_or(0);
                    let mut full = 0u64;
                    tg.clear();
                    wt.clear();
                    for &x in c.nbrs(v, &mut cur) {
                        if dr.dropped(v as u32, x) {
                            dropn += 1;
                            continue;
                        }
                        full += 1;
                        if lv != NONE && labels.as_ref().map(|l| l[x as usize] == lv).unwrap_or(true) {
                            tg.push(nmap(x));
                        }
                    }
                    if !degs.is_empty() {
                        degs[v].store(full, Relaxed);
                    }
                    if lv == NONE {
                        continue;
                    }
                    let nv = nmap(v as u32) as usize;
                    if first == usize::MAX {
                        first = nv;
                    }
                    rel.push(out.len() as u32);
                    kept += tg.len() as u64;
                    if !tg.is_empty() {
                        per_label[lv as usize].fetch_add(tg.len() as u64, Relaxed);
                    }
                    let sl = full - tg.len() as u64;
                    if selfloop {
                        wt.resize(tg.len(), 1);
                        if sl > 0 {
                            let p = tg.partition_point(|&x| (x as usize) < nv);
                            tg.insert(p, nv as u32);
                            wt.insert(p, u32::try_from(sl).expect("self-loop weight above u32"));
                            lw += sl;
                        }
                    }
                    deg.push(tg.len() as u32);
                    encode_record(nv, &tg, if selfloop { Some(&wt) } else { None }, &mut out, &mut tmp);
                }
                kept_edges.fetch_add(kept, Relaxed);
                loop_weight.fetch_add(lw, Relaxed);
                dropped_entries.fetch_add(dropn, Relaxed);
                let cnt = rel.len();
                (out, rel, deg, first, cnt)
            })
            .collect();
        let offs: Vec<u64> = parts.iter().map(|p| if p.4 == 0 { 0 } else { w.place(p.3, p.3 + p.4, p.0.len(), &p.1, &p.2) }).collect();
        parts.par_iter().zip(&offs).for_each(|(p, &o)| {
            if p.4 > 0 {
                w.write_at(o, &p.0)
            }
        });
        i = e;
    }
    let s = w.finish_dir(&tmp, false).expect("finish");
    if let Some(p) = get(a, "--out-stats") {
        let v: Vec<u64> = per_label.into_iter().map(|x| x.into_inner() / 2).collect();
        write_u64(&format!("{p}.internal.u64"), &v);
    }
    if let Some(p) = get(a, "--out-deg") {
        let v: Vec<u32> = degs.into_iter().map(|x| x.into_inner() as u32).collect();
        write_u32(&p, &v);
    }
    if let Some(l) = &labels {
        let map: Vec<u32> = (0..c.n as u32).filter(|&v| l[v as usize] != NONE).collect();
        write_u32(tmp.join("map.u32").to_str().unwrap(), &map);
    }
    let _ = std::fs::remove_dir_all(&out);
    std::fs::rename(&tmp, &out).expect("rename");
    println!(
        "{}",
        json!({"cmd": "cutgraph", "store": out.display().to_string(), "n": s.n, "n_source": c.n, "entries": s.entries, "kept_directed_entries": kept_edges.load(Relaxed),
               "dropped_directed_entries": dropped_entries.load(Relaxed), "selfloop_weight": loop_weight.load(Relaxed), "drop": frac, "seed": seed,
               "drop_keep": !dr.keeph.is_empty(), "weighted": selfloop, "data_bytes": s.data_len, "seconds": t.elapsed().as_secs_f64(), "peak_rss_bytes": peak_rss()})
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
