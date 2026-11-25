//! The adjacency abstraction every kernel in this experiment is written against, and the
//! uncompressed implementation (configuration A, and configuration C after decoding).
//! Buffers and loading are taken from lab/b1-core (glab-core/src/lib.rs).
use rayon::prelude::*;
use std::fs::File;
use std::ops::Deref;
use std::os::unix::fs::FileExt;
use std::path::Path;

#[derive(Clone, Copy)]
pub struct SyncPtr<T>(pub *mut T);
unsafe impl<T> Send for SyncPtr<T> {}
unsafe impl<T> Sync for SyncPtr<T> {}
impl<T> SyncPtr<T> {
    #[inline(always)]
    pub fn get(&self) -> *mut T {
        self.0
    }
    #[inline(always)]
    pub unsafe fn write(&self, i: usize, v: T) {
        self.0.add(i).write(v)
    }
    /// # Safety: a <= b within the buffer, and no other live reference to [a, b).
    // `mut_from_ref` (clippy, deny by default): intended. SyncPtr is a raw pointer shared by the
    // threads of a parallel loop; each thread takes a disjoint range, which the caller guarantees
    // (the function is `unsafe` for that reason). Inherited unchanged from lab/m1-13.
    #[allow(clippy::mut_from_ref)]
    #[inline(always)]
    pub unsafe fn slice_mut(&self, a: usize, b: usize) -> &mut [T] {
        std::slice::from_raw_parts_mut(self.0.add(a), b - a)
    }
}

