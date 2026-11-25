// lab/m1-13-compressed/src/leiden.rs (itself lab/b2-leiden's Louvain / Leiden, async and
// deterministic modes, with the level-0 graph opened from the compressed store), with the
// out-of-core mode of issue #16 (m1-02), marked "m1-02":
//  * `--ooc`: every aggregation is the sequential one (seqagg.rs) and every quotient graph is a
//    weighted compressed store mapped from an unlinked file in `--spill DIR`;
//  * every large allocation of `run` and `eval` is reserved in the memory-accountant stub
//    (acct.rs) first; without `--ooc` the budget is unlimited and the reservations are only
//    recorded, so the in-RAM reference reports what it would have needed;
//  * `--check-agg`: also runs the member-order `aggregate` at every level and checks that the
//    two quotient graphs are equal (dmap, k', every list as a set of (target, weight)).
// The local-moving and refinement code is unchanged.
#![allow(clippy::too_many_arguments, clippy::needless_range_loop)]
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering::Relaxed, Ordering::SeqCst};
use std::sync::Mutex;
use std::time::Instant;

use crate::acct::{res, sz, Res};
use crate::lutil::*;

#[path = "seqagg.rs"]
mod seqagg;

pub type W = f64;

const NONE: u32 = u32::MAX;

// ---------------------------------------------------------------- per-thread state
pub struct Tab {
    keys: Vec<u32>,
    vals: Vec<f64>,
    used: Vec<u32>,
    mask: usize,
    shift: u32,
}
impl Tab {
    fn new() -> Self {
        Tab { keys: vec![NONE; 16], vals: vec![0.0; 16], used: Vec::with_capacity(64), mask: 15, shift: 28 }
    }
    #[inline(always)]
    fn begin(&mut self, need: usize) {
        let cap = (need.max(4) * 2).next_power_of_two().max(16).min(1usize << 32);
        if cap > self.keys.len() {
            self.keys.resize(cap, NONE);
            self.vals.resize(cap, 0.0);
        }
        self.mask = cap - 1;
        self.shift = 32 - cap.trailing_zeros().min(32);
    }
    #[inline(always)]
    fn add(&mut self, key: u32, w: f64) {
        let mut i = if self.shift >= 32 { 0 } else { (key.wrapping_mul(0x9E37_79B1) >> self.shift) as usize };
        unsafe {
            loop {
                let kk = *self.keys.get_unchecked(i);
                if kk == key {
                    *self.vals.get_unchecked_mut(i) += w;
                    return;
                }
                if kk == NONE {
                    *self.keys.get_unchecked_mut(i) = key;
                    *self.vals.get_unchecked_mut(i) = w;
                    self.used.push(i as u32);
                    return;
                }
                i = (i + 1) & self.mask;
            }
        }
    }
    fn shrink(&mut self) {
        if self.keys.len() > (1 << 20) {
            self.keys = vec![NONE; 1 << 16];
            self.vals = vec![0.0; 1 << 16];
            self.used.shrink_to(1 << 16);
        }
    }
    #[inline(always)]
    fn clear(&mut self) {
        for &i in &self.used {
            unsafe { *self.keys.get_unchecked_mut(i as usize) = NONE };
        }
        self.used.clear();
        if self.keys.len() > TAB_KEEP {
            // m1-02: a table grown for a huge list (a supernode of a giant community has
            // millions of neighbours) is given back at once instead of staying with the thread
            self.keys = vec![NONE; 1 << 16];
            self.vals = vec![0.0; 1 << 16];
            self.used = Vec::with_capacity(64);
        }
    }
}
/// m1-02: tables up to this many slots (48 MiB) stay with their thread.
const TAB_KEEP: usize = 1 << 18; // = the table of a node of GIANT_DEG neighbours
/// m1-02: a node with more neighbours than this is visited by at most GIANT_PERMITS threads at
/// once, so the scratch to reserve is GIANT_PERMITS x the largest one, not threads x.
pub const GIANT_DEG: usize = 1 << 17;
const GIANT_PERMITS: usize = 1;
static GIANTS: AtomicUsize = AtomicUsize::new(0);
pub struct GiantGate(bool);
impl Drop for GiantGate {
    fn drop(&mut self) {
        if self.0 {
            GIANTS.fetch_sub(1, SeqCst);
        }
    }
}
/// Takes a permit when `deg` is a giant's degree (waits for one); a no-op otherwise.
#[inline(always)]
pub fn giant_gate(deg: usize) -> GiantGate {
    if deg <= GIANT_DEG {
        return GiantGate(false);
    }
    loop {
        if GIANTS.fetch_add(1, SeqCst) < GIANT_PERMITS {
            return GiantGate(true);
        }
        GIANTS.fetch_sub(1, SeqCst);
        std::thread::yield_now();
    }
}

/// m1-02: worst-case scratch of the threads while they visit one level: a hash table of
/// next_pow2(2 deg) slots (12 B) plus its index list, and decode buffers of the compressed
/// path (targets and weights, 4 B each), for the `nt` largest degrees at the same time.
fn scratch_bytes(g: &Graph, nt: usize) -> u64 {
    let top: Vec<u32> = match g.comp() {
        Some(c) if !c.top_degs.is_empty() => c.top_degs.clone(),
        Some(c) => vec![c.max_deg as u32; nt],
        None => {
            let mut d: Vec<u32> = (0..g.n).map(|u| g.deg(u) as u32).collect();
            let k = nt.min(d.len());
            if k > 0 && k < d.len() {
                d.select_nth_unstable_by(k - 1, |a, b| b.cmp(a));
            }
            d.truncate(k);
            d
        }
    };
    let per = |d: u64| -> u64 { (2 * d).max(16).next_power_of_two() * 12 + d * 4 + (d + 16).next_power_of_two() * 8 };
    let mut top = top;
    top.sort_unstable_by(|a, b| b.cmp(a));
    // at most GIANT_PERMITS threads on giants (their scratch is freed after each), the others on
    // the largest non-giants (or holding a table kept from one: at most TAB_KEEP slots)
    let giants: Vec<u64> = top.iter().filter(|&&d| d as usize > GIANT_DEG).take(GIANT_PERMITS.min(nt)).map(|&d| per(d as u64)).collect();
    let mut rest: Vec<u64> = top.iter().filter(|&&d| d as usize <= GIANT_DEG).map(|&d| per(d as u64)).collect();
    let others = nt - giants.len();
    let fill = if top.iter().any(|&d| d as usize > GIANT_DEG) { per(GIANT_DEG as u64) } else { per(64) };
    rest.resize(others.max(rest.len()), fill);
    rest.sort_unstable_by(|a, b| b.cmp(a));
    giants.iter().sum::<u64>() + rest[..others].iter().sum::<u64>()
}

pub struct Tls {
    tab: Tab,
    moves: Vec<(u32, u32, i64)>,
    act: Vec<Vec<u32>>,
    gain: f64,
    gain_fx: i64,
    scanned: u64,
    processed: u64,
    nmoves: u64,
}
fn new_tls(t: usize) -> Vec<Mutex<Tls>> {
    (0..t)
        .map(|_| Mutex::new(Tls { tab: Tab::new(), moves: Vec::new(), act: Vec::new(), gain: 0.0, gain_fx: 0, scanned: 0, processed: 0, nmoves: 0 }))
        .collect()
}

// ---------------------------------------------------------------- options
#[derive(Clone)]
pub struct Opts {
    leiden: bool,
    det: bool,
    sub_rounds: usize,
    gamma: f64,
    cap: u32,
    max_iters: usize,
    max_passes: usize,
    tol: f64,
    tol_drop: f64,
    agg_tol: f64,
    lowmem: bool,
    keep_graphs: bool,
    split_last: bool,
    refine_iters: usize,
    chunk: usize,
    verbose: bool,
    order_block: usize,
    permute: bool,
    tie_hash: bool,
    indep: bool,
    iterations: usize,
    cap_stall: f64,
    // m1-02
    ooc: bool,
    spill: String,
    check_agg: bool,
    order_window: usize,
    trace_io: bool,
    /// out of core with --out-hier: the quotient store of every level is kept here
    hier_dir: Option<String>,
}

#[derive(Clone, Default)]
pub struct PassStat {
    n: usize,
    entries: usize,
    iters: usize,
    comms: usize,
    t_lm: f64,
    t_ref: f64,
    t_agg: f64,
    scanned: u64,
    moves: u64,
    // m1-02: details of the sequential aggregation (JSON), accountant use at the end of the pass
    agg: String,
    acct_used: u64,
    anon_kb: u64,
    io_lm_read: u64,
    io_agg_read: u64,
    io_agg_write: u64,
}

struct Cx<'a> {
    g: &'a Graph,
    k: &'a [f64],
    comm: &'a [AtomicU32],
    sigma: &'a [AtomicU64],
    size: &'a [AtomicU32],
    flag: &'a [AtomicU8],
    minv: f64,
    gamma: f64,
    cap: u32,
    det: bool,
    tie_hash: bool,
    want: &'a [AtomicU32],
    indep: bool,
}

#[inline(always)]
fn tie_less(d: u32, best: u32, u: u32, hash: bool) -> bool {
    if best == NONE {
        return true;
    }
    if hash {
        let h = |x: u32| (x ^ u.wrapping_mul(0x85EB_CA6B)).wrapping_mul(0x9E37_79B1);
        let (a, b) = (h(d), h(best));
        a < b || (a == b && d < best)
    } else {
        d < best
    }
}
/// Deterministic pseudo-random visiting order: blocks of `ob` consecutive vertex ids, block order permuted by a
/// multiplicative bijection. Keeps cache/disk locality inside a block, breaks the id-order bias of local moving.
/// m1-02 (survey idea L2): with `win` > 0 the permutation is applied inside consecutive windows
/// of `win` blocks, and the windows are visited in disk order (a fixed function of the graph:
/// never of the budget). win = 0: the whole graph is one window (ADR 0015's order).
struct Order {
    ob: usize,
    nb: usize,
    n: usize,
    permute: bool,
    win: usize,
}
impl Order {
    fn new(n: usize, o: &Opts) -> Order {
        let ob = o.order_block.max(1);
        Order { ob, nb: n.div_ceil(ob), n, permute: o.permute, win: o.order_window }
    }
    fn chunk(&self, vertices: usize) -> usize {
        (vertices / self.ob).max(1)
    }
    #[inline(always)]
    fn block(&self, j: usize) -> std::ops::Range<usize> {
        const P: u64 = 2_654_435_761; // prime > any block count we handle (nb < 2^31)
        let jj = if !self.permute {
            j
        } else if self.win == 0 || self.win >= self.nb {
            ((j as u64 * P) % self.nb as u64) as usize
        } else {
            let w0 = j / self.win * self.win;
            let sz = self.win.min(self.nb - w0) as u64;
            w0 + (((j - w0) as u64 * P) % sz) as usize
        };
        jj * self.ob..((jj + 1) * self.ob).min(self.n)
    }
}

