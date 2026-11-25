//! The lab/b1-core kernels (glab-core/src/kernels.rs), made generic over `Adj` so the same body
//! runs on raw arrays (A, C) and on the compressed store with on-the-fly decode (B).
//! Algorithms, constants and work partition are unchanged; what changes is how a list is obtained.
use crate::adj::{edge_chunks, Adj, SyncPtr};
use crate::store::Comp;
use rayon::prelude::*;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering::Relaxed};

pub trait Real: Copy + Send + Sync + std::ops::AddAssign + std::ops::Add<Output = Self> + std::ops::Mul<Output = Self> + std::ops::Div<Output = Self> + 'static {
    const NAME: &'static str;
    fn from_f64(x: f64) -> Self;
    fn to_f64(self) -> f64;
}
impl Real for f64 {
    const NAME: &'static str = "f64";
    #[inline(always)]
    fn from_f64(x: f64) -> f64 {
        x
    }
    #[inline(always)]
    fn to_f64(self) -> f64 {
        self
    }
}
impl Real for f32 {
    const NAME: &'static str = "f32";
    #[inline(always)]
    fn from_f64(x: f64) -> f32 {
        x as f32
    }
    #[inline(always)]
    fn to_f64(self) -> f64 {
        self as f64
    }
}

/// Sum contrib[u] over a neighbor slice. Four accumulators: the adds do not serialize the gathers.
#[inline(always)]
pub fn gather_sum<F: Real>(contrib: &[F], nb: &[u32]) -> F {
    let z = F::from_f64(0.0);
    let (mut s0, mut s1, mut s2, mut s3) = (z, z, z, z);
    let mut it = nb.chunks_exact(4);
    unsafe {
        for c in &mut it {
            s0 += *contrib.get_unchecked(c[0] as usize);
            s1 += *contrib.get_unchecked(c[1] as usize);
            s2 += *contrib.get_unchecked(c[2] as usize);
            s3 += *contrib.get_unchecked(c[3] as usize);
        }
        for &u in it.remainder() {
            s0 += *contrib.get_unchecked(u as usize);
        }
    }
    (s0 + s1) + (s2 + s3)
}

/// Degrees of all nodes (one sequential pass over the record headers on a compressed source).
pub fn degrees<G: Adj>(g: &G) -> Vec<u32> {
    let mut dg: Vec<u32> = Vec::with_capacity(g.n());
    #[allow(clippy::uninit_vec)]
    unsafe {
        dg.set_len(g.n())
    };
    let dp = SyncPtr(dg.as_mut_ptr());
    g.chunks().par_iter().for_each(|&(a, b)| g.for_range_deg(a, b, |v, d| unsafe { dp.write(v, d as u32) }));
    dg
}

/// Pull-based PageRank, fixed number of iterations, uniform teleport, dangling mass redistributed.
pub fn pagerank<F: Real, G: Adj>(g: &G, damping: f64, iters: usize) -> Vec<F> {
    let n = g.n();
    let inv_n = 1.0 / n as f64;
    let chunks = g.chunks();
    let mut contrib: Vec<F> = (0..n).into_par_iter().map(|_| F::from_f64(0.0)).collect();
    let cp = SyncPtr(contrib.as_mut_ptr());
    let mut dangling: f64 = chunks
        .par_iter()
        .map(|&(a, b)| {
            let mut dang = 0f64;
            g.for_range_deg(a, b, |v, d| unsafe {
                if d == 0 {
                    dang += inv_n;
                } else {
                    cp.write(v, F::from_f64(inv_n / d as f64));
                }
            });
            dang
        })
        .sum();
    let mut next: Vec<F> = (0..n).into_par_iter().map(|_| F::from_f64(0.0)).collect();
    let mut rank: Vec<F> = (0..n).into_par_iter().map(|_| F::from_f64(inv_n)).collect();
    for it in 0..iters {
        let base = F::from_f64((1.0 - damping) * inv_n + damping * dangling * inv_n);
        let dmp = F::from_f64(damping);
        let last = it + 1 == iters;
        let np = SyncPtr(next.as_mut_ptr());
        let rp = SyncPtr(rank.as_mut_ptr());
        let c = &contrib[..];
        dangling = chunks
            .par_iter()
            .with_max_len(1)
            .map(|&(a, b)| {
                let mut dang = 0f64;
                let mut cur = g.cur();
                g.for_range(a, b, &mut cur, |v, nb| unsafe {
                    let sum = gather_sum(c, nb);
                    let r = base + dmp * sum;
                    if nb.is_empty() {
                        dang += r.to_f64();
                        np.write(v, F::from_f64(0.0));
                    } else {
                        np.write(v, r / F::from_f64(nb.len() as f64));
                    }
                    if last {
                        rp.write(v, r);
                    }
                });
                dang
            })
            .sum();
        std::mem::swap(&mut contrib, &mut next);
    }
    rank
}

