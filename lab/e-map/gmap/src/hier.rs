//! Fallback hierarchy builder (used when no Louvain/Leiden hierarchy is published):
//! multilevel size-constrained label propagation (the coarsening scheme of multilevel
//! partitioners). Lower modularity than Louvain, but O(m) per level and bounded community sizes.
use crate::common::*;
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use std::time::Instant;

fn lp(n: usize, off: &[u64], tgt: &[u32], w: Option<&[u32]>, size: &[u32], cap: u32, rounds: usize) -> Vec<u32> {
    let mut label: Vec<u32> = (0..n as u32).collect();
    let mut csz = size.to_vec();
    let mut acc = vec![0u64; n];
    let mut touched: Vec<u32> = vec![];
    for _ in 0..rounds {
        let mut moved = 0usize;
        for u in 0..n {
            let lu = label[u];
            touched.clear();
            for e in off[u] as usize..off[u + 1] as usize {
                let l = label[tgt[e] as usize];
                if acc[l as usize] == 0 {
                    touched.push(l);
                }
                acc[l as usize] += w.map_or(1, |w| w[e] as u64);
            }
            let mut best = lu;
            let mut bw = acc[lu as usize];
            for &l in &touched {
                if l != lu && acc[l as usize] > bw && csz[l as usize] + size[u] <= cap {
                    best = l;
                    bw = acc[l as usize];
                }
            }
            for &l in &touched {
                acc[l as usize] = 0;
            }
            if best != lu {
                csz[lu as usize] -= size[u];
                csz[best as usize] += size[u];
                label[u] = best;
                moved += 1;
            }
        }
        if moved * 200 < n {
            break;
        }
    }
    // compact
    let mut map = vec![u32::MAX; n];
    let mut c = 0u32;
    for u in 0..n {
        let l = label[u] as usize;
        if map[l] == u32::MAX {
            map[l] = c;
            c += 1;
        }
        label[u] = map[l];
    }
    label
}

pub fn run() {
    let a = args();
    let csr = Csr::load(&args_s(&a, "graph"));
    let out = args_s(&a, "out");
    std::fs::create_dir_all(&out).unwrap();
    let t0 = Instant::now();
    let mut cap = argf(&a, "cap", 48.0) as u32;
    let mult = argf(&a, "mult", 24.0) as u32;
    let mut n = csr.n;
    let mut off = csr.off.to_vec();
    let mut tgt = csr.tgt.to_vec();
    let mut w: Option<Vec<u32>> = None;
    let mut size = vec![1u32; n];
    let mut levels = vec![];
    let mut k = 0;
    loop {
        let label = lp(n, &off, &tgt, w.as_deref(), &size, cap, 4);
        let c = *label.iter().max().unwrap() as usize + 1;
        if k > 0 && (c as f64 > 0.95 * n as f64 || c < 8) {
            break;
        }
        write_slice(&format!("{out}/level_{k}.u32"), &label);
        levels.push(c);
        eprintln!("level {k}: {c} communities (cap {cap}) {:.1}s", t0.elapsed().as_secs_f64());
        // aggregate
        let mut nsize = vec![0u32; c];
        let mut cnt = vec![0u32; c + 1];
        for u in 0..n {
            nsize[label[u] as usize] += size[u];
            cnt[label[u] as usize + 1] += 1;
        }
        for i in 0..c {
            cnt[i + 1] += cnt[i];
        }
        let mut fill = cnt.clone();
        let mut members = vec![0u32; n];
        for u in 0..n {
            members[fill[label[u] as usize] as usize] = u as u32;
            fill[label[u] as usize] += 1;
        }
        let rows: Vec<Vec<(u32, u32)>> = (0..c)
            .into_par_iter()
            .map(|ci| {
                let mut m: FxHashMap<u32, u32> = FxHashMap::default();
                for &u in &members[cnt[ci] as usize..cnt[ci + 1] as usize] {
                    for e in off[u as usize] as usize..off[u as usize + 1] as usize {
                        let l = label[tgt[e] as usize];
                        if l as usize != ci {
                            *m.entry(l).or_insert(0) += w.as_ref().map_or(1, |w| w[e]);
                        }
                    }
                }
                let mut v: Vec<(u32, u32)> = m.into_iter().collect();
                v.sort_unstable();
                v
            })
            .collect();
        let mut noff = vec![0u64; c + 1];
        for i in 0..c {
            noff[i + 1] = noff[i] + rows[i].len() as u64;
        }
        let mut ntgt = Vec::with_capacity(noff[c] as usize);
        let mut nw = Vec::with_capacity(noff[c] as usize);
        for r in &rows {
            for &(t, x) in r {
                ntgt.push(t);
                nw.push(x);
            }
        }
        n = c;
        off = noff;
        tgt = ntgt;
        w = Some(nw);
        size = nsize;
        k += 1;
        if c <= 24 || cap as usize >= csr.n / 6 {
            break;
        }
        cap = cap.saturating_mul(mult);
    }
    let j = format!("{{\"graph\":\"{}\",\"n\":{},\"levels\":{:?},\"method\":\"size-constrained label propagation (gmap hier fallback)\",\"seconds\":{:.2}}}", args_s(&a, "graph"), csr.n, levels, t0.elapsed().as_secs_f64());
    std::fs::write(format!("{out}/hier.json"), &j).unwrap();
    println!("{j}");
}