#[inline(always)]
fn decide(cx: &Cx, u: usize, tab: &mut Tab) -> Option<(u32, u32, f64)> {
    let g = cx.g;
    let deg = g.deg(u);
    if deg == 0 {
        return None;
    }
    let c = cx.comm[u].load(Relaxed);
    let _gg = giant_gate(deg);
    tab.begin(deg);
    let uu = u as u32;
    g.for_edges(u, |v, w| {
        if v != uu {
            tab.add(cx.comm[v as usize].load(Relaxed), w)
        }
    });
    let ku = cx.k[u];
    let f = cx.gamma * ku * cx.minv;
    let mut kuc = 0.0;
    let mut best = NONE;
    let mut bestval = f64::NEG_INFINITY;
    for &s in &tab.used {
        let d = tab.keys[s as usize];
        let kud = tab.vals[s as usize];
        if d == c {
            kuc = kud;
            continue;
        }
        if cx.cap > 0 && cx.size[d as usize].load(Relaxed) >= cx.cap {
            continue;
        }
        let val = kud - f * fload(&cx.sigma[d as usize]);
        if val > bestval || (val == bestval && tie_less(d, best, uu, cx.tie_hash)) {
            best = d;
            bestval = val;
        }
    }
    tab.clear();
    if best == NONE {
        return None;
    }
    let stay = kuc - f * (fload(&cx.sigma[c as usize]) - ku);
    if bestval > stay {
        if cx.det && !cx.indep && best > c && cx.size[c as usize].load(Relaxed) == 1 && cx.size[best as usize].load(Relaxed) == 1 {
            return None; // singleton-swap rule: singletons only merge towards the smaller label
        }
        Some((c, best, 2.0 * (bestval - stay) * cx.minv))
    } else {
        None
    }
}

#[inline(always)]
fn mark_neighbors(cx: &Cx, u: usize) {
    let _gg = giant_gate(cx.g.deg(u));
    cx.g.for_edges(u, |v, _| {
        let f = &cx.flag[v as usize];
        if f.load(Relaxed) == 0 {
            f.store(1, Relaxed)
        }
    });
}

#[inline(always)]
fn bucket_of(u: u32, lb: u32, shift: u32) -> usize {
    if lb == 0 {
        0
    } else {
        ((u >> shift).wrapping_mul(0x9E37_79B1) >> (32 - lb)) as usize
    }
}

/// Put every vertex into its (hashed) sub-round bucket, per thread.
fn bucketize(pool: &rayon::ThreadPool, tls: &[Mutex<Tls>], n: usize, lb: u32, shift: u32) {
    for t in tls {
        let mut t = t.lock().unwrap();
        t.act.iter_mut().for_each(|v| v.clear());
        t.act.resize(1usize << lb, Vec::new());
    }
    par_for(pool, n, 8192, |tid, r| {
        let mut t = tls[tid].lock().unwrap();
        for u in r {
            t.act[bucket_of(u as u32, lb, shift)].push(u as u32);
        }
    });
}
fn take_bucket(tls: &[Mutex<Tls>], b: usize) -> (Vec<Vec<u32>>, Vec<usize>, usize) {
    let cur: Vec<Vec<u32>> = tls.iter().map(|t| std::mem::take(&mut t.lock().unwrap().act[b])).collect();
    let mut pre = Vec::with_capacity(cur.len() + 1);
    let mut a = 0;
    for l in &cur {
        pre.push(a);
        a += l.len();
    }
    pre.push(a);
    (cur, pre, a)
}
#[inline(always)]
fn for_range_in_lists<F: FnMut(u32)>(cur: &[Vec<u32>], pre: &[usize], r: std::ops::Range<usize>, mut f: F) {
    let mut li = 0;
    while pre[li + 1] <= r.start {
        li += 1;
    }
    let mut pos = r.start;
    while pos < r.end {
        let l = &cur[li];
        let s = pos - pre[li];
        let e = (r.end - pre[li]).min(l.len());
        for &u in &l[s..e] {
            f(u)
        }
        pos = pre[li] + e;
        li += 1;
    }
}
const SEQ_BELOW: usize = 1024;
static BUCKET_SHIFT: AtomicU32 = AtomicU32::new(0);
static PEAK_ANON_KB: AtomicU64 = AtomicU64::new(0);
fn sample_anon() {
    PEAK_ANON_KB.fetch_max(rss_kb().1, Relaxed);
}

/// Local-moving phase. Returns (iterations, moves, scanned entries).
fn local_move(cx: &Cx, pool: &rayon::ThreadPool, tls: &[Mutex<Tls>], o: &Opts, tol: f64, sub_rounds: usize, shift: u32) -> (usize, u64, u64) {
    let n = cx.g.n;
    let use_size = cx.det || cx.cap > 0;
    let ord = Order::new(n, o);
    let lb = sub_rounds.max(1).trailing_zeros();
    if cx.det {
        bucketize(pool, tls, n, lb, shift);
    }
    let mut iters = 0;
    let mut tot_moves = 0u64;
    let mut tot_scanned = 0u64;
    // m1-02 trace (survey idea L3): per iteration, 64-node blocks with at least one active node
    // and 4 KiB pages of the compressed store touched by the active nodes' records
    let comp = if o.trace_io { cx.g.comp() } else { None };
    let tb: Vec<AtomicU64> = if comp.is_some() { (0..ord.nb.div_ceil(64)).map(|_| AtomicU64::new(0)).collect() } else { Vec::new() };
    let tp: Vec<AtomicU64> = comp.map(|c| (0..(c.data_len / 4096 + 1).div_ceil(64)).map(|_| AtomicU64::new(0)).collect()).unwrap_or_default();
    for _it in 0..o.max_iters {
        for t in tls {
            let mut t = t.lock().unwrap();
            t.gain = 0.0;
            t.gain_fx = 0;
            t.processed = 0;
        }
        let io0 = io_read_bytes();
        let t_it = Instant::now();
        if !cx.det {
            par_for(pool, ord.nb, ord.chunk(o.chunk), |tid, r| {
                let mut guard = tls[tid].lock().unwrap();
                let t = &mut *guard;
                for u in r.flat_map(|j| ord.block(j)) {
                    if cx.flag[u].load(Relaxed) == 0 {
                        continue;
                    }
                    if let Some(c) = comp {
                        let b = u / ord.ob;
                        tb[b / 64].fetch_or(1 << (b % 64), Relaxed);
                        let (a, e) = (c.boff.get(u) as usize / 4096, c.boff.get(u + 1) as usize / 4096);
                        for pg in a..=e {
                            tp[pg / 64].fetch_or(1 << (pg % 64), Relaxed);
                        }
                    }
                    cx.flag[u].store(0, Relaxed);
                    t.scanned += cx.g.deg(u) as u64;
                    t.processed += 1;
                    if let Some((c, d, gain)) = decide(cx, u, &mut t.tab) {
                        if cx.cap > 0 {
                            let s = cx.size[d as usize].fetch_add(1, Relaxed);
                            if s >= cx.cap {
                                cx.size[d as usize].fetch_sub(1, Relaxed);
                                continue;
                            }
                            cx.size[c as usize].fetch_sub(1, Relaxed);
                        }
                        let ku = cx.k[u];
                        fadd(&cx.sigma[c as usize], -ku);
                        fadd(&cx.sigma[d as usize], ku);
                        cx.comm[u].store(d, Relaxed);
                        mark_neighbors(cx, u);
                        t.gain += gain;
                        t.nmoves += 1;
                    }
                }
            });
        } else {
            {
                let shared = std::sync::RwLock::new((Vec::<Vec<u32>>::new(), Vec::<usize>::new(), 0usize));
                par_subrounds(
                    pool,
                    1usize << lb,
                    256,
                    SEQ_BELOW,
                    |b| {
                        let (cur, pre, total) = take_bucket(tls, b);
                        *shared.write().unwrap() = (cur, pre, b);
                        total
                    },
                    |tid, r| {
                        // phase 1: decisions against the frozen state of this sub-round
                        let sh = shared.read().unwrap();
                        let mut guard = tls[tid].lock().unwrap();
                        let t = &mut *guard;
                        for_range_in_lists(&sh.0, &sh.1, r, |u32_| {
                            let u = u32_ as usize;
                            cx.flag[u].store(0, Relaxed);
                            t.scanned += cx.g.deg(u) as u64;
                            t.processed += 1;
                            if let Some((_c, d, gain)) = decide(cx, u, &mut t.tab) {
                                let gfx = (gain * (1u64 << 50) as f64) as i64;
                                t.moves.push((u32_, d, gfx));
                                if cx.indep {
                                    cx.want[u].store(d, Relaxed);
                                }
                            }
                        });
                    },
                    |tid| {
                        // phase 2: commit. Deterministic reservation: the movers of one sub-round must form an
                        // independent set; the neighbour with the smaller hashed priority wins, losers retry.
                        let b = shared.read().unwrap().2;
                        let mut guard = tls[tid].lock().unwrap();
                        let Tls { moves, act, gain_fx, nmoves, .. } = &mut *guard;
                        let prio = |x: u32| x.wrapping_mul(0x85EB_CA6B);
                        for &(u32_, d, gfx) in moves.iter() {
                            let u = u32_ as usize;
                            if cx.indep {
                                let pu = prio(u32_);
                                let mut lose = false;
                                cx.g.for_edges(u, |v, _| {
                                    if v != u32_ && cx.want[v as usize].load(Relaxed) != NONE && prio(v) < pu {
                                        lose = true;
                                    }
                                });
                                if lose {
                                    if cx.flag[u].swap(1, Relaxed) == 0 {
                                        act[b].push(u32_);
                                    }
                                    continue;
                                }
                            }
                            let c = cx.comm[u].load(Relaxed);
                            let ku = cx.k[u];
                            fadd(&cx.sigma[c as usize], -ku);
                            fadd(&cx.sigma[d as usize], ku);
                            if use_size {
                                cx.size[c as usize].fetch_sub(1, Relaxed);
                                cx.size[d as usize].fetch_add(1, Relaxed);
                            }
                            cx.comm[u].store(d, Relaxed);
                            let _gg = giant_gate(cx.g.deg(u));
                            cx.g.for_edges(u, |v, _| {
                                let f = &cx.flag[v as usize];
                                if f.load(Relaxed) == 0 && f.swap(1, Relaxed) == 0 {
                                    act[bucket_of(v, lb, shift)].push(v);
                                }
                            });
                            *gain_fx += gfx;
                            *nmoves += 1;
                        }
                    },
                    |tid| {
                        let mut guard = tls[tid].lock().unwrap();
                        let t = &mut *guard;
                        if cx.indep {
                            for &(u, _, _) in &t.moves {
                                cx.want[u as usize].store(NONE, Relaxed);
                            }
                        }
                        t.moves.clear();
                    },
                );
            }
        }
        iters += 1;
        let mut gain = 0.0;
        let mut gain_fx = 0i64;
        let mut processed = 0;
        for t in tls {
            let t = t.lock().unwrap();
            gain += t.gain;
            gain_fx += t.gain_fx;
            processed += t.processed;
        }
        let g = if cx.det { gain_fx as f64 / (1u64 << 50) as f64 } else { gain };
        let io = io_read_bytes() - io0;
        let it_s = t_it.elapsed().as_secs_f64();
        if let Some(c) = comp {
            let cb: u64 = tb.iter().map(|x| x.swap(0, Relaxed).count_ones() as u64).sum();
            let cp: u64 = tp.iter().map(|x| x.swap(0, Relaxed).count_ones() as u64).sum();
            TRACE.lock().unwrap().push(format!(
                "{{\"n\":{},\"iter\":{},\"processed\":{},\"active_blocks\":{},\"blocks\":{},\"touched_pages\":{},\"store_pages\":{},\"read_bytes\":{},\"s\":{:.4}}}",
                n,
                iters,
                processed,
                cb,
                ord.nb,
                cp,
                c.data_len / 4096 + 1,
                io,
                it_s
            ));
        }
        if o.verbose {
            eprintln!("    iter {} processed {} gain {:.6} read {:.2} GB {:.3}s", iters, processed, g, io as f64 / 1e9, it_s);
        }
        if g <= tol || processed == 0 {
            break;
        }
    }
    for t in tls {
        let mut t = t.lock().unwrap();
        tot_moves += t.nmoves;
        tot_scanned += t.scanned;
        t.nmoves = 0;
        t.scanned = 0;
        t.act.iter_mut().for_each(|v| v.clear());
    }
    (iters, tot_moves, tot_scanned)
}

