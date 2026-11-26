// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `scatterab`: interleaved A/B of issue #189 on synthetic community labels.
//!
//! The input is what the lab Leiden's aggregation has at level 0 after compacting the
//! labels (`lab/b2-leiden/src/main.rs`, `aggregate`): a community id per node and the
//! member offsets `moff` (prefix sums of the community sizes). The output is the member
//! lists: nodes grouped by community, increasing node id inside each.
//!
//! Single indirect writes (building the member lists), three variants:
//! - `lab`: the lab code, copied: positions from `fetch_add` on per-community counters,
//!   written through a raw pointer, then each list sorted through a raw slice;
//! - `safe`: `par::scatter` of packed (community, node) pairs into at most 4,096
//!   buckets of community ranges, `par::for_each_range_mut` sorting each bucket, then
//!   the node ids extracted with `par::for_each_chunk_mut`;
//! - `sort`: the same pairs sorted at once with rayon's `par_sort_unstable` (the facade
//!   has no sort; this is the issue's "parallel sort" alternative), then extracted.
//!
//! Ranged indirect writes alone (sorting each community's list of an unsorted member
//! array, the second half of `lab`): `lab-ranged` through raw slices against
//! `safe-ranged` through `par::for_each_range_mut`. Each round copies the same unsorted
//! array first; only the sort is timed.
//!
//! Every round runs all variants, the order rotating each round. One JSON line on
//! standard output: median seconds per variant, the median over rounds of each variant's
//! time over its baseline's in the same round, and the hash of each variant's output,
//! plus the hashes of `safe` and `sort` at 1, 4 and the requested thread count. Each
//! variant also reports its peak heap above what was held before it started, output
//! included (`peak_heap_bytes`, from a counting allocator; the largest over rounds).
//! The ranged variants sort a copy made before that point, which is not counted.

use std::alloc::{GlobalAlloc, Layout, System};
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

use everygraph_core::{Memory, par};
use rayon::prelude::*;

/// Heap bytes held now and their peak, for the memory of each variant.
static HELD: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

struct Counting;

fn grew(bytes: usize) {
    let now = HELD.fetch_add(bytes, Relaxed) + bytes;
    PEAK.fetch_max(now, Relaxed);
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            grew(layout.size());
        }
        p
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            grew(layout.size());
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        unsafe { System.dealloc(p, layout) };
        HELD.fetch_sub(layout.size(), Relaxed);
    }
    unsafe fn realloc(&self, p: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let q = unsafe { System.realloc(p, layout, size) };
        if !q.is_null() {
            HELD.fetch_sub(layout.size(), Relaxed);
            grew(size);
        }
        q
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// The lab's raw-pointer wrapper (`lab/b2-leiden/src/util.rs`), reduced to what is used.
#[derive(Clone, Copy)]
struct SyncPtr<T>(*mut T);
unsafe impl<T> Send for SyncPtr<T> {}
unsafe impl<T> Sync for SyncPtr<T> {}
impl<T> SyncPtr<T> {
    fn p(self) -> *mut T {
        self.0
    }
    /// Safety: index in bounds, no concurrent access to the same index.
    unsafe fn write(self, i: usize, v: T) {
        unsafe { self.0.add(i).write(v) }
    }
}

/// The lab's `par_for` (`lab/b2-leiden/src/util.rs`): chunks claimed with `fetch_add`.
fn par_for<F: Fn(std::ops::Range<usize>) + Sync>(pool: &rayon::ThreadPool, n: usize, chunk: usize, f: F) {
    if n == 0 {
        return;
    }
    if n <= chunk || pool.current_num_threads() == 1 {
        let mut s = 0;
        while s < n {
            f(s..(s + chunk).min(n));
            s += chunk;
        }
        return;
    }
    let next = AtomicUsize::new(0);
    pool.broadcast(|_| loop {
        let s = next.fetch_add(chunk, Relaxed);
        if s >= n {
            break;
        }
        f(s..(s + chunk).min(n));
    });
}

struct Input {
    comm: Vec<u32>,
    moff: Vec<usize>,
}

/// Community ids with sizes skewed by `skew` (1 = uniform; larger puts more nodes in
/// the low ids), from a fixed generator.
fn input(nodes: usize, communities: usize, skew: f64, seed: u64) -> Input {
    let mut state = seed;
    let comm: Vec<u32> = (0..nodes)
        .map(|_| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let x = (state >> 11) as f64 / (1u64 << 53) as f64;
            ((x.powf(skew) * communities as f64) as usize).min(communities - 1) as u32
        })
        .collect();
    let mut moff = vec![0usize; communities + 1];
    for &c in &comm {
        moff[c as usize + 1] += 1;
    }
    for c in 0..communities {
        moff[c + 1] += moff[c];
    }
    Input { comm, moff }
}

