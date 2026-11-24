//! In-RAM kernels over CSR slices (work identically on mmap-backed slices).
use crate::{Csr, SyncPtr};
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
pub trait NodeId: Copy + Send + Sync + 'static {
    fn idx(self) -> usize;
}
impl NodeId for u32 {
    #[inline(always)]
    fn idx(self) -> usize {
        self as usize
    }
}
impl NodeId for u64 {
    #[inline(always)]
    fn idx(self) -> usize {
        self as usize
    }
}

/// Sum contrib[u] over a neighbor slice. Four accumulators: the adds do not serialize the gathers.
#[inline(always)]
pub fn gather_sum<F: Real, I: NodeId>(contrib: &[F], nb: &[I]) -> F {
    let z = F::from_f64(0.0);
    let (mut s0, mut s1, mut s2, mut s3) = (z, z, z, z);
    let mut it = nb.chunks_exact(4);
    unsafe {
        for c in &mut it {
            s0 += *contrib.get_unchecked(c[0].idx());
            s1 += *contrib.get_unchecked(c[1].idx());
            s2 += *contrib.get_unchecked(c[2].idx());
            s3 += *contrib.get_unchecked(c[3].idx());
        }
        for &u in it.remainder() {
            s0 += *contrib.get_unchecked(u.idx());
        }
    }
    (s0 + s1) + (s2 + s3)
}

