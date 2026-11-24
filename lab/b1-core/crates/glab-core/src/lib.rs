//! glab-core: feasibility prototype (experiment B1).
//! Ingest (text edge list -> glab-csr-v0), CSR storage, in-RAM and semi-external kernels.
pub mod extmem;
pub mod ingest;
pub mod kernels;

use rayon::prelude::*;
use std::fs::File;
use std::ops::Deref;
use std::os::unix::fs::FileExt;
use std::path::Path;

/// Raw pointer that may be shared across threads; caller guarantees disjoint writes.
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
    #[inline(always)]
    pub unsafe fn slice_mut(&self, a: usize, b: usize) -> &mut [T] {
        std::slice::from_raw_parts_mut(self.0.add(a), b - a)
    }
}

pub fn as_bytes<T: Copy>(s: &[T]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(s.as_ptr() as *const u8, std::mem::size_of_val(s)) }
}
pub fn as_bytes_mut<T: Copy>(s: &mut [T]) -> &mut [u8] {
    unsafe { std::slice::from_raw_parts_mut(s.as_mut_ptr() as *mut u8, std::mem::size_of_val(s)) }
}

fn proc_status_kb(key: &str) -> u64 {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    for l in s.lines() {
        if l.starts_with(key) {
            return l.split_whitespace().nth(1).and_then(|x| x.parse().ok()).unwrap_or(0);
        }
    }
    0
}
/// Peak resident set size of this process in bytes (VmHWM).
pub fn peak_rss() -> u64 {
    proc_status_kb("VmHWM:") * 1024
}
pub fn cur_rss() -> u64 {
    proc_status_kb("VmRSS:") * 1024
}
/// Anonymous part of RSS (excludes file-backed mmap pages).
pub fn rss_anon() -> u64 {
    proc_status_kb("RssAnon:") * 1024
}

pub fn raise_nofile() {
    unsafe {
        let mut r: libc::rlimit = std::mem::zeroed();
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut r) == 0 {
            r.rlim_cur = r.rlim_max.min(1 << 16);
            libc::setrlimit(libc::RLIMIT_NOFILE, &r);
        }
    }
}

/// Typed buffer: file mmap, anonymous mmap (heap with THP control) or Vec.
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
pub enum LoadMode {
    /// mmap the files read-only (page cache backed, 4 KiB pages)
    Mmap,
    /// read into anonymous memory, kernel default THP policy
    Heap,
    /// read into anonymous memory + madvise(MADV_HUGEPAGE)
    HeapHuge,
    /// read into anonymous memory + madvise(MADV_NOHUGEPAGE)
    HeapNoHuge,
}

pub fn anon_buf<T: Copy>(len: usize, mode: LoadMode) -> (memmap2::MmapMut, usize) {
    let bytes = (len * std::mem::size_of::<T>()).max(4096);
    let m = memmap2::MmapMut::map_anon(bytes).expect("anon mmap");
    unsafe {
        match mode {
            LoadMode::HeapHuge => {
                libc::madvise(m.as_ptr() as *mut _, bytes, libc::MADV_HUGEPAGE);
            }
            LoadMode::HeapNoHuge => {
                libc::madvise(m.as_ptr() as *mut _, bytes, libc::MADV_NOHUGEPAGE);
            }
            _ => {}
        }
    }
    (m, len)
}

fn load_file<T: Copy + Send + Sync>(path: &Path, mode: LoadMode) -> std::io::Result<Buf<T>> {
    let f = File::open(path)?;
    let bytes = f.metadata()?.len() as usize;
    let len = bytes / std::mem::size_of::<T>();
    if mode == LoadMode::Mmap {
        let m = unsafe { memmap2::Mmap::map(&f)? };
        return Ok(Buf::Mmap(m, len));
    }
    let (mut m, len) = anon_buf::<T>(len, mode);
    const C: usize = 8 << 20;
    let p = SyncPtr(m.as_mut_ptr());
    let nch = bytes.div_ceil(C);
    (0..nch).into_par_iter().for_each(|i| {
        let a = i * C;
        let b = (a + C).min(bytes);
        let s = unsafe { p.slice_mut(a, b) };
        f.read_exact_at(s, a as u64).expect("read");
    });
    Ok(Buf::Anon(m, len))
}