/// Typed buffer: file mmap, anonymous mmap or Vec.
pub enum Buf<T> {
    Mmap(memmap2::Mmap, usize),
    Anon(memmap2::MmapMut, usize),
    Vec(Vec<T>),
}
impl<T> Deref for Buf<T> {
    type Target = [T];
    #[inline(always)]
    fn deref(&self) -> &[T] {
        match self {
            Buf::Mmap(m, len) => unsafe { std::slice::from_raw_parts(m.as_ptr() as *const T, *len) },
            Buf::Anon(m, len) => unsafe { std::slice::from_raw_parts(m.as_ptr() as *const T, *len) },
            Buf::Vec(v) => v,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mem {
    /// read into anonymous memory (kernel THP policy applies: `always` on the lab machine)
    Heap,
    /// mmap the file read-only and pre-fault it (page cache, 4 KiB pages)
    Mmap,
    /// mmap without pre-faulting (pages come in on first touch; used for cold opens)
    MmapLazy,
}
impl Mem {
    pub fn parse(s: &str) -> Mem {
        match s {
            "heap" => Mem::Heap,
            "mmap" => Mem::Mmap,
            "mmap-lazy" => Mem::MmapLazy,
            x => panic!("bad --mem {x}"),
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Mem::Heap => "heap",
            Mem::Mmap => "mmap",
            Mem::MmapLazy => "mmap-lazy",
        }
    }
}

pub fn anon_buf<T: Copy>(len: usize) -> Buf<T> {
    let bytes = (len * std::mem::size_of::<T>()).max(4096);
    Buf::Anon(memmap2::MmapMut::map_anon(bytes).expect("anon mmap"), len)
}
pub fn anon_ptr<T>(b: &mut Buf<T>) -> *mut T {
    match b {
        Buf::Anon(m, _) => m.as_mut_ptr() as *mut T,
        Buf::Vec(v) => v.as_mut_ptr(),
        Buf::Mmap(..) => panic!("read-only buffer"),
    }
}

/// Load a file of T. `extra` elements of zero padding are guaranteed readable after the end.
pub fn load_file<T: Copy + Send + Sync>(path: &Path, mem: Mem, extra: usize) -> std::io::Result<Buf<T>> {
    let f = File::open(path)?;
    let bytes = f.metadata()?.len() as usize;
    let len = bytes / std::mem::size_of::<T>();
    if mem != Mem::Heap {
        // the file itself must carry the padding in this mode (adj.bin does)
        let mut o = memmap2::MmapOptions::new();
        if mem == Mem::Mmap {
            o.populate();
        }
        let m = unsafe { o.map(&f)? };
        return Ok(Buf::Mmap(m, len));
    }
    let mut b: Buf<T> = anon_buf(len + extra);
    if let Buf::Anon(_, l) = &mut b {
        *l = len;
    }
    const C: usize = 8 << 20;
    let p = SyncPtr(anon_ptr(&mut b) as *mut u8);
    (0..bytes.div_ceil(C)).into_par_iter().for_each(|i| {
        let a = i * C;
        let e = (a + C).min(bytes);
        f.read_exact_at(unsafe { p.slice_mut(a, e) }, a as u64).expect("read");
    });
    Ok(b)
}

/// Split nodes into contiguous ranges of roughly `target` adjacency entries (lab/b1-core edge_chunks).
/// These are both the storage blocks of the compressed format and the work units of every kernel,
/// so configurations A, B and C run with the same partition.
pub fn edge_chunks(offsets: &[u64], target: u64) -> Vec<(usize, usize)> {
    let n = offsets.len() - 1;
    let mut out = Vec::new();
    let mut a = 0usize;
    const MAX_NODES: usize = 1 << 16;
    while a < n {
        let lim = offsets[a] + target;
        let hi = (a + MAX_NODES).min(n);
        let rel = offsets[a + 1..=hi].partition_point(|&o| o < lim);
        let b = (a + 1 + rel).min(hi);
        out.push((a, b));
        a = b;
    }
    out
}
pub const BLOCK_ENTRIES: u64 = 1 << 16;

/// Read-only adjacency. `Cur` is the per-thread scratch a compressed source decodes into
/// (zero-sized for raw arrays, so the raw kernels compile to the lab code).
pub trait Adj: Sync {
    type Cur: Send;
    fn kind(&self) -> &'static str;
    fn cur(&self) -> Self::Cur;
    fn n(&self) -> usize;
    fn entries(&self) -> usize;
    fn chunks(&self) -> &[(usize, usize)];
    /// Degree of one node (random access).
    fn deg(&self, v: usize) -> usize;
    /// Neighbors of one node (random access).
    fn nbrs<'a>(&'a self, v: usize, c: &'a mut Self::Cur) -> &'a [u32];
    /// First min(k, deg) neighbors and the degree (random access).
    fn prefix<'a>(&'a self, v: usize, k: usize, c: &'a mut Self::Cur) -> (&'a [u32], usize);
    /// f(v, neighbors) for v in lo..hi, in order (sequential access).
    fn for_range<F: FnMut(usize, &[u32])>(&self, lo: usize, hi: usize, c: &mut Self::Cur, f: F);
    /// f(v, first min(k, deg) neighbors, deg) for v in lo..hi.
    fn for_range_prefix<F: FnMut(usize, &[u32], usize)>(&self, lo: usize, hi: usize, k: usize, c: &mut Self::Cur, f: F);
    /// f(v, deg) for v in lo..hi.
    fn for_range_deg<F: FnMut(usize, usize)>(&self, lo: usize, hi: usize, f: F);
    /// Calls f on the neighbors of v in order until it returns true. Returns (found, neighbors tested).
    fn any<F: FnMut(u32) -> bool>(&self, v: usize, c: &mut Self::Cur, f: F) -> (bool, usize);

    fn max_degree_node(&self) -> usize {
        self.chunks()
            .par_iter()
            .map(|&(a, b)| {
                let mut best = (0usize, std::cmp::Reverse(a));
                self.for_range_deg(a, b, |v, d| best = best.max((d, std::cmp::Reverse(v))));
                best
            })
            .max()
            .map(|(_, r)| r.0)
            .unwrap_or(0)
    }
}

/// Uncompressed CSR in memory.
pub struct Raw {
    pub n: usize,
    pub offsets: Buf<u64>,
    pub targets: Buf<u32>,
    pub chunks: Vec<(usize, usize)>,
    // plain pointers into the two buffers, so the hot loops do not re-derive them per node
    op: *const u64,
    tp: *const u32,
}
unsafe impl Send for Raw {}
unsafe impl Sync for Raw {}

impl Raw {
    pub fn load(dir: &Path, mem: Mem) -> std::io::Result<Raw> {
        // offsets are O(n) state: always in anonymous memory, as in lab/b1-core --load heap
        let offsets: Buf<u64> = load_file(&dir.join("offsets.u64"), mem, 0)?;
        let targets: Buf<u32> = load_file(&dir.join("targets.u32"), mem, 0)?;
        Ok(Raw::from_bufs(offsets, targets))
    }
    pub fn from_bufs(offsets: Buf<u64>, targets: Buf<u32>) -> Raw {
        let n = offsets.len() - 1;
        assert_eq!(offsets[n] as usize, targets.len(), "offsets/targets mismatch");
        let chunks = edge_chunks(&offsets, BLOCK_ENTRIES);
        let (op, tp) = (offsets.as_ptr(), targets.as_ptr());
        Raw { n, offsets, targets, chunks, op, tp }
    }
    #[inline(always)]
    pub fn list(&self, v: usize) -> &[u32] {
        unsafe {
            let a = *self.op.add(v) as usize;
            let b = *self.op.add(v + 1) as usize;
            std::slice::from_raw_parts(self.tp.add(a), b - a)
        }
    }
}

impl Adj for Raw {
    type Cur = ();
    fn kind(&self) -> &'static str {
        "raw"
    }
    #[inline(always)]
    fn cur(&self) {}
    #[inline(always)]
    fn n(&self) -> usize {
        self.n
    }
    #[inline(always)]
    fn entries(&self) -> usize {
        self.targets.len()
    }
    #[inline(always)]
    fn chunks(&self) -> &[(usize, usize)] {
        &self.chunks
    }
    #[inline(always)]
    fn deg(&self, v: usize) -> usize {
        unsafe { (*self.op.add(v + 1) - *self.op.add(v)) as usize }
    }
    #[inline(always)]
    fn nbrs<'a>(&'a self, v: usize, _c: &'a mut ()) -> &'a [u32] {
        self.list(v)
    }
    #[inline(always)]
    fn prefix<'a>(&'a self, v: usize, k: usize, _c: &'a mut ()) -> (&'a [u32], usize) {
        let l = self.list(v);
        (unsafe { l.get_unchecked(..k.min(l.len())) }, l.len())
    }
    #[inline(always)]
    fn for_range<F: FnMut(usize, &[u32])>(&self, lo: usize, hi: usize, _c: &mut (), mut f: F) {
        for v in lo..hi {
            f(v, self.list(v));
        }
    }
    #[inline(always)]
    fn for_range_prefix<F: FnMut(usize, &[u32], usize)>(&self, lo: usize, hi: usize, k: usize, _c: &mut (), mut f: F) {
        for v in lo..hi {
            let l = self.list(v);
            f(v, unsafe { l.get_unchecked(..k.min(l.len())) }, l.len());
        }
    }
    #[inline(always)]
    fn for_range_deg<F: FnMut(usize, usize)>(&self, lo: usize, hi: usize, mut f: F) {
        for v in lo..hi {
            f(v, self.deg(v));
        }
    }
    #[inline(always)]
    fn any<F: FnMut(u32) -> bool>(&self, v: usize, _c: &mut (), mut f: F) -> (bool, usize) {
        let l = self.list(v);
        for (i, &u) in l.iter().enumerate() {
            if f(u) {
                return (true, i + 1);
            }
        }
        (false, l.len())
    }
}

