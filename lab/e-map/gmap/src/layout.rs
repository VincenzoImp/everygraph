//! Hierarchy-constrained multilevel circle layout ("hcl").
//! Top-down over the community tree; every parent lays out its <= kmax children inside its
//! own disc with a small dense force-directed solve (sibling quotient weights + pull towards
//! the already-placed regions of external neighbours), then packs them without overlap.
//! Parents with more than kmax children get derived intermediate levels (size-constrained
//! label propagation on the sibling quotient graph).
use crate::common::*;
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

pub struct Params {
    pub kmax: usize,
    pub phi: f32,
    pub phi_leaf: f32,
    pub seed: u64,
    pub use_ext: bool,
    pub iters: usize,
}

struct Ctx<'a> {
    csr: &'a Csr,
    tn: &'a [TNode],
    kids: &'a [Vec<u32>],
    order: &'a [u32],
    cur: &'a [u32],
    childidx: SyncPtr<u32>,
    cxy: &'a [[f32; 2]],
    pos: SyncPtr<[f32; 2]>,
    prm: &'a Params,
    t_scan: &'a AtomicU64,
    t_fr: &'a AtomicU64,
    t_group: &'a AtomicU64,
}

struct Acc {
    dense: Vec<u32>,
    hash: FxHashMap<u64, u32>,
    ew: Vec<f32>,
    eux: Vec<f32>,
    euy: Vec<f32>,
    internal: u64,
}

fn scan_chunk(c: &Ctx, p: u32, members: &[u32], k: usize, dense: bool) -> Acc {
    let mut a = Acc {
        dense: if dense { vec![0; k * k] } else { vec![] },
        hash: FxHashMap::default(),
        ew: vec![0.0; k],
        eux: vec![0.0; k],
        euy: vec![0.0; k],
        internal: 0,
    };
    let pc = c.cxy[p as usize];
    for &u in members {
        let ia = unsafe { c.childidx.get(u as usize) } as usize;
        for &v in c.csr.adj(u as usize) {
            let cv = c.cur[v as usize];
            if cv == p {
                a.internal += 1;
                let ib = unsafe { c.childidx.get(v as usize) } as usize;
                if ia < ib {
                    if dense {
                        a.dense[ia * k + ib] += 1;
                    } else {
                        *a.hash.entry((ia as u64) << 32 | ib as u64).or_insert(0) += 1;
                    }
                }
            } else {
                a.ew[ia] += 1.0;
                if c.prm.use_ext {
                    let t = c.cxy[cv as usize];
                    let dx = t[0] - pc[0];
                    let dy = t[1] - pc[1];
                    let d = (dx * dx + dy * dy).sqrt().max(1e-12);
                    a.eux[ia] += dx / d;
                    a.euy[ia] += dy / d;
                }
            }
        }
    }
    a
}
fn merge(mut a: Acc, b: Acc) -> Acc {
    for (x, y) in a.dense.iter_mut().zip(&b.dense) {
        *x += *y;
    }
    for (k, v) in b.hash {
        *a.hash.entry(k).or_insert(0) += v;
    }
    for i in 0..a.ew.len() {
        a.ew[i] += b.ew[i];
        a.eux[i] += b.eux[i];
        a.euy[i] += b.euy[i];
    }
    a.internal += b.internal;
    a
}

