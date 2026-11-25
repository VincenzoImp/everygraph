//! lab/b2-leiden/src/util.rs with one change: `Graph` can also wrap the compressed store
//! (configuration B), in which case `for_edges` decodes the list into a per-thread buffer.
use crate::adj::Adj;
use crate::leiden::W;
use std::ops::Range;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering::Acquire, Ordering::Relaxed, Ordering::SeqCst};
use std::sync::Mutex;

pub struct SyncPtr<T>(pub *mut T);
unsafe impl<T> Send for SyncPtr<T> {}
unsafe impl<T> Sync for SyncPtr<T> {}
impl<T> SyncPtr<T> {
    #[inline(always)]
    pub fn p(&self) -> *mut T {
        self.0
    }
    /// # Safety: index in bounds, no concurrent access to the same index.
    #[inline(always)]
    pub unsafe fn write(&self, i: usize, v: T) {
        self.0.add(i).write(v)
    }
    /// Same value may be written concurrently by several threads (benign).
    #[inline(always)]
    pub unsafe fn write_racy(&self, i: usize, v: T) {
        self.0.add(i).write_volatile(v)
    }
}

pub fn uninit_vec<T>(n: usize) -> Vec<T> {
    let mut v = Vec::with_capacity(n);
    #[allow(clippy::uninit_vec)]
    unsafe {
        v.set_len(n)
    };
    v
}

pub fn as_atomic_u32(v: &mut [u32]) -> &[AtomicU32] {
    unsafe { &*(v as *mut [u32] as *const [AtomicU32]) }
}
pub fn as_atomic_u64(v: &mut [u64]) -> &[AtomicU64] {
    unsafe { &*(v as *mut [u64] as *const [AtomicU64]) }
}
pub fn as_atomic_u8(v: &mut [u8]) -> &[AtomicU8] {
    unsafe { &*(v as *mut [u8] as *const [AtomicU8]) }
}

#[inline(always)]
pub fn fload(a: &AtomicU64) -> f64 {
    f64::from_bits(a.load(Relaxed))
}
/// atomic f64 add, returns the old value
#[inline(always)]
pub fn fadd(a: &AtomicU64, x: f64) -> f64 {
    let mut cur = a.load(Relaxed);
    loop {
        let new = (f64::from_bits(cur) + x).to_bits();
        match a.compare_exchange_weak(cur, new, Relaxed, Relaxed) {
            Ok(_) => return f64::from_bits(cur),
            Err(c) => cur = c,
        }
    }
}

// ---------------------------------------------------------------- parallel helpers
pub fn par_for<F: Fn(usize, Range<usize>) + Sync>(pool: &rayon::ThreadPool, n: usize, chunk: usize, f: F) {
    if n == 0 {
        return;
    }
    if n <= chunk || pool.current_num_threads() == 1 {
        let mut s = 0;
        while s < n {
            f(0, s..(s + chunk).min(n));
            s += chunk;
        }
        return;
    }
    let next = AtomicUsize::new(0);
    pool.broadcast(|ctx| {
        let tid = ctx.index();
        loop {
            let s = next.fetch_add(chunk, Relaxed);
            if s >= n {
                break;
            }
            f(tid, s..(s + chunk).min(n));
        }
    });
}

fn spin_barrier(arrived: &AtomicUsize, target: usize) {
    arrived.fetch_add(1, SeqCst);
    let mut i = 0u32;
    while arrived.load(Acquire) < target {
        i += 1;
        if i < 256 {
            std::hint::spin_loop()
        } else {
            std::thread::yield_now()
        }
    }
}