pub fn pr_summary<F: Real>(rank: &[F]) -> serde_json::Value {
    let sum: f64 = rank.par_iter().map(|x| x.to_f64()).sum();
    let mut top: Vec<(usize, f64)> = Vec::new();
    let mut idx: Vec<u32> = (0..rank.len() as u32).collect();
    let k = 5.min(idx.len());
    if k > 0 {
        idx.select_nth_unstable_by(k - 1, |&a, &b| rank[b as usize].to_f64().partial_cmp(&rank[a as usize].to_f64()).unwrap());
        let mut t: Vec<u32> = idx[..k].to_vec();
        t.sort_by(|&a, &b| rank[b as usize].to_f64().partial_cmp(&rank[a as usize].to_f64()).unwrap());
        top = t.iter().map(|&i| (i as usize, rank[i as usize].to_f64())).collect();
    }
    serde_json::json!({"sum": sum, "top5": top})
}

// ---------------------------------------------------------------- WCC (Afforest)

#[inline(always)]
pub fn uf_link(comp: &[AtomicU32], u: u32, v: u32) {
    unsafe {
        let mut p1 = comp.get_unchecked(u as usize).load(Relaxed);
        let mut p2 = comp.get_unchecked(v as usize).load(Relaxed);
        while p1 != p2 {
            let (high, low) = if p1 > p2 { (p1, p2) } else { (p2, p1) };
            let ph = comp.get_unchecked(high as usize).load(Relaxed);
            if ph == low {
                break;
            }
            if ph == high && comp.get_unchecked(high as usize).compare_exchange(high, low, Relaxed, Relaxed).is_ok() {
                break;
            }
            let g = comp.get_unchecked(high as usize).load(Relaxed);
            p1 = comp.get_unchecked(g as usize).load(Relaxed);
            p2 = comp.get_unchecked(low as usize).load(Relaxed);
        }
    }
}

pub fn uf_compress(comp: &[AtomicU32]) {
    (0..comp.len()).into_par_iter().with_min_len(4096).for_each(|v| unsafe {
        loop {
            let p = comp.get_unchecked(v).load(Relaxed);
            let pp = comp.get_unchecked(p as usize).load(Relaxed);
            if p == pp {
                break;
            }
            comp.get_unchecked(v).store(pp, Relaxed);
        }
    });
}

pub fn count_components(comp: &[AtomicU32]) -> usize {
    comp.par_iter().enumerate().filter(|(i, c)| c.load(Relaxed) as usize == *i).count()
}

fn sample_frequent(comp: &[AtomicU32]) -> u32 {
    let n = comp.len();
    let mut counts: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    let mut x = 0x9E3779B97F4A7C15u64;
    for _ in 0..1024 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *counts.entry(comp[(x % n as u64) as usize].load(Relaxed)).or_insert(0) += 1;
    }
    counts.into_iter().max_by_key(|&(c, k)| (k, std::cmp::Reverse(c))).map(|(c, _)| c).unwrap_or(0)
}

