//! Shared helpers for F1: mmap, eviction, timing, CSR access.
use memmap2::{Mmap, MmapOptions};
use std::fs::File;
use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::Instant;

pub fn now() -> Instant {
    Instant::now()
}
pub fn secs(t: Instant) -> f64 {
    t.elapsed().as_secs_f64()
}

/// Read-only file mapping that can be fully evicted from the page cache.
pub struct MFile {
    pub path: PathBuf,
    pub file: File,
    pub map: Mmap,
    pub random: bool,
}

impl MFile {
    pub fn open(p: impl AsRef<Path>, random: bool) -> MFile {
        let path = p.as_ref().to_path_buf();
        let file = File::open(&path).unwrap_or_else(|e| panic!("open {:?}: {}", path, e));
        let map = unsafe { MmapOptions::new().map(&file).unwrap() };
        let m = MFile { path, file, map, random };
        m.advise();
        m
    }
    fn advise(&self) {
        if self.map.len() == 0 {
            return;
        }
        let adv = if self.random { libc::MADV_RANDOM } else { libc::MADV_NORMAL };
        unsafe {
            libc::madvise(self.map.as_ptr() as *mut _, self.map.len(), adv);
        }
    }
    pub fn bytes(&self) -> &[u8] {
        &self.map[..]
    }
    pub fn as_slice<T: Copy>(&self) -> &[T] {
        let b = self.bytes();
        let sz = std::mem::size_of::<T>();
        assert!(b.len() % sz == 0);
        unsafe { std::slice::from_raw_parts(b.as_ptr() as *const T, b.len() / sz) }
    }
    /// Drop this process' page-table entries and ask the kernel to drop the page cache.
    pub fn evict(&self) {
        if self.map.len() == 0 {
            return;
        }
        unsafe {
            libc::madvise(self.map.as_ptr() as *mut _, self.map.len(), libc::MADV_DONTNEED);
            libc::posix_fadvise(self.file.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED);
        }
        self.advise();
    }
    /// Fraction of pages resident (mincore).
    pub fn resident(&self) -> f64 {
        let len = self.map.len();
        if len == 0 {
            return 0.0;
        }
        let pg = 4096usize;
        let np = len.div_ceil(pg);
        let mut v = vec![0u8; np];
        unsafe {
            libc::mincore(self.map.as_ptr() as *mut _, len, v.as_mut_ptr());
        }
        v.iter().filter(|&&x| x & 1 == 1).count() as f64 / np as f64
    }
    /// Touch every page (warm-up).
    pub fn warm(&self) -> u64 {
        let b = self.bytes();
        let mut s = 0u64;
        let mut i = 0;
        while i < b.len() {
            s = s.wrapping_add(unsafe { std::ptr::read_volatile(&b[i]) } as u64);
            i += 4096;
        }
        std::hint::black_box(s)
    }
}

pub fn evict_path(p: impl AsRef<Path>) {
    if let Ok(f) = File::open(p.as_ref()) {
        unsafe {
            libc::posix_fadvise(f.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED);
        }
    }
}

pub fn write_slice<T: Copy>(p: impl AsRef<Path>, s: &[T]) {
    let b = unsafe { std::slice::from_raw_parts(s.as_ptr() as *const u8, std::mem::size_of_val(s)) };
    let mut f = std::io::BufWriter::with_capacity(1 << 22, File::create(p.as_ref()).unwrap());
    f.write_all(b).unwrap();
    f.flush().unwrap();
    f.get_ref().sync_all().ok();
}

pub fn read_vec<T: Copy + Default>(p: impl AsRef<Path>) -> Vec<T> {
    let b = std::fs::read(p.as_ref()).unwrap();
    let sz = std::mem::size_of::<T>();
    let n = b.len() / sz;
    let mut v = vec![T::default(); n];
    unsafe {
        std::ptr::copy_nonoverlapping(b.as_ptr(), v.as_mut_ptr() as *mut u8, n * sz);
    }
    v
}

pub fn file_size(p: impl AsRef<Path>) -> u64 {
    std::fs::metadata(p.as_ref()).map(|m| m.len()).unwrap_or(0)
}