/// Leiden refinement: greedy constrained merging of isolated vertices inside their bound community.
fn refine(
    g: &Graph,
    k: &[f64],
    bound: &[AtomicU32],
    rcomm: &[AtomicU32],
    rsigma: &[AtomicU64],
    prop: &[AtomicU32],
    minv: f64,
    pool: &rayon::ThreadPool,
    tls: &[Mutex<Tls>],
    o: &Opts,
    sub_rounds: usize,
) {
    let n = g.n;
    let ord = Order::new(n, o);
    let lb = sub_rounds.max(1).trailing_zeros();
    let pick = |u: usize, tab: &mut Tab| -> Option<u32> {
        let ku = k[u];
        if rcomm[u].load(Relaxed) != u as u32 || fload(&rsigma[u]) != ku {
            return None;
        }
        let deg = g.deg(u);
        if deg == 0 {
            return None;
        }
        let b = bound[u].load(Relaxed);
        let _gg = giant_gate(deg);
        tab.begin(deg);
        let uu = u as u32;
        g.for_edges(u, |v, w| {
            if v != uu && bound[v as usize].load(Relaxed) == b {
                tab.add(rcomm[v as usize].load(Relaxed), w)
            }
        });
        let f = o.gamma * ku * minv;
        let mut best = NONE;
        let mut bestval = 0.0;
        for &s in &tab.used {
            let d = tab.keys[s as usize];
            if d == uu {
                continue;
            }
            let val = tab.vals[s as usize] - f * fload(&rsigma[d as usize]);
            if val > bestval || (val == bestval && best != NONE && tie_less(d, best, uu, o.tie_hash)) {
                best = d;
                bestval = val;
            }
        }
        tab.clear();
        if best == NONE {
            None
        } else {
            Some(best)
        }
    };
    for _ in 0..o.refine_iters {
        if !o.det {
            par_for(pool, ord.nb, ord.chunk(o.chunk), |tid, r| {
                let mut guard = tls[tid].lock().unwrap();
                let t = &mut *guard;
                for u in r.flat_map(|j| ord.block(j)) {
                    if let Some(d) = pick(u, &mut t.tab) {
                        let ku = k[u];
                        if rsigma[u].compare_exchange(ku.to_bits(), 0f64.to_bits(), Relaxed, Relaxed).is_err() {
                            continue;
                        }
                        let old = fadd(&rsigma[d as usize], ku);
                        if old == 0.0 {
                            fadd(&rsigma[d as usize], -ku);
                            fadd(&rsigma[u], ku);
                        } else {
                            rcomm[u].store(d, Relaxed);
                        }
                    }
                }
            });
        } else {
            bucketize(pool, tls, n, lb, 0);
            {
                let shared = std::sync::RwLock::new((Vec::<Vec<u32>>::new(), Vec::<usize>::new(), 0usize));
                par_subrounds(
                    pool,
                    1usize << lb,
                    256,
                    SEQ_BELOW,
                    |b| {
                        let (cur, pre, total) = take_bucket(tls, b);
                        *shared.write().unwrap() = (cur, pre, b);
                        total
                    },
                    |tid, r| {
                        let sh = shared.read().unwrap();
                        let mut guard = tls[tid].lock().unwrap();
                        let t = &mut *guard;
                        for_range_in_lists(&sh.0, &sh.1, r, |u| {
                            if let Some(d) = pick(u as usize, &mut t.tab) {
                                prop[u as usize].store(d, Relaxed);
                                t.moves.push((u, d, 0));
                            }
                        });
                    },
                    |tid| {
                        // accept iff the seed vertex of the target sub-community is not itself moving
                        let t = tls[tid].lock().unwrap();
                        for &(u, d, _) in &t.moves {
                            if prop[d as usize].load(Relaxed) == NONE {
                                let ku = k[u as usize];
                                rcomm[u as usize].store(d, Relaxed);
                                fadd(&rsigma[d as usize], ku);
                                fadd(&rsigma[u as usize], -ku);
                            }
                        }
                    },
                    |tid| {
                        let mut t = tls[tid].lock().unwrap();
                        for &(u, _, _) in &t.moves {
                            prop[u as usize].store(NONE, Relaxed);
                        }
                        t.moves.clear();
                    },
                );
            }
        }
    }
}

/// Aggregate g by labels `lab` (arbitrary labels in [0,n)). Returns (supergraph, k', dmap, C).
fn aggregate(g: &Graph, k: &[f64], lab: &[AtomicU32], pool: &rayon::ThreadPool, tls: &[Mutex<Tls>], o: &Opts) -> (Graph, Vec<f64>, Vec<u32>) {
    let n = g.n;
    let mut cnt_v = vec![0u32; n];
    let cnt = as_atomic_u32(&mut cnt_v);
    par_for(pool, n, 8192, |_, r| {
        for u in r {
            cnt[lab[u].load(Relaxed) as usize].fetch_add(1, Relaxed);
        }
    });
    // dense ids by increasing label (deterministic), member offsets
    let mut moff: Vec<usize> = Vec::new();
    let mut acc = 0usize;
    for l in 0..n {
        let c = cnt_v[l];
        if c > 0 {
            cnt_v[l] = moff.len() as u32;
            moff.push(acc);
            acc += c as usize;
        } else {
            cnt_v[l] = NONE;
        }
    }
    let c_new = moff.len();
    moff.push(acc);
    let newid = &cnt_v;
    let mut dmap = vec![0u32; n];
    let mut members = vec![0u32; n];
    {
        let dp = SyncPtr(dmap.as_mut_ptr());
        let mp = SyncPtr(members.as_mut_ptr());
        let pos: Vec<AtomicUsize> = moff[..c_new].iter().map(|&x| AtomicUsize::new(x)).collect();
        par_for(pool, n, 8192, |_, r| {
            for u in r {
                let c = newid[lab[u].load(Relaxed) as usize];
                unsafe {
                    dp.write(u, c);
                    mp.write(pos[c as usize].fetch_add(1, Relaxed), u as u32);
                }
            }
        });
        // sort member lists => fixed scan order independent of thread count
        par_for(pool, c_new, 512, |_, r| {
            for c in r {
                let s = unsafe { std::slice::from_raw_parts_mut(mp.p().add(moff[c]), moff[c + 1] - moff[c]) };
                if s.len() > 1 {
                    s.sort_unstable();
                }
            }
        });
    }
    // upper bound degree, k'
    let mut ub = vec![0usize; c_new + 1];
    let mut knew = vec![0f64; c_new];
    {
        let up = SyncPtr(ub.as_mut_ptr());
        let kp = SyncPtr(knew.as_mut_ptr());
        par_for(pool, c_new, 1024, |_, r| {
            for c in r {
                let mut d = 0usize;
                let mut kk = 0f64;
                for &u in &members[moff[c]..moff[c + 1]] {
                    d += g.deg(u as usize);
                    kk += k[u as usize];
                }
                unsafe {
                    up.write(c, d);
                    kp.write(c, kk);
                }
            }
        });
    }
    let scan = |c: usize, tab: &mut Tab| {
        tab.begin(ub[c].min(c_new));
        for &u in &members[moff[c]..moff[c + 1]] {
            g.for_edges(u as usize, |v, w| tab.add(unsafe { *dmap.get_unchecked(v as usize) }, w));
        }
    };
    let mut deg = vec![0u32; c_new];
    let degp = SyncPtr(deg.as_mut_ptr());
    let mut noff = vec![0u64; c_new + 1];
    let (tgt, wgt);
    if o.lowmem {
        // two-pass: count, then fill (no O(m) temporary)
        par_for(pool, c_new, 256, |tid, r| {
            let mut t = tls[tid].lock().unwrap();
            for c in r {
                scan(c, &mut t.tab);
                unsafe { degp.write(c, t.tab.used.len() as u32) };
                t.tab.clear();
                t.tab.shrink();
            }
        });
        let mut a = 0u64;
        for c in 0..c_new {
            noff[c] = a;
            a += deg[c] as u64;
        }
        noff[c_new] = a;
        let mut t_: Vec<u32> = uninit_vec(a as usize);
        let mut w_: Vec<W> = uninit_vec(a as usize);
        let tp = SyncPtr(t_.as_mut_ptr());
        let wp = SyncPtr(w_.as_mut_ptr());
        par_for(pool, c_new, 256, |tid, r| {
            let mut t = tls[tid].lock().unwrap();
            for c in r {
                scan(c, &mut t.tab);
                let base = noff[c] as usize;
                for (j, &s) in t.tab.used.iter().enumerate() {
                    unsafe {
                        tp.write(base + j, t.tab.keys[s as usize]);
                        wp.write(base + j, t.tab.vals[s as usize] as W);
                    }
                }
                t.tab.clear();
            }
        });
        tgt = t_;
        wgt = w_;
    } else {
        // holey CSR (upper-bound offsets), then compaction
        let mut a = 0usize;
        for c in 0..c_new {
            let d = ub[c];
            ub[c] = a;
            a += d.min(c_new);
        }
        ub[c_new] = a;
        let hoff = &ub;
        let mut ht: Vec<u32> = uninit_vec(a);
        let mut hw: Vec<W> = uninit_vec(a);
        let tp = SyncPtr(ht.as_mut_ptr());
        let wp = SyncPtr(hw.as_mut_ptr());
        par_for(pool, c_new, 256, |tid, r| {
            let mut t = tls[tid].lock().unwrap();
            for c in r {
                let need = hoff[c + 1] - hoff[c];
                t.tab.begin(need);
                for &u in &members[moff[c]..moff[c + 1]] {
                    g.for_edges(u as usize, |v, w| t.tab.add(unsafe { *dmap.get_unchecked(v as usize) }, w));
                }
                let base = hoff[c];
                for (j, &s) in t.tab.used.iter().enumerate() {
                    unsafe {
                        tp.write(base + j, t.tab.keys[s as usize]);
                        wp.write(base + j, t.tab.vals[s as usize] as W);
                    }
                }
                unsafe { degp.write(c, t.tab.used.len() as u32) };
                t.tab.clear();
                t.tab.shrink();
            }
        });
        let mut a2 = 0u64;
        for c in 0..c_new {
            noff[c] = a2;
            a2 += deg[c] as u64;
        }
        noff[c_new] = a2;
        let mut t_: Vec<u32> = uninit_vec(a2 as usize);
        let mut w_: Vec<W> = uninit_vec(a2 as usize);
        let tp2 = SyncPtr(t_.as_mut_ptr());
        let wp2 = SyncPtr(w_.as_mut_ptr());
        par_for(pool, c_new, 1024, |_, r| {
            for c in r {
                let d = deg[c] as usize;
                unsafe {
                    std::ptr::copy_nonoverlapping(tp.p().add(hoff[c]), tp2.p().add(noff[c] as usize), d);
                    std::ptr::copy_nonoverlapping(wp.p().add(hoff[c]), wp2.p().add(noff[c] as usize), d);
                }
            }
        });
        tgt = t_;
        wgt = w_;
    }
    (Graph::from_vecs(c_new, noff, tgt, Some(wgt)), knew, dmap)
}