/// Dense force-directed layout of k circles in the unit disc. Returns (x, y, r) and area retention.
fn fr(k: usize, rad: &[f32], dense: &[u32], ew: &[f32], eux: &[f32], euy: &[f32], seed: u64, iters: usize, shrink: bool) -> Vec<[f32; 3]> {
    if k == 1 {
        return vec![[0.0, 0.0, rad[0]]];
    }
    let mut wdeg = vec![0f32; k];
    for i in 0..k {
        for j in i + 1..k {
            let w = dense[i * k + j] as f32;
            wdeg[i] += w;
            wdeg[j] += w;
        }
    }
    let mut wn = vec![0f32; k * k];
    for i in 0..k {
        for j in i + 1..k {
            let w = dense[i * k + j] as f32;
            if w > 0.0 {
                wn[i * k + j] = w / ((wdeg[i] + ew[i]) * (wdeg[j] + ew[j])).sqrt();
            }
        }
    }
    let mut ax = vec![0f32; k];
    let mut ay = vec![0f32; k];
    let mut aw = vec![0f32; k];
    for i in 0..k {
        if ew[i] > 0.0 {
            ax[i] = eux[i] / ew[i];
            ay[i] = euy[i] / ew[i];
            aw[i] = ew[i] / (wdeg[i] + ew[i]);
        }
    }
    let mut s = seed;
    let mut px = vec![0f32; k];
    let mut py = vec![0f32; k];
    for i in 0..k {
        let a = rnd01(&mut s) * std::f32::consts::TAU;
        let r = rnd01(&mut s).sqrt() * 0.5;
        px[i] = r * a.cos() + 0.4 * aw[i] * ax[i];
        py[i] = r * a.sin() + 0.4 * aw[i] * ay[i];
    }
    let kc = 0.9 * (std::f32::consts::PI / k as f32).sqrt();
    let kc2 = kc * kc;
    let grav = k as f32 * kc2 * 1.4;
    let eps = 0.05 * kc;
    const ATT: f32 = 4.0;
    const EXT: f32 = 2.5;
    let mut dx = vec![0f32; k];
    let mut dy = vec![0f32; k];
    for it in 0..iters {
        let f = 1.0 - it as f32 / iters as f32;
        let temp = 0.25 * f * f + 0.002;
        dx.iter_mut().for_each(|v| *v = 0.0);
        dy.iter_mut().for_each(|v| *v = 0.0);
        for i in 0..k {
            let (xi, yi, ri) = (px[i], py[i], rad[i]);
            let (mut fx, mut fy) = (0f32, 0f32);
            let wrow = &wn[i * k..(i + 1) * k];
            for j in i + 1..k {
                let ddx = xi - px[j];
                let ddy = yi - py[j];
                let d = (ddx * ddx + ddy * ddy + 1e-12).sqrt();
                let ds = d - ri - rad[j];
                let f = if ds > eps { kc2 / ds - wrow[j] * ATT * ds * ds / kc } else { kc2 / eps };
                let f = f / d;
                fx += ddx * f;
                fy += ddy * f;
                dx[j] -= ddx * f;
                dy[j] -= ddy * f;
            }
            dx[i] += fx;
            dy[i] += fy;
        }
        for i in 0..k {
            let fx = dx[i] - grav * px[i] + EXT * aw[i] * (ax[i] - px[i]);
            let fy = dy[i] - grav * py[i] + EXT * aw[i] * (ay[i] - py[i]);
            let len = (fx * fx + fy * fy).sqrt().max(1e-12);
            let st = len.min(temp) / len;
            px[i] += fx * st;
            py[i] += fy * st;
            let lim = (1.0 - rad[i]).max(0.0);
            let d = (px[i] * px[i] + py[i] * py[i]).sqrt();
            if d > lim {
                px[i] *= lim / d;
                py[i] *= lim / d;
            }
        }
    }
    // expand to use the parent disc
    let mut m = 0f32;
    for i in 0..k {
        m = m.max((px[i] * px[i] + py[i] * py[i]).sqrt() + rad[i]);
    }
    if m > 1e-6 && m < 1.0 {
        // scale centres so that the outermost circle touches the boundary
        let mut lo = 1.0f32;
        let mut hi = 1.6f32;
        for _ in 0..12 {
            let mid = 0.5 * (lo + hi);
            let ok = (0..k).all(|i| (px[i] * px[i] + py[i] * py[i]).sqrt() * mid + rad[i] <= 1.0);
            if ok { lo = mid } else { hi = mid }
        }
        for i in 0..k {
            px[i] *= lo;
            py[i] *= lo;
        }
    }
    // overlap removal sweeps
    let sweeps = if shrink { 40 } else { 12 };
    for _ in 0..sweeps {
        let mut any = false;
        for i in 0..k {
            for j in i + 1..k {
                let ddx = px[i] - px[j];
                let ddy = py[i] - py[j];
                let need = (rad[i] + rad[j]) * 1.04;
                let d2 = ddx * ddx + ddy * ddy;
                if d2 < need * need {
                    any = true;
                    let d = d2.sqrt().max(1e-9);
                    let push = need - d;
                    let (mi, mj) = (rad[i] * rad[i], rad[j] * rad[j]);
                    let (ux, uy) = if d > 1e-8 { (ddx / d, ddy / d) } else { (1.0, 0.0) };
                    let fi = push * mj / (mi + mj);
                    let fj = push * mi / (mi + mj);
                    px[i] += ux * fi;
                    py[i] += uy * fi;
                    px[j] -= ux * fj;
                    py[j] -= uy * fj;
                }
            }
        }
        for i in 0..k {
            let lim = (1.0 - rad[i]).max(0.0);
            let d = (px[i] * px[i] + py[i] * py[i]).sqrt();
            if d > lim {
                px[i] *= lim / d;
                py[i] *= lim / d;
            }
        }
        if !any {
            break;
        }
    }
    let mut out = Vec::with_capacity(k);
    for i in 0..k {
        let mut r = rad[i];
        if shrink {
            let d0 = (px[i] * px[i] + py[i] * py[i]).sqrt();
            r = r.min(1.0 - d0);
            for j in 0..k {
                if j != i {
                    let ddx = px[i] - px[j];
                    let ddy = py[i] - py[j];
                    let d = (ddx * ddx + ddy * ddy).sqrt();
                    r = r.min(d * rad[i] / (rad[i] + rad[j]));
                }
            }
            r = (r * 0.985).max(rad[i] * 0.05);
        }
        out.push([px[i], py[i], r]);
    }
    out
}

