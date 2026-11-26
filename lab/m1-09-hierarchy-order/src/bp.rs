// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Recursive graph bisection (BP): Dhulipala, Kabiljo, Karrer, Ottaviano, Pupyrev and
//! Shalita, "Compressing graphs and indexes with recursive graph bisection", KDD 2016,
//! with the level-synchronous schedule and the cooling tolerance of Mackenzie, Petri and
//! Moffat, "Faster index reordering with bipartite graph partitioning", SIGIR 2021
//! (their `rgbisect`, MIT, read for this; not linked, because it allocates two arrays of
//! `n` counters per subproblem, which is quadratic at depth log n − 5).
//!
//! A graph is its own bipartite input: every node is a document, its neighbours are
//! its terms. A split of a range of documents into halves L and R costs, for each term
//! q with d_L documents in L and d_R in R, `d_L log2(|L| / (d_L + 1)) + d_R log2(|R| /
//! (d_R + 1))` (the paper's log-gap estimate). Each iteration computes every document's
//! gain from moving to the other half, sorts both halves by gain, and swaps pairs while
//! the pair's total gain exceeds the iteration number (cooling); it stops at the cap or
//! when nothing moves. Then each half is split the same way, `depth` times.
//!
//! Deterministic at any thread count: the start order is a seeded hash order (or the
//! input order), gains are summed per document in list order, the sort key is
//! (gain, id) and the degree counts are integers.

use std::ops::Range;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicI32, Ordering::Relaxed};
use std::time::Instant;

use rayon::prelude::*;

use crate::Graph;
use crate::order::random_order;

pub struct Config {
    pub seed: u64,
    pub iters: u32,
    pub depth: u32,
    pub random_start: bool,
    /// The swap threshold grows by this much per iteration (cooling; 0: the KDD paper's
    /// "any positive gain").
    pub tol_step: f32,
    /// Each node is also one of its own terms, so that it is placed near its neighbours
    /// (its list's first gap is taken from its own id).
    pub self_term: bool,
}

/// floor(log2 n) − 5: leaves of 32 to 63 nodes.
pub fn default_depth(n: usize) -> u32 {
    (usize::BITS - 1 - n.max(1).leading_zeros()).saturating_sub(5)
}

#[derive(Clone, Copy)]
struct Item {
    gain: f32,
    doc: u32,
}

/// Per term: documents of the current range in L and in R.
type Degrees = Vec<[AtomicI32; 2]>;

fn degrees(n: usize) -> Degrees {
    (0..n).into_par_iter().map(|_| [AtomicI32::new(0), AtomicI32::new(0)]).collect()
}

const TABLE: usize = 4096;

fn lg(x: i32) -> f32 {
    static T: OnceLock<Vec<f32>> = OnceLock::new();
    let t = T.get_or_init(|| (0..TABLE).map(|i| if i == 0 { 0.0 } else { (i as f32).log2() }).collect());
    if (x as usize) < TABLE { t[x as usize] } else { (x as f32).log2() }
}

/// The cost of one term with `d1` documents in a half of log-size `l1` and `d2` in the
/// other.
#[inline]
fn cost(l1: f32, l2: f32, d1: i32, d2: i32) -> f32 {
    let (f1, f2) = (d1 as f32, d2 as f32);
    f1 * (l1 - lg(d1 + 1)) + f2 * (l2 - lg(d2 + 1))
}

#[inline]
fn move_gain(l_from: f32, l_to: f32, from: i32, to: i32) -> f32 {
    cost(l_from, l_to, from, to) - cost(l_from, l_to, from - 1, to + 1)
}

#[inline]
fn add(a: &AtomicI32, d: i32, par: bool) {
    if par {
        a.fetch_add(d, Relaxed);
    } else {
        a.store(a.load(Relaxed) + d, Relaxed);
    }
}

fn each(items: &[Item], par: bool, f: impl Fn(&Item) + Sync + Send) {
    if par {
        items.par_iter().with_min_len(256).for_each(f);
    } else {
        items.iter().for_each(f);
    }
}

fn by_gain(a: &Item, b: &Item) -> std::cmp::Ordering {
    b.gain.total_cmp(&a.gain).then(a.doc.cmp(&b.doc))
}