/// m1-02 `--check-agg`: the member-order aggregation of the same labels must give the same
/// quotient graph: same dmap, same k' (bit for bit), and every list equal as a set of
/// (target, weight); plus the oracle's invariants community_sizes_sum_to_n and
/// quotient_weights_sum_to_m (here: sum of all weights = sum of k = 2m of level 0, exact).
fn check_agg_equal(g: &Graph, k: &[f64], lab: &[AtomicU32], pool: &rayon::ThreadPool, tls: &[Mutex<Tls>], o: &Opts, gq: &Graph, kq: &[f64], dm: &[u32]) {
    let t = Instant::now();
    let (g2, k2, d2) = aggregate(g, k, lab, pool, tls, &Opts { lowmem: true, ..o.clone() });
    assert_eq!(gq.n, g2.n, "check-agg: number of communities");
    assert!(dm == &d2[..], "check-agg: dmap differs");
    assert!(kq.iter().zip(&k2).all(|(a, b)| a.to_bits() == b.to_bits()), "check-agg: k' differs");
    assert_eq!(gq.entries(), g2.entries(), "check-agg: entries");
    let bad = AtomicUsize::new(0);
    par_for(pool, gq.n, 1024, |_, r| {
        let mut a: Vec<(u32, u64)> = Vec::new();
        let mut b: Vec<(u32, u64)> = Vec::new();
        for c in r {
            a.clear();
            b.clear();
            gq.for_edges(c, |v, w| a.push((v, w.to_bits())));
            g2.for_edges(c, |v, w| b.push((v, w.to_bits())));
            b.sort_unstable();
            if a != b {
                bad.fetch_add(1, Relaxed);
            }
        }
    });
    assert_eq!(bad.load(Relaxed), 0, "check-agg: lists differ");
    // invariants
    let mut sizes = vec![0u64; gq.n];
    for &c in dm {
        sizes[c as usize] += 1;
    }
    assert_eq!(sizes.iter().sum::<u64>(), g.n as u64, "community_sizes_sum_to_n");
    let wsum = par_sum(pool, gq.n, |c| {
        let mut s = 0.0;
        gq.for_edges(c, |_, w| s += w);
        s
    });
    let ksum = par_sum(pool, g.n, |u| k[u]);
    assert_eq!(wsum, ksum, "quotient_weights_sum_to_m (sum of weights = 2m)");
    eprintln!("    check-agg: equal to member-order aggregation ({} communities, {} entries, sum w = {}) in {:.2}s", gq.n, gq.entries(), wsum, t.elapsed().as_secs_f64());
    CHECKS.fetch_add(1, Relaxed);
}
/// m1-02: gives the pages of [p + from, p + len) back to the OS (anonymous memory reads as zero
/// afterwards). Only whole pages inside the range; the caller never reads the range again
/// before writing it.
fn dontneed(p: *const u8, len: usize, from: usize) {
    let a = (p as usize + from).next_multiple_of(4096);
    let e = (p as usize + len) & !4095;
    if e > a {
        unsafe { libc::madvise(a as *mut libc::c_void, e - a, libc::MADV_DONTNEED) };
    }
}
static CHECKS: AtomicUsize = AtomicUsize::new(0);
static TRACE: Mutex<Vec<String>> = Mutex::new(Vec::new());
fn proc_io(key: &str) -> u64 {
    std::fs::read_to_string("/proc/self/io").ok().and_then(|s| s.lines().find_map(|l| l.strip_prefix(key).and_then(|x| x.trim().parse().ok()))).unwrap_or(0)
}
/// Bytes read from / written to the machine's whole-disk devices (/proc/diskstats, 512-byte
/// sectors). Machine-wide: meaningful only when nothing else uses the disk. Used instead of
/// /proc/self/io, which does not count page faults on a mapped file.
fn diskstats() -> (u64, u64) {
    let s = std::fs::read_to_string("/proc/diskstats").unwrap_or_default();
    let mut r = 0;
    let mut w = 0;
    for l in s.lines() {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() < 10 {
            continue;
        }
        let name = f[2];
        let whole = (name.starts_with("nvme") && !name.contains('p')) || ((name.starts_with("sd") || name.starts_with("vd")) && !name.ends_with(|c: char| c.is_ascii_digit()));
        if whole {
            r += f[5].parse::<u64>().unwrap_or(0) * 512;
            w += f[9].parse::<u64>().unwrap_or(0) * 512;
        }
    }
    (r, w)
}
pub fn io_read_bytes() -> u64 {
    diskstats().0
}
pub fn io_write_bytes() -> u64 {
    diskstats().1
}
/// What /proc/self/io says (read() and write() traffic of this process; no mmap faults).
pub fn proc_io_rw() -> (u64, u64) {
    (proc_io("read_bytes:"), proc_io("write_bytes:"))
}

pub struct RunOut {
    levels: Vec<Vec<u32>>,
    graphs: Vec<(Graph, Vec<f64>)>, // quotient graph of level i communities (if keep_graphs)
    stats: Vec<PassStat>,
    total_s: f64,
    state_bytes_per_node: f64,
    split_extra: usize,
}