fn radii(sizes: &[f32], phi: f32) -> Vec<f32> {
    let k = sizes.len();
    if k == 1 {
        return vec![0.93];
    }
    let s: f64 = sizes.iter().map(|&x| x as f64).sum();
    let mut r: Vec<f32> = sizes.iter().map(|&x| (phi as f64 * x as f64 / s).sqrt() as f32).collect();
    let (mut m1, mut m2) = (0f32, 0f32);
    for &x in &r {
        if x > m1 {
            m2 = m1;
            m1 = x;
        } else if x > m2 {
            m2 = x;
        }
    }
    let f = 1f32.min(0.9 / m1).min(0.97 / (m1 + m2));
    if f < 1.0 {
        r.iter_mut().for_each(|x| *x *= f);
    }
    r
}

/// Size-constrained label propagation on the sibling quotient graph -> cluster id per child.
fn group(k: usize, sizes: &[f32], ent: &[(u64, u32)], gt: usize) -> (Vec<u32>, usize) {
    let mut off = vec![0u32; k + 1];
    for &(key, _) in ent {
        off[(key >> 32) as usize + 1] += 1;
        off[(key & 0xffff_ffff) as usize + 1] += 1;
    }
    for i in 0..k {
        off[i + 1] += off[i];
    }
    let mut fill = off.clone();
    let mut nb = vec![(0u32, 0f32); ent.len() * 2];
    for &(key, w) in ent {
        let a = (key >> 32) as usize;
        let b = (key & 0xffff_ffff) as usize;
        nb[fill[a] as usize] = (b as u32, w as f32);
        fill[a] += 1;
        nb[fill[b] as usize] = (a as u32, w as f32);
        fill[b] += 1;
    }
    let total: f64 = sizes.iter().map(|&x| x as f64).sum();
    let cap_cnt = ((2 * k) / gt).max(2) as u32;
    let cap_size = (2.0 * total / gt as f64) as f32;
    let mut label: Vec<u32> = (0..k as u32).collect();
    let mut cnt = vec![1u32; k];
    let mut csz = sizes.to_vec();
    let mut ord: Vec<u32> = (0..k as u32).collect();
    ord.sort_by(|&a, &b| sizes[a as usize].partial_cmp(&sizes[b as usize]).unwrap().then(a.cmp(&b)));
    let mut wacc = vec![0f32; k];
    let mut touched: Vec<u32> = vec![];
    for _round in 0..5 {
        let mut moved = 0;
        for &i in &ord {
            let i = i as usize;
            let li = label[i];
            touched.clear();
            for &(v, w) in &nb[off[i] as usize..off[i + 1] as usize] {
                let l = label[v as usize];
                if wacc[l as usize] == 0.0 {
                    touched.push(l);
                }
                wacc[l as usize] += w;
            }
            let mut best = li;
            let mut bw = wacc[li as usize];
            for &l in &touched {
                if l != li && wacc[l as usize] > bw && cnt[l as usize] < cap_cnt && csz[l as usize] + sizes[i] <= cap_size {
                    best = l;
                    bw = wacc[l as usize];
                }
            }
            for &l in &touched {
                wacc[l as usize] = 0.0;
            }
            if best != li {
                cnt[li as usize] -= 1;
                csz[li as usize] -= sizes[i];
                cnt[best as usize] += 1;
                csz[best as usize] += sizes[i];
                label[i] = best;
                moved += 1;
            }
        }
        if moved == 0 {
            break;
        }
    }
    // merge leftover small clusters into bins, keyed by their strongest outside neighbour cluster
    let is_small = |l: usize, cnt: &[u32], csz: &[f32]| cnt[l] > 0 && cnt[l] <= cap_cnt / 4 && csz[l] < cap_size / 4.0;
    let mut bestw = vec![0f32; k];
    let mut bestn = vec![u32::MAX; k];
    for i in 0..k {
        let li = label[i] as usize;
        if is_small(li, &cnt, &csz) {
            for &(v, w) in &nb[off[i] as usize..off[i + 1] as usize] {
                let l2 = label[v as usize];
                if l2 as usize != li && w > bestw[li] {
                    bestw[li] = w;
                    bestn[li] = l2;
                }
            }
        }
    }
    let mut small: Vec<u32> = (0..k as u32).filter(|&l| is_small(l as usize, &cnt, &csz)).collect();
    small.sort_by_key(|&l| (bestn[l as usize], l));
    let mut map: Vec<u32> = (0..k as u32).collect();
    let (mut bin, mut bc, mut bs) = (u32::MAX, 0u32, 0f32);
    for &l in &small {
        let l = l as usize;
        if bin == u32::MAX || bc + cnt[l] > cap_cnt || bs + csz[l] > cap_size {
            bin = l as u32;
            bc = 0;
            bs = 0.0;
        }
        map[l] = bin;
        bc += cnt[l];
        bs += csz[l];
    }
    let mut compact = vec![u32::MAX; k];
    let mut g = 0u32;
    for i in 0..k {
        let l = map[label[i] as usize] as usize;
        if compact[l] == u32::MAX {
            compact[l] = g;
            g += 1;
        }
        label[i] = compact[l];
    }
    (label, g as usize)
}

