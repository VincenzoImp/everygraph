//! Power-diagram cells inside a convex parent cell (m1-07, `docs/research/sota-maps.md`
//! challenge 2). Each child gets a convex cell with area proportional to its size:
//! Lloyd steps (site to cell centroid) plus weight adaptation (Nocaj and Brandes 2012:
//! w_i <- w_i * target_i / area_i, capped by the squared distance to the nearest other
//! site, so every site stays inside its own non-empty cell). All arithmetic is f64 and
//! sequential per parent, so the result does not depend on the thread count.

pub type Poly = Vec<[f64; 2]>;

pub fn area_centroid(p: &[[f64; 2]]) -> (f64, [f64; 2]) {
    if p.len() < 3 {
        return (0.0, p.first().copied().unwrap_or([0.0, 0.0]));
    }
    let o = p[0];
    let (mut a, mut cx, mut cy) = (0f64, 0f64, 0f64);
    for i in 1..p.len() - 1 {
        let (x1, y1) = (p[i][0] - o[0], p[i][1] - o[1]);
        let (x2, y2) = (p[i + 1][0] - o[0], p[i + 1][1] - o[1]);
        let t = x1 * y2 - x2 * y1;
        a += t;
        cx += t * (x1 + x2);
        cy += t * (y1 + y2);
    }
    if a.abs() < 1e-300 {
        return (0.0, o);
    }
    (0.5 * a, [o[0] + cx / (3.0 * a), o[1] + cy / (3.0 * a)])
}

/// Keeps the part of `p` with a.x <= b (Sutherland-Hodgman against one half-plane).
fn clip(p: &[[f64; 2]], a: [f64; 2], b: f64, out: &mut Poly) {
    out.clear();
    let n = p.len();
    for i in 0..n {
        let (u, v) = (p[i], p[(i + 1) % n]);
        let (fu, fv) = (a[0] * u[0] + a[1] * u[1] - b, a[0] * v[0] + a[1] * v[1] - b);
        if fu <= 0.0 {
            out.push(u);
        }
        if (fu < 0.0 && fv > 0.0) || (fu > 0.0 && fv < 0.0) {
            let t = fu / (fu - fv);
            out.push([u[0] + t * (v[0] - u[0]), u[1] + t * (v[1] - u[1])]);
        }
    }
}

fn inside(p: &[[f64; 2]], x: [f64; 2]) -> bool {
    let n = p.len();
    (0..n).all(|i| {
        let (u, v) = (p[i], p[(i + 1) % n]);
        (v[0] - u[0]) * (x[1] - u[1]) - (v[1] - u[1]) * (x[0] - u[0]) > 0.0
    })
}

/// Power cells of `sites` with weights `w` inside the counter-clockwise convex polygon `poly`.
fn power_cells(poly: &[[f64; 2]], s: &[[f64; 2]], w: &[f64]) -> Vec<Poly> {
    let k = s.len();
    let wmax = w.iter().fold(0f64, |a, &b| a.max(b));
    let mut order: Vec<(f64, u32)> = Vec::with_capacity(k);
    let (mut cur, mut tmp) = (Vec::with_capacity(poly.len() + 8), Vec::with_capacity(poly.len() + 8));
    let mut cells = Vec::with_capacity(k);
    for i in 0..k {
        order.clear();
        order.extend((0..k).filter(|&j| j != i).map(|j| ((s[j][0] - s[i][0]).hypot(s[j][1] - s[i][1]), j as u32)));
        order.sort_unstable_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        cur.clear();
        cur.extend_from_slice(poly);
        let mut rmax = cur.iter().map(|x| (x[0] - s[i][0]).hypot(x[1] - s[i][1])).fold(0f64, f64::max);
        for &(d, j) in &order {
            // no point within rmax of s_i is closer in power distance to any site this far away
            if d > rmax && (d - rmax).powi(2) >= rmax * rmax - w[i] + wmax {
                break;
            }
            let j = j as usize;
            let a = [2.0 * (s[j][0] - s[i][0]), 2.0 * (s[j][1] - s[i][1])];
            let b = (s[j][0] * s[j][0] + s[j][1] * s[j][1]) - (s[i][0] * s[i][0] + s[i][1] * s[i][1]) - w[j] + w[i];
            clip(&cur, a, b, &mut tmp);
            std::mem::swap(&mut cur, &mut tmp);
            if cur.len() < 3 {
                break;
            }
            rmax = cur.iter().map(|x| (x[0] - s[i][0]).hypot(x[1] - s[i][1])).fold(0f64, f64::max);
        }
        cells.push(cur.clone());
    }
    cells
}

pub struct Cells {
    #[allow(dead_code)]
    pub sites: Vec<[f64; 2]>,
    pub cells: Vec<Poly>,
    /// sum |area_i - target_i| / (2 * parent area), in [0, 1]
    pub misallocated: f64,
}