pub fn run(g0: &Graph, pool: &rayon::ThreadPool, o: &Opts) -> RunOut {
    let t_start = Instant::now();
    let nthreads = pool.current_num_threads();
    let tls = new_tls(nthreads);
    let n0 = g0.n;
    // m1-02: the per-node state, reserved before it is allocated
    let state_alloc = {
        let mut b = 4 + 8 + 1 + 8; // comm, sigma, flag, k
        if o.det || o.cap > 0 {
            b += 4 // size
        }
        if o.leiden {
            b += 4 + 8 + 4 // rcomm, rsigma, tmp
        }
        if o.det {
            b += 4 // prop
        }
        b as u64 * n0 as u64
    };
    // m1-02 (out of core, one iteration): the state follows the level. sigma, rsigma and flag
    // are not needed during aggregation and are given back to the OS (MADV_DONTNEED) before it;
    // after it, the tails beyond the next level's size of the other arrays are given back; level
    // 0's strengths k are given back after level 0. Each part is reserved for the level's size.
    let shrink_state = o.ooc && o.iterations == 1;
    let tmp_per_node = 8 + 1 + if o.leiden { 8 } else { 0 }; // sigma, flag, rsigma
    let keep_per_node = (state_alloc / n0.max(1) as u64) - 8 - tmp_per_node; // without k
    let mut r_keep = res("leiden state: per-node arrays kept through aggregation", keep_per_node * n0 as u64);
    let mut r_tmp: Option<Res> = None;
    let mut r_k = Some(res("leiden state: level-0 strengths", 8 * n0 as u64));
    // state arrays sized for level 0, reused for coarser levels
    let mut comm_v = vec![0u32; n0];
    let mut sigma_v = vec![0u64; n0];
    let mut size_v = vec![0u32; if o.det || o.cap > 0 { n0 } else { 0 }];
    let mut flag_v = vec![0u8; n0];
    let mut rcomm_v = vec![0u32; if o.leiden { n0 } else { 0 }];
    let mut rsigma_v = vec![0u64; if o.leiden { n0 } else { 0 }];
    let mut prop_v = vec![NONE; if o.det { n0 } else { 0 }];
    let mut tmp_v = vec![0u32; if o.leiden { n0 } else { 0 }];
    let comm = as_atomic_u32(&mut comm_v);
    let sigma = as_atomic_u64(&mut sigma_v);
    let size = as_atomic_u32(&mut size_v);
    let flag = as_atomic_u8(&mut flag_v);
    let rcomm = as_atomic_u32(&mut rcomm_v);
    let rsigma = as_atomic_u64(&mut rsigma_v);
    let prop = as_atomic_u32(&mut prop_v);
    let tmp = as_atomic_u32(&mut tmp_v);
    let use_size = o.det || o.cap > 0;
    let mut state_bytes = 4 + 8 + 1 + 8; // comm, sigma, flag, k
    if use_size {
        state_bytes += 4
    }
    if o.leiden {
        state_bytes += 4 + 8 + 4
    }
    if o.det {
        state_bytes += 4
    }

    let mut k: Vec<f64> = vec![0.0; n0];
    {
        let kp = SyncPtr(k.as_mut_ptr());
        par_for(pool, n0, 8192, |_, r| {
            for u in r {
                let mut s = 0.0;
                g0.for_edges(u, |_, w| s += w);
                unsafe { kp.write(u, s) };
            }
        });
    }
    let m2: f64 = par_sum(pool, n0, |u| k[u]);
    let minv = 1.0 / m2;

    let mut owned: Vec<(Graph, Vec<f64>)> = Vec::new();
    let mut levels: Vec<Vec<u32>> = Vec::new();
    let mut stats: Vec<PassStat> = Vec::new();
    let mut tol = o.tol;
    let mut cur_owned: Option<(Graph, Vec<f64>)> = None;
    let mut next_bound: Option<Vec<u32>> = None; // Leiden: initial labels of supervertices
    let mut split_extra = 0usize;
    let mut iter = 0;
    let mut cur_cap;
    // m1-02: reservations that live with `levels` and with `cur_owned`
    let mut levels_res: Vec<Res> = Vec::new();
    let mut cur_res: Vec<Res> = Vec::new();
    loop {
    cur_cap = o.cap;
    owned.clear();
    levels.clear();
    levels_res.clear();
    cur_owned = None;
    cur_res.clear();
    tol = o.tol * 0.1f64.powi(iter as i32);
    for pass in 0..o.max_passes {
        let (g, kk): (&Graph, &[f64]) = match &cur_owned {
            Some((g, kk)) => (g, kk),
            None => (g0, &k),
        };
        let n = g.n;
        let mut st = PassStat { n, entries: g.entries(), ..Default::default() };
        r_tmp = None;
        r_tmp = Some(res("leiden state: sigma, flag, rsigma of the level", tmp_per_node * n as u64));
        let io_p0 = io_read_bytes();
        let t0 = Instant::now();
        // init
        match &next_bound {
            None => par_for(pool, n, 8192, |_, r| {
                for u in r {
                    comm[u].store(u as u32, Relaxed);
                    sigma[u].store(kk[u].to_bits(), Relaxed);
                    flag[u].store(1, Relaxed);
                    if use_size {
                        size[u].store(1, Relaxed)
                    }
                }
            }),
            Some(b) => {
                par_for(pool, n, 8192, |_, r| {
                    for u in r {
                        sigma[u].store(0f64.to_bits(), Relaxed);
                        if use_size {
                            size[u].store(0, Relaxed)
                        }
                    }
                });
                par_for(pool, n, 8192, |_, r| {
                    for u in r {
                        let c = b[u];
                        comm[u].store(c, Relaxed);
                        flag[u].store(1, Relaxed);
                        fadd(&sigma[c as usize], kk[u]);
                        if use_size {
                            size[c as usize].fetch_add(1, Relaxed);
                        }
                    }
                });
            }
        }
        let cx = Cx { g, k: kk, comm: &comm[..n], sigma: &sigma[..n], size, flag: &flag[..n], minv, gamma: o.gamma, cap: cur_cap, det: o.det, tie_hash: o.tie_hash, want: prop, indep: o.indep };
        let sub = if o.sub_rounds > 0 {
            o.sub_rounds
        } else {
            // auto: depends on the graph only (never on the thread count): sub-rounds of >= ~16k vertices
            (n / 16384).next_power_of_two().clamp(256, 1024)
        };
        let _r_scratch = res("local moving and refinement: per-thread tables and decode buffers", scratch_bytes(g, pool.current_num_threads()));
        for t in tls.iter() {
            t.lock().unwrap().tab = Tab::new(); // m1-02: nothing grown on another level survives
        }
        let (iters, moves, scanned) = local_move(&cx, pool, &tls, o, tol, sub, if pass == 0 { BUCKET_SHIFT.load(Relaxed) } else { 0 });
        st.iters = iters;
        st.moves = moves;
        st.scanned = scanned;
        st.t_lm = t0.elapsed().as_secs_f64();
        sample_anon();
        let t1 = Instant::now();
        let lab: &[AtomicU32] = if o.leiden {
            par_for(pool, n, 8192, |_, r| {
                for u in r {
                    rcomm[u].store(u as u32, Relaxed);
                    rsigma[u].store(kk[u].to_bits(), Relaxed);
                }
            });
            refine(g, kk, &comm[..n], &rcomm[..n], &rsigma[..n], prop, minv, pool, &tls, o, sub);
            &rcomm[..n]
        } else {
            &comm[..n]
        };
        st.t_ref = t1.elapsed().as_secs_f64();
        st.io_lm_read = io_read_bytes() - io_p0;
        if o.ooc {
            for t in tls.iter() {
                t.lock().unwrap().tab = Tab::new(); // the aggregation's own tables come next
            }
        }
        let (io_r2, io_w2) = (io_read_bytes(), io_write_bytes());
        let t2 = Instant::now();
        // count communities of lab
        let c_cnt = par_sum_u(pool, n, |u| {
            // a label l is "present"; count labels whose representative (min-id member) ... use owner test below
            let _ = u;
            0
        });
        let _ = c_cnt;
        par_for(pool, n, 8192, |_, r| {
            for u in r {
                flag[u].store(0, Relaxed)
            }
        });
        par_for(pool, n, 8192, |_, r| {
            for u in r {
                let f = &flag[lab[u].load(Relaxed) as usize];
                if f.load(Relaxed) == 0 {
                    f.store(1, Relaxed)
                }
            }
        });
        let c_new = par_sum_u(pool, n, |u| flag[u].load(Relaxed) as u64) as usize;
        st.comms = c_new;
        let converged = (moves == 0 && !(iter > 0 && pass == 0)) || pass + 1 == o.max_passes || (cur_cap == 0 && (c_new as f64) > o.agg_tol * (n as f64));
        if o.verbose {
            eprintln!(
                "pass {} n {} entries {} iters {} moves {} -> {} comms  lm {:.3}s ref {:.3}s",
                pass, n, st.entries, iters, moves, c_new, st.t_lm, st.t_ref
            );
        }
        if converged {
            // final level: communities = comm (bound communities for Leiden)
            if o.leiden && o.split_last {
                // split-last: connected components inside final communities on the current (refined-vertex) graph
                let labv: Vec<u32> = (0..n).map(|u| comm[u].load(Relaxed)).collect();
                let before = count_distinct(&labv, n);
                let par = uf_components(g, &labv, pool);
                for u in 0..n {
                    comm[u].store(uf_find(&par, u as u32), Relaxed);
                }
                let labv2: Vec<u32> = (0..n).map(|u| comm[u].load(Relaxed)).collect();
                split_extra = count_distinct(&labv2, n) - before;
            }
            let labv: Vec<u32> = (0..n).map(|u| comm[u].load(Relaxed)).collect();
            let (dm, c_top) = dense_renumber(&labv, n);
            st.comms = c_top;
            if c_top == n && !levels.is_empty() {
                // nothing merged in this pass: the previous level is already the top (avoid an identity level)
                st.t_agg = t2.elapsed().as_secs_f64();
                stats.push(st);
                if o.keep_graphs {
                    if let Some(c) = cur_owned.take() {
                        owned.push(c)
                    }
                }
                break;
            }
            if o.keep_graphs {
                let dma: Vec<AtomicU32> = dm.iter().map(|&x| AtomicU32::new(x)).collect();
                let (gq, kq, dm2) = aggregate(g, kk, &dma, pool, &tls, o);
                debug_assert!(dm2 == dm);
                levels.push(dm2);
                st.t_agg = t2.elapsed().as_secs_f64();
                stats.push(st);
                if let Some(c) = cur_owned.take() {
                    owned.push(c)
                }
                owned.push((gq, kq));
            } else {
                levels.push(dm);
                st.t_agg = t2.elapsed().as_secs_f64();
                stats.push(st);
            }
            break;
        }
        if cur_cap > 0 && (c_new as f64) > o.cap_stall * (n as f64) {
            // capped coarsening stalled (only a few giants keep absorbing): finish with uncapped passes
            cur_cap = 0;
        }
        sample_anon();
        if shrink_state {
            // not read again before the next level writes them
            dontneed(sigma.as_ptr() as *const u8, sigma.len() * 8, 0);
            dontneed(flag.as_ptr() as *const u8, flag.len(), 0);
            dontneed(rsigma.as_ptr() as *const u8, rsigma.len() * 8, 0);
            r_tmp = None;
        }
        // m1-02: sequential aggregation out of core, member-order aggregation in RAM
        let (gq, kq, dm, mut agg_res) = if o.ooc {
            let so = seqagg::aggregate_seq(g, kk, lab, pool, &o.spill, o.hier_dir.as_deref());
            st.agg = so.st.json();
            (so.g, so.knew, so.dmap, so.res)
        } else {
            let (gq, kq, dm) = aggregate(g, kk, lab, pool, &tls, o);
            // recorded after the fact (the in-RAM reference has no budget): output arrays only
            let r = vec![
                res("quotient graph: raw arrays", sz::<u64>(gq.n + 1) + sz::<u32>(gq.entries()) + sz::<W>(gq.entries())),
                res("level state: k' of the next level", sz::<f64>(kq.len())),
                res("aggregation: dmap", sz::<u32>(dm.len())),
            ];
            (gq, kq, dm, r)
        };
        if o.check_agg {
            check_agg_equal(g, kk, lab, pool, &tls, o, &gq, &kq, &dm);
        }
        levels_res.push(agg_res.pop().unwrap()); // dmap lives in `levels`
        sample_anon();
        next_bound = None;
        if o.leiden {
            // initial labels of supervertices: bound community, relabeled to min refined id inside it
            par_for(pool, n, 8192, |_, r| {
                for u in r {
                    tmp[u].store(NONE, Relaxed)
                }
            });
            par_for(pool, n, 8192, |_, r| {
                for u in r {
                    tmp[comm[u].load(Relaxed) as usize].fetch_min(dm[u], Relaxed);
                }
            });
            let mut nb = vec![0u32; gq.n];
            let np = SyncPtr(nb.as_mut_ptr());
            par_for(pool, n, 8192, |_, r| {
                for u in r {
                    unsafe { np.write_racy(dm[u] as usize, tmp[comm[u].load(Relaxed) as usize].load(Relaxed)) };
                }
            });
            next_bound = Some(nb);
        }
        if shrink_state {
            // the next levels touch only the first gq.n entries (one iteration: no restart at level 0)
            let m = gq.n;
            dontneed(comm.as_ptr() as *const u8, comm.len() * 4, m * 4);
            dontneed(size.as_ptr() as *const u8, size.len() * 4, m * 4);
            dontneed(rcomm.as_ptr() as *const u8, rcomm.len() * 4, m * 4);
            dontneed(tmp.as_ptr() as *const u8, tmp.len() * 4, m * 4);
            dontneed(prop.as_ptr() as *const u8, prop.len() * 4, m * 4);
            r_keep.shrink_to(keep_per_node * m as u64);
            if pass == 0 {
                // level 0's strengths are not read again (kk of the next level is k')
                dontneed(k.as_ptr() as *const u8, k.len() * 8, 0);
                r_k = None;
            }
        }
        levels.push(dm);
        st.t_agg = t2.elapsed().as_secs_f64();
        st.io_agg_read = io_read_bytes() - io_r2;
        st.io_agg_write = io_write_bytes() - io_w2;
        st.acct_used = crate::acct::acct().used();
        st.anon_kb = rss_kb().1;
        if o.verbose {
            eprintln!("    agg {:.3}s {} accountant {:.2} GiB anon {:.2} GiB", st.t_agg, st.agg, st.acct_used as f64 / 1073741824.0, st.anon_kb as f64 / 1048576.0);
        }
        stats.push(st);
        if o.keep_graphs {
            if let Some(c) = cur_owned.take() {
                owned.push(c)
            }
        }
        cur_owned = None;
        cur_res.clear(); // the previous quotient graph is gone
        cur_owned = Some((gq, kq));
        cur_res = agg_res;
        tol /= o.tol_drop;
    }
    iter += 1;
    if iter >= o.iterations {
        break;
    }
    // next Leiden/Louvain iteration starts from the current flat partition
    next_bound = Some(flat_membership(&levels));
    }
    RunOut { levels, graphs: owned, stats, total_s: t_start.elapsed().as_secs_f64(), state_bytes_per_node: state_bytes as f64, split_extra }
}

