//! CPU rasterizer for evaluation images: node density coloured by top-level community,
//! optional edges (sampled to a cap) and community outlines. Writes binary PPM.
use crate::common::*;
use std::io::Write;

pub fn color(rank: u32) -> [f32; 3] {
    let h = (rank as f32 * 0.618_034).fract() * 6.0;
    let s = 0.55 + 0.15 * ((rank % 3) as f32);
    let v = 1.0;
    let c = v * s;
    let x = c * (1.0 - ((h % 2.0) - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [r + m, g + m, b + m]
}

fn line(buf: &mut [f32], size: usize, mut x0: f32, mut y0: f32, mut x1: f32, mut y1: f32, w: f32) {
    // Liang-Barsky clip to [0, size-1]
    let mx = size as f32 - 1.001;
    let (dx, dy) = (x1 - x0, y1 - y0);
    let (mut t0, mut t1) = (0f32, 1f32);
    for (p, q) in [(-dx, x0), (dx, mx - x0), (-dy, y0), (dy, mx - y0)] {
        if p == 0.0 {
            if q < 0.0 {
                return;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                if r > t1 {
                    return;
                }
                if r > t0 {
                    t0 = r;
                }
            } else {
                if r < t0 {
                    return;
                }
                if r < t1 {
                    t1 = r;
                }
            }
        }
    }
    x1 = x0 + t1 * dx;
    y1 = y0 + t1 * dy;
    x0 += t0 * dx;
    y0 += t0 * dy;
    let steps = (x1 - x0).abs().max((y1 - y0).abs()).ceil().max(1.0) as usize;
    let (sx, sy) = ((x1 - x0) / steps as f32, (y1 - y0) / steps as f32);
    let (mut x, mut y) = (x0, y0);
    for _ in 0..=steps {
        let (ix, iy) = (x as usize, y as usize);
        if ix < size && iy < size {
            buf[iy * size + ix] += w;
        }
        x += sx;
        y += sy;
    }
}

pub fn run() {
    let a = args();
    let lay = args_s(&a, "layout");
    let out = args_s(&a, "out");
    let size = argf(&a, "size", 2048.0) as usize;
    let cx = argf(&a, "cx", 0.5) as f32;
    let cy = argf(&a, "cy", 0.5) as f32;
    let half = argf(&a, "half", 0.5) as f32;
    let edge_cap = argf(&a, "edges", 0.0);
    let pos: Vec<[f32; 2]> = read_vec(&format!("{lay}/pos.f32"));
    let top: Vec<u32> = read_vec(&format!("{lay}/top.u32"));
    let n = pos.len();
    let has_tree = std::path::Path::new(&format!("{lay}/tnodes.bin")).exists();
    let tn: Vec<TNode> = if has_tree { read_vec(&format!("{lay}/tnodes.bin")) } else { vec![] };
    let lp: Vec<u32> = if has_tree { read_vec(&format!("{lay}/leafparent.u32")) } else { vec![] };
    let fixed_r = argf(&a, "node-r", 0.0) as f32; // world-units node radius when there is no tree
    let scale = size as f32 / (2.0 * half);
    let (x0, y0) = (cx - half, cy - half);
    let mut acc = vec![[0f32; 4]; size * size];
    let inview = |p: [f32; 2], m: f32| p[0] >= x0 - m && p[0] <= x0 + 2.0 * half + m && p[1] >= y0 - m && p[1] <= y0 + 2.0 * half + m;
    let mut shown = 0u64;
    for u in 0..n {
        let p = pos[u];
        if !inview(p, half * 0.02) {
            continue;
        }
        shown += 1;
        let px = (p[0] - x0) * scale;
        let py = (p[1] - y0) * scale;
        let nr = if has_tree { tn[lp[u] as usize].node_r * scale * 0.55 } else { fixed_r * scale };
        let nr = nr.min(28.0);
        let c = color(top[u]);
        if nr < 0.9 {
            let (ix, iy) = (px as isize, py as isize);
            if ix >= 0 && iy >= 0 && (ix as usize) < size && (iy as usize) < size {
                let e = &mut acc[iy as usize * size + ix as usize];
                e[0] += c[0];
                e[1] += c[1];
                e[2] += c[2];
                e[3] += 1.0;
            }
        } else {
            let lo_x = ((px - nr - 1.0).floor().max(0.0)) as usize;
            let hi_x = ((px + nr + 1.0).ceil().min(size as f32 - 1.0)).max(0.0) as usize;
            let lo_y = ((py - nr - 1.0).floor().max(0.0)) as usize;
            let hi_y = ((py + nr + 1.0).ceil().min(size as f32 - 1.0)).max(0.0) as usize;
            for iy in lo_y..=hi_y {
                for ix in lo_x..=hi_x {
                    let d = ((ix as f32 + 0.5 - px).powi(2) + (iy as f32 + 0.5 - py).powi(2)).sqrt();
                    let cov = (nr + 0.5 - d).clamp(0.0, 1.0);
                    if cov > 0.0 {
                        let e = &mut acc[iy * size + ix];
                        e[0] += c[0] * cov;
                        e[1] += c[1] * cov;
                        e[2] += c[2] * cov;
                        e[3] += cov;
                    }
                }
            }
        }
    }
    // edges
    let mut ebuf = vec![0f32; if edge_cap > 0.0 { size * size } else { 0 }];
    let mut edges_total = 0u64;
    let mut edges_drawn = 0u64;
    if edge_cap > 0.0 {
        let csr = Csr::load(&args_s(&a, "graph"));
        for u in 0..n {
            if inview(pos[u], 0.0) {
                for &v in csr.adj(u) {
                    if !inview(pos[v as usize], 0.0) || (u as u32) < v {
                        edges_total += 1;
                    }
                }
            }
        }
        let prob = (edge_cap / edges_total.max(1) as f64).min(1.0);
        let thr = (prob * u32::MAX as f64) as u64;
        for u in 0..n {
            if inview(pos[u], 0.0) {
                for &v in csr.adj(u) {
                    if !inview(pos[v as usize], 0.0) || (u as u32) < v {
                        let mut hsh = (u as u64) << 32 | v as u64;
                        if (splitmix(&mut hsh) & 0xffff_ffff) <= thr {
                            edges_drawn += 1;
                            let q = pos[v as usize];
                            let wgt = if inview(q, 0.0) { 1.0 } else { 0.3 };
                            line(&mut ebuf, size, (pos[u][0] - x0) * scale, (pos[u][1] - y0) * scale, (q[0] - x0) * scale, (q[1] - y0) * scale, wgt);
                        }
                    }
                }
            }
        }
    }
    // tone map
    let (mut s, mut c) = (0f64, 0u64);
    for e in &acc {
        if e[3] > 0.0 {
            s += e[3] as f64;
            c += 1;
        }
    }
    let gain = (1.4 / (s / c.max(1) as f64)).clamp(0.02, 4.0) as f32;
    let (mut es, mut ec) = (0f64, 0u64);
    for &e in &ebuf {
        if e > 0.0 {
            es += e as f64;
            ec += 1;
        }
    }
    let egain = (argf(&a, "egain", 0.35) / (es / ec.max(1) as f64)).min(0.5) as f32;
    let mut img = vec![0u8; size * size * 3];
    for i in 0..size * size {
        let mut rgb = [9.0f32, 10.0, 16.0];
        if !ebuf.is_empty() && ebuf[i] > 0.0 {
            let e = 1.0 - (-ebuf[i] * egain).exp();
            rgb[0] += 120.0 * e;
            rgb[1] += 135.0 * e;
            rgb[2] += 160.0 * e;
        }
        let e = acc[i];
        if e[3] > 0.0 {
            let v = 1.0 - (-e[3] * gain).exp();
            for ch in 0..3 {
                rgb[ch] = rgb[ch] * (1.0 - v) + 255.0 * (e[ch] / e[3]) * v;
            }
        }
        for ch in 0..3 {
            img[i * 3 + ch] = rgb[ch].min(255.0) as u8;
        }
    }
    // community outlines
    let mut outlines = 0;
    if has_tree && !a.contains_key("no-bubbles") {
        for (ti, t) in tn.iter().enumerate() {
            if ti == 0 || t.flags & F_DEAD != 0 {
                continue;
            }
            let rp = t.r * scale;
            if rp < 6.0 || rp > 6.0 * size as f32 {
                continue;
            }
            let (px, py) = ((t.x - x0) * scale, (t.y - y0) * scale);
            if px + rp < 0.0 || py + rp < 0.0 || px - rp > size as f32 || py - rp > size as f32 {
                continue;
            }
            outlines += 1;
            let alpha = if t.flags & F_VIRTUAL != 0 { 0.3 } else if t.flags & F_LEAFP != 0 { 0.5 } else { 0.9 };
            let steps = (rp * 7.0) as usize;
            for sidx in 0..steps {
                let ang = sidx as f32 / steps as f32 * std::f32::consts::TAU;
                let (x, y) = (px + rp * ang.cos(), py + rp * ang.sin());
                if x >= 0.0 && y >= 0.0 && (x as usize) < size && (y as usize) < size {
                    let i = (y as usize * size + x as usize) * 3;
                    for ch in 0..3 {
                        img[i + ch] = (img[i + ch] as f32 * (1.0 - alpha * 0.5) + 235.0 * alpha * 0.5) as u8;
                    }
                }
            }
        }
    }
    let mut f = std::io::BufWriter::new(std::fs::File::create(&out).unwrap());
    write!(f, "P6\n{size} {size}\n255\n").unwrap();
    f.write_all(&img).unwrap();
    println!("{{\"out\":\"{out}\",\"nodes_in_view\":{shown},\"edges_in_view\":{edges_total},\"edges_drawn\":{edges_drawn},\"outlines\":{outlines},\"half\":{half}}}");
}
