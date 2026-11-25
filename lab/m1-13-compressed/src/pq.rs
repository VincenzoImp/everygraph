//! Point queries of lab/f1-query (src/bin/pq.rs), generic over `Adj`: degree, neighbors, common
//! neighbors, 2-hop size, 3-hop with a 10k budget, bidirectional BFS with path reconstruction.
//! Query bodies, parameter generation and the per-query timer are the lab's; the only change is
//! that lists come from `Adj::nbrs` (a slice of the raw array, or a decode into a buffer).
//! Warm, single thread. Each query is timed with its own `Instant` pair as in the lab (so ~40 ns of
//! timer cost is inside every latency); `batch_mean_us` is the same queries timed as one batch.
use crate::adj::Adj;
use rustc_hash::FxHashMap;
use serde_json::json;
use std::time::Instant;

/// Simple bitset with touched-word list for cheap clearing (lab/f1-query lib.rs).
pub struct Visited {
    pub words: Vec<u64>,
    pub touched: Vec<u32>,
}
impl Visited {
    pub fn new(n: usize) -> Self {
        Visited { words: vec![0; n.div_ceil(64)], touched: Vec::new() }
    }
    #[inline]
    pub fn set(&mut self, v: u32) -> bool {
        let w = (v >> 6) as usize;
        let b = 1u64 << (v & 63);
        let x = self.words[w];
        if x & b != 0 {
            return false;
        }
        if x == 0 {
            self.touched.push(w as u32);
        }
        self.words[w] = x | b;
        true
    }
    #[inline]
    pub fn get(&self, v: u32) -> bool {
        self.words[(v >> 6) as usize] >> (v & 63) & 1 == 1
    }
    pub fn clear(&mut self) {
        for &w in &self.touched {
            self.words[w as usize] = 0;
        }
        self.touched.clear();
    }
}

pub struct Rng(pub u64);
impl Rng {
    #[inline]
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    #[inline]
    pub fn below(&mut self, n: u64) -> u64 {
        ((self.next() as u128 * n as u128) >> 64) as u64
    }
}

pub fn lat_summary(v: &mut [f64]) -> serde_json::Value {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = v.len();
    let q = |p: f64| v[((n as f64 - 1.0) * p).round() as usize];
    let mean = v.iter().sum::<f64>() / n as f64;
    json!({"n": n, "mean_us": mean, "p50_us": q(0.5), "p95_us": q(0.95), "p99_us": q(0.99), "max_us": v[n-1]})
}

struct Ctx<'a, G: Adj> {
    g: &'a G,
    gin: &'a G,
    c1: G::Cur,
    c2: G::Cur,
    vis: Visited,
    vis2: Visited,
    q: Vec<u32>,
    nq: Vec<u32>,
}

