//! Semi-external kernels: O(n) state in RAM, adjacency streamed from the targets file with
//! explicit large reads (O_DIRECT or buffered) by each worker. Also an mmap variant with
//! per-chunk WILLNEED hints for comparison with naive mmap.
use crate::kernels::{gather_sum, uf_compress, Real};
use crate::SyncPtr;
use rayon::prelude::*;
use std::fs::File;
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use std::path::Path;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering::Relaxed};
use std::sync::Mutex;
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum IoMode {
    Direct,
    Buffered,
}

pub struct AlignedBuf {
    ptr: *mut u8,
    cap: usize,
}
unsafe impl Send for AlignedBuf {}
impl AlignedBuf {
    pub fn new(cap: usize) -> AlignedBuf {
        let cap = cap.max(4096).next_multiple_of(4096);
        let ptr = unsafe { std::alloc::alloc(std::alloc::Layout::from_size_align(cap, 4096).unwrap()) };
        assert!(!ptr.is_null());
        AlignedBuf { ptr, cap }
    }
    pub fn cap(&self) -> usize {
        self.cap
    }
    pub fn slice_mut(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.cap) }
    }
}
impl Drop for AlignedBuf {
    fn drop(&mut self) {
        unsafe { std::alloc::dealloc(self.ptr, std::alloc::Layout::from_size_align(self.cap, 4096).unwrap()) }
    }
}

pub struct EdgeStream {
    file: File,
    flen: u64,
    pool: Mutex<Vec<AlignedBuf>>,
    buf_cap: usize,
    pub bytes_read: AtomicU64,
    pub io_nanos: AtomicU64,
}

impl EdgeStream {
    pub fn open(targets: &Path, io: IoMode, offsets: &[u64], chunks: &[(usize, usize)]) -> std::io::Result<EdgeStream> {
        let mut o = File::options();
        o.read(true);
        if io == IoMode::Direct {
            o.custom_flags(libc::O_DIRECT);
        }
        let file = o.open(targets)?;
        let flen = file.metadata()?.len();
        let max_bytes = chunks.iter().map(|&(a, b)| (offsets[b] - offsets[a]) * 4).max().unwrap_or(0) as usize;
        Ok(EdgeStream { file, flen, pool: Mutex::new(Vec::new()), buf_cap: max_bytes + 8192, bytes_read: AtomicU64::new(0), io_nanos: AtomicU64::new(0) })
    }
    /// Read the adjacency entries [e0, e1) and hand them to `f`.
    #[inline]
    pub fn with_entries<R>(&self, e0: u64, e1: u64, f: impl FnOnce(&[u32]) -> R) -> R {
        let mut buf = self.pool.lock().unwrap().pop().unwrap_or_else(|| AlignedBuf::new(self.buf_cap));
        let b0 = e0 * 4;
        let b1 = e1 * 4;
        let a0 = b0 & !4095;
        let a1 = b1.next_multiple_of(4096);
        let want = (b1 - a0) as usize;
        let t = Instant::now();
        let s = buf.slice_mut();
        let mut got = 0usize;
        while got < want {
            let k = self.file.read_at(&mut s[got..(a1 - a0) as usize], a0 + got as u64).expect("read targets");
            if k == 0 {
                break;
            }
            got += k;
        }
        assert!(got >= want || a0 + got as u64 >= self.flen, "short read");
        self.io_nanos.fetch_add(t.elapsed().as_nanos() as u64, Relaxed);
        self.bytes_read.fetch_add(got as u64, Relaxed);
        let r = {
            let sl = unsafe { std::slice::from_raw_parts(buf.ptr.add((b0 - a0) as usize) as *const u32, (e1 - e0) as usize) };
            f(sl)
        };
        self.pool.lock().unwrap().push(buf);
        r
    }
}

/// Semi-external pull PageRank. RAM: offsets (8n) + 2 contrib arrays + rank (3 * n * sizeof F)
/// + threads * chunk buffer. Edges: one sequential pass over targets.u32 per iteration.
pub fn pagerank_semiext<F: Real>(offsets: &[u64], es: &EdgeStream, damping: f64, iters: usize, chunks: &[(usize, usize)]) -> (Vec<F>, Vec<f64>) {
    let n = offsets.len() - 1;
    let inv_n = 1.0 / n as f64;
    let mut contrib: Vec<F> = (0..n)
        .into_par_iter()
        .map(|v| {
            let d = offsets[v + 1] - offsets[v];
            if d == 0 { F::from_f64(0.0) } else { F::from_f64(inv_n / d as f64) }
        })
        .collect();
    let mut dangling: f64 = (0..n).into_par_iter().map(|v| if offsets[v + 1] == offsets[v] { inv_n } else { 0.0 }).sum();
    let mut next: Vec<F> = (0..n).into_par_iter().map(|_| F::from_f64(0.0)).collect();
    let mut rank: Vec<F> = (0..n).into_par_iter().map(|_| F::from_f64(inv_n)).collect();
    let mut iter_secs = Vec::new();
    for it in 0..iters {
        let t = Instant::now();
        let base = F::from_f64((1.0 - damping) * inv_n + damping * dangling * inv_n);
        let dmp = F::from_f64(damping);
        let last = it + 1 == iters;
        let np = SyncPtr(next.as_mut_ptr());
        let rp = SyncPtr(rank.as_mut_ptr());
        let c = &contrib[..];
        dangling = chunks
            .par_iter()
            .with_max_len(1)
            .map(|&(a, b)| {
                let e0 = offsets[a];
                es.with_entries(e0, offsets[b], |tg| {
                    let mut dang = 0f64;
                    for v in a..b {
                        unsafe {
                            let s = (*offsets.get_unchecked(v) - e0) as usize;
                            let e = (*offsets.get_unchecked(v + 1) - e0) as usize;
                            let sum = gather_sum(c, tg.get_unchecked(s..e));
                            let r = base + dmp * sum;
                            if e == s {
                                dang += r.to_f64();
                                np.write(v, F::from_f64(0.0));
                            } else {
                                np.write(v, r / F::from_f64((e - s) as f64));
                            }
                            if last {
                                rp.write(v, r);
                            }
                        }
                    }
                    dang
                })
            })
            .sum();
        std::mem::swap(&mut contrib, &mut next);
        iter_secs.push(t.elapsed().as_secs_f64());
    }
    (rank, iter_secs)
}