/// Pull-based PageRank, fixed number of iterations, uniform teleport, dangling mass redistributed.
/// Per iteration: sequential read of offsets+targets, random read of contrib (n*F), sequential write (n*F).
pub fn pagerank<F: Real, I: NodeId>(offsets: &[u64], targets: &[I], damping: f64, iters: usize, chunks: &[(usize, usize)]) -> Vec<F> {
    let n = offsets.len() - 1;
    let inv_n = 1.0 / n as f64;
    let mut contrib: Vec<F> = (0..n)
        .into_par_iter()
        .map(|v| {
            let d = offsets[v + 1] - offsets[v];
            if d == 0 { F::from_f64(0.0) } else { F::from_f64(inv_n / d as f64) }
        })
        .collect();
    let mut dangling: f64 = (0..n).into_par_iter().map(|v| if offsets[v + 1] == offsets[v] { inv_n } else { 0.0 }).sum();
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
                for v in a..b {
                    unsafe {
                        let s = *offsets.get_unchecked(v) as usize;
                        let e = *offsets.get_unchecked(v + 1) as usize;
                        let sum = gather_sum(c, targets.get_unchecked(s..e));
                        let r = base + dmp * sum;
                        if e == s {
                            dang += r.to_f64();
                            np.write(v, F::from_f64(0.0));
                        } else {
                            np.write(v, r / F::from_f64((e - s) as f64));
                        }
                        if last {
                            rp.write(v, r);
                        }
                    }
                }
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
    // top-5 by score
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

// ---------------------------------------------------------------- WCC

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

/// Root of x with lock-free path halving.
#[inline(always)]
pub fn uf_find(comp: &[AtomicU32], mut x: u32) -> u32 {
    unsafe {
        loop {
            let p = comp.get_unchecked(x as usize).load(Relaxed);
            if p == x {
                return x;
            }
            let gp = comp.get_unchecked(p as usize).load(Relaxed);
            if gp == p {
                return p;
            }
            let _ = comp.get_unchecked(x as usize).compare_exchange_weak(p, gp, Relaxed, Relaxed);
            x = gp;
        }
    }
}

/// Union by index (larger root hooks under smaller root => final label = min id). Returns the root of the merged set.
#[inline(always)]
pub fn uf_union(comp: &[AtomicU32], mut ru: u32, v: u32) -> u32 {
    let mut rv = uf_find(comp, v);
    loop {
        if ru == rv {
            return ru;
        }
        let (hi, lo) = if ru > rv { (ru, rv) } else { (rv, ru) };
        match unsafe { comp.get_unchecked(hi as usize) }.compare_exchange(hi, lo, Relaxed, Relaxed) {
            Ok(_) => return lo,
            Err(_) => {
                ru = uf_find(comp, ru);
                rv = uf_find(comp, rv);
            }
        }
    }
}

/// One node's adjacency in the streaming union-find (only v < u is needed on a symmetric graph).
#[inline(always)]
pub fn uf_stream_node(comp: &[AtomicU32], u: u32, nb: &[u32]) {
    const PF: usize = 8;
    let mut ru = uf_find(comp, u);
    for (i, &v) in nb.iter().enumerate() {
        if v >= u {
            break; // adjacency is sorted
        }
        if i + PF < nb.len() {
            unsafe {
                use std::arch::x86_64::{_mm_prefetch, _MM_HINT_T0};
                _mm_prefetch(comp.as_ptr().add(*nb.get_unchecked(i + PF) as usize) as *const i8, _MM_HINT_T0);
            }
        }
        // fast path: neighbor already points at our root
        if unsafe { comp.get_unchecked(v as usize) }.load(Relaxed) == ru {
            continue;
        }
        ru = uf_union(comp, ru, v);
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

/// Afforest (Sutton, Ben-Nun, Besta 2018; the GAP benchmark CC): union-find with lock-free hooking
/// (always hook the larger root id under the smaller => final label = min node id of the component,
/// deterministic output), 2 neighbor-sampling rounds, then skip the giant component.
/// Returns (labels, fraction of adjacency entries actually examined).
pub fn wcc_afforest(g: &Csr, chunks: &[(usize, usize)]) -> (Vec<AtomicU32>, f64) {
    let n = g.n;
    let comp: Vec<AtomicU32> = (0..n as u32).into_par_iter().map(AtomicU32::new).collect();
    if n == 0 {
        return (comp, 0.0);
    }
    const ROUNDS: usize = 2;
    for r in 0..ROUNDS {
        chunks.par_iter().with_max_len(1).for_each(|&(a, b)| {
            for u in a..b {
                let nb = g.nbrs(u);
                if nb.len() > r {
                    uf_link(&comp, u as u32, nb[r]);
                }
            }
        });
        uf_compress(&comp);
    }
    let giant = sample_frequent(&comp);
    let examined: u64 = chunks
        .par_iter()
        .with_max_len(1)
        .map(|&(a, b)| {
            let mut ex = 0u64;
            for u in a..b {
                if comp[u].load(Relaxed) == giant {
                    continue;
                }
                let nb = g.nbrs(u);
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

/// Single pass over all adjacency entries with concurrent union-find (edge streaming order).
/// Same algorithm the semi-external engine uses; here on in-memory slices.
pub fn wcc_stream(g: &Csr, chunks: &[(usize, usize)]) -> Vec<AtomicU32> {
    let comp: Vec<AtomicU32> = (0..g.n as u32).into_par_iter().map(AtomicU32::new).collect();
    chunks.par_iter().with_max_len(1).for_each(|&(a, b)| {
        for u in a..b {
            uf_stream_node(&comp, u as u32, g.nbrs(u));
        }
    });
    uf_compress(&comp);
    comp
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

pub fn bfs_do(g: &Csr, src: usize, chunks: &[(usize, usize)]) -> BfsResult {
    const UNV: u32 = u32::MAX;
    const ALPHA: u64 = 15;
    const BETA: u64 = 18;
    let n = g.n;
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
                        for v in a..b {
                            if depth[v].load(Relaxed) != UNV {
                                continue;
                            }
                            for &u in g.nbrs(v) {
                                ex += 1;
                                if unsafe { front.get_unchecked((u >> 6) as usize) }.load(Relaxed) >> (u & 63) & 1 != 0 {
                                    depth[v].store(level + 1, Relaxed);
                                    nextb[v >> 6].fetch_or(1 << (v & 63), Relaxed);
                                    aw += 1;
                                    break;
                                }
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
            // bitmap -> queue
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
            let parts: Vec<(Vec<u32>, u64, u64)> = frontier
                .par_iter()
                .with_min_len(64)
                .fold(
                    || (Vec::new(), 0u64, 0u64),
                    |mut acc, &u| {
                        let nb = g.nbrs(u as usize);
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

/// BFS visit order (unreached nodes appended in id order); used for the reordering probe.
pub fn bfs_order(g: &Csr, src: usize) -> Vec<u32> {
    let n = g.n;
    let mut seen = vec![false; n];
    let mut order: Vec<u32> = Vec::with_capacity(n);
    let mut next_unseen = 0usize;
    let mut head = 0usize;
    seen[src] = true;
    order.push(src as u32);
    loop {
        while head < order.len() {
            let u = order[head] as usize;
            head += 1;
            for &v in g.nbrs(u) {
                if !seen[v as usize] {
                    seen[v as usize] = true;
                    order.push(v);
                }
            }
        }
        while next_unseen < n && seen[next_unseen] {
            next_unseen += 1;
        }
        if next_unseen >= n {
            break;
        }
        seen[next_unseen] = true;
        order.push(next_unseen as u32);
    }
    order
}

// ---------------------------------------------------------------- k-core (parallel peeling)

/// Returns (coreness, max core, number of levels processed).
/// PKC-style parallel peeling (Kabir & Madduri): one parallel pass over the surviving nodes per
/// level k; each task peels its own degree-k nodes *and* everything that drops to k because of
/// them from a task-local stack (no synchronized sub-rounds). A neighbor's degree is decremented
/// atomically; the task whose decrement reaches exactly k owns that node; over-decrements are undone.
pub fn kcore(g: &Csr) -> (Vec<u32>, u32, u64) {
    let n = g.n;
    let deg: Vec<AtomicU32> = (0..n).into_par_iter().map(|v| AtomicU32::new(g.deg(v) as u32)).collect();
    let mut core: Vec<u32> = vec![0; n];
    let cp = SyncPtr(core.as_mut_ptr());
    let mut alive: Vec<u32> = (0..n as u32).collect();
    let mut levels = 0u64;
    let mut k = 0u32;
    while !alive.is_empty() {
        levels += 1;
        let chunk = (alive.len() / (rayon::current_num_threads() * 8)).clamp(1024, 1 << 16);
        // phase 1: scan (all scans finish before any peeling, so every node has exactly one owner)
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
                    // d < k: removed in an earlier level
                }
                (keep, stack)
            })
            .collect();
        // phase 2: peel from task-local stacks
        if parts.iter().any(|p| !p.1.is_empty()) {
            parts.par_iter_mut().with_max_len(1).for_each(|(_, stack)| {
                while let Some(u) = stack.pop() {
                    unsafe { cp.write(u as usize, k) };
                    for &v in g.nbrs(u as usize) {
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
        // survivors recorded before a later decrement are filtered by the next level's scan
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

/// Degree-ordered forward adjacency (u -> v iff (deg,id)(u) < (deg,id)(v)), lists stay id-sorted;
/// count = sum over forward edges of |N+(u) ∩ N+(v)| by merge. Returns (triangles, build seconds).
pub fn triangles(g: &Csr) -> (u64, f64) {
    let t0 = std::time::Instant::now();
    let n = g.n;
    let dg: Vec<u32> = (0..n).into_par_iter().map(|v| g.deg(v) as u32).collect();
    let less = |u: usize, v: usize| -> bool {
        let (du, dv) = unsafe { (*dg.get_unchecked(u), *dg.get_unchecked(v)) };
        du < dv || (du == dv && u < v)
    };
    let fdeg: Vec<u32> = (0..n).into_par_iter().with_min_len(1024).map(|u| g.nbrs(u).iter().filter(|&&v| less(u, v as usize)).count() as u32).collect();
    let mut foff = vec![0u64; n + 1];
    for i in 0..n {
        foff[i + 1] = foff[i] + fdeg[i] as u64;
    }
    drop(fdeg);
    let m = foff[n] as usize;
    let mut ftgt: Vec<u32> = Vec::with_capacity(m);
    unsafe { ftgt.set_len(m) };
    let fp = SyncPtr(ftgt.as_mut_ptr());
    (0..n).into_par_iter().with_min_len(1024).for_each(|u| {
        let s = unsafe { fp.slice_mut(foff[u] as usize, foff[u + 1] as usize) };
        let mut i = 0;
        for &v in g.nbrs(u) {
            if less(u, v as usize) {
                s[i] = v;
                i += 1;
            }
        }
    });
    let build = t0.elapsed().as_secs_f64();
    let fo = &foff[..];
    let ft = &ftgt[..];
    let fchunks = crate::edge_chunks(fo, 1 << 14);
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

pub fn degree_desc_order(g: &Csr) -> Vec<u32> {
    let mut order: Vec<u32> = (0..g.n as u32).collect();
    order.par_sort_unstable_by_key(|&v| (std::cmp::Reverse(g.deg(v as usize)), v));
    order
}

pub fn random_order(n: usize) -> Vec<u32> {
    let mut order: Vec<u32> = (0..n as u32).collect();
    let mut x = 0x2545F4914F6CDD1Du64;
    for i in (1..n).rev() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let j = (x % (i as u64 + 1)) as usize;
        order.swap(i, j);
    }
    order
}
