//! Minimal glab-csr-v0 reader + kernels (PageRank, WCC, CSR build).
use memmap2::Mmap;
use rayon::prelude::*;
use std::fs::File;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

pub enum Store<T: 'static> {
    Mapped(Mmap, std::marker::PhantomData<T>),
    Owned(Vec<T>),
}
impl<T: Copy> Store<T> {
    pub fn as_slice(&self) -> &[T] {
        match self {
            Store::Mapped(m, _) => unsafe {
                std::slice::from_raw_parts(m.as_ptr() as *const T, m.len() / std::mem::size_of::<T>())
            },
            Store::Owned(v) => v.as_slice(),
        }
    }
}

pub struct Csr {
    pub offsets: Store<u64>,
    pub targets: Store<u32>,
}

fn map<T>(p: &Path) -> std::io::Result<Store<T>> {
    let f = File::open(p)?;
    let m = unsafe { Mmap::map(&f)? };
    Ok(Store::Mapped(m, std::marker::PhantomData))
}

impl Csr {
    pub fn open(dir: &str) -> std::io::Result<Csr> {
        let d = Path::new(dir);
        Ok(Csr { offsets: map(&d.join("offsets.u64"))?, targets: map(&d.join("targets.u32"))? })
    }
    #[inline] pub fn n(&self) -> usize { self.offsets.as_slice().len() - 1 }
    #[inline] pub fn entries(&self) -> usize { self.targets.as_slice().len() }
    #[inline] pub fn neighbors(&self, v: usize) -> &[u32] {
        let o = self.offsets.as_slice();
        &self.targets.as_slice()[o[v] as usize..o[v + 1] as usize]
    }
    pub fn degree(&self) -> Vec<u32> {
        let o = self.offsets.as_slice();
        (0..self.n()).into_par_iter().map(|i| (o[i + 1] - o[i]) as u32).collect()
    }

    /// Build a symmetric, sorted, deduplicated, loop-free CSR from an edge list (any direction).
    pub fn from_edges(src: &[u32], dst: &[u32], n: usize) -> Csr {
        assert_eq!(src.len(), dst.len());
        let deg: Vec<AtomicU32> = (0..n).map(|_| AtomicU32::new(0)).collect();
        src.par_iter().zip(dst.par_iter()).for_each(|(&a, &b)| {
            if a != b {
                deg[a as usize].fetch_add(1, Ordering::Relaxed);
                deg[b as usize].fetch_add(1, Ordering::Relaxed);
            }
        });
        let mut off = vec![0u64; n + 1];
        for i in 0..n { off[i + 1] = off[i] + deg[i].load(Ordering::Relaxed) as u64; }
        let cursor: Vec<AtomicU32> = (0..n).map(|_| AtomicU32::new(0)).collect();
        let total = off[n] as usize;
        let tg: Vec<AtomicU32> = (0..total).map(|_| AtomicU32::new(0)).collect();
        src.par_iter().zip(dst.par_iter()).for_each(|(&a, &b)| {
            if a != b {
                let p = off[a as usize] + cursor[a as usize].fetch_add(1, Ordering::Relaxed) as u64;
                tg[p as usize].store(b, Ordering::Relaxed);
                let q = off[b as usize] + cursor[b as usize].fetch_add(1, Ordering::Relaxed) as u64;
                tg[q as usize].store(a, Ordering::Relaxed);
            }
        });
        let mut tg: Vec<u32> = unsafe { std::mem::transmute(tg) };
        // sort + dedup each adjacency in parallel, then compact
        let mut chunks: Vec<&mut [u32]> = Vec::with_capacity(n);
        let mut rest: &mut [u32] = &mut tg;
        for i in 0..n {
            let (a, b) = rest.split_at_mut((off[i + 1] - off[i]) as usize);
            chunks.push(a);
            rest = b;
        }
        let newdeg: Vec<u64> = chunks.par_iter_mut().map(|c| {
            c.sort_unstable();
            let mut w = 0usize;
            for r in 0..c.len() { if r == 0 || c[r] != c[w - 1] { c[w] = c[r]; w += 1; } }
            w as u64
        }).collect();
        let mut noff = vec![0u64; n + 1];
        for i in 0..n { noff[i + 1] = noff[i] + newdeg[i]; }
        let mut ntg = Vec::with_capacity(noff[n] as usize);
        for i in 0..n { ntg.extend_from_slice(&tg[off[i] as usize..(off[i] + newdeg[i]) as usize]); }
        Csr { offsets: Store::Owned(noff), targets: Store::Owned(ntg) }
    }

    /// PageRank with NetworkX semantics on a symmetric graph (each undirected edge = 2 arcs,
    /// dangling mass redistributed uniformly, stop when L1 change < n*tol).
    pub fn pagerank(&self, alpha: f64, max_iter: usize, tol: f64) -> (Vec<f64>, usize) {
        let n = self.n();
        let o = self.offsets.as_slice();
        let t = self.targets.as_slice();
        let deg: Vec<f64> = (0..n).into_par_iter().map(|i| (o[i + 1] - o[i]) as f64).collect();
        let mut x = vec![1.0 / n as f64; n];
        let mut contrib = vec![0f64; n];
        let mut xn = vec![0f64; n];
        for it in 0..max_iter {
            let dangling: f64 = contrib.par_iter_mut().zip(x.par_iter()).zip(deg.par_iter())
                .map(|((c, &xi), &d)| { if d > 0.0 { *c = xi / d; 0.0 } else { *c = 0.0; xi } }).sum();
            let base = (1.0 - alpha) / n as f64 + alpha * dangling / n as f64;
            let err: f64 = xn.par_iter_mut().enumerate().with_min_len(4096).map(|(v, out)| {
                let mut s = 0.0;
                for &u in &t[o[v] as usize..o[v + 1] as usize] { s += contrib[u as usize]; }
                *out = base + alpha * s;
                (*out - x[v]).abs()
            }).sum();
            std::mem::swap(&mut x, &mut xn);
            if err < n as f64 * tol { return (x, it + 1); }
        }
        (x, max_iter)
    }

    /// Connected components via parallel lock-free union-find (Afforest-lite).
    /// Returns a label per node = min node id of its component.
    pub fn wcc(&self) -> Vec<u32> {
        let n = self.n();
        let parent: Vec<AtomicU32> = (0..n as u32).map(AtomicU32::new).collect();
        fn find(p: &[AtomicU32], mut x: u32) -> u32 {
            loop {
                let px = p[x as usize].load(Ordering::Relaxed);
                if px == x { return x; }
                let gp = p[px as usize].load(Ordering::Relaxed);
                if gp != px { let _ = p[x as usize].compare_exchange(px, gp, Ordering::Relaxed, Ordering::Relaxed); }
                x = gp;
            }
        }
        (0..n).into_par_iter().with_min_len(1024).for_each(|v| {
            for &u in self.neighbors(v) {
                if (u as usize) < v { continue; }
                let mut a = find(&parent, v as u32);
                let mut b = find(&parent, u);
                while a != b {
                    if a < b { std::mem::swap(&mut a, &mut b); }
                    // hook larger root under smaller
                    match parent[a as usize].compare_exchange(a, b, Ordering::AcqRel, Ordering::Relaxed) {
                        Ok(_) => break,
                        Err(_) => { a = find(&parent, a); b = find(&parent, b); }
                    }
                }
            }
        });
        (0..n).into_par_iter().map(|v| find(&parent, v as u32)).collect()
    }
}