/// /proc/self/status field in kB
pub fn status_kb(field: &str) -> u64 {
    let s = std::fs::read_to_string("/proc/self/status").unwrap();
    for l in s.lines() {
        if l.starts_with(field) {
            return l.split_whitespace().nth(1).unwrap().parse().unwrap();
        }
    }
    0
}
pub fn rss_mb() -> f64 {
    status_kb("VmRSS:") as f64 / 1024.0
}
pub fn hwm_mb() -> f64 {
    status_kb("VmHWM:") as f64 / 1024.0
}
/// Reset VmHWM to current RSS.
pub fn reset_hwm() {
    let _ = std::fs::write("/proc/self/clear_refs", "5");
}

pub struct Rng(pub u64);
impl Rng {
    #[inline]
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        splitmix(self.0)
    }
    #[inline]
    pub fn below(&mut self, n: u64) -> u64 {
        ((self.next() as u128 * n as u128) >> 64) as u64
    }
    pub fn f64(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}
#[inline]
pub fn splitmix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

/// Latency summary in microseconds.
pub fn lat_summary(v: &mut [f64]) -> serde_json::Value {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = v.len();
    let q = |p: f64| v[((n as f64 - 1.0) * p).round() as usize];
    let mean = v.iter().sum::<f64>() / n as f64;
    serde_json::json!({"n": n, "mean_us": mean, "p50_us": q(0.5), "p95_us": q(0.95), "p99_us": q(0.99), "max_us": v[n-1]})
}

pub fn pool(threads: usize) -> rayon::ThreadPool {
    rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap()
}

pub fn save_json(path: impl AsRef<Path>, v: &serde_json::Value) {
    std::fs::write(path.as_ref(), serde_json::to_string_pretty(v).unwrap()).unwrap();
    eprintln!("wrote {:?}", path.as_ref());
}

/// Memory-mapped glab-csr-v0 graph.
pub struct Csr {
    pub dir: PathBuf,
    pub n: usize,
    pub off: MFile,
    pub tgt: MFile,
}
impl Csr {
    pub fn open(dir: impl AsRef<Path>, random: bool) -> Csr {
        Self::open_files(dir.as_ref(), "offsets.u64", "targets.u32", random)
    }
    pub fn open_files(dir: &Path, off: &str, tgt: &str, random: bool) -> Csr {
        let off = MFile::open(dir.join(off), random);
        let tgt = MFile::open(dir.join(tgt), random);
        let n = off.as_slice::<u64>().len() - 1;
        Csr { dir: dir.to_path_buf(), n, off, tgt }
    }
    #[inline]
    pub fn offs(&self) -> &[u64] {
        self.off.as_slice()
    }
    #[inline]
    pub fn tgts(&self) -> &[u32] {
        self.tgt.as_slice()
    }
    #[inline]
    pub fn nbrs(&self, v: u32) -> &[u32] {
        let o = self.offs();
        let (a, b) = (o[v as usize] as usize, o[v as usize + 1] as usize);
        &self.tgts()[a..b]
    }
    #[inline]
    pub fn range(&self, v: u32) -> (usize, usize) {
        let o = self.offs();
        (o[v as usize] as usize, o[v as usize + 1] as usize)
    }
    #[inline]
    pub fn deg(&self, v: u32) -> usize {
        let o = self.offs();
        (o[v as usize + 1] - o[v as usize]) as usize
    }
    pub fn m(&self) -> usize {
        self.tgts().len()
    }
    pub fn evict(&self) {
        self.off.evict();
        self.tgt.evict();
    }
    pub fn warm(&self) -> u64 {
        self.off.warm() ^ self.tgt.warm()
    }
    pub fn resident(&self) -> f64 {
        (self.off.resident() * self.off.map.len() as f64 + self.tgt.resident() * self.tgt.map.len() as f64)
            / (self.off.map.len() + self.tgt.map.len()) as f64
    }
}

/// Simple bitset with touched-word list for cheap clearing.
pub struct Visited {
    pub words: Vec<u64>,
    pub touched: Vec<u32>,
}
impl Visited {
    pub fn new(n: usize) -> Self {
        Visited { words: vec![0; n.div_ceil(64)], touched: Vec::new() }
    }
    /// returns true if newly set
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

pub fn arg<T: std::str::FromStr>(args: &[String], name: &str, def: T) -> T {
    let key = format!("--{}", name);
    for i in 0..args.len() {
        if args[i] == key && i + 1 < args.len() {
            return args[i + 1].parse().ok().unwrap_or(def);
        }
    }
    def
}
pub fn arg_s(args: &[String], name: &str, def: &str) -> String {
    arg(args, name, def.to_string())
}