enum Out {
    Direct(Vec<[f32; 3]>),
    Leaf(f32),
    Group { clusters: Vec<Vec<u32>>, lay: Vec<[f32; 3]>, new_order: Vec<u32> },
}
struct Res {
    tid: u32,
    internal: u64,
    out: Out,
    area: (f64, f64),
}

fn process(c: &Ctx, p: u32) -> Res {
    let t = c.tn[p as usize];
    let kids = &c.kids[p as usize];
    let leafp = kids.is_empty();
    let members = &c.order[t.start as usize..t.end as usize];
    let k = if leafp { members.len() } else { kids.len() };
    let dense = k <= c.prm.kmax;
    let t0 = Instant::now();
    let acc = if members.len() > 300_000 {
        // fixed chunking + ordered sequential merge: float sums must not depend on the thread count
        let parts: Vec<Acc> = members.par_chunks(50_000).map(|ch| scan_chunk(c, p, ch, k, dense)).collect();
        parts.into_iter().reduce(merge).unwrap()
    } else {
        scan_chunk(c, p, members, k, dense)
    };
    c.t_scan.fetch_add(t0.elapsed().as_nanos() as u64, Ordering::Relaxed);
    let seed = c.prm.seed ^ (p as u64).wrapping_mul(0x9E3779B97F4A7C15) ^ ((t.start as u64) << 20);
    let sizes: Vec<f32> = if leafp { vec![1.0; k] } else { kids.iter().map(|&q| (c.tn[q as usize].end - c.tn[q as usize].start) as f32).collect() };
    if dense {
        let t1 = Instant::now();
        let rad = radii(&sizes, if leafp { c.prm.phi_leaf } else { c.prm.phi });
        let lay = fr(k, &rad, &acc.dense, &acc.ew, &acc.eux, &acc.euy, seed, c.prm.iters, !leafp);
        c.t_fr.fetch_add(t1.elapsed().as_nanos() as u64, Ordering::Relaxed);
        let a0: f64 = rad.iter().map(|&r| (r * r) as f64).sum();
        let a1: f64 = lay.iter().map(|l| (l[2] * l[2]) as f64).sum();
        if leafp {
            for (i, &u) in members.iter().enumerate() {
                unsafe { c.pos.set(u as usize, [t.x + t.r * lay[i][0], t.y + t.r * lay[i][1]]) };
            }
            return Res { tid: p, internal: acc.internal, out: Out::Leaf(rad[0]), area: (0.0, 0.0) };
        }
        return Res { tid: p, internal: acc.internal, out: Out::Direct(lay), area: (a0, a1) };
    }
    // too many children: derive an intermediate level
    let t1 = Instant::now();
    let mut ent: Vec<(u64, u32)> = acc.hash.iter().map(|(&a, &b)| (a, b)).collect();
    ent.sort_unstable();
    let gt = ((k as f64).sqrt().ceil() as usize).clamp(4, (c.prm.kmax / 6).max(4));
    let (label, g) = group(k, &sizes, &ent, gt);
    let mut clusters: Vec<Vec<u32>> = vec![vec![]; g];
    for i in 0..k {
        clusters[label[i] as usize].push(i as u32);
    }
    let mut gs = vec![0f32; g];
    let mut gew = vec![0f32; g];
    let mut gux = vec![0f32; g];
    let mut guy = vec![0f32; g];
    for i in 0..k {
        let l = label[i] as usize;
        gs[l] += sizes[i];
        gew[l] += acc.ew[i];
        gux[l] += acc.eux[i];
        guy[l] += acc.euy[i];
    }
    let mut d = vec![0u32; g * g];
    for &(key, w) in &ent {
        let a = label[(key >> 32) as usize] as usize;
        let b = label[(key & 0xffff_ffff) as usize] as usize;
        if a != b {
            d[a.min(b) * g + a.max(b)] += w;
        }
    }
    c.t_group.fetch_add(t1.elapsed().as_nanos() as u64, Ordering::Relaxed);
    let t2 = Instant::now();
    let rad = radii(&gs, c.prm.phi);
    let lay = fr(g, &rad, &d, &gew, &gux, &guy, seed, c.prm.iters, true);
    c.t_fr.fetch_add(t2.elapsed().as_nanos() as u64, Ordering::Relaxed);
    let a0: f64 = rad.iter().map(|&r| (r * r) as f64).sum();
    let a1: f64 = lay.iter().map(|l| (l[2] * l[2]) as f64).sum();
    let mut new_order = Vec::with_capacity(members.len());
    for cl in &clusters {
        for &ci in cl {
            if leafp {
                new_order.push(members[ci as usize]);
            } else {
                let q = c.tn[kids[ci as usize] as usize];
                new_order.extend_from_slice(&c.order[q.start as usize..q.end as usize]);
            }
        }
    }
    Res { tid: p, internal: acc.internal, out: Out::Group { clusters, lay, new_order }, area: (a0, a1) }
}