/// The lab's member scatter and sort (`aggregate`, lines 623–646), unchanged but for
/// names. With `sort == false` it stops after the scatter (the unsorted input of the
/// ranged A/B).
fn lab(pool: &rayon::ThreadPool, inp: &Input, sort: bool) -> Vec<u32> {
    let n = inp.comm.len();
    let c_new = inp.moff.len() - 1;
    let mut members = vec![0u32; n];
    let mp = SyncPtr(members.as_mut_ptr());
    let pos: Vec<AtomicUsize> = inp.moff[..c_new].iter().map(|&x| AtomicUsize::new(x)).collect();
    par_for(pool, n, 8192, |r| {
        for u in r {
            let c = inp.comm[u];
            unsafe { mp.write(pos[c as usize].fetch_add(1, Relaxed), u as u32) };
        }
    });
    if sort {
        lab_ranged(pool, &inp.moff, &mut members);
    }
    members
}

/// The lab's per-community sort through raw slices.
fn lab_ranged(pool: &rayon::ThreadPool, moff: &[usize], members: &mut [u32]) {
    let mp = SyncPtr(members.as_mut_ptr());
    par_for(pool, moff.len() - 1, 512, |r| {
        for c in r {
            let s = unsafe { std::slice::from_raw_parts_mut(mp.p().add(moff[c]), moff[c + 1] - moff[c]) };
            if s.len() > 1 {
                s.sort_unstable();
            }
        }
    });
}

/// The same through the safe ranged helper.
fn safe_ranged(moff: &[usize], members: &mut [u32]) {
    par::for_each_range_mut(members, moff, NonZeroUsize::new(512).unwrap(), |_, s| {
        if s.len() > 1 {
            s.sort_unstable();
        }
    });
}

const BLOCK: NonZeroUsize = NonZeroUsize::new(1 << 16).unwrap();

fn pack(inp: &Input, u: usize) -> u64 {
    (u64::from(inp.comm[u]) << 32) | u as u64
}

fn extract(pairs: &[u64]) -> Vec<u32> {
    let mut members = vec![0u32; pairs.len()];
    par::for_each_chunk_mut(&mut members, BLOCK, |start, chunk| {
        for (j, m) in chunk.iter_mut().enumerate() {
            *m = pairs[start + j] as u32;
        }
    });
    members
}

/// Safe: scatter pairs into buckets of community ranges, sort each bucket, extract.
fn safe(memory: &Memory, inp: &Input) -> Vec<u32> {
    let n = inp.comm.len();
    let communities = inp.moff.len() - 1;
    let shift = (communities.div_ceil(4096)).next_power_of_two().trailing_zeros();
    let buckets = NonZeroUsize::new(((communities - 1) >> shift) + 1).unwrap();
    let mut pairs = vec![0u64; n];
    let block = BLOCK.get();
    let emit = |k: usize, sink: &mut par::Sink<'_, &mut [u64]>| {
        for u in k * block..n.min((k + 1) * block) {
            sink.push((inp.comm[u] >> shift) as usize, pack(inp, u));
        }
    };
    let offsets = par::scatter(memory, "member scatter", n.div_ceil(block), buckets, emit, [&mut pairs[..]])
        .expect("unlimited budget");
    par::for_each_range_mut(&mut pairs[..], &offsets, NonZeroUsize::MIN, |_, s| s.sort_unstable());
    extract(&pairs)
}

/// The parallel sort of (community, node) pairs.
fn sort(inp: &Input) -> Vec<u32> {
    let mut pairs = vec![0u64; inp.comm.len()];
    par::for_each_chunk_mut(&mut pairs, BLOCK, |start, chunk| {
        for (j, p) in chunk.iter_mut().enumerate() {
            *p = pack(inp, start + j);
        }
    });
    pairs.par_sort_unstable();
    extract(&pairs)
}