// ---------------------------------------------------------------- process statistics
fn proc_status_kb(key: &str) -> u64 {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    for l in s.lines() {
        if l.starts_with(key) {
            return l.split_whitespace().nth(1).and_then(|x| x.parse().ok()).unwrap_or(0);
        }
    }
    0
}
pub fn peak_rss() -> u64 {
    proc_status_kb("VmHWM:") * 1024
}
pub fn rss_anon() -> u64 {
    proc_status_kb("RssAnon:") * 1024
}
pub fn rss_file() -> u64 {
    proc_status_kb("RssFile:") * 1024
}
/// Anonymous memory currently backed by transparent huge pages.
pub fn anon_huge() -> u64 {
    let s = std::fs::read_to_string("/proc/self/smaps_rollup").unwrap_or_default();
    for l in s.lines() {
        if l.starts_with("AnonHugePages:") {
            return l.split_whitespace().nth(1).and_then(|x| x.parse::<u64>().ok()).unwrap_or(0) * 1024;
        }
    }
    0
}
/// m1-02: memory.max of this process's cgroup (None if unlimited or unreadable).
pub fn cgroup_memory_max() -> Option<u64> {
    let cg = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    let path = cg.lines().find_map(|l| l.strip_prefix("0::"))?.to_string();
    let s = std::fs::read_to_string(format!("/sys/fs/cgroup{}/memory.max", path)).ok()?;
    s.trim().parse().ok()
}
pub fn loadavg() -> f64 {
    std::fs::read_to_string("/proc/loadavg").ok().and_then(|s| s.split_whitespace().next().and_then(|x| x.parse().ok())).unwrap_or(-1.0)
}
pub fn gb(x: u64) -> f64 {
    x as f64 / (1u64 << 30) as f64
}
/// Evict a file from the page cache (only clean, unmapped pages go).
pub fn evict_file(path: &Path) -> std::io::Result<()> {
    use std::os::unix::io::AsRawFd;
    let f = File::open(path)?;
    unsafe {
        libc::fdatasync(f.as_raw_fd());
        libc::posix_fadvise(f.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED);
    }
    Ok(())
}
/// Fraction of a file's pages resident in the page cache.
pub fn resident_fraction(path: &Path) -> f64 {
    let Ok(f) = File::open(path) else { return -1.0 };
    let len = f.metadata().map(|m| m.len() as usize).unwrap_or(0);
    if len == 0 {
        return 0.0;
    }
    let Ok(m) = (unsafe { memmap2::Mmap::map(&f) }) else { return -1.0 };
    let np = len.div_ceil(4096);
    let mut v = vec![0u8; np];
    unsafe { libc::mincore(m.as_ptr() as *mut _, len, v.as_mut_ptr()) };
    v.iter().filter(|&&x| x & 1 == 1).count() as f64 / np as f64
}
