// SPDX-License-Identifier: Apache-2.0 OR MIT

//! m1-09 (#23): node orderings of a symmetric graph, and what they do to our codec, the
//! log-gap cost and the proof kernels.
//!
//! ```text
//! m109 order --graph CSR --method M --out PERM [--hier DIR] [--seed S] [--threads T]
//!            [--iters I] [--depth D] [--start random|input]
//! m109 eval  --graph CSR --perm PERM|identity --name NAME [--threads T] [--reps R]
//!            [--kernels pr,bfs,wcc] [--sources raw,compressed]
//! m109 arcs  --graph CSR [--perm PERM]   (TAB-separated arcs on stdout, for webgraph)
//! m109 permute --graph CSR --perm PERM --out CSR2  (the permuted graph as glab-csr-v0)
//! m109 frombin --perm FILE --out PERM    (webgraph's java permutation: u64 big-endian)
//! ```
//!
//! CSR is a glab-csr-v0 directory (`offsets.u64`, `targets.u32`). A permutation file
//! holds `n` little-endian u32: the new id of each old id. Every command prints one
//! JSON line on stdout.

mod bp;
mod eval;
mod order;

use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

pub type Args = HashMap<String, String>;

/// A symmetric CSR in memory.
pub struct Graph {
    pub offsets: Vec<u64>,
    pub targets: Targets,
}

/// The targets on the heap, or memory-mapped (page cache, which the kernel can reclaim:
/// for the two largest graphs, which do not fit twice on the heap next to other work).
pub enum Targets {
    Heap(Vec<u32>),
    Map(memmap2::Mmap),
}

impl std::ops::Deref for Targets {
    type Target = [u32];
    fn deref(&self) -> &[u32] {
        match self {
            Targets::Heap(v) => v,
            // SAFETY: the map is page-aligned and its length was checked to be a multiple of 4.
            Targets::Map(m) => unsafe { std::slice::from_raw_parts(m.as_ptr().cast::<u32>(), m.len() / 4) },
        }
    }
}

impl Graph {
    pub fn n(&self) -> usize {
        self.offsets.len() - 1
    }
    #[inline]
    pub fn list(&self, v: u32) -> &[u32] {
        let v = v as usize;
        &self.targets[self.offsets[v] as usize..self.offsets[v + 1] as usize]
    }
    pub fn degree(&self, v: u32) -> u32 {
        let v = v as usize;
        (self.offsets[v + 1] - self.offsets[v]) as u32
    }
}

fn map(path: &Path) -> memmap2::Mmap {
    let f = std::fs::File::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    // SAFETY: the lab's data files are read-only while the lab runs.
    unsafe { memmap2::Mmap::map(&f).expect("mmap") }
}

/// The words of a little-endian file, copied to the heap (so that timings do not touch
/// the page cache).
pub fn read_words<T: Copy + Send + Sync + Default>(path: &Path) -> Vec<T> {
    let m = map(path);
    assert_eq!(m.len() % size_of::<T>(), 0, "{}", path.display());
    let len = m.len() / size_of::<T>();
    let mut v = vec![T::default(); len];
    // SAFETY: the map is page-aligned, so aligned for T; T is a plain integer type.
    let src: &[T] = unsafe { std::slice::from_raw_parts(m.as_ptr().cast::<T>(), len) };
    use rayon::prelude::*;
    v.par_chunks_mut(1 << 20)
        .zip(src.par_chunks(1 << 20))
        .for_each(|(d, s)| d.copy_from_slice(s));
    v
}

pub fn load(dir: &Path) -> Graph {
    load_as(dir, false)
}

pub fn load_as(dir: &Path, mapped: bool) -> Graph {
    let offsets = read_words::<u64>(&dir.join("offsets.u64"));
    let path = dir.join("targets.u32");
    let targets = if mapped {
        let m = map(&path);
        assert_eq!(m.len() % 4, 0);
        Targets::Map(m)
    } else {
        Targets::Heap(read_words::<u32>(&path))
    };
    assert_eq!(*offsets.last().unwrap() as usize, targets.len());
    Graph { offsets, targets }
}