pub struct Csr {
    pub n: usize,
    pub offsets: Buf<u64>,
    pub targets: Buf<u32>,
}

impl Csr {
    pub fn load(dir: &Path, mode: LoadMode) -> std::io::Result<Csr> {
        // offsets are O(n): always read into RAM unless pure mmap was requested
        let offsets: Buf<u64> = load_file(&dir.join("offsets.u64"), mode)?;
        let targets: Buf<u32> = load_file(&dir.join("targets.u32"), mode)?;
        let n = offsets.len() - 1;
        assert_eq!(offsets[n] as usize, targets.len(), "offsets/targets mismatch");
        Ok(Csr { n, offsets, targets })
    }
    pub fn from_vecs(offsets: Vec<u64>, targets: Vec<u32>) -> Csr {
        let n = offsets.len() - 1;
        Csr { n, offsets: Buf::Vec(offsets), targets: Buf::Vec(targets) }
    }
    #[inline(always)]
    pub fn entries(&self) -> usize {
        self.targets.len()
    }
    #[inline(always)]
    pub fn deg(&self, v: usize) -> usize {
        (self.offsets[v + 1] - self.offsets[v]) as usize
    }
    #[inline(always)]
    pub fn nbrs(&self, v: usize) -> &[u32] {
        unsafe {
            let a = *self.offsets.get_unchecked(v) as usize;
            let b = *self.offsets.get_unchecked(v + 1) as usize;
            self.targets.get_unchecked(a..b)
        }
    }
    pub fn max_degree_node(&self) -> usize {
        let off = &self.offsets[..];
        (0..self.n)
            .into_par_iter()
            .map(|v| (off[v + 1] - off[v], std::cmp::Reverse(v)))
            .max()
            .map(|(_, r)| r.0)
            .unwrap_or(0)
    }
}

/// Split nodes into contiguous ranges holding roughly `target` adjacency entries each.
pub fn edge_chunks(offsets: &[u64], target: u64) -> Vec<(usize, usize)> {
    let n = offsets.len() - 1;
    let mut out = Vec::new();
    let mut a = 0usize;
    const MAX_NODES: usize = 1 << 16;
    while a < n {
        let lim = offsets[a] + target;
        let hi = (a + MAX_NODES).min(n);
        // first index b in (a, hi] with offsets[b] >= lim
        let rel = offsets[a + 1..=hi].partition_point(|&o| o < lim);
        let b = (a + 1 + rel).min(hi);
        out.push((a, b));
        a = b;
    }
    out
}

/// Relabel a CSR: `order[new] = old`. Result is symmetric/sorted like the input.
pub fn reorder(g: &Csr, order: &[u32]) -> Csr {
    let n = g.n;
    let mut old2new = vec![0u32; n];
    {
        let p = SyncPtr(old2new.as_mut_ptr());
        order.par_iter().enumerate().for_each(|(new, &old)| unsafe { p.write(old as usize, new as u32) });
    }
    let mut offsets = vec![0u64; n + 1];
    for i in 0..n {
        offsets[i + 1] = offsets[i] + g.deg(order[i] as usize) as u64;
    }
    let mut targets = vec![0u32; g.entries()];
    let tp = SyncPtr(targets.as_mut_ptr());
    let off = &offsets[..];
    let o2n = &old2new[..];
    (0..n).into_par_iter().with_min_len(1024).for_each(|new| {
        let s = unsafe { tp.slice_mut(off[new] as usize, off[new + 1] as usize) };
        for (d, &t) in s.iter_mut().zip(g.nbrs(order[new] as usize)) {
            *d = o2n[t as usize];
        }
        s.sort_unstable();
    });
    Csr::from_vecs(offsets, targets)
}

/// Evict a file from the page cache (needs no privileges; only clean, unmapped pages go).
pub fn evict_file(path: &Path) -> std::io::Result<()> {
    use std::os::unix::io::AsRawFd;
    let f = File::open(path)?;
    unsafe {
        libc::posix_fadvise(f.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED);
    }
    Ok(())
}