/// Phase 1: dynamic chunks over 0..n. Barrier. Phase 2: once per thread. Barrier. Phase 3: once per thread.
pub fn par_phases<F1, F2, F3>(pool: &rayon::ThreadPool, n: usize, chunk: usize, force_seq: bool, f1: F1, f2: F2, f3: F3)
where
    F1: Fn(usize, Range<usize>) + Sync,
    F2: Fn(usize) + Sync,
    F3: Fn(usize) + Sync,
{
    if n == 0 {
        return;
    }
    let t = pool.current_num_threads();
    if force_seq || n <= chunk || t == 1 {
        let mut s = 0;
        while s < n {
            f1(0, s..(s + chunk).min(n));
            s += chunk;
        }
        f2(0);
        f3(0);
        return;
    }
    let next = AtomicUsize::new(0);
    let b1 = AtomicUsize::new(0);
    let b2 = AtomicUsize::new(0);
    pool.broadcast(|ctx| {
        let tid = ctx.index();
        loop {
            let s = next.fetch_add(chunk, Relaxed);
            if s >= n {
                break;
            }
            f1(tid, s..(s + chunk).min(n));
        }
        spin_barrier(&b1, t);
        f2(tid);
        spin_barrier(&b2, t);
        f3(tid);
    });
}

/// Reusable barrier: spins briefly, then blocks (robust when cores are oversubscribed).
/// The last thread to arrive runs `leader` before everybody is released.
pub struct SpinBarrier {
    count: AtomicUsize,
    gen: AtomicUsize,
    n: usize,
    m: Mutex<()>,
    cv: std::sync::Condvar,
}
impl SpinBarrier {
    pub fn new(n: usize) -> Self {
        SpinBarrier { count: AtomicUsize::new(0), gen: AtomicUsize::new(0), n, m: Mutex::new(()), cv: std::sync::Condvar::new() }
    }
    pub fn wait<F: FnOnce()>(&self, leader: F) {
        let g = self.gen.load(SeqCst);
        if self.count.fetch_add(1, SeqCst) + 1 == self.n {
            leader();
            self.count.store(0, SeqCst);
            {
                let _l = self.m.lock().unwrap();
                self.gen.fetch_add(1, SeqCst);
            }
            self.cv.notify_all();
        } else {
            for _ in 0..3000 {
                if self.gen.load(Acquire) != g {
                    return;
                }
                std::hint::spin_loop();
            }
            let mut l = self.m.lock().unwrap();
            while self.gen.load(SeqCst) == g {
                l = self.cv.wait(l).unwrap();
            }
        }
    }
}

/// Deterministic sub-rounds inside ONE broadcast (threads stay hot between sub-rounds).
/// For each sub-round b: `prepare(b)` (run by one thread while the others are parked) returns the number of items.
/// Small sub-rounds (< seq_below items) are executed entirely by that thread; otherwise all threads run
/// f1 on dynamic chunks | barrier | f2 once per thread | barrier | f3 once per thread.
/// The result must not (and does not) depend on which path is taken.
pub fn par_subrounds<P, F1, F2, F3>(pool: &rayon::ThreadPool, nsub: usize, chunk: usize, seq_below: usize, prepare: P, f1: F1, f2: F2, f3: F3)
where
    P: Fn(usize) -> usize + Sync,
    F1: Fn(usize, Range<usize>) + Sync,
    F2: Fn(usize) + Sync,
    F3: Fn(usize) + Sync,
{
    let t = pool.current_num_threads();
    if t == 1 {
        for b in 0..nsub {
            let tot = prepare(b);
            if tot > 0 {
                f1(0, 0..tot);
                f2(0);
                f3(0);
            }
        }
        return;
    }
    let barrier = SpinBarrier::new(t);
    let next = AtomicUsize::new(0);
    let total = AtomicUsize::new(0);
    let cur_b = AtomicUsize::new(0);
    let done = std::sync::atomic::AtomicBool::new(false);
    pool.broadcast(|ctx| {
        let tid = ctx.index();
        loop {
            barrier.wait(|| loop {
                let b = cur_b.load(SeqCst);
                if b >= nsub {
                    done.store(true, SeqCst);
                    break;
                }
                cur_b.store(b + 1, SeqCst);
                let tot = prepare(b);
                if tot == 0 {
                    continue;
                }
                if tot < seq_below {
                    f1(tid, 0..tot);
                    f2(tid);
                    f3(tid);
                    continue;
                }
                total.store(tot, SeqCst);
                next.store(0, SeqCst);
                break;
            });
            if done.load(SeqCst) {
                break;
            }
            let tot = total.load(SeqCst);
            loop {
                let s = next.fetch_add(chunk, Relaxed);
                if s >= tot {
                    break;
                }
                f1(tid, s..(s + chunk).min(tot));
            }
            barrier.wait(|| {});
            f2(tid);
            barrier.wait(|| {});
            f3(tid);
        }
    });
}