/// Semi-external WCC: one sequential pass, concurrent union-find over n u32 labels in RAM.
pub fn wcc_semiext(offsets: &[u64], es: &EdgeStream, chunks: &[(usize, usize)]) -> Vec<AtomicU32> {
    let n = offsets.len() - 1;
    let comp: Vec<AtomicU32> = (0..n as u32).into_par_iter().map(AtomicU32::new).collect();
    chunks.par_iter().with_max_len(1).for_each(|&(a, b)| {
        let e0 = offsets[a];
        es.with_entries(e0, offsets[b], |tg| {
            for u in a..b {
                let s = (offsets[u] - e0) as usize;
                let e = (offsets[u + 1] - e0) as usize;
                crate::kernels::uf_stream_node(&comp, u as u32, &tg[s..e]);
            }
        })
    });
    uf_compress(&comp);
    comp
}

/// mmap + explicit hints: WILLNEED on the chunk before use, then drop the mapping's pages
/// (`MADV_PAGEOUT` on kernels that have it) so resident file pages stay bounded.
pub fn pagerank_mmap_hinted<F: Real>(offsets: &[u64], targets: &[u32], damping: f64, iters: usize, chunks: &[(usize, usize)], pageout: bool) -> (Vec<F>, Vec<f64>) {
    let n = offsets.len() - 1;
    let inv_n = 1.0 / n as f64;
    let mut contrib: Vec<F> = (0..n)
        .into_par_iter()
        .map(|v| {
            let d = offsets[v + 1] - offsets[v];
            if d == 0 { F::from_f64(0.0) } else { F::from_f64(inv_n / d as f64) }
        })
        .collect();
    let mut dangling: f64 = 0.0;
    let mut next: Vec<F> = (0..n).into_par_iter().map(|_| F::from_f64(0.0)).collect();
    let mut rank: Vec<F> = (0..n).into_par_iter().map(|_| F::from_f64(inv_n)).collect();
    let mut iter_secs = Vec::new();
    let base_ptr = targets.as_ptr() as usize;
    for it in 0..iters {
        let t = Instant::now();
        let base = F::from_f64((1.0 - damping) * inv_n + damping * dangling * inv_n);
        let dmp = F::from_f64(damping);
        let last = it + 1 == iters;
        let np = SyncPtr(next.as_mut_ptr());
        let rp = SyncPtr(rank.as_mut_ptr());
        let c = &contrib[..];
        dangling = chunks
            .par_iter()
            .with_max_len(1)
            .map(|&(a, b)| {
                let p0 = (base_ptr + offsets[a] as usize * 4) & !4095;
                let p1 = (base_ptr + offsets[b] as usize * 4).next_multiple_of(4096);
                unsafe {
                    libc::madvise(p0 as *mut _, p1 - p0, libc::MADV_WILLNEED);
                }
                let mut dang = 0f64;
                for v in a..b {
                    unsafe {
                        let s = *offsets.get_unchecked(v) as usize;
                        let e = *offsets.get_unchecked(v + 1) as usize;
                        let sum = gather_sum(c, targets.get_unchecked(s..e));
                        let r = base + dmp * sum;
                        if e == s {
                            dang += r.to_f64();
                            np.write(v, F::from_f64(0.0));
                        } else {
                            np.write(v, r / F::from_f64((e - s) as f64));
                        }
                        if last {
                            rp.write(v, r);
                        }
                    }
                }
                if pageout && p1 - p0 > 8192 {
                    unsafe {
                        libc::madvise((p0 + 4096) as *mut _, p1 - p0 - 8192, libc::MADV_PAGEOUT);
                    }
                }
                dang
            })
            .sum();
        std::mem::swap(&mut contrib, &mut next);
        iter_secs.push(t.elapsed().as_secs_f64());
    }
    (rank, iter_secs)
}