pub fn write_u32(path: &Path, v: &[u32]) {
    let mut bytes = Vec::with_capacity(v.len() * 4);
    for x in v {
        bytes.extend_from_slice(&x.to_le_bytes());
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).expect("write");
    std::fs::rename(tmp, path).expect("rename");
}

/// The permutation in `path`, checked to be a bijection of `0..n`.
pub fn read_perm(path: &Path, n: usize) -> Vec<u32> {
    let p = read_words::<u32>(path);
    assert_eq!(p.len(), n, "{}: not n entries", path.display());
    let mut seen = vec![false; n];
    for &x in &p {
        assert!(!std::mem::replace(&mut seen[x as usize], true), "not a bijection");
    }
    p
}

/// `order[i]` is the old id placed at position `i`: the permutation is its inverse.
pub fn perm_of_order(order: &[u32]) -> Vec<u32> {
    let mut p = vec![u32::MAX; order.len()];
    for (i, &v) in order.iter().enumerate() {
        assert_eq!(p[v as usize], u32::MAX, "order repeats {v}");
        p[v as usize] = i as u32;
    }
    p
}

pub fn get<T: std::str::FromStr>(a: &Args, k: &str, default: T) -> T {
    a.get(k).map_or(default, |s| s.parse().unwrap_or_else(|_| panic!("--{k} {s}")))
}

fn args() -> (String, Args) {
    let mut it = std::env::args().skip(1);
    let cmd = it.next().unwrap_or_default();
    let mut a = HashMap::new();
    while let Some(k) = it.next() {
        a.insert(k.trim_start_matches("--").to_owned(), it.next().unwrap_or_default());
    }
    (cmd, a)
}

fn arcs(a: &Args) {
    use std::io::Write;
    let g = load(Path::new(&a["graph"]));
    let g = match a.get("perm") {
        Some(p) => eval::permute(&g, &read_perm(Path::new(p), g.n())),
        None => g,
    };
    let mut out = std::io::BufWriter::with_capacity(1 << 22, std::io::stdout().lock());
    for v in 0..g.n() as u32 {
        for &x in g.list(v) {
            writeln!(out, "{v}\t{x}").expect("stdout");
        }
    }
}

fn permute(a: &Args) {
    let g = load(Path::new(&a["graph"]));
    let h = eval::permute(&g, &read_perm(Path::new(&a["perm"]), g.n()));
    let out = Path::new(&a["out"]);
    std::fs::create_dir_all(out).expect("out");
    let mut o = Vec::with_capacity(h.offsets.len() * 8);
    for x in &h.offsets {
        o.extend_from_slice(&x.to_le_bytes());
    }
    std::fs::write(out.join("offsets.u64"), o).expect("write");
    write_u32(&out.join("targets.u32"), &h.targets);
    let meta = std::fs::read_to_string(Path::new(&a["graph"]).join("meta.json")).expect("meta");
    std::fs::write(out.join("meta.json"), meta).expect("write");
}

fn frombin(a: &Args) {
    let bytes = std::fs::read(&a["perm"]).expect("read");
    let p: Vec<u32> = bytes
        .chunks_exact(8)
        .map(|c| u32::try_from(u64::from_be_bytes(c.try_into().unwrap())).unwrap())
        .collect();
    write_u32(Path::new(&a["out"]), &p);
}

fn main() {
    let (cmd, a) = args();
    let threads: usize = get(&a, "threads", 16);
    rayon::ThreadPoolBuilder::new().num_threads(threads).build_global().unwrap();
    let t = Instant::now();
    match cmd.as_str() {
        "order" => order::main(&a, threads),
        "eval" => eval::main(&a, threads),
        "arcs" => arcs(&a),
        "permute" => permute(&a),
        "frombin" => frombin(&a),
        _ => {
            eprintln!("usage: m109 order|eval|arcs|frombin --graph DIR ... (see the source)");
            std::process::exit(2);
        }
    }
    eprintln!("m109 {cmd}: {:.1} s", t.elapsed().as_secs_f64());
}