pub fn par_sum<F: Fn(usize) -> f64 + Sync>(pool: &rayon::ThreadPool, n: usize, f: F) -> f64 {
    // deterministic: fixed blocks, summed in block order
    let block = 1 << 16;
    let nb = n.div_ceil(block);
    let mut parts = vec![0f64; nb];
    let pp = SyncPtr(parts.as_mut_ptr());
    par_for(pool, nb, 1, |_, r| {
        for b in r {
            let mut s = 0.0;
            for u in b * block..((b + 1) * block).min(n) {
                s += f(u);
            }
            unsafe { pp.write(b, s) };
        }
    });
    parts.iter().sum()
}
pub fn par_sum_u<F: Fn(usize) -> u64 + Sync>(pool: &rayon::ThreadPool, n: usize, f: F) -> u64 {
    let acc = Mutex::new(0u64);
    par_for(pool, n, 1 << 14, |_, r| {
        let mut s = 0;
        for u in r {
            s += f(u);
        }
        *acc.lock().unwrap() += s;
    });
    let v = *acc.lock().unwrap();
    v
}

pub fn pin_thread(i: usize) {
    unsafe {
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        libc::CPU_SET(i % 32, &mut set);
        libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &set);
    }
}

pub fn rss_kb() -> (u64, u64) {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let grab = |k: &str| s.lines().find(|l| l.starts_with(k)).and_then(|l| l.split_whitespace().nth(1)).and_then(|x| x.parse().ok()).unwrap_or(0);
    (grab("VmHWM:"), grab("RssAnon:"))
}