pub struct Hier {
    pub levels: Vec<usize>,
    pub l0: Vec<u32>,
    pub parents: Vec<Vec<u32>>,
    /// m1-05: per level, region kind (1 community, 2 aggregate, 3 subdivision) and, for an
    /// aggregate, the number of communities it holds; empty when the hierarchy has no kinds
    pub kinds: Vec<Vec<u8>>,
    pub counts: Vec<Vec<u32>>,
}
pub fn load_hier(dir: &str) -> Hier {
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(format!("{dir}/hier.json")).unwrap()).unwrap();
    let levels: Vec<usize> = j["levels"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as usize).collect();
    let l0: Vec<u32> = read_vec(&format!("{dir}/level_0.u32"));
    let mut parents = vec![];
    for k in 1..levels.len() {
        let p: Vec<u32> = read_vec(&format!("{dir}/level_{k}.u32"));
        assert_eq!(p.len(), levels[k - 1]);
        parents.push(p);
    }
    let (mut kinds, mut counts) = (vec![], vec![]);
    if std::path::Path::new(&format!("{dir}/level_0.kind.u8")).exists() {
        for k in 0..levels.len() {
            let kk: Vec<u8> = read_vec(&format!("{dir}/level_{k}.kind.u8"));
            let cc: Vec<u32> = read_vec(&format!("{dir}/level_{k}.count.u32"));
            assert_eq!((kk.len(), cc.len()), (levels[k], levels[k]));
            kinds.push(kk);
            counts.push(cc);
        }
    }
    Hier { levels, l0, parents, kinds, counts }
}

