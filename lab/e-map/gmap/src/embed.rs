//! Baseline layouts for comparison.
//!  embed: FastRP-like random-projection embedding (d dims, q rounds of D^-1 A smoothing), PCA -> 2D.
//!  eval : edge-length statistics of a layout (world units, world is the unit square).
use crate::common::*;
use rayon::prelude::*;
use std::time::Instant;

pub fn run_embed() {
    let a = args();
    let csr = Csr::load(&args_s(&a, "graph"));
    let out = args_s(&a, "out");
    let d = argf(&a, "dim", 8.0) as usize;
    let q = argf(&a, "rounds", 4.0) as usize;
    let seed = argf(&a, "seed", 1.0) as u64;
    std::fs::create_dir_all(&out).unwrap();
    let n = csr.n;
    let t0 = Instant::now();
    let mut x = vec![0f32; n * d];
    x.par_chunks_mut(d).enumerate().for_each(|(u, r)| {
        let mut s = seed ^ (u as u64).wrapping_mul(0x9E3779B97F4A7C15);
        for v in r.iter_mut() {
            *v = if splitmix(&mut s) & 1 == 0 { 1.0 } else { -1.0 };
        }
    });
    let mut e = vec![0f32; n * d];
    let weights = [0.0f32, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0];
    for it in 0..q {
        let mut y = vec![0f32; n * d];
        y.par_chunks_mut(d).enumerate().for_each(|(u, r)| {
            let adj = csr.adj(u);
            for &v in adj {
                let xv = &x[v as usize * d..v as usize * d + d];
                for k in 0..d {
                    r[k] += xv[k];
                }
            }
            let inv = 1.0 / (adj.len().max(1) as f32);
            for k in 0..d {
                r[k] *= inv;
            }
        });
        // centre + normalise columns
        for k in 0..d {
            let mean: f64 = (0..n).into_par_iter().map(|u| y[u * d + k] as f64).sum::<f64>() / n as f64;
            let var: f64 = (0..n).into_par_iter().map(|u| (y[u * d + k] as f64 - mean).powi(2)).sum::<f64>() / n as f64;
            let sd = var.sqrt().max(1e-12);
            y.par_chunks_mut(d).for_each(|r| r[k] = ((r[k] as f64 - mean) / sd) as f32);
        }
        let w = weights[it.min(7)];
        if w > 0.0 {
            e.par_iter_mut().zip(y.par_iter()).for_each(|(a, b)| *a += w * *b);
        }
        x = y;
    }
    // PCA: covariance d x d, top-2 eigenvectors by power iteration with deflation
    let mut cov = vec![0f64; d * d];
    for i in 0..d {
        for j in i..d {
            let s: f64 = (0..n).into_par_iter().map(|u| e[u * d + i] as f64 * e[u * d + j] as f64).sum();
            cov[i * d + j] = s;
            cov[j * d + i] = s;
        }
    }
    let mut vecs: Vec<Vec<f64>> = vec![];
    for c in 0..2 {
        let mut v: Vec<f64> = (0..d).map(|i| 1.0 + 0.37 * ((i + c * 3) as f64).sin()).collect();
        for _ in 0..500 {
            let mut w = vec![0f64; d];
            for i in 0..d {
                for j in 0..d {
                    w[i] += cov[i * d + j] * v[j];
                }
            }
            for p in &vecs {
                let dot: f64 = w.iter().zip(p).map(|(a, b)| a * b).sum();
                for i in 0..d {
                    w[i] -= dot * p[i];
                }
            }
            let nrm = w.iter().map(|a| a * a).sum::<f64>().sqrt();
            v = w.iter().map(|a| a / nrm).collect();
        }
        vecs.push(v);
    }
    let mut pos: Vec<[f32; 2]> = (0..n)
        .into_par_iter()
        .map(|u| {
            let r = &e[u * d..u * d + d];
            let px: f64 = (0..d).map(|i| r[i] as f64 * vecs[0][i]).sum();
            let py: f64 = (0..d).map(|i| r[i] as f64 * vecs[1][i]).sum();
            [px as f32, py as f32]
        })
        .collect();
    // robust rescale to the unit square (0.5 .. 99.5 percentile)
    for c in 0..2 {
        let mut v: Vec<f32> = pos.iter().step_by(37).map(|p| p[c]).collect();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let lo = v[v.len() / 200];
        let hi = v[v.len() - 1 - v.len() / 200];
        for p in pos.iter_mut() {
            p[c] = (0.03 + 0.94 * (p[c] - lo) / (hi - lo)).clamp(0.0, 1.0);
        }
    }
    let secs = t0.elapsed().as_secs_f64();
    write_slice(&format!("{out}/pos.f32"), &pos);
    if let Some(t) = a.get("top") {
        std::fs::copy(t, format!("{out}/top.u32")).unwrap();
    }
    let j = format!("{{\"method\":\"fastrp-pca\",\"n\":{n},\"dim\":{d},\"rounds\":{q},\"threads\":{},\"layout_s\":{secs:.3},\"peak_rss_mb\":{:.0}}}", rayon::current_num_threads(), peak_rss_mb());
    std::fs::write(format!("{out}/layout.json"), &j).unwrap();
    println!("{j}");
}

