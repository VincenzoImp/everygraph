//! Sibling placement by stress on the parent's quotient graph (m1-18, candidate (a) of
//! `docs/research/sota-maps.md` challenge 1). Replaces the random start and force loop of
//! `layout::fr` when `--solve stress` is given; the packing that follows is unchanged.
//!
//! Target distance: shortest path over the sibling quotient graph with edge length
//! r_i + r_j + gap * (1 + ln(a_max / a_ij)), a_ij = w_ij / sqrt(vol_i vol_j), so adjacent
//! strong pairs touch and every target is at least r_i + r_j. Start: classical MDS by power
//! iteration from a fixed vector, sign fixed so the largest child has x >= 0 and y >= 0.
//! Then stress SGD (Zheng, Pawar, Goodman 2019) with weights d^-2, a step schedule from
//! 1/w_min down to 0.1/w_max, and a pair order shuffled from the parent's seed.
//! Everything is sequential per parent, so the result does not depend on the thread count.
use crate::common::splitmix;

const ITERS: usize = 30;

/// m1-07 (b): where the external neighbours of each child sit, in the parent's unit-disc frame.
pub struct Anchors<'a> {
    /// mean position of the external neighbours' regions
    pub ax: &'a [f32],
    pub ay: &'a [f32],
    /// share of the child's edges that leave the parent
    pub aw: &'a [f32],
    pub beta: f32,
}

pub fn place(k: usize, rad: &[f32], dense: &[u32], vol: &[f32], seed: u64, anchors: Option<Anchors>) -> (Vec<f32>, Vec<f32>) {
    let mean_r = rad.iter().sum::<f32>() / k as f32;
    let gap = 0.15 * mean_r;
    let inf = f32::INFINITY;
    let mut d = vec![inf; k * k];
    let mut amax = 0f32;
    for i in 0..k {
        for j in i + 1..k {
            let w = dense[i * k + j] as f32;
            if w > 0.0 {
                amax = amax.max(w / (vol[i] * vol[j]).max(1e-12).sqrt());
            }
        }
    }
    for i in 0..k {
        d[i * k + i] = 0.0;
        for j in i + 1..k {
            let w = dense[i * k + j] as f32;
            if w > 0.0 {
                let a = w / (vol[i] * vol[j]).max(1e-12).sqrt();
                let l = rad[i] + rad[j] + gap * (1.0 + (amax / a).ln());
                d[i * k + j] = l;
                d[j * k + i] = l;
            }
        }
    }
    for m in 0..k {
        for i in 0..k {
            let dim = d[i * k + m];
            if dim == inf {
                continue;
            }
            for j in 0..k {
                let v = dim + d[m * k + j];
                if v < d[i * k + j] {
                    d[i * k + j] = v;
                }
            }
        }
    }
    // unreachable siblings (no edge path inside the parent): a little beyond the farthest pair
    let far = d.iter().filter(|x| x.is_finite()).fold(0f32, |a, &b| a.max(b));
    for i in 0..k {
        for j in 0..k {
            if !d[i * k + j].is_finite() {
                d[i * k + j] = 1.2 * far.max(rad[i] + rad[j] + gap) + rad[i] + rad[j];
            }
        }
    }
    let (mut x, mut y) = mds(k, &d, rad);
    sgd(k, &d, &mut x, &mut y, seed);
    if let Some(a) = anchors {
        anchor(k, &d, rad, &mut x, &mut y, &a, seed);
    }
    (x, y)
}

/// Fit the stress layout to the unit disc, turn it (rotation or mirror) to face the anchors,
/// then run stress epochs with a spring from each child to its anchor of strength
/// beta * aw_i, keeping every child inside the disc. The step decays from 1 to 0.1.
fn anchor(k: usize, d: &[f32], rad: &[f32], x: &mut [f32], y: &mut [f32], a: &Anchors, seed: u64) {
    const EPOCHS: usize = 20;
    let fit = (0..k).map(|i| (1.0 - rad[i]) / (x[i] * x[i] + y[i] * y[i]).sqrt().max(1e-9)).fold(f32::MAX, f32::min);
    x.iter_mut().chain(y.iter_mut()).for_each(|v| *v *= fit);
    crate::layout::rotate_to_outside(x, y, a.aw, a.ax, a.ay);
    let mut pairs: Vec<(u32, u32)> = (0..k as u32).flat_map(|i| (i + 1..k as u32).map(move |j| (i, j))).collect();
    let dmin2 = pairs.iter().map(|&(i, j)| d[i as usize * k + j as usize].powi(2)).fold(f32::MAX, f32::min);
    let lambda = 10f32.ln() / (EPOCHS - 1) as f32;
    let mut s = seed ^ 0xA2C4_0B5E;
    for t in 0..EPOCHS {
        let eta = (-lambda * t as f32).exp();
        for i in (1..pairs.len()).rev() {
            let j = (splitmix(&mut s) % (i as u64 + 1)) as usize;
            pairs.swap(i, j);
        }
        for &(i, j) in &pairs {
            let (i, j) = (i as usize, j as usize);
            let dij = d[i * k + j];
            let mu = (eta * dmin2 / (dij * dij)).min(1.0);
            let (dx, dy) = (x[i] - x[j], y[i] - y[j]);
            let len = (dx * dx + dy * dy).sqrt().max(1e-9);
            let r = mu * (len - dij) / (2.0 * len);
            x[i] -= r * dx;
            y[i] -= r * dy;
            x[j] += r * dx;
            y[j] += r * dy;
        }
        for i in 0..k {
            let m = (a.beta * a.aw[i] * eta).min(1.0);
            x[i] += m * (a.ax[i] - x[i]);
            y[i] += m * (a.ay[i] - y[i]);
            let lim = (1.0 - rad[i]).max(0.0);
            let l = (x[i] * x[i] + y[i] * y[i]).sqrt();
            if l > lim {
                x[i] *= lim / l;
                y[i] *= lim / l;
            }
        }
    }
}