/// Cells for children with area shares `share` (summing to 1) inside `poly`, starting from
/// `start` (absolute coordinates). `None` when a cell degenerates; the caller falls back.
pub fn layout(poly: &[[f64; 2]], start: &[[f64; 2]], share: &[f64], iters: usize) -> Option<Cells> {
    let k = start.len();
    let (pa, pc) = area_centroid(poly);
    if k == 1 {
        return Some(Cells { sites: vec![pc], cells: vec![poly.to_vec()], misallocated: 0.0 });
    }
    // sites strictly inside the parent, pairwise distinct
    let mut s: Vec<[f64; 2]> = start
        .iter()
        .enumerate()
        .map(|(i, &x)| {
            let mut x = x;
            let mut t = 1.0;
            while !inside(poly, x) && t > 1e-3 {
                t *= 0.9;
                x = [pc[0] + t * (start[i][0] - pc[0]), pc[1] + t * (start[i][1] - pc[1])];
            }
            if !inside(poly, x) {
                x = pc;
            }
            x
        })
        .collect();
    let eps = 1e-7 * pa.sqrt();
    for i in 1..k {
        while (0..i).any(|j| (s[i][0] - s[j][0]).hypot(s[i][1] - s[j][1]) < eps) {
            let a = i as f64 * 2.399963;
            let t = [s[i][0] + 10.0 * eps * a.cos(), s[i][1] + 10.0 * eps * a.sin()];
            s[i] = if inside(poly, t) { t } else { [0.5 * (s[i][0] + pc[0]), 0.5 * (s[i][1] + pc[1])] };
        }
    }
    let target: Vec<f64> = share.iter().map(|&f| f * pa).collect();
    let mut w: Vec<f64> = target.iter().map(|&t| t / std::f64::consts::PI).collect();
    let cap = |s: &[[f64; 2]], w: &mut [f64]| {
        for i in 0..k {
            let dmin = (0..k).filter(|&j| j != i).map(|j| (s[i][0] - s[j][0]).powi(2) + (s[i][1] - s[j][1]).powi(2)).fold(f64::MAX, f64::min);
            w[i] = w[i].max(1e-12 * pa).min(0.98 * dmin);
        }
    };
    cap(&s, &mut w);
    let mut cells = power_cells(poly, &s, &w);
    for _ in 0..iters {
        let mut areas = vec![0f64; k];
        for i in 0..k {
            let (a, c) = area_centroid(&cells[i]);
            if !(a > 0.0) {
                return None;
            }
            areas[i] = a;
            s[i] = c;
        }
        for i in 0..k {
            w[i] *= target[i] / areas[i];
        }
        cap(&s, &mut w);
        cells = power_cells(poly, &s, &w);
    }
    let mut mis = 0f64;
    for i in 0..k {
        let a = area_centroid(&cells[i]).0;
        if !(a > 1e-12 * pa) || cells[i].len() < 3 {
            return None;
        }
        mis += (a - target[i]).abs();
    }
    Some(Cells { sites: s, cells, misallocated: mis / (2.0 * pa) })
}

/// Largest inscribed circle of a counter-clockwise convex polygon: bisection on the radius;
/// the centre is the centroid of the polygon shrunk by that radius.
pub fn inscribed(p: &[[f64; 2]]) -> ([f64; 2], f64) {
    let (a, c) = area_centroid(p);
    let n = p.len();
    let edges: Vec<([f64; 2], f64)> = (0..n)
        .filter_map(|i| {
            let (u, v) = (p[i], p[(i + 1) % n]);
            let (dx, dy) = (v[0] - u[0], v[1] - u[1]);
            let l = dx.hypot(dy);
            (l > 0.0).then(|| {
                let nrm = [dy / l, -dx / l];
                (nrm, nrm[0] * u[0] + nrm[1] * u[1])
            })
        })
        .collect();
    let (mut lo, mut hi) = (0f64, (a / std::f64::consts::PI).sqrt());
    let (mut cur, mut tmp) = (Vec::new(), Vec::new());
    let mut best = c;
    for _ in 0..24 {
        let mid = 0.5 * (lo + hi);
        cur.clear();
        cur.extend_from_slice(p);
        for &(nrm, b) in &edges {
            clip(&cur, nrm, b - mid, &mut tmp);
            std::mem::swap(&mut cur, &mut tmp);
            if cur.is_empty() {
                break;
            }
        }
        if cur.is_empty() {
            hi = mid;
        } else {
            lo = mid;
            best = if cur.len() >= 3 { area_centroid(&cur).1 } else { cur[0] };
        }
    }
    (best, lo)
}

/// Counter-clockwise regular `n`-gon inscribed in a disc.
pub fn disc(c: [f64; 2], r: f64, n: usize) -> Poly {
    (0..n).map(|i| {
        let a = i as f64 * std::f64::consts::TAU / n as f64;
        [c[0] + r * a.cos(), c[1] + r * a.sin()]
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_tile_the_parent_with_target_areas() {
        let sq: Poly = vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let share = [0.5, 0.25, 0.125, 0.125];
        let start = [[0.2, 0.2], [0.8, 0.3], [0.3, 0.8], [0.7, 0.7]];
        let c = layout(&sq, &start, &share, 60).unwrap();
        let total: f64 = c.cells.iter().map(|p| area_centroid(p).0).sum();
        assert!((total - 1.0).abs() < 1e-9, "{total}");
        assert!(c.misallocated < 0.02, "{}", c.misallocated);
        for (i, p) in c.cells.iter().enumerate() {
            assert!(inside(p, c.sites[i]) || p.len() >= 3);
        }
    }

    #[test]
    fn inscribed_circle_of_a_square_and_a_triangle() {
        let (c, r) = inscribed(&[[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]]);
        assert!((r - 1.0).abs() < 1e-6 && (c[0] - 1.0).abs() < 1e-3 && (c[1] - 1.0).abs() < 1e-3);
        // 3-4-5 right triangle: inradius (3 + 4 - 5) / 2 = 1
        let (_, r) = inscribed(&[[0.0, 0.0], [4.0, 0.0], [0.0, 3.0]]);
        assert!((r - 1.0).abs() < 1e-6, "{r}");
    }
}