pub fn run_eval() {
    let a = args();
    let csr = Csr::load(&args_s(&a, "graph"));
    let lay = args_s(&a, "layout");
    let pos: Vec<[f32; 2]> = read_vec(&format!("{lay}/pos.f32"));
    let n = csr.n;
    const B: usize = 4000;
    // histogram of log10 length in [-7, 1)
    let bin = |d: f32| (((d.max(1e-7).log10() + 7.0) / 8.0) * B as f32).clamp(0.0, B as f32 - 1.0) as usize;
    let (hist, sum, cnt) = (0..n)
        .into_par_iter()
        .fold(
            || (vec![0u64; B], 0f64, 0u64),
            |mut acc, u| {
                for &v in csr.adj(u) {
                    if (u as u32) < v {
                        let (p, q) = (pos[u], pos[v as usize]);
                        let d = ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt();
                        acc.0[bin(d)] += 1;
                        acc.1 += d as f64;
                        acc.2 += 1;
                    }
                }
                acc
            },
        )
        .reduce(
            || (vec![0u64; B], 0f64, 0u64),
            |mut x, y| {
                for i in 0..B {
                    x.0[i] += y.0[i];
                }
                (x.0, x.1 + y.1, x.2 + y.2)
            },
        );
    let pct = |p: f64| {
        let target = (cnt as f64 * p) as u64;
        let mut c = 0;
        for i in 0..B {
            c += hist[i];
            if c >= target {
                return 10f64.powf((i as f64 + 0.5) / B as f64 * 8.0 - 7.0);
            }
        }
        10.0
    };
    let below = |x: f32| hist[..bin(x)].iter().sum::<u64>() as f64 / cnt as f64;
    // random-pair baseline
    let mut s = 12345u64;
    let mut rs = 0f64;
    let m = 2_000_000;
    let mut rv: Vec<f32> = Vec::with_capacity(m);
    for _ in 0..m {
        let u = (splitmix(&mut s) % n as u64) as usize;
        let v = (splitmix(&mut s) % n as u64) as usize;
        let d = ((pos[u][0] - pos[v][0]).powi(2) + (pos[u][1] - pos[v][1]).powi(2)).sqrt();
        rs += d as f64;
        rv.push(d);
    }
    rv.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let j = format!(
        "{{\"layout\":\"{lay}\",\"edges\":{cnt},\"edge_len_mean\":{:.6},\"edge_len_p50\":{:.6},\"edge_len_p90\":{:.6},\"frac_lt_0.001\":{:.4},\"frac_lt_0.01\":{:.4},\"frac_lt_0.05\":{:.4},\"random_pair_mean\":{:.4},\"random_pair_p50\":{:.4},\"mean_ratio\":{:.4},\"median_ratio\":{:.5}}}",
        sum / cnt as f64,
        pct(0.5),
        pct(0.9),
        below(0.001),
        below(0.01),
        below(0.05),
        rs / m as f64,
        rv[m / 2],
        (sum / cnt as f64) / (rs / m as f64),
        pct(0.5) / rv[m / 2] as f64
    );
    if let Some(o) = a.get("json") {
        std::fs::write(o, &j).unwrap();
    }
    println!("{j}");
}