// ---------------------------------------------------------------- graph
enum Own {
    Map(#[allow(dead_code)] memmap2::Mmap, #[allow(dead_code)] memmap2::Mmap),
    Vecs(#[allow(dead_code)] Vec<u64>, #[allow(dead_code)] Vec<u32>),
    Raw(#[allow(dead_code)] crate::adj::Raw),
    None,
}
pub struct Graph {
    pub n: usize,
    off: *const u64,
    tgt: *const u32,
    wgt: Option<Vec<W>>,
    entries: usize,
    /// Some: the level-0 graph lives in the compressed store (unweighted); off/tgt are null.
    comp: Option<crate::store::Comp>,
    _own: Own,
}
unsafe impl Send for Graph {}
unsafe impl Sync for Graph {}

thread_local! { static LCUR: std::cell::UnsafeCell<crate::store::CompCur> = std::cell::UnsafeCell::new(crate::store::CompCur::new()); }

impl Graph {
    /// The lab loader: both files mmapped (page cache, 4 KiB pages), optionally pre-faulted.
    pub fn load(dir: &str, populate: bool) -> Graph {
        let fo = std::fs::File::open(format!("{}/offsets.u64", dir)).expect("offsets.u64");
        let ft = std::fs::File::open(format!("{}/targets.u32", dir)).expect("targets.u32");
        let mut oo = memmap2::MmapOptions::new();
        let mut ot = memmap2::MmapOptions::new();
        if populate {
            oo.populate();
            ot.populate();
        }
        let mo = unsafe { oo.map(&fo).unwrap() };
        let mt = unsafe { ot.map(&ft).unwrap() };
        let n = mo.len() / 8 - 1;
        let entries = mt.len() / 4;
        let off = mo.as_ptr() as *const u64;
        let tgt = mt.as_ptr() as *const u32;
        assert_eq!(unsafe { *off.add(n) } as usize, entries);
        // optional weights (f32, one per entry)
        let wgt = std::fs::read(format!("{}/weights.f32", dir)).ok().map(|b| {
            assert_eq!(b.len(), entries * 4);
            b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]) as W).collect::<Vec<W>>()
        });
        Graph { n, off, tgt, wgt, entries, comp: None, _own: Own::Map(mo, mt) }
    }
    pub fn from_vecs(n: usize, off: Vec<u64>, tgt: Vec<u32>, wgt: Option<Vec<W>>) -> Graph {
        let entries = tgt.len();
        Graph { n, off: off.as_ptr(), tgt: tgt.as_ptr(), wgt, entries, comp: None, _own: Own::Vecs(off, tgt) }
    }
    /// Uncompressed arrays in anonymous memory (heap-loaded, or decoded from the compressed store).
    pub fn from_raw(r: crate::adj::Raw) -> Graph {
        Graph { n: r.n, off: r.offsets.as_ptr(), tgt: r.targets.as_ptr(), wgt: None, entries: r.targets.len(), comp: None, _own: Own::Raw(r) }
    }
    /// Compressed store, decoded on the fly.
    pub fn from_comp(c: crate::store::Comp) -> Graph {
        Graph { n: c.n, off: std::ptr::null(), tgt: std::ptr::null(), wgt: None, entries: c.entries, comp: Some(c), _own: Own::None }
    }
    /// m1-02: the compressed store behind this graph, if any.
    pub fn comp(&self) -> Option<&crate::store::Comp> {
        self.comp.as_ref()
    }
    #[inline(always)]
    pub fn off(&self) -> &[u64] {
        assert!(self.comp.is_none(), "no raw offsets on a compressed graph");
        unsafe { std::slice::from_raw_parts(self.off, self.n + 1) }
    }
    #[inline(always)]
    pub fn tgt(&self) -> &[u32] {
        assert!(self.comp.is_none(), "no raw targets on a compressed graph");
        unsafe { std::slice::from_raw_parts(self.tgt, self.entries) }
    }
    pub fn wgt(&self) -> Option<&[W]> {
        self.wgt.as_deref()
    }
    pub fn entries(&self) -> usize {
        self.entries
    }
    #[inline(always)]
    pub fn deg(&self, u: usize) -> usize {
        if let Some(c) = &self.comp {
            return c.deg(u);
        }
        unsafe { (*self.off.add(u + 1) - *self.off.add(u)) as usize }
    }
    /// Never called re-entrantly on one thread (checked by reading every call site): the
    /// compressed path hands out the thread-local decode buffer.
    #[inline(always)]
    pub fn for_edges<F: FnMut(u32, f64)>(&self, u: usize, mut f: F) {
        if let Some(c) = &self.comp {
            LCUR.with(|cur| {
                let cur = unsafe { &mut *cur.get() };
                if c.weighted {
                    // m1-02: spilled quotient graphs (integer weights)
                    let (t, w) = c.nbrs_w(u, cur);
                    for i in 0..t.len() {
                        unsafe { f(*t.get_unchecked(i), *w.get_unchecked(i) as f64) }
                    }
                } else {
                    for &v in c.nbrs(u, cur) {
                        f(v, 1.0)
                    }
                }
                if cur.len_of_last() > crate::leiden::GIANT_DEG {
                    cur.trim(); // m1-02: decode buffers grown for a giant list are given back
                }
            });
            return;
        }
        unsafe {
            let s = *self.off.add(u) as usize;
            let e = *self.off.add(u + 1) as usize;
            match &self.wgt {
                None => {
                    for i in s..e {
                        f(*self.tgt.add(i), 1.0)
                    }
                }
                Some(w) => {
                    for i in s..e {
                        f(*self.tgt.add(i), *w.get_unchecked(i) as f64)
                    }
                }
            }
        }
    }
}

pub fn write_slice<T: Copy>(path: &str, v: &[T]) -> std::io::Result<()> {
    let bytes = unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, std::mem::size_of_val(v)) };
    std::fs::write(path, bytes)
}
pub fn read_vec<T: Copy>(path: &str) -> Vec<T> {
    let b = std::fs::read(path).expect("read");
    let n = b.len() / std::mem::size_of::<T>();
    let mut v: Vec<T> = uninit_vec(n);
    unsafe { std::ptr::copy_nonoverlapping(b.as_ptr(), v.as_mut_ptr() as *mut u8, n * std::mem::size_of::<T>()) };
    v
}