impl<'a, G: Adj> Ctx<'a, G> {
    fn new(g: &'a G, gin: &'a G) -> Self {
        Ctx { g, gin, c1: g.cur(), c2: g.cur(), vis: Visited::new(g.n()), vis2: Visited::new(g.n()), q: vec![], nq: vec![] }
    }
    fn neighbors(&mut self, v: u32) -> usize {
        self.q.clear();
        self.q.extend_from_slice(self.g.nbrs(v as usize, &mut self.c1));
        self.q.len()
    }
    fn two_hop(&mut self, v: u32) -> usize {
        self.vis.clear();
        self.vis.set(v);
        let mut c = 0;
        let n1 = self.g.nbrs(v as usize, &mut self.c1);
        for &u in n1 {
            if self.vis.set(u) {
                c += 1;
            }
        }
        for &u in n1 {
            for &w in self.g.nbrs(u as usize, &mut self.c2) {
                if self.vis.set(w) {
                    c += 1;
                }
            }
        }
        c
    }
    fn khop(&mut self, v: u32, k: usize, budget: usize) -> usize {
        self.vis.clear();
        self.vis.set(v);
        self.q.clear();
        self.q.push(v);
        let mut c = 1;
        'outer: for _ in 0..k {
            self.nq.clear();
            for i in 0..self.q.len() {
                let u = self.q[i];
                for &w in self.g.nbrs(u as usize, &mut self.c1) {
                    if self.vis.set(w) {
                        c += 1;
                        self.nq.push(w);
                        if c >= budget {
                            break 'outer;
                        }
                    }
                }
            }
            std::mem::swap(&mut self.q, &mut self.nq);
            if self.q.is_empty() {
                break;
            }
        }
        c
    }
    fn common(&mut self, u: u32, w: u32) -> usize {
        let a = self.g.nbrs(u as usize, &mut self.c1);
        let b = self.g.nbrs(w as usize, &mut self.c2);
        let (mut i, mut j, mut c) = (0, 0, 0);
        while i < a.len() && j < b.len() {
            let (x, y) = (a[i], b[j]);
            c += (x == y) as usize;
            i += (x <= y) as usize;
            j += (y <= x) as usize;
        }
        c
    }
    /// bidirectional BFS; returns (path length, nodes touched). Reconstructs the path.
    fn bibfs(&mut self, s: u32, t: u32, maxd: usize) -> (i32, usize) {
        if s == t {
            return (0, 1);
        }
        let mut vs = std::mem::replace(&mut self.vis, Visited::new(0));
        let mut vt = std::mem::replace(&mut self.vis2, Visited::new(0));
        vs.clear();
        vt.clear();
        vs.set(s);
        vt.set(t);
        let mut ls: Vec<(u32, u32, u32)> = vec![(s, u32::MAX, 0)];
        let mut lt: Vec<(u32, u32, u32)> = vec![(t, u32::MAX, 0)];
        let (mut fs, mut ft) = (vec![s], vec![t]);
        let (mut ds, mut dt) = (0u32, 0u32);
        let (go, gi) = (self.g, self.gin);
        let mut result = -1i32;
        for _ in 0..maxd {
            let ws: u64 = fs.iter().map(|&v| go.deg(v as usize) as u64).sum();
            let wt: u64 = ft.iter().map(|&v| gi.deg(v as usize) as u64).sum();
            let side_s = ws <= wt;
            let (front, mine, other, log, d) = if side_s { (&mut fs, &mut vs, &vt, &mut ls, &mut ds) } else { (&mut ft, &mut vt, &vs, &mut lt, &mut dt) };
            if front.is_empty() {
                break;
            }
            let g = if side_s { go } else { gi };
            let mut next = Vec::new();
            let mut hits: Vec<(u32, u32)> = Vec::new();
            for &u in front.iter() {
                for &w in g.nbrs(u as usize, &mut self.c1) {
                    if other.get(w) {
                        hits.push((u, w));
                    }
                    if mine.set(w) {
                        log.push((w, u, *d + 1));
                        next.push(w);
                    }
                }
            }
            if !hits.is_empty() {
                let du = *d;
                let (lm, lo) = if side_s { (&ls, &lt) } else { (&lt, &ls) };
                let mm: FxHashMap<u32, (u32, u32)> = lm.iter().map(|&(x, p, dd)| (x, (p, dd))).collect();
                let om: FxHashMap<u32, (u32, u32)> = lo.iter().map(|&(x, p, dd)| (x, (p, dd))).collect();
                let (mut bl, mut bu, mut bw) = (u32::MAX, 0, 0);
                for &(u, w) in &hits {
                    let l = du + 1 + om[&w].1;
                    if l < bl {
                        (bl, bu, bw) = (l, u, w);
                    }
                }
                let mut path = vec![bw];
                let mut x = bu;
                while x != u32::MAX {
                    path.push(x);
                    x = mm[&x].0;
                }
                let mut x = om[&bw].0;
                while x != u32::MAX {
                    path.insert(0, x);
                    x = om[&x].0;
                }
                debug_assert_eq!(path.len() as u32, bl + 1);
                result = bl as i32;
                break;
            }
            *d += 1;
            *front = next;
        }
        let touched = ls.len() + lt.len();
        self.vis = vs;
        self.vis2 = vt;
        (result, touched)
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum QT {
    Nbrs,
    Deg,
    TwoHop,
    KHop,
    BiBfs,
    Common,
}
pub const QTS: [(QT, &str); 6] = [
    (QT::Deg, "degree"),
    (QT::Nbrs, "neighbors"),
    (QT::Common, "common_neighbors"),
    (QT::TwoHop, "two_hop_size"),
    (QT::KHop, "khop3_budget10k"),
    (QT::BiBfs, "bidir_bfs_path"),
];

/// Query parameters derived deterministically from an index (same generator as the lab).
fn params<G: Adj>(g: &G, c: &mut G::Cur, qt: QT, i: u64) -> (u32, u32) {
    let mut r = Rng(i.wrapping_mul(7919) ^ 0x5EED);
    let n = g.n() as u64;
    let a = r.below(n) as u32;
    match qt {
        QT::Common => {
            // (u, w) with w a 2-hop neighbour of u when possible
            let na = g.nbrs(a as usize, c);
            if na.is_empty() {
                return (a, r.below(n) as u32);
            }
            let m = na[r.below(na.len() as u64) as usize];
            let nm = g.nbrs(m as usize, c);
            if nm.is_empty() {
                return (a, m);
            }
            (a, nm[r.below(nm.len() as u64) as usize])
        }
        QT::BiBfs => (a, r.below(n) as u32),
        _ => (a, 0),
    }
}

#[inline(always)]
fn run1<G: Adj>(c: &mut Ctx<G>, qt: QT, a: u32, b: u32) -> usize {
    match qt {
        QT::Deg => c.g.deg(a as usize),
        QT::Nbrs => c.neighbors(a),
        QT::TwoHop => c.two_hop(a),
        QT::KHop => c.khop(a, 3, 10_000),
        QT::BiBfs => {
            let (l, t) = c.bibfs(a, b, 30);
            (l as usize) << 32 | t
        }
        QT::Common => c.common(a, b),
    }
}

/// Runs every query type `reps` times over `nq(type)` inputs; per type, reports the repetition
/// with the lowest p50. `gin` is the in-adjacency for directed graphs (same graph if undirected).
pub fn run_all<G: Adj>(g: &G, gin: &G, only: &[String], n_small: usize, n_big: usize, reps: usize) -> serde_json::Value {
    let mut c = Ctx::new(g, gin);
    let mut pc = g.cur();
    let mut out = json!({});
    for (qt, nm) in QTS {
        if !only.is_empty() && !only.iter().any(|o| o == nm) {
            continue;
        }
        let nq = match qt {
            QT::Deg | QT::Nbrs | QT::Common => n_small,
            _ => n_big,
        };
        let ps: Vec<(u32, u32)> = (0..nq as u64).map(|i| params(g, &mut pc, qt, i)).collect();
        let mut best: Option<serde_json::Value> = None;
        for _ in 0..reps {
            let mut lat = Vec::with_capacity(nq);
            let mut sink = 0u64;
            let mut extra = 0usize;
            let la = crate::adj::loadavg();
            for &(a, b) in &ps {
                let t = Instant::now();
                let r = run1(&mut c, qt, a, b);
                lat.push(t.elapsed().as_nanos() as f64 / 1000.0);
                sink = sink.wrapping_mul(31).wrapping_add(r as u64);
                extra += if qt == QT::BiBfs { r & 0xffffffff } else { r };
            }
            let mut s = lat_summary(&mut lat);
            if matches!(qt, QT::Deg | QT::Nbrs | QT::Common) {
                // the same queries as one batch, without per-query timers
                let t = Instant::now();
                let mut sink2 = 0u64;
                for &(a, b) in &ps {
                    sink2 = sink2.wrapping_mul(31).wrapping_add(run1(&mut c, qt, a, b) as u64);
                }
                s["batch_mean_us"] = json!(t.elapsed().as_secs_f64() * 1e6 / nq as f64);
                assert_eq!(sink, sink2);
            }
            s["avg_result"] = json!(extra as f64 / nq as f64);
            s["result_digest"] = json!(format!("{sink:016x}"));
            s["loadavg_1m_start"] = json!(la);
            if best.as_ref().map(|b| s["p50_us"].as_f64() < b["p50_us"].as_f64()).unwrap_or(true) {
                best = Some(s);
            }
        }
        eprintln!("  {nm}: {}", best.as_ref().unwrap());
        out[nm] = best.unwrap();
    }
    out
}