/// Classical MDS: top two eigenvectors of B = -1/2 J D^2 J by power iteration with deflation.
fn mds(k: usize, d: &[f32], rad: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let mut b = vec![0f64; k * k];
    let mut row = vec![0f64; k];
    let mut all = 0f64;
    for i in 0..k {
        for j in 0..k {
            let v = (d[i * k + j] as f64).powi(2);
            b[i * k + j] = v;
            row[i] += v / k as f64;
        }
        all += row[i] / k as f64;
    }
    for i in 0..k {
        for j in 0..k {
            b[i * k + j] = -0.5 * (b[i * k + j] - row[i] - row[j] + all);
        }
    }
    let mut vecs: Vec<(Vec<f64>, f64)> = vec![];
    for e in 0..2 {
        let mut v: Vec<f64> = (0..k).map(|i| 1.0 + ((i * 7 + e * 3) % 11) as f64 / 11.0).collect();
        let mut lam = 0f64;
        for _ in 0..100 {
            let mut nv = vec![0f64; k];
            for i in 0..k {
                let r = &b[i * k..(i + 1) * k];
                nv[i] = r.iter().zip(&v).map(|(a, b)| a * b).sum();
            }
            for (u, _) in &vecs {
                let dot: f64 = nv.iter().zip(u).map(|(a, b)| a * b).sum();
                nv.iter_mut().zip(u).for_each(|(a, b)| *a -= dot * b);
            }
            let norm = nv.iter().map(|a| a * a).sum::<f64>().sqrt();
            if norm < 1e-18 {
                break;
            }
            lam = norm;
            v = nv.into_iter().map(|a| a / norm).collect();
        }
        vecs.push((v, lam));
    }
    let big = (0..k).max_by(|&a, &b| rad[a].total_cmp(&rad[b]).then(b.cmp(&a))).unwrap();
    let mut out = vecs.into_iter().map(|(v, lam)| {
        let s = lam.max(0.0).sqrt();
        let sign = if v[big] < 0.0 { -1.0 } else { 1.0 };
        v.into_iter().map(|a| (sign * s * a) as f32).collect::<Vec<f32>>()
    });
    (out.next().unwrap(), out.next().unwrap())
}

fn sgd(k: usize, d: &[f32], x: &mut [f32], y: &mut [f32], seed: u64) {
    let mut pairs: Vec<(u32, u32)> = (0..k as u32).flat_map(|i| (i + 1..k as u32).map(move |j| (i, j))).collect();
    if pairs.is_empty() {
        return;
    }
    let (mut dmin, mut dmax) = (f32::MAX, 0f32);
    for &(i, j) in &pairs {
        let v = d[i as usize * k + j as usize];
        dmin = dmin.min(v);
        dmax = dmax.max(v);
    }
    let (eta_max, eta_min) = ((dmax * dmax) as f64, 0.1 * (dmin * dmin) as f64);
    let lambda = (eta_max / eta_min).ln() / (ITERS - 1).max(1) as f64;
    let mut s = seed ^ 0x5EED_57E55;
    for t in 0..ITERS {
        for i in (1..pairs.len()).rev() {
            let j = (splitmix(&mut s) % (i as u64 + 1)) as usize;
            pairs.swap(i, j);
        }
        let eta = eta_max * (-lambda * t as f64).exp();
        for &(i, j) in &pairs {
            let (i, j) = (i as usize, j as usize);
            let dij = d[i * k + j];
            let mu = ((eta as f32) / (dij * dij)).min(1.0);
            let (dx, dy) = (x[i] - x[j], y[i] - y[j]);
            let len = (dx * dx + dy * dy).sqrt().max(1e-9);
            let r = mu * (len - dij) / (2.0 * len);
            x[i] -= r * dx;
            y[i] -= r * dy;
            x[j] += r * dx;
            y[j] += r * dy;
        }
    }
}
