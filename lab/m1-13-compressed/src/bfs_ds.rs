//! Direction-optimizing BFS with the degree kept in the traversal state.
//!
//! Same traversal and same output as `kernels::bfs_do` (the lab kernel), restructured for a source
//! where a random degree lookup is not free: the depth word of an unvisited node holds its degree
//! (top bit set), as in the GAP reference implementation, so the top-down step takes the scout
//! count from the word it has to CAS anyway instead of asking the graph for deg(v). The source
//! (max-degree node) falls out of the same initial pass. Bottom-up steps are unchanged.
use crate::adj::{Adj, SyncPtr};
use crate::kernels::BfsResult;
use rayon::prelude::*;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering::Relaxed};

pub fn bfs_do_degree_in_state<G: Adj>(g: &G) -> (BfsResult, usize) {
    const UNV: u32 = 1 << 31;
    const ALPHA: u64 = 15;
    const BETA: u64 = 18;
    let n = g.n();
    let chunks = g.chunks();
    let mut depth: Vec<AtomicU32> = Vec::with_capacity(n);
    #[allow(clippy::uninit_vec)]
    unsafe {
        depth.set_len(n)
    };
    let dp = SyncPtr(depth.as_mut_ptr() as *mut u32);
    let src = chunks
        .par_iter()
        .map(|&(a, b)| {
            let mut best = (0usize, std::cmp::Reverse(a));
            g.for_range_deg(a, b, |v, d| {
                unsafe { dp.write(v, UNV | (d as u32).min(UNV - 1)) };
                best = best.max((d, std::cmp::Reverse(v)));
            });
            best
        })
        .max()
        .map(|(_, r)| r.0)
        .unwrap_or(0);
    let mut scout: u64 = (depth[src].load(Relaxed) & (UNV - 1)) as u64;
    depth[src].store(0, Relaxed);
    let nw = n.div_ceil(64);
    let mut frontier: Vec<u32> = vec![src as u32];
    let mut reached = 1usize;
    let mut level = 0u32;
    let mut edges_to_check = g.entries() as u64;
    let (mut td_steps, mut bu_steps) = (0u32, 0u32);
    let mut examined = 0u64;
    while !frontier.is_empty() {
        if scout > edges_to_check / ALPHA {
            // ---- bottom-up phase
            let mut front: Vec<AtomicU64> = (0..nw).into_par_iter().map(|_| AtomicU64::new(0)).collect();
            frontier.par_iter().for_each(|&u| {
                front[(u >> 6) as usize].fetch_or(1 << (u & 63), Relaxed);
            });
            let mut awake = frontier.len() as u64;
            loop {
                let old_awake = awake;
                let nextb: Vec<AtomicU64> = (0..nw).into_par_iter().map(|_| AtomicU64::new(0)).collect();
                let (aw, ex): (u64, u64) = chunks
                    .par_iter()
                    .with_max_len(1)
                    .map(|&(a, b)| {
                        let mut aw = 0u64;
                        let mut ex = 0u64;
                        let mut cur = g.cur();
                        for v in a..b {
                            if depth[v].load(Relaxed) & UNV == 0 {
                                continue;
                            }
                            let (found, tested) = g.any(v, &mut cur, |u| unsafe { front.get_unchecked((u >> 6) as usize) }.load(Relaxed) >> (u & 63) & 1 != 0);
                            ex += tested as u64;
                            if found {
                                depth[v].store(level + 1, Relaxed);
                                nextb[v >> 6].fetch_or(1 << (v & 63), Relaxed);
                                aw += 1;
                            }
                        }
                        (aw, ex)
                    })
                    .reduce(|| (0, 0), |x, y| (x.0 + y.0, x.1 + y.1));
                awake = aw;
                examined += ex;
                reached += aw as usize;
                level += 1;
                bu_steps += 1;
                front = nextb;
                if awake == 0 || (awake < old_awake && awake <= n as u64 / BETA) {
                    break;
                }
            }
            frontier = (0..nw)
                .into_par_iter()
                .fold(Vec::new, |mut acc: Vec<u32>, w| {
                    let mut x = front[w].load(Relaxed);
                    while x != 0 {
                        acc.push(((w as u32) << 6) | x.trailing_zeros());
                        x &= x - 1;
                    }
                    acc
                })
                .flatten()
                .collect();
            scout = 1;
        } else {
            // ---- top-down step
            edges_to_check = edges_to_check.saturating_sub(scout);
            let parts: Vec<(Vec<u32>, u64, u64, G::Cur)> = frontier
                .par_iter()
                .with_min_len(64)
                .fold(
                    || (Vec::new(), 0u64, 0u64, g.cur()),
                    |mut acc, &u| {
                        let nb = g.nbrs(u as usize, &mut acc.3);
                        acc.2 += nb.len() as u64;
                        for &v in nb {
                            let d = unsafe { depth.get_unchecked(v as usize) };
                            let old = d.load(Relaxed);
                            if old & UNV != 0 && d.compare_exchange(old, level + 1, Relaxed, Relaxed).is_ok() {
                                acc.0.push(v);
                                acc.1 += (old & (UNV - 1)) as u64;
                            }
                        }
                        acc
                    },
                )
                .collect();
            scout = parts.iter().map(|p| p.1).sum();
            examined += parts.iter().map(|p| p.2).sum::<u64>();
            frontier = parts.into_par_iter().flat_map_iter(|p| p.0).collect();
            reached += frontier.len();
            level += 1;
            td_steps += 1;
        }
    }
    // unreached nodes get the lab's marker so that the summaries are comparable
    depth.par_iter().for_each(|d| {
        if d.load(Relaxed) & UNV != 0 {
            d.store(u32::MAX, Relaxed)
        }
    });
    let levels = depth.par_iter().map(|d| d.load(Relaxed)).filter(|&d| d != u32::MAX).max().unwrap_or(0);
    (BfsResult { depth, reached, levels, td_steps, bu_steps, edges_examined: examined }, src)
}