/// Afforest: 2 neighbor-sampling rounds, then skip the giant component.
/// On a compressed source the sampling rounds decode only the first two neighbors of each list
/// (`for_range_prefix`), and full lists are decoded only for nodes outside the giant component.
/// Returns (labels, fraction of adjacency entries actually examined).
pub fn wcc_afforest<G: Adj>(g: &G) -> (Vec<AtomicU32>, f64) {
    let n = g.n();
    let chunks = g.chunks();
    let comp: Vec<AtomicU32> = (0..n as u32).into_par_iter().map(AtomicU32::new).collect();
    if n == 0 {
        return (comp, 0.0);
    }
    const ROUNDS: usize = 2;
    for r in 0..ROUNDS {
        chunks.par_iter().with_max_len(1).for_each(|&(a, b)| {
            let mut cur = g.cur();
            g.for_range_prefix(a, b, ROUNDS, &mut cur, |u, nb, _| {
                if nb.len() > r {
                    uf_link(&comp, u as u32, nb[r]);
                }
            });
        });
        uf_compress(&comp);
    }
    let giant = sample_frequent(&comp);
    let examined: u64 = chunks
        .par_iter()
        .with_max_len(1)
        .map(|&(a, b)| {
            let mut ex = 0u64;
            let mut cur = g.cur();
            for u in a..b {
                if comp[u].load(Relaxed) == giant {
                    continue;
                }
                let nb = g.nbrs(u, &mut cur);
                if nb.len() > ROUNDS {
                    ex += (nb.len() - ROUNDS) as u64;
                    for &v in &nb[ROUNDS..] {
                        uf_link(&comp, u as u32, v);
                    }
                }
            }
            ex
        })
        .sum();
    uf_compress(&comp);
    let frac = (examined as f64 + (ROUNDS * n) as f64) / g.entries().max(1) as f64;
    (comp, frac)
}

// ---------------------------------------------------------------- BFS (direction optimizing)

pub struct BfsResult {
    pub depth: Vec<AtomicU32>,
    pub reached: usize,
    pub levels: u32,
    pub td_steps: u32,
    pub bu_steps: u32,
    pub edges_examined: u64,
}