fn hash(v: &[u32]) -> u64 {
    v.iter().fold(0xcbf2_9ce4_8422_2325, |h, &x| (h ^ u64::from(x)).wrapping_mul(0x100_0000_01b3))
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn arg<T: std::str::FromStr>(args: &[String], name: &str, default: T) -> T {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .map(|v| v.parse().ok().expect("bad argument"))
        .unwrap_or(default)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let nodes: usize = arg(&args, "--nodes", 4_847_571);
    let communities: usize = arg(&args, "--communities", nodes / 8);
    let skew: f64 = arg(&args, "--skew", 1.0);
    let threads: usize = arg(&args, "--threads", 16);
    let rounds: usize = arg(&args, "--rounds", 15);
    let inp = input(nodes, communities, skew, 189);
    let memory = Memory::unlimited();
    let pool = |t| rayon::ThreadPoolBuilder::new().num_threads(t).build().unwrap();
    let main_pool = pool(threads);

    // Determinism at 1, 4 and `threads` threads.
    let mut thread_hashes = Vec::new();
    for t in [1, 4, threads] {
        let p = pool(t);
        thread_hashes.push((t, p.install(|| (hash(&safe(&memory, &inp)), hash(&sort(&inp))))));
    }

    let unsorted = lab(&main_pool, &inp, false);
    let names = ["lab", "safe", "sort", "lab-ranged", "safe-ranged"];
    let mut times: Vec<Vec<f64>> = vec![Vec::new(); names.len()];
    let mut hashes = [0u64; 5];
    let mut peaks = [0usize; 5];
    for round in 0..rounds {
        for k in 0..names.len() {
            let v = (round + k) % names.len();
            let mut work = unsorted.clone();
            let held = HELD.load(Relaxed);
            PEAK.store(held, Relaxed);
            let start = Instant::now();
            let out = main_pool.install(|| match v {
                0 => lab(&main_pool, &inp, true),
                1 => safe(&memory, &inp),
                2 => sort(&inp),
                3 => {
                    lab_ranged(&main_pool, &inp.moff, &mut work);
                    std::mem::take(&mut work)
                }
                _ => {
                    safe_ranged(&inp.moff, &mut work);
                    std::mem::take(&mut work)
                }
            });
            times[v].push(start.elapsed().as_secs_f64());
            peaks[v] = peaks[v].max(PEAK.load(Relaxed) - held);
            hashes[v] = hash(&out);
        }
    }
    let ratio = |v: usize, base: usize| {
        let mut r: Vec<f64> = times[v].iter().zip(&times[base]).map(|(a, b)| a / b).collect();
        median(&mut r)
    };
    let ratios = [1.0, ratio(1, 0), ratio(2, 0), 1.0, ratio(4, 3)];
    let fields: Vec<String> = names
        .iter()
        .enumerate()
        .map(|(v, name)| {
            let mut t = times[v].clone();
            format!(
                "\"{name}\":{{\"median_s\":{:.6},\"ratio_to_baseline\":{:.4},\"peak_heap_bytes\":{},\"hash\":\"{:016x}\"}}",
                median(&mut t),
                ratios[v],
                peaks[v],
                hashes[v]
            )
        })
        .collect();
    let threads_field: Vec<String> = thread_hashes
        .iter()
        .map(|(t, (a, b))| format!("\"{t}\":{{\"safe\":\"{a:016x}\",\"sort\":\"{b:016x}\"}}"))
        .collect();
    println!(
        "{{\"nodes\":{nodes},\"communities\":{communities},\"skew\":{skew},\"threads\":{threads},\"rounds\":{rounds},{},\"hashes_by_threads\":{{{}}}}}",
        fields.join(","),
        threads_field.join(",")
    );
    let same = hashes[..3].iter().all(|&h| h == hashes[0]) && hashes[3] == hashes[0] && hashes[4] == hashes[0];
    let stable = thread_hashes.iter().all(|(_, (a, b))| *a == hashes[0] && *b == hashes[0]);
    assert!(same && stable, "outputs differ");
}