fn count_distinct(lab: &[u32], space: usize) -> usize {
    let mut f = vec![false; space];
    let mut c = 0;
    for &l in lab {
        if !f[l as usize] {
            f[l as usize] = true;
            c += 1;
        }
    }
    c
}

fn dense_renumber(lab: &[u32], space: usize) -> (Vec<u32>, usize) {
    let mut id = vec![NONE; space];
    for &l in lab {
        id[l as usize] = 0;
    }
    let mut c = 0u32;
    for x in id.iter_mut() {
        if *x == 0 {
            *x = c;
            c += 1;
        }
    }
    (lab.iter().map(|&l| id[l as usize]).collect(), c as usize)
}

// ---------------------------------------------------------------- union-find (components inside labels)
fn uf_find(p: &[AtomicU32], mut x: u32) -> u32 {
    loop {
        let px = p[x as usize].load(Relaxed);
        if px == x {
            return x;
        }
        let gp = p[px as usize].load(Relaxed);
        if gp != px {
            let _ = p[x as usize].compare_exchange(px, gp, Relaxed, Relaxed);
        }
        x = px;
    }
}
fn uf_union(p: &[AtomicU32], mut a: u32, mut b: u32) {
    loop {
        a = uf_find(p, a);
        b = uf_find(p, b);
        if a == b {
            return;
        }
        if a < b {
            std::mem::swap(&mut a, &mut b);
        }
        if p[a as usize].compare_exchange(a, b, SeqCst, SeqCst).is_ok() {
            return;
        }
    }
}
fn uf_components(g: &Graph, lab: &[u32], pool: &rayon::ThreadPool) -> Vec<AtomicU32> {
    let p: Vec<AtomicU32> = (0..g.n as u32).map(AtomicU32::new).collect();
    par_for(pool, g.n, 4096, |_, r| {
        for u in r {
            let lu = lab[u];
            g.for_edges(u, |v, _| {
                if (v as usize) < u && lab[v as usize] == lu {
                    uf_union(&p, u as u32, v)
                }
            });
        }
    });
    p
}

pub struct Eval {
    modularity: f64,
    communities: usize,
    disconnected: usize,
    largest: usize,
}
/// Common evaluation routine (used for our output and all reference partitions).
pub fn eval(g: &Graph, memb: &[u32], gamma: f64, pool: &rayon::ThreadPool, check_conn: bool) -> Eval {
    let n = g.n;
    let space = memb.iter().copied().max().map(|x| x as usize + 1).unwrap_or(0);
    let _r_eval = res("evaluation arrays", sz::<f64>(n) + sz::<f64>(space) + 2 * sz::<u32>(space) + if check_conn { sz::<u32>(n) } else { 0 });
    let mut tot = vec![0f64; space];
    let mut cnt = vec![0u32; space];
    let acc = Mutex::new((0f64, 0f64));
    let kv: Vec<f64> = {
        let mut k = vec![0f64; n];
        let kp = SyncPtr(k.as_mut_ptr());
        par_for(pool, n, 8192, |_, r| {
            let mut si = 0f64;
            let mut sm = 0f64;
            for u in r {
                let mut s = 0.0;
                let cu = memb[u];
                g.for_edges(u, |v, w| {
                    s += w;
                    if memb[v as usize] == cu {
                        si += w
                    }
                });
                sm += s;
                unsafe { kp.write(u, s) };
            }
            let mut a = acc.lock().unwrap();
            a.0 += si;
            a.1 += sm;
        });
        k
    };
    for u in 0..n {
        tot[memb[u] as usize] += kv[u];
        cnt[memb[u] as usize] += 1;
    }
    let (sin, m2) = *acc.lock().unwrap();
    let mut q = sin / m2;
    for &t in &tot {
        q -= gamma * (t / m2) * (t / m2);
    }
    let communities = cnt.iter().filter(|&&c| c > 0).count();
    let largest = cnt.iter().copied().max().unwrap_or(0) as usize;
    let mut disconnected = 0;
    if check_conn {
        let p = uf_components(g, memb, pool);
        let mut roots = vec![0u32; space];
        for u in 0..n {
            if p[u].load(Relaxed) == u as u32 {
                roots[memb[u] as usize] += 1;
            }
        }
        disconnected = roots.iter().filter(|&&r| r > 1).count();
    }
    Eval { modularity: q, communities, disconnected, largest }
}

/// Level compaction: keep a level only if it shrinks the previously kept one by >= min_ratio (the top is always kept).
/// Skipped levels are composed into the next kept one, so nesting and exact aggregates are preserved.
fn compact_levels(out: &mut RunOut, n0: usize, min_ratio: f64) {
    let levels = std::mem::take(&mut out.levels);
    let nl = levels.len();
    let mut graphs: Vec<Option<(Graph, Vec<f64>)>> = std::mem::take(&mut out.graphs).into_iter().map(Some).collect();
    let has_graphs = graphs.len() == nl;
    let mut kept: Vec<Vec<u32>> = Vec::new();
    let mut kept_graphs = Vec::new();
    let mut kept_sizes: Vec<usize> = Vec::new();
    let mut pending: Option<Vec<u32>> = None;
    for (i, l) in levels.into_iter().enumerate() {
        let size = l.iter().copied().max().unwrap() as usize + 1;
        let composed = match pending.take() {
            None => l,
            Some(mut p) => {
                for x in p.iter_mut() {
                    *x = l[*x as usize];
                }
                p
            }
        };
        let prev = kept_sizes.last().copied().unwrap_or(n0);
        if i + 1 == nl || prev as f64 >= min_ratio * size as f64 {
            if i + 1 == nl && kept.len() >= 1 && (prev as f64) < min_ratio * size as f64 {
                // the top is too close to the previously kept level: replace that level by the top
                let below = kept.pop().unwrap();
                kept_sizes.pop();
                if has_graphs {
                    kept_graphs.pop();
                }
                let mut b = below;
                for x in b.iter_mut() {
                    *x = composed[*x as usize];
                }
                kept.push(b);
            } else {
                kept.push(composed);
            }
            kept_sizes.push(size);
            if has_graphs {
                kept_graphs.push(graphs[i].take().unwrap());
            }
        } else {
            pending = Some(composed);
        }
    }
    out.levels = kept;
    out.graphs = kept_graphs;
}

fn flat_membership(levels: &[Vec<u32>]) -> Vec<u32> {
    let mut m = levels[0].clone();
    for l in &levels[1..] {
        for x in m.iter_mut() {
            *x = l[*x as usize];
        }
    }
    m
}

fn fnv(h: &mut u64, data: &[u32]) {
    for &x in data {
        for b in x.to_le_bytes() {
            *h ^= b as u64;
            *h = h.wrapping_mul(0x100000001b3);
        }
    }
}

fn fanout_stats(level: &[u32], c: usize) -> String {
    let mut f = vec![0u32; c];
    for &p in level {
        f[p as usize] += 1;
    }
    let mut s = f.clone();
    s.sort_unstable();
    let q = |p: f64| s[((s.len() - 1) as f64 * p) as usize];
    let ones = s.iter().filter(|&&x| x == 1).count();
    let in_band: u64 = f.iter().filter(|&&x| (4..=64).contains(&x)).map(|&x| x as u64).sum();
    let over: u64 = f.iter().filter(|&&x| x > 64).map(|&x| x as u64).sum();
    format!(
        "{{\"parents\":{},\"children\":{},\"mean\":{:.2},\"p50\":{},\"p90\":{},\"p99\":{},\"max\":{},\"parents_with_1_child\":{},\"children_in_parents_4_64\":{:.4},\"children_in_parents_gt64\":{:.4}}}",
        c,
        level.len(),
        level.len() as f64 / c as f64,
        q(0.5),
        q(0.9),
        q(0.99),
        s[s.len() - 1],
        ones,
        in_band as f64 / level.len() as f64,
        over as f64 / level.len() as f64
    )
}