/// Bisects one range in place; returns (iterations run, documents swapped). `deg` is
/// zero on entry and on return.
fn bisect(items: &mut [Item], g: &Graph, deg: &Degrees, cfg: &Config, par: bool) -> (u64, u64) {
    let mid = items.len() / 2;
    let own = cfg.self_term;
    let terms = |doc: u32| g.list(doc).iter().copied().chain(std::iter::once(doc).filter(move |_| own));
    let count = |half: &[Item], side: usize, d: i32| {
        each(half, par, |it| {
            for q in terms(it.doc) {
                add(&deg[q as usize][side], d, par);
            }
        });
    };
    count(&items[..mid], 0, 1);
    count(&items[mid..], 1, 1);
    let (ll, lr) = ((mid as f32).log2(), ((items.len() - mid) as f32).log2());
    let (mut done, mut swapped) = (0, 0);
    for iter in 0..cfg.iters {
        done += 1;
        let gains = |it: &mut Item, from: usize, l_from: f32, l_to: f32| {
            let mut s = 0.0f32;
            for q in terms(it.doc) {
                let d = &deg[q as usize];
                s += move_gain(l_from, l_to, d[from].load(Relaxed), d[1 - from].load(Relaxed));
            }
            it.gain = s;
        };
        let (left, right) = items.split_at_mut(mid);
        if par {
            rayon::join(
                || left.par_iter_mut().with_min_len(256).for_each(|it| gains(it, 0, ll, lr)),
                || right.par_iter_mut().with_min_len(256).for_each(|it| gains(it, 1, lr, ll)),
            );
            rayon::join(|| left.par_sort_unstable_by(by_gain), || right.par_sort_unstable_by(by_gain));
        } else {
            left.iter_mut().for_each(|it| gains(it, 0, ll, lr));
            right.iter_mut().for_each(|it| gains(it, 1, lr, ll));
            left.sort_unstable_by(by_gain);
            right.sort_unstable_by(by_gain);
        }
        let tol = iter as f32 * cfg.tol_step;
        let k = left.iter().zip(right.iter()).take_while(|(a, b)| a.gain + b.gain > tol).count();
        if k == 0 {
            break;
        }
        swapped += k as u64;
        count(&left[..k], 0, -1);
        count(&left[..k], 1, 1);
        count(&right[..k], 1, -1);
        count(&right[..k], 0, 1);
        left[..k].swap_with_slice(&mut right[..k]);
    }
    each(items, par, |it| {
        for q in terms(it.doc) {
            deg[q as usize][0].store(0, Relaxed);
            deg[q as usize][1].store(0, Relaxed);
        }
    });
    (done, swapped)
}

/// Disjoint mutable views of `items` at sorted, disjoint `ranges`.
fn views<'a>(mut items: &'a mut [Item], ranges: &[Range<usize>]) -> Vec<&'a mut [Item]> {
    let mut pos = 0;
    let mut out = Vec::with_capacity(ranges.len());
    for r in ranges {
        let (_, tail) = std::mem::take(&mut items).split_at_mut(r.start - pos);
        let (s, tail) = tail.split_at_mut(r.len());
        out.push(s);
        items = tail;
        pos = r.end;
    }
    out
}

/// The BP order (old ids by new position) and its per-level statistics as JSON fields.
pub fn order(g: &Graph, cfg: &Config) -> (Vec<u32>, String) {
    let n = g.n();
    let start: Vec<u32> = if cfg.random_start { random_order(n, cfg.seed) } else { (0..n as u32).collect() };
    let mut items: Vec<Item> = start.into_iter().map(|doc| Item { gain: 0.0, doc }).collect();
    let threads = rayon::current_num_threads();
    let pool: Mutex<Vec<Degrees>> = Mutex::new(Vec::new());
    let mut ranges = vec![0..n];
    let mut levels = Vec::new();
    for level in 0..cfg.depth {
        let t = Instant::now();
        let slices = views(&mut items, &ranges);
        let sum = |a: (u64, u64), b: (u64, u64)| (a.0 + b.0, a.1 + b.1);
        // Fewer ranges than threads: each range is also split across threads.
        let par = slices.len() < threads;
        let (it, sw) = slices
            .into_par_iter()
            .map(|s| {
                // Not under the lock: `degrees` runs on the pool, which may start another
                // range's task on this thread.
                let free = pool.lock().unwrap().pop();
                let deg = free.unwrap_or_else(|| degrees(n));
                let r = bisect(s, g, &deg, cfg, par);
                pool.lock().unwrap().push(deg);
                r
            })
            .reduce(|| (0, 0), sum);
        levels.push(format!(
            "{{\"level\": {level}, \"ranges\": {}, \"iters\": {it}, \"swapped\": {sw}, \"s\": {:.3}}}",
            ranges.len(),
            t.elapsed().as_secs_f64()
        ));
        ranges = ranges
            .iter()
            .flat_map(|r| {
                let mid = r.start + r.len() / 2;
                [r.start..mid, mid..r.end]
            })
            .filter(|r| r.len() >= 2)
            .collect();
        if ranges.is_empty() {
            break;
        }
    }
    let scratch = pool.lock().unwrap().len();
    let order = items.into_iter().map(|it| it.doc).collect();
    (order, format!("\"scratch_arrays\": {scratch}, \"levels\": [{}]", levels.join(", ")))
}