fn build_tree(n: usize, h: &Hier) -> (Vec<TNode>, Vec<Vec<u32>>, Vec<u32>, Vec<u32>) {
    let nl = h.levels.len();
    let top = nl - 1;
    let mut base = vec![0usize; nl];
    base[top] = 1;
    for k in (0..top).rev() {
        base[k] = base[k + 1] + h.levels[k + 1];
    }
    let ntn = base[0] + h.levels[0];
    let mut tn = vec![TNode::default(); ntn];
    tn[0].level = 127;
    // sizes
    let mut size: Vec<Vec<u32>> = h.levels.iter().map(|&c| vec![0u32; c]).collect();
    for &c in &h.l0 {
        size[0][c as usize] += 1;
    }
    for k in 0..top {
        for c in 0..h.levels[k] {
            let p = h.parents[k][c] as usize;
            size[k + 1][p] += size[k][c];
        }
    }
    // dfs ranks, top-down
    let mut rank: Vec<u32> = (0..h.levels[top] as u32).collect();
    for k in (0..top).rev() {
        let mut ids: Vec<u32> = (0..h.levels[k] as u32).collect();
        ids.sort_by_key(|&c| (rank[h.parents[k][c as usize] as usize], c));
        let mut r = vec![0u32; h.levels[k]];
        for (i, &c) in ids.iter().enumerate() {
            r[c as usize] = i as u32;
        }
        rank = r;
    }
    // node order: counting sort by rank of level-0 community
    let c0 = h.levels[0];
    let mut start = vec![0u32; c0 + 1];
    for c in 0..c0 {
        start[rank[c] as usize + 1] = size[0][c];
    }
    for i in 0..c0 {
        start[i + 1] += start[i];
    }
    let mut fill = start.clone();
    let mut order = vec![0u32; n];
    for u in 0..n {
        let r = rank[h.l0[u] as usize] as usize;
        order[fill[r] as usize] = u as u32;
        fill[r] += 1;
    }
    for c in 0..c0 {
        let t = &mut tn[base[0] + c];
        t.start = start[rank[c] as usize];
        t.end = t.start + size[0][c];
    }
    let mut kids: Vec<Vec<u32>> = vec![vec![]; ntn];
    for k in 0..nl {
        for c in 0..h.levels[k] {
            let id = base[k] + c;
            tn[id].level = k as i8;
            tn[id].comm = c as u32;
            if !h.kinds.is_empty() {
                match h.kinds[k][c] {
                    2 => {
                        tn[id].flags |= F_AGG;
                        tn[id].comm = h.counts[k][c]; // an aggregate's comm is its community count
                    }
                    3 => tn[id].flags |= F_SUBDIV,
                    _ => {}
                }
            }
            if size[k][c] == 0 {
                tn[id].flags |= F_DEAD;
                continue;
            }
            let par = if k == top { 0 } else { base[k + 1] + h.parents[k][c] as usize };
            tn[id].parent = par as u32;
            kids[par].push(id as u32);
        }
    }
    // ranges of upper levels (bottom-up; ids of finer levels are larger)
    for id in (1..base[0]).rev() {
        if kids[id].is_empty() {
            continue;
        }
        let s = kids[id].iter().map(|&q| tn[q as usize].start).min().unwrap();
        let e = kids[id].iter().map(|&q| tn[q as usize].end).max().unwrap();
        tn[id].start = s;
        tn[id].end = e;
    }
    tn[0].start = 0;
    tn[0].end = n as u32;
    // sort kids by range start so that ranges stay contiguous and deterministic
    for id in 0..ntn {
        let mut kk = std::mem::take(&mut kids[id]);
        kk.sort_by_key(|&q| tn[q as usize].start);
        kids[id] = kk;
    }
    // collapse single-child chains (parent adopts the identity of the finer community)
    for id in 0..ntn {
        if tn[id].flags & F_DEAD != 0 {
            continue;
        }
        while kids[id].len() == 1 {
            let c = kids[id][0] as usize;
            tn[c].flags |= F_DEAD;
            if id != 0 {
                tn[id].level = tn[c].level;
                tn[id].comm = tn[c].comm;
                tn[id].flags = (tn[id].flags & !(F_AGG | F_SUBDIV)) | (tn[c].flags & (F_AGG | F_SUBDIV));
            }
            kids[id] = std::mem::take(&mut kids[c]);
            for &q in &kids[id] {
                tn[q as usize].parent = id as u32;
            }
        }
    }
    // colour rank of each node's top-level community (0 = largest)
    let mut tops: Vec<u32> = (0..h.levels[top] as u32).collect();
    tops.sort_by_key(|&c| (std::cmp::Reverse(size[top][c as usize]), c));
    let mut trank = vec![0u32; h.levels[top]];
    for (i, &c) in tops.iter().enumerate() {
        trank[c as usize] = i as u32;
    }
    let mut l0top: Vec<u32> = (0..c0 as u32).collect();
    for k in 0..top {
        for c in 0..c0 {
            l0top[c] = h.parents[k][l0top[c] as usize];
        }
    }
    let topc: Vec<u32> = h.l0.iter().map(|&c| trank[l0top[c as usize] as usize]).collect();
    (tn, kids, order, topc)
}