fn write_hier(dir: &str, name: &str, g0: &Graph, out: &RunOut, q: f64, q_top: f64, notes: &str) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = format!("{}.tmp.{}", dir.trim_end_matches('/'), std::process::id());
    std::fs::create_dir_all(&tmp)?;
    let sizes: Vec<usize> = out.levels.iter().map(|l| l.iter().copied().max().unwrap() as usize + 1).collect();
    // node counts per community per level
    let mut nodes: Vec<u64> = vec![1; g0.n];
    let mut fan = Vec::new();
    for (i, l) in out.levels.iter().enumerate() {
        write_slice(&format!("{}/level_{}.u32", tmp, i), l)?;
        let mut nn = vec![0u64; sizes[i]];
        for (j, &p) in l.iter().enumerate() {
            nn[p as usize] += nodes[j];
        }
        write_slice(&format!("{}/level_{}.nodes.u64", tmp, i), &nn)?;
        nodes = nn;
        fan.push(fanout_stats(l, sizes[i]));
        if i < out.graphs.len() {
            let (gq, kq) = &out.graphs[i];
            assert_eq!(gq.n, sizes[i]);
            // exact aggregates (W=f64: integer-exact up to 2^53)
            let vol: Vec<u64> = kq.iter().map(|&x| x as u64).collect();
            let mut internal = vec![0u64; gq.n];
            for c in 0..gq.n {
                gq.for_edges(c, |v, w| {
                    if v as usize == c {
                        internal[c] = (w / 2.0) as u64
                    }
                });
            }
            write_slice(&format!("{}/level_{}.volume.u64", tmp, i), &vol)?;
            write_slice(&format!("{}/level_{}.internal_edges.u64", tmp, i), &internal)?;
            write_slice(&format!("{}/quot_{}.offsets.u64", tmp, i), gq.off())?;
            write_slice(&format!("{}/quot_{}.targets.u32", tmp, i), gq.tgt())?;
            let wv: Vec<f64> = gq.wgt().unwrap().iter().map(|&x| x as f64).collect();
            write_slice(&format!("{}/quot_{}.weights.f64", tmp, i), &wv)?;
        }
    }
    let mut f = std::fs::File::create(format!("{}/hier.json", tmp))?;
    writeln!(
        f,
        "{{\"graph\": \"{}\", \"n\": {}, \"levels\": {:?}, \"modularity_level0\": {:.6}, \"modularity_top\": {:.6}, \"fanout\": [{}], \"files\": \"level_k.u32 (parent map), level_k.nodes.u64, level_k.volume.u64 (sum of degrees), level_k.internal_edges.u64 (undirected edges inside), quot_k.{{offsets.u64,targets.u32,weights.f64}} = quotient graph CSR of level-k communities; self-loop weight = 2*internal edges, off-diagonal weight = number of edges between the two communities\", \"notes\": \"{}\"}}",
        name,
        g0.n,
        sizes,
        q,
        q_top,
        fan.join(","),
        notes
    )?;
    drop(f);
    let old = format!("{}.old.{}", dir.trim_end_matches('/'), std::process::id());
    if std::path::Path::new(dir).exists() {
        std::fs::rename(dir, &old)?;
    }
    std::fs::rename(&tmp, dir)?;
    let _ = std::fs::remove_dir_all(&old);
    Ok(())
}