pub fn bfs_do<G: Adj>(g: &G, src: usize) -> BfsResult {
    const UNV: u32 = u32::MAX;
    const ALPHA: u64 = 15;
    const BETA: u64 = 18;
    let n = g.n();
    let chunks = g.chunks();
    let depth: Vec<AtomicU32> = (0..n).into_par_iter().map(|_| AtomicU32::new(UNV)).collect();
    depth[src].store(0, Relaxed);
    let nw = n.div_ceil(64);
    let mut frontier: Vec<u32> = vec![src as u32];
    let mut reached = 1usize;
    let mut level = 0u32;
    let mut edges_to_check = g.entries() as u64;
    let mut scout: u64 = g.deg(src) as u64;
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
                            if depth[v].load(Relaxed) != UNV {
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
                            if d.load(Relaxed) == UNV && d.compare_exchange(UNV, level + 1, Relaxed, Relaxed).is_ok() {
                                acc.0.push(v);
                                acc.1 += g.deg(v as usize) as u64;
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
    let levels = depth.par_iter().map(|d| d.load(Relaxed)).filter(|&d| d != UNV).max().unwrap_or(0);
    BfsResult { depth, reached, levels, td_steps, bu_steps, edges_examined: examined }
}

// ---------------------------------------------------------------- k-core (PKC parallel peeling)

/// Returns (coreness, max core, number of levels processed).
pub fn kcore<G: Adj>(g: &G) -> (Vec<u32>, u32, u64) {
    let n = g.n();
    let mut deg_v: Vec<u32> = degrees(g);
    let deg: &[AtomicU32] = unsafe { &*(deg_v.as_mut_slice() as *mut [u32] as *const [AtomicU32]) };
    let mut core: Vec<u32> = vec![0; n];
    let cp = SyncPtr(core.as_mut_ptr());
    let mut alive: Vec<u32> = (0..n as u32).collect();
    let mut levels = 0u64;
    let mut k = 0u32;
    while !alive.is_empty() {
        levels += 1;
        let chunk = (alive.len() / (rayon::current_num_threads() * 8)).clamp(1024, 1 << 16);
        let mut parts: Vec<(Vec<u32>, Vec<u32>)> = alive
            .par_chunks(chunk)
            .map(|c| {
                let mut keep: Vec<u32> = Vec::with_capacity(c.len());
                let mut stack: Vec<u32> = Vec::new();
                for &v in c {
                    let d = deg[v as usize].load(Relaxed);
                    if d > k {
                        keep.push(v);
                    } else if d == k {
                        stack.push(v);
                    }
                }
                (keep, stack)
            })
            .collect();
        if parts.iter().any(|p| !p.1.is_empty()) {
            parts.par_iter_mut().with_max_len(1).for_each(|(_, stack)| {
                let mut cur = g.cur();
                while let Some(u) = stack.pop() {
                    unsafe { cp.write(u as usize, k) };
                    for &v in g.nbrs(u as usize, &mut cur) {
                        let d = unsafe { deg.get_unchecked(v as usize) };
                        if d.load(Relaxed) > k {
                            let old = d.fetch_sub(1, Relaxed);
                            if old == k + 1 {
                                stack.push(v);
                            } else if old <= k {
                                d.fetch_add(1, Relaxed);
                            }
                        }
                    }
                }
            });
        }
        let parts: Vec<Vec<u32>> = parts.into_iter().map(|p| p.0).collect();
        let total: usize = parts.iter().map(|p| p.len()).sum();
        let mut na: Vec<u32> = Vec::with_capacity(total);
        for p in &parts {
            na.extend_from_slice(p);
        }
        alive = na;
        k += 1;
    }
    let maxc = core.par_iter().copied().max().unwrap_or(0);
    (core, maxc, levels)
}

// ---------------------------------------------------------------- triangle counting

#[inline(always)]
fn intersect_count(a: &[u32], b: &[u32]) -> u64 {
    let (mut i, mut j, mut c) = (0usize, 0usize, 0u64);
    let (la, lb) = (a.len(), b.len());
    unsafe {
        while i < la && j < lb {
            let x = *a.get_unchecked(i);
            let y = *b.get_unchecked(j);
            c += (x == y) as u64;
            i += (x <= y) as usize;
            j += (y <= x) as usize;
        }
    }
    c
}

/// Lab algorithm: build the degree-ordered forward adjacency (u -> v iff (deg,id)(u) < (deg,id)(v))
/// as *uncompressed* arrays owned by the kernel (4 bytes per undirected edge + 8 per node), then
/// count by merge intersection on them. The input is only scanned sequentially (twice).
/// Returns (triangles, build seconds).
pub fn triangles_fwd_raw<G: Adj>(g: &G) -> (u64, f64) {
    let t0 = std::time::Instant::now();
    let n = g.n();
    let chunks = g.chunks();
    let dg = degrees(g);
    let less = |u: usize, v: usize| -> bool {
        let (du, dv) = unsafe { (*dg.get_unchecked(u), *dg.get_unchecked(v)) };
        du < dv || (du == dv && u < v)
    };
    let mut fdeg: Vec<u32> = vec![0; n];
    let fdp = SyncPtr(fdeg.as_mut_ptr());
    chunks.par_iter().for_each(|&(a, b)| {
        let mut cur = g.cur();
        g.for_range(a, b, &mut cur, |u, nb| unsafe { fdp.write(u, nb.iter().filter(|&&v| less(u, v as usize)).count() as u32) });
    });
    let mut foff = vec![0u64; n + 1];
    for i in 0..n {
        foff[i + 1] = foff[i] + fdeg[i] as u64;
    }
    drop(fdeg);
    let m = foff[n] as usize;
    let mut ftgt: Vec<u32> = Vec::with_capacity(m);
    #[allow(clippy::uninit_vec)]
    unsafe {
        ftgt.set_len(m)
    };
    let fp = SyncPtr(ftgt.as_mut_ptr());
    chunks.par_iter().for_each(|&(a, b)| {
        let mut cur = g.cur();
        g.for_range(a, b, &mut cur, |u, nb| {
            let s = unsafe { fp.slice_mut(foff[u] as usize, foff[u + 1] as usize) };
            let mut i = 0;
            for &v in nb {
                if less(u, v as usize) {
                    s[i] = v;
                    i += 1;
                }
            }
        });
    });
    let build = t0.elapsed().as_secs_f64();
    let fo = &foff[..];
    let ft = &ftgt[..];
    let fchunks = edge_chunks(fo, 1 << 14);
    let total: u64 = fchunks
        .par_iter()
        .with_max_len(1)
        .map(|&(a, b)| {
            let mut c = 0u64;
            for u in a..b {
                let nu = unsafe { ft.get_unchecked(fo[u] as usize..fo[u + 1] as usize) };
                for &v in nu {
                    let nv = unsafe { ft.get_unchecked(fo[v as usize] as usize..fo[v as usize + 1] as usize) };
                    c += intersect_count(nu, nv);
                }
            }
            c
        })
        .sum();
    (total, build)
}

/// Same algorithm, but the forward adjacency is itself kept in the compressed format and every
/// intersection decodes N+(v) on the fly (random access to one compressed list per forward edge).
/// Kernel-owned memory: compressed forward lists + Elias-Fano offsets + 4 bytes per node.
/// Returns (triangles, build seconds, bytes of the compressed forward adjacency).
pub fn triangles_fwd_comp<G: Adj>(g: &G) -> (u64, f64, usize) {
    let t0 = std::time::Instant::now();
    let n = g.n();
    let dg = degrees(g);
    let less = |u: usize, v: usize| -> bool {
        let (du, dv) = unsafe { (*dg.get_unchecked(u), *dg.get_unchecked(v)) };
        du < dv || (du == dv && u < v)
    };
    // forward degrees, for the work partition of the counting phase
    let mut fdeg: Vec<u32> = vec![0; n];
    let fdp = SyncPtr(fdeg.as_mut_ptr());
    g.chunks().par_iter().for_each(|&(a, b)| {
        let mut cur = g.cur();
        g.for_range(a, b, &mut cur, |u, nb| unsafe { fdp.write(u, nb.iter().filter(|&&v| less(u, v as usize)).count() as u32) });
    });
    let mut foff = vec![0u64; n + 1];
    for i in 0..n {
        foff[i + 1] = foff[i] + fdeg[i] as u64;
    }
    drop(fdeg);
    let fchunks = edge_chunks(&foff, 1 << 14);
    let m = foff[n] as usize;
    drop(foff);
    // one encoder task per forward block; each re-reads its node range from the input
    let (fwd, _) = Comp::build_with(
        n,
        m,
        fchunks,
        || g.cur(),
        |cur, u, out| {
            for &v in g.nbrs(u, cur) {
                if less(u, v as usize) {
                    out.push(v);
                }
            }
        },
    );
    let build = t0.elapsed().as_secs_f64();
    let bytes = fwd.data_len + fwd.boff.ram_bytes();
    let total: u64 = fwd
        .chunks()
        .par_iter()
        .with_max_len(1)
        .map(|&(a, b)| {
            let mut c = 0u64;
            let mut ca = fwd.cur();
            let mut cb = fwd.cur();
            fwd.for_range(a, b, &mut ca, |_, nu| {
                for &v in nu {
                    c += intersect_count(nu, fwd.nbrs(v as usize, &mut cb));
                }
            });
            c
        })
        .sum();
    (total, build, bytes)
}

/// No kernel-owned adjacency at all: orient by id (u < v < w) and intersect the *input* lists,
/// decoding N(v) in full for every edge (u, v), u < v. Decode work is sum over edges of deg(v):
/// only meant to show why a derived oriented adjacency is needed. Returns triangles.
pub fn triangles_naive<G: Adj>(g: &G) -> u64 {
    g.chunks()
        .par_iter()
        .with_max_len(1)
        .map(|&(a, b)| {
            let mut c = 0u64;
            let mut ca = g.cur();
            let mut cb = g.cur();
            for u in a..b {
                let nu = g.nbrs(u, &mut ca);
                let s = nu.partition_point(|&x| (x as usize) <= u);
                let nu = &nu[s..];
                for (i, &v) in nu.iter().enumerate() {
                    let nv = g.nbrs(v as usize, &mut cb);
                    let sv = nv.partition_point(|&x| x <= v);
                    c += intersect_count(&nu[i + 1..], &nv[sv..]);
                }
            }
            c
        })
        .sum()
}

/// Sum over nodes of deg(v)^2 (the decode work of `triangles_naive` is about half of it).
pub fn sum_deg_sq<G: Adj>(g: &G) -> f64 {
    g.chunks()
        .par_iter()
        .map(|&(a, b)| {
            let mut s = 0f64;
            g.for_range_deg(a, b, |_, d| s += (d as f64) * (d as f64));
            s
        })
        .sum()
}