pub fn run() {
    let a = args();
    let graph = args_s(&a, "graph");
    let hier = args_s(&a, "hier");
    let out = args_s(&a, "out");
    let prm = Params {
        kmax: argf(&a, "kmax", 384.0) as usize,
        phi: argf(&a, "phi", 0.62) as f32,
        phi_leaf: argf(&a, "phi-leaf", 0.5) as f32,
        seed: argf(&a, "seed", 1.0) as u64,
        use_ext: !a.contains_key("no-ext"),
        iters: argf(&a, "iters", 50.0) as usize,
    };
    std::fs::create_dir_all(&out).unwrap();
    let t_all = Instant::now();
    let csr = Csr::load(&graph);
    let n = csr.n;
    let h = load_hier(&hier);
    let t_load = t_all.elapsed().as_secs_f64();
    let t_lay = Instant::now();
    let (mut tn, mut kids, mut order, topc) = build_tree(n, &h);
    let t_tree = t_lay.elapsed().as_secs_f64();
    tn[0].x = 0.5;
    tn[0].y = 0.5;
    tn[0].r = 0.5;
    let mut cxy: Vec<[f32; 2]> = tn.iter().map(|t| [t.x, t.y]).collect();
    let mut cur = vec![0u32; n];
    let mut childidx = vec![0u32; n];
    let mut pos = vec![[0f32; 2]; n];
    let mut frontier: Vec<u32> = vec![0];
    let (t_scan, t_fr, t_group) = (AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0));
    let mut passes = vec![];
    let mut area = (0f64, 0f64);
    let mut n_virtual = 0usize;
    let mut depth = 0u8;
    while !frontier.is_empty() {
        let tp = Instant::now();
        // pass A: child index of every member
        {
            let cp = SyncPtr(childidx.as_mut_ptr());
            let (tn, kids, order) = (&tn, &kids, &order);
            frontier.par_iter().for_each(|&p| {
                let t = tn[p as usize];
                let kk = &kids[p as usize];
                if kk.is_empty() {
                    for (i, &u) in order[t.start as usize..t.end as usize].iter().enumerate() {
                        unsafe { cp.set(u as usize, i as u32) };
                    }
                } else {
                    for (ci, &q) in kk.iter().enumerate() {
                        let qt = tn[q as usize];
                        for &u in &order[qt.start as usize..qt.end as usize] {
                            unsafe { cp.set(u as usize, ci as u32) };
                        }
                    }
                }
            });
        }
        // pass B: per-parent layout
        let res: Vec<Res> = {
            let ctx = Ctx {
                csr: &csr,
                tn: &tn,
                kids: &kids,
                order: &order,
                cur: &cur,
                childidx: SyncPtr(childidx.as_mut_ptr()),
                cxy: &cxy,
                pos: SyncPtr(pos.as_mut_ptr()),
                prm: &prm,
                t_scan: &t_scan,
                t_fr: &t_fr,
                t_group: &t_group,
            };
            frontier.par_iter().map(|&p| process(&ctx, p)).collect()
        };
        // pass C: apply
        let mut next: Vec<u32> = vec![];
        for r in res {
            let p = r.tid as usize;
            let pt = tn[p];
            tn[p].internal = r.internal / 2;
            area.0 += r.area.0;
            area.1 += r.area.1;
            match r.out {
                Out::Leaf(nr) => {
                    tn[p].flags |= F_LEAFP;
                    tn[p].node_r = nr * pt.r;
                    tn[p].nkids = pt.end - pt.start;
                }
                Out::Direct(lay) => {
                    tn[p].nkids = lay.len() as u32;
                    for (i, &q) in kids[p].iter().enumerate() {
                        let q = q as usize;
                        tn[q].x = pt.x + pt.r * lay[i][0];
                        tn[q].y = pt.y + pt.r * lay[i][1];
                        tn[q].r = pt.r * lay[i][2];
                        tn[q].depth = pt.depth + 1;
                        cxy[q] = [tn[q].x, tn[q].y];
                        for &u in &order[tn[q].start as usize..tn[q].end as usize] {
                            cur[u as usize] = q as u32;
                        }
                        next.push(q as u32);
                    }
                }
                Out::Group { clusters, lay, new_order } => {
                    let leafp = kids[p].is_empty();
                    order[pt.start as usize..pt.end as usize].copy_from_slice(&new_order);
                    let mut offp = pt.start;
                    let old = std::mem::take(&mut kids[p]);
                    let mut newk = vec![];
                    for (ci, cl) in clusters.iter().enumerate() {
                        let s = offp;
                        let ent: usize;
                        if leafp {
                            offp += cl.len() as u32;
                            ent = tn.len();
                            tn.push(TNode { parent: p as u32, start: s, end: offp, comm: u32::MAX, level: -1, flags: F_VIRTUAL, ..Default::default() });
                            kids.push(vec![]);
                            cxy.push([0.0; 2]);
                            n_virtual += 1;
                        } else {
                            for &ci2 in cl {
                                let q = old[ci2 as usize] as usize;
                                let len = tn[q].end - tn[q].start;
                                let delta = offp.wrapping_sub(tn[q].start);
                                if delta != 0 {
                                    // the whole subtree's member ranges move with the block
                                    let mut stack = vec![q as u32];
                                    while let Some(x) = stack.pop() {
                                        let x = x as usize;
                                        tn[x].start = tn[x].start.wrapping_add(delta);
                                        tn[x].end = tn[x].end.wrapping_add(delta);
                                        stack.extend_from_slice(&kids[x]);
                                    }
                                }
                                offp += len;
                            }
                            if cl.len() == 1 {
                                ent = old[cl[0] as usize] as usize;
                            } else {
                                ent = tn.len();
                                tn.push(TNode { parent: p as u32, start: s, end: offp, comm: u32::MAX, level: -1, flags: F_VIRTUAL, ..Default::default() });
                                let kk: Vec<u32> = cl.iter().map(|&ci2| old[ci2 as usize]).collect();
                                for &q in &kk {
                                    tn[q as usize].parent = ent as u32;
                                }
                                kids.push(kk);
                                cxy.push([0.0; 2]);
                                n_virtual += 1;
                            }
                        }
                        tn[ent].x = pt.x + pt.r * lay[ci][0];
                        tn[ent].y = pt.y + pt.r * lay[ci][1];
                        tn[ent].r = pt.r * lay[ci][2];
                        tn[ent].depth = pt.depth + 1;
                        cxy[ent] = [tn[ent].x, tn[ent].y];
                        for &u in &order[tn[ent].start as usize..tn[ent].end as usize] {
                            cur[u as usize] = ent as u32;
                        }
                        newk.push(ent as u32);
                        next.push(ent as u32);
                    }
                    tn[p].nkids = newk.len() as u32;
                    kids[p] = newk;
                }
            }
        }
        passes.push((depth, frontier.len(), tp.elapsed().as_secs_f64()));
        eprintln!("depth {depth}: {} parents, {:.2}s", frontier.len(), tp.elapsed().as_secs_f64());
        frontier = next;
        depth += 1;
    }
    // exact volumes
    {
        let csr = &csr;
        let order = &order;
        tn.par_iter_mut().for_each(|t| {
            if t.flags & F_DEAD == 0 && t.depth >= 2 {
                t.volume = order[t.start as usize..t.end as usize].iter().map(|&u| csr.deg(u as usize) as u64).sum();
            }
        });
        // shallow nodes via prefix sums (large ranges)
        let mut pre = vec![0u64; n + 1];
        for i in 0..n {
            pre[i + 1] = pre[i] + csr.deg(order[i] as usize) as u64;
        }
        for t in tn.iter_mut() {
            if t.flags & F_DEAD == 0 && t.depth < 2 {
                t.volume = pre[t.end as usize] - pre[t.start as usize];
            }
        }
    }
    let lay_s = t_lay.elapsed().as_secs_f64();
    let tw = Instant::now();
    write_slice(&format!("{out}/pos.f32"), &pos);
    write_slice(&format!("{out}/order.u32"), &order);
    write_slice(&format!("{out}/tnodes.bin"), &tn);
    write_slice(&format!("{out}/leafparent.u32"), &cur);
    write_slice(&format!("{out}/top.u32"), &topc);
    let alive = tn.iter().filter(|t| t.flags & F_DEAD == 0).count();
    let maxdepth = tn.iter().map(|t| t.depth).max().unwrap();
    let leafps = tn.iter().filter(|t| t.flags & F_LEAFP != 0).count();
    let passes_j: Vec<String> = passes.iter().map(|p| format!("{{\"depth\":{},\"parents\":{},\"seconds\":{:.3}}}", p.0, p.1, p.2)).collect();
    let j = format!(
        "{{\"graph\":\"{graph}\",\"hier\":\"{hier}\",\"n\":{n},\"entries\":{},\"threads\":{},\"seed\":{},\"kmax\":{},\"phi\":{},\"iters\":{},\"use_ext\":{},\
\"load_s\":{t_load:.3},\"tree_s\":{t_tree:.3},\"layout_s\":{lay_s:.3},\"write_s\":{:.3},\"total_s\":{:.3},\
\"cpu_scan_s\":{:.3},\"cpu_fr_s\":{:.3},\"cpu_group_s\":{:.3},\"peak_rss_mb\":{:.0},\
\"tnodes_alive\":{alive},\"virtual\":{n_virtual},\"leaf_parents\":{leafps},\"max_depth\":{maxdepth},\"hier_levels\":{:?},\"area_retention\":{:.4},\"passes\":[{}]}}",
        csr.tgt.len(),
        rayon::current_num_threads(),
        prm.seed,
        prm.kmax,
        prm.phi,
        prm.iters,
        prm.use_ext,
        tw.elapsed().as_secs_f64(),
        t_all.elapsed().as_secs_f64(),
        t_scan.load(Ordering::Relaxed) as f64 / 1e9,
        t_fr.load(Ordering::Relaxed) as f64 / 1e9,
        t_group.load(Ordering::Relaxed) as f64 / 1e9,
        peak_rss_mb(),
        h.levels,
        area.1 / area.0.max(1e-9),
        passes_j.join(",")
    );
    std::fs::write(format!("{out}/layout.json"), &j).unwrap();
    println!("{j}");
}