pub fn main_leiden(args: Vec<String>) {
    // m1-02: a fixed mmap threshold (glibc's dynamic one rises to 32 MiB after the first large
    // free, after which the 50-100 KB bucket chunks come from the heap and freed ones stay in RSS:
    // measured 8.5 GiB anonymous against 2.8 GiB reserved after Friendster's level 1).
    unsafe {
        libc::mallopt(libc::M_MMAP_THRESHOLD, 32 << 10);
    }
    let (disk_r0, disk_w0) = diskstats();
    let get = |k: &str| -> Option<String> { args.iter().position(|a| a == k).and_then(|i| args.get(i + 1).cloned()) };
    let has = |k: &str| args.iter().any(|a| a == k);
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("");
    let gdir = get("--graph").or_else(|| get("--store")).expect("--graph DIR or --store DIR");
    let threads: usize = get("--threads").map(|s| s.parse().unwrap()).unwrap_or(1);
    // m1-02: the anonymous-memory budget. Out of core: --budget-gb, else a fraction of the
    // cgroup's memory.max (the hard limit the run is under) minus 64 MiB for what the
    // accountant does not see; the rest of the limit is left to the page cache.
    let ooc = has("--ooc");
    let cg_max = crate::adj::cgroup_memory_max();
    let budget: u64 = if let Some(b) = get("--budget-gb") {
        (b.parse::<f64>().unwrap() * 1073741824.0) as u64
    } else if ooc {
        let frac: f64 = get("--budget-frac").map(|s| s.parse().unwrap()).unwrap_or(0.80);
        let lim = cg_max.expect("--ooc without --budget-gb needs a cgroup memory.max (run under systemd-run -p MemoryMax=...)");
        ((lim as f64 * frac) as u64).saturating_sub(64 << 20)
    } else {
        u64::MAX
    };
    crate::acct::init(budget);
    let sampler_stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let stop = sampler_stop.clone();
        std::thread::spawn(move || {
            while !stop.load(Relaxed) {
                sample_anon();
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        });
    }
    let pin = has("--pin");
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .start_handler(move |i| {
            if pin {
                pin_thread(i)
            }
        })
        .build()
        .unwrap();
    let gamma: f64 = get("--gamma").map(|s| s.parse().unwrap()).unwrap_or(1.0);
    let loadavg_start = crate::adj::loadavg();
    let config = get("--config").unwrap_or_else(|| "raw".into());
    let mem = get("--mem").unwrap_or_else(|| "mmap".into());
    let t_load = Instant::now();
    // raw + mmap is the lab loader (lab/b2-leiden); the other combinations are new in this experiment
    let g0 = pool.install(|| match (config.as_str(), mem.as_str()) {
        ("raw", "mmap") => Graph::load(&gdir, !has("--no-populate")),
        ("raw", "heap") => Graph::from_raw(crate::adj::Raw::load(std::path::Path::new(&gdir), crate::adj::Mem::Heap).expect("load raw")),
        ("comp", m) => Graph::from_comp(crate::store::Comp::open(std::path::Path::new(&get("--store").expect("--store DIR")), crate::adj::Mem::parse(m)).expect("open store")),
        ("decoded", m) => {
            let c = crate::store::Comp::open(std::path::Path::new(&get("--store").expect("--store DIR")), crate::adj::Mem::parse(m)).expect("open store");
            Graph::from_raw(c.decode_to_raw().0)
        }
        (c, m) => panic!("bad --config {c} / --mem {m}"),
    });
    let load_s = t_load.elapsed().as_secs_f64();
    // m1-02: the level-0 graph in the accountant (anonymous if read into the heap, else file-backed)
    let _r_g0: Vec<Res> = match g0.comp() {
        Some(c) => {
            if let Some(a) = get("--advise") {
                c.advise(match a.as_str() {
                    "random" => memmap2::Advice::Random,
                    "sequential" => memmap2::Advice::Sequential,
                    "normal" => memmap2::Advice::Normal,
                    x => panic!("bad --advise {x}"),
                });
            }
            let mut v = vec![res("level-0 store: offsets index", c.index_ram_bytes())];
            if mem == "heap" {
                v.push(res("level-0 store: records in heap", c.data_len as u64));
            } else {
                v.push(crate::acct::file_backed("level-0 store: mapped records", c.data_len as u64));
            }
            v
        }
        None if config == "raw" && mem == "mmap" => vec![crate::acct::file_backed("level-0 graph: mapped raw arrays", sz::<u64>(g0.n + 1) + sz::<u32>(g0.entries()))],
        None => vec![res("level-0 graph: raw arrays", sz::<u64>(g0.n + 1) + sz::<u32>(g0.entries()))],
    };
    if ooc && config != "comp" {
        panic!("--ooc runs on the compressed store (--config comp --store DIR)");
    }
    let gname = get("--name").unwrap_or_else(|| gdir.trim_end_matches('/').rsplit('/').next().unwrap().to_string());
    if cmd == "eval" {
        let memb: Vec<u32> = read_vec(&get("--memb").expect("--memb FILE"));
        assert_eq!(memb.len(), g0.n);
        let t = Instant::now();
        let e = eval(&g0, &memb, gamma, &pool, true);
        println!(
            "{{\"graph\":\"{}\",\"memb\":\"{}\",\"modularity\":{:.6},\"communities\":{},\"disconnected\":{},\"largest\":{},\"eval_s\":{:.3}}}",
            gname,
            get("--memb").unwrap(),
            e.modularity,
            e.communities,
            e.disconnected,
            e.largest,
            t.elapsed().as_secs_f64()
        );
        return;
    }
    assert_eq!(cmd, "run", "usage: b2 run|eval --graph DIR ...");
    let algo = get("--algo").unwrap_or_else(|| "leiden".into());
    let o = Opts {
        leiden: algo == "leiden",
        det: has("--det"),
        sub_rounds: get("--sub-rounds").map(|s| s.parse().unwrap()).unwrap_or(0),
        gamma,
        cap: get("--cap").map(|s| s.parse().unwrap()).unwrap_or(0),
        max_iters: get("--max-iters").map(|s| s.parse().unwrap()).unwrap_or(20),
        max_passes: get("--max-passes").map(|s| s.parse().unwrap()).unwrap_or(10),
        tol: get("--tol").map(|s| s.parse().unwrap()).unwrap_or(0.01),
        tol_drop: get("--tol-drop").map(|s| s.parse().unwrap()).unwrap_or(10.0),
        agg_tol: get("--agg-tol").map(|s| s.parse().unwrap()).unwrap_or(0.8),
        lowmem: has("--lowmem"),
        keep_graphs: get("--out-hier").is_some() && !ooc,
        split_last: !has("--no-split"),
        refine_iters: get("--refine-iters").map(|s| s.parse().unwrap()).unwrap_or(1),
        chunk: get("--chunk").map(|s| s.parse().unwrap()).unwrap_or(2048),
        verbose: has("--verbose"),
        order_block: get("--order-block").map(|s| s.parse().unwrap()).unwrap_or(64),
        permute: !has("--natural"),
        tie_hash: !has("--tie-min"),
        indep: has("--indep"),
        iterations: get("--iterations").map(|s| s.parse().unwrap()).unwrap_or(1),
        cap_stall: get("--cap-stall").map(|s| s.parse().unwrap()).unwrap_or(0.5),
        ooc,
        spill: get("--spill").unwrap_or_else(|| "spill".into()),
        check_agg: has("--check-agg"),
        order_window: get("--order-window").map(|s| s.parse().unwrap()).unwrap_or(0),
        trace_io: has("--trace-io"),
        hier_dir: if ooc { get("--out-hier").map(|d| format!("{}.quot.{}", d.trim_end_matches('/'), std::process::id())) } else { None },
    };
    BUCKET_SHIFT.store(get("--bucket-shift").map(|s| s.parse().unwrap()).unwrap_or(4), Relaxed);
    let repeat: usize = get("--repeat").map(|s| s.parse().unwrap()).unwrap_or(1);
    let mut times = Vec::new();
    let mut best: Option<RunOut> = None;
    for _ in 0..repeat {
        let out = run(&g0, &pool, &o);
        times.push(out.total_s);
        if best.as_ref().map(|b| out.total_s < b.total_s).unwrap_or(true) {
            best = Some(out);
        }
    }
    let mut out = best.unwrap();
    let min_ratio: f64 = get("--min-ratio").map(|s| s.parse().unwrap()).unwrap_or(1.0);
    if min_ratio > 1.0 {
        compact_levels(&mut out, g0.n, min_ratio);
    }
    let _r_levels = res("output: levels", out.levels.iter().map(|l| sz::<u32>(l.len())).sum());
    let _r_memb = res("output: flat membership", sz::<u32>(g0.n));
    let memb = flat_membership(&out.levels);
    let mut h = 0xcbf29ce484222325u64;
    for l in &out.levels {
        fnv(&mut h, l);
    }
    let mut hflat = 0xcbf29ce484222325u64;
    fnv(&mut hflat, &memb);
    let do_eval = !has("--no-eval");
    let t_eval = Instant::now();
    let e = if do_eval { eval(&g0, &memb, gamma, &pool, true) } else { Eval { modularity: f64::NAN, communities: 0, disconnected: 0, largest: 0 } };
    let eval_s = t_eval.elapsed().as_secs_f64();
    let q0 = if do_eval && (get("--out-hier").is_some() || has("--eval-level0")) { eval(&g0, &out.levels[0], gamma, &pool, false).modularity } else { f64::NAN };
    let sizes: Vec<usize> = out.levels.iter().map(|l| l.iter().copied().max().unwrap() as usize + 1).collect();
    let (rss_peak, rss_anon_peak) = rss_kb();
    let scanned0 = out.stats[0].scanned;
    let t_lm: f64 = out.stats.iter().map(|s| s.t_lm).sum();
    let t_ref: f64 = out.stats.iter().map(|s| s.t_ref).sum();
    let t_agg: f64 = out.stats.iter().map(|s| s.t_agg).sum();
    let passes: Vec<String> = out
        .stats
        .iter()
        .map(|s| {
            format!(
                "{{\"n\":{},\"entries\":{},\"iters\":{},\"moves\":{},\"comms\":{},\"lm_s\":{:.4},\"refine_s\":{:.4},\"agg_s\":{:.4},\"edge_passes\":{:.2},\"seq_agg\":{},\"acct_used_bytes\":{},\"anon_kb\":{},\"lm_ref_read_bytes\":{},\"agg_read_bytes\":{},\"agg_write_bytes\":{}}}",
                s.n,
                s.entries,
                s.iters,
                s.moves,
                s.comms,
                s.t_lm,
                s.t_ref,
                s.t_agg,
                s.scanned as f64 / s.entries.max(1) as f64,
                if s.agg.is_empty() { "null" } else { &s.agg },
                s.acct_used,
                s.anon_kb,
                s.io_lm_read,
                s.io_agg_read,
                s.io_agg_write
            )
        })
        .collect();
    let fan: Vec<String> = out.levels.iter().zip(&sizes).map(|(l, &c)| fanout_stats(l, c)).collect();
    let json = format!(
        "{{\"graph\":\"{}\",\"n\":{},\"entries\":{},\"algo\":\"{}\",\"det\":{},\"sub_rounds\":{},\"threads\":{},\"pinned\":{},\"gamma\":{},\"iterations\":{},\"cap\":{},\"weight_bits\":{},\"time_s\":{:.4},\"times_s\":{:?},\"edges_per_s\":{:.0},\"lm_s\":{:.4},\"refine_s\":{:.4},\"agg_s\":{:.4},\"load_s\":{:.3},\"modularity\":{:.6},\"modularity_level0\":{:.6},\"communities\":{},\"disconnected\":{},\"largest\":{},\"split_last_added\":{},\"n_passes\":{},\"total_iters\":{},\"level_sizes\":{:?},\"level0_edge_passes\":{:.2},\"hash_levels\":\"{:016x}\",\"hash_flat\":\"{:016x}\",\"state_bytes_per_node\":{},\"rss_peak_kb\":{},\"rss_anon_peak_kb\":{},\"passes\":[{}],\"fanout\":[{}]}}",
        gname,
        g0.n,
        g0.entries(),
        algo,
        o.det,
        o.sub_rounds,
        threads,
        pin,
        gamma,
        o.iterations,
        o.cap,
        std::mem::size_of::<W>() * 8,
        out.total_s,
        times,
        g0.entries() as f64 / out.total_s,
        t_lm,
        t_ref,
        t_agg,
        load_s,
        e.modularity,
        q0,
        e.communities,
        e.disconnected,
        e.largest,
        out.split_extra,
        out.stats.len(),
        out.stats.iter().map(|s| s.iters).sum::<usize>(),
        sizes,
        scanned0 as f64 / g0.entries() as f64,
        h,
        hflat,
        out.state_bytes_per_node,
        rss_peak,
        PEAK_ANON_KB.load(Relaxed).max(rss_anon_peak),
        passes.join(","),
        fan.join(",")
    );
    // cgroup accounting (meaningful under systemd-run --scope -p MemoryMax=...)
    let cg = std::fs::read_to_string("/proc/self/cgroup").ok().and_then(|c| c.lines().find_map(|l| l.strip_prefix("0::").map(|s| s.to_string()))).unwrap_or_default();
    let cgread = |f: &str| std::fs::read_to_string(format!("/sys/fs/cgroup{}/{}", cg, f)).unwrap_or_default();
    let stat = cgread("memory.stat");
    let sget = |k: &str| stat.lines().find(|l| l.starts_with(k)).and_then(|l| l.split_whitespace().nth(1)).unwrap_or("0").to_string();
    let json = format!(
        "{},\"cgroup\":{{\"memory_peak\":{},\"memory_max\":\"{}\",\"anon_now\":{},\"file_now\":{},\"pgmajfault\":{},\"workingset_refault_file\":{}}},\"lowmem\":{},\"populate\":{}}}",
        &json[..json.len() - 1],
        cgread("memory.peak").trim().parse::<u64>().unwrap_or(0),
        cgread("memory.max").trim(),
        sget("anon "),
        sget("file "),
        sget("pgmajfault "),
        sget("workingset_refault_file "),
        o.lowmem,
        !has("--no-populate")
    );
    let json = format!(
        "{},\"config\":\"{}\",\"mem\":\"{}\",\"decoder\":\"{}\",\"loadavg_1m_start\":{},\"loadavg_1m_end\":{},\"anon_huge_gb\":{:.3},\"busy_cpus_before\":{},\"mem_available_gb_before\":{},\"cmd\":\"leiden\"}}",
        &json[..json.len() - 1],
        config,
        mem,
        crate::store::decoder_name(),
        loadavg_start,
        crate::adj::loadavg(),
        crate::adj::gb(crate::adj::anon_huge()),
        std::env::var("M113_BUSY").ok().and_then(|b| b.parse::<f64>().ok()).unwrap_or(-1.0),
        std::env::var("M113_MEMAVAIL").ok().and_then(|b| b.parse::<f64>().ok()).unwrap_or(-1.0)
    );
    // m1-02: out-of-core fields. Phase totals over all levels: level 0 alone is passes[0].
    sampler_stop.store(true, Relaxed);
    sample_anon();
    let level0 = &out.stats[0];
    let json = format!(
        "{},\"ooc\":{},\"budget_bytes\":{},\"cgroup_memory_max\":{},\"accountant\":{},\"eval_s\":{:.4},\"time_with_eval_s\":{:.4},\"level0_lm_s\":{:.4},\"level0_refine_s\":{:.4},\"level0_agg_s\":{:.4},\"upper_levels_s\":{:.4},\"check_agg_levels\":{},\"rss_anon_peak_sampled_kb\":{},\"order_window\":{},\"permute\":{},\"read_bytes\":{},\"write_bytes\":{},\"trace\":[{}],\"status\":\"ok\"}}",
        &json[..json.len() - 1],
        o.ooc,
        if budget == u64::MAX { "null".to_string() } else { budget.to_string() },
        cg_max.map(|x| x.to_string()).unwrap_or_else(|| "null".into()),
        crate::acct::acct().report(),
        eval_s,
        out.total_s + eval_s,
        level0.t_lm,
        level0.t_ref,
        level0.t_agg,
        out.stats[1..].iter().map(|s| s.t_lm + s.t_ref + s.t_agg).sum::<f64>(),
        CHECKS.load(Relaxed),
        PEAK_ANON_KB.load(Relaxed),
        o.order_window,
        o.permute,
        io_read_bytes() - disk_r0,
        io_write_bytes() - disk_w0,
        TRACE.lock().unwrap().join(",")
    );
    // m1-02: valid JSON for the harness (the lab printed NaN for values not computed)
    let json = json.replace(":NaN", ":null").replace(":inf", ":null");
    println!("{}", json);
    if let Some(p) = get("--json") {
        use std::io::Write;
        let mut f = std::fs::File::options().create(true).append(true).open(&p).expect("json out");
        writeln!(f, "{}", json).unwrap();
    }
    if let Some(p) = get("--out-memb") {
        write_slice(&p, &memb).unwrap();
    }
    if let Some(dir) = get("--out-hier") {
        let mut notes = get("--notes").unwrap_or_default();
        if o.hier_dir.is_some() {
            notes.push_str(" | out of core: no quot_k raw arrays, volume or internal-edge files; quotients/quot_p = weighted compressed store of the quotient graph built by aggregation pass p (before level compaction)");
        }
        write_hier(&dir, &gname, &g0, &out, q0, e.modularity, &notes).unwrap();
        if let Some(q) = &o.hier_dir {
            if std::path::Path::new(q).exists() {
                std::fs::rename(q, std::path::Path::new(&dir).join("quotients")).unwrap();
            }
        }
    }
}
