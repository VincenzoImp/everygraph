use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};

pub fn read_vec<T: Copy + Default>(path: &str) -> Vec<T> {
    let mut f = File::open(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let len = f.metadata().unwrap().len() as usize;
    let n = len / std::mem::size_of::<T>();
    let mut v = vec![T::default(); n];
    let bytes = unsafe { std::slice::from_raw_parts_mut(v.as_mut_ptr() as *mut u8, n * std::mem::size_of::<T>()) };
    f.read_exact(bytes).unwrap();
    v
}
pub fn write_slice<T: Copy>(path: &str, v: &[T]) {
    let mut f = File::create(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let bytes = unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * std::mem::size_of::<T>()) };
    f.write_all(bytes).unwrap();
}
mod plain {
    /// Plain integer types: every bit pattern is a valid value. Sealed: implemented
    /// here only, so `Arr` can view raw file bytes as `[T]` for these types alone.
    pub trait Plain: Copy + Default {}
    impl Plain for u32 {}
    impl Plain for u64 {}
}
pub use plain::Plain;

enum Backing<T> {
    Heap(Vec<T>),
    Map(memmap2::Mmap),
}
/// An array of plain integers read into the heap (the lab's default) or memory-mapped
/// from its file. The backing is private: an `Arr` exists only through `load`, which
/// checks length and alignment of a mapping.
pub struct Arr<T: Plain>(Backing<T>);
impl<T: Plain> Arr<T> {
    /// `GMAP_LOAD=mmap` maps the file read-only (added for issue #15, the tracer bullet,
    /// to tell the graph's own footprint from the stage's); anything else reads it into
    /// the heap, as experiment E did. The mapping uses the kernel's default advice: no
    /// `madvise` is called (MADV_RANDOM was not tried), so every mmap figure of the
    /// tracer is for default readahead. The file must not be truncated or modified
    /// while mapped; the tracer only ever replaces outputs by rename.
    pub fn load(path: &str) -> Arr<T> {
        if std::env::var("GMAP_LOAD").as_deref() == Ok("mmap") {
            let f = File::open(path).unwrap_or_else(|e| panic!("{path}: {e}"));
            // SAFETY: read-only mapping of a file the pipeline never modifies in place.
            let m = unsafe { memmap2::Mmap::map(&f) }.unwrap_or_else(|e| panic!("{path}: {e}"));
            assert_eq!(m.len() % std::mem::size_of::<T>(), 0, "{path}: length is not a multiple of the element size");
            assert_eq!(m.as_ptr() as usize % std::mem::align_of::<T>(), 0, "{path}: mapping is not aligned");
            // Issue #172 (m1-15): `GMAP_MADV=random|sequential|normal` applies that advice
            // to the whole mapping; unset or `none` means no madvise call, as in the tracer.
            match std::env::var("GMAP_MADV").as_deref() {
                Ok("none") => Ok(()),
                Ok("random") => m.advise(memmap2::Advice::Random),
                Ok("sequential") => m.advise(memmap2::Advice::Sequential),
                Ok("normal") => m.advise(memmap2::Advice::Normal),
                Ok(other) => panic!("GMAP_MADV={other}: expected random, sequential or normal"),
                Err(_) => Ok(()),
            }
            .unwrap_or_else(|e| panic!("{path}: madvise: {e}"));
            Arr(Backing::Map(m))
        } else {
            Arr(Backing::Heap(read_vec(path)))
        }
    }
}
impl<T: Plain> std::ops::Deref for Arr<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        match &self.0 {
            Backing::Heap(v) => v,
            // SAFETY: `load` checked length and alignment; T is a plain integer type
            // (sealed trait), so any bytes are valid values; the slice borrows `self`
            // and cannot outlive the mapping.
            Backing::Map(m) => unsafe { std::slice::from_raw_parts(m.as_ptr() as *const T, m.len() / std::mem::size_of::<T>()) },
        }
    }
}
pub struct Csr {
    pub n: usize,
    pub off: Arr<u64>,
    pub tgt: Arr<u32>,
}
impl Csr {
    pub fn load(dir: &str) -> Csr {
        let off: Arr<u64> = Arr::load(&format!("{dir}/offsets.u64"));
        let tgt: Arr<u32> = Arr::load(&format!("{dir}/targets.u32"));
        Csr { n: off.len() - 1, off, tgt }
    }
    #[inline]
    pub fn adj(&self, u: usize) -> &[u32] {
        &self.tgt[self.off[u] as usize..self.off[u + 1] as usize]
    }
    #[inline]
    pub fn deg(&self, u: usize) -> usize {
        (self.off[u + 1] - self.off[u]) as usize
    }
}
#[derive(Clone, Copy)]
pub struct SyncPtr<T>(pub *mut T);
unsafe impl<T> Send for SyncPtr<T> {}
unsafe impl<T> Sync for SyncPtr<T> {}
impl<T: Copy> SyncPtr<T> {
    #[inline]
    pub unsafe fn set(&self, i: usize, v: T) {
        *self.0.add(i) = v;
    }
    #[inline]
    pub unsafe fn get(&self, i: usize) -> T {
        *self.0.add(i)
    }
}

pub const F_VIRTUAL: u8 = 1;
pub const F_LEAFP: u8 = 2;
pub const F_DEAD: u8 = 4;

/// Tree node of the map hierarchy (56 bytes on disk, little endian, repr(C)).
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct TNode {
    pub parent: u32,
    pub start: u32, // member range in order.u32
    pub end: u32,
    pub comm: u32, // community id at `level` (or u32::MAX for virtual)
    pub level: i8, // hierarchy level (0 = finest), -1 virtual, 127 root
    pub depth: u8,
    pub flags: u8,
    pub pad: u8,
    pub x: f32,
    pub y: f32,
    pub r: f32,
    pub node_r: f32, // for leaf-parents: nominal radius of a member node
    pub nkids: u32,
    pub internal: u64, // exact number of edges with both ends inside
    pub volume: u64,   // exact sum of degrees
}

pub fn args() -> HashMap<String, String> {
    let a: Vec<String> = std::env::args().skip(2).collect();
    let mut m = HashMap::new();
    let mut i = 0;
    while i < a.len() {
        let k = a[i].trim_start_matches("--").to_string();
        if i + 1 < a.len() && !a[i + 1].starts_with("--") {
            m.insert(k, a[i + 1].clone());
            i += 2;
        } else {
            m.insert(k, "1".to_string());
            i += 1;
        }
    }
    m
}
pub fn argf(m: &HashMap<String, String>, k: &str, d: f64) -> f64 {
    m.get(k).map(|s| s.parse().unwrap()).unwrap_or(d)
}
pub fn args_s(m: &HashMap<String, String>, k: &str) -> String {
    m.get(k).unwrap_or_else(|| panic!("missing --{k}")).clone()
}
pub fn splitmix(x: &mut u64) -> u64 {
    *x = x.wrapping_add(0x9E3779B97F4A7C15);
    let mut z = *x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}
pub fn rnd01(x: &mut u64) -> f32 {
    (splitmix(x) >> 40) as f32 / (1u64 << 24) as f32
}
pub fn peak_rss_mb() -> f64 {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    for l in s.lines() {
        if l.starts_with("VmHWM:") {
            let kb: f64 = l.split_whitespace().nth(1).unwrap().parse().unwrap();
            return kb / 1024.0;
        }
    }
    0.0
}
