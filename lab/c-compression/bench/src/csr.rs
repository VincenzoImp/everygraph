use anyhow::Result;
use rayon::prelude::*;

pub struct SendPtr<T>(pub *mut T);
unsafe impl<T> Send for SendPtr<T> {}
unsafe impl<T> Sync for SendPtr<T> {}

pub trait Lists {
    /// Calls f(v, list) for v in lo..hi in order.
    fn for_range<F: FnMut(usize, &[u32])>(&self, lo: usize, hi: usize, buf: &mut Vec<u32>, f: F);
    /// Random access to one list.
    fn get<'b>(&self, v: usize, buf: &'b mut Vec<u32>) -> &'b [u32];
    /// Work partition preferred by the codec (default: the given one).
    fn work_chunks(&self, default: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
        default
    }
}

pub struct Csr {
    pub n: usize,
    pub m: usize,
    _om: memmap2::Mmap,
    _tm: Option<memmap2::Mmap>,
    pub offsets: &'static [u64],
    pub targets: &'static [u32],
}

impl Csr {
    pub fn open(dir: &str) -> Result<Self> {
        let of = std::fs::File::open(format!("{dir}/offsets.u64"))?;
        let om = unsafe { memmap2::Mmap::map(&of)? };
        let offsets: &'static [u64] = unsafe { std::slice::from_raw_parts(om.as_ptr() as *const u64, om.len() / 8) };
        let n = offsets.len() - 1;
        let m = offsets[n] as usize;
        let tf = std::fs::File::open(format!("{dir}/targets.u32"))?;
        let (tm, targets) = if m > 0 {
            let tm = unsafe { memmap2::Mmap::map(&tf)? };
            let t: &'static [u32] = unsafe { std::slice::from_raw_parts(tm.as_ptr() as *const u32, m) };
            (Some(tm), t)
        } else {
            (None, &[][..])
        };
        Ok(Csr { n, m, _om: om, _tm: tm, offsets, targets })
    }
    #[inline(always)]
    pub fn list(&self, v: usize) -> &[u32] {
        unsafe { self.targets.get_unchecked(self.offsets[v] as usize..self.offsets[v + 1] as usize) }
    }
    pub fn checksum(&self) -> (u64, u64) {
        let s = self.targets.par_chunks(1 << 20).map(|c| c.iter().fold(0u64, |a, &x| a.wrapping_add(x as u64))).reduce(|| 0, |a, b| a.wrapping_add(b));
        (s, self.m as u64)
    }
}

impl Lists for Csr {
    #[inline]
    fn for_range<F: FnMut(usize, &[u32])>(&self, lo: usize, hi: usize, _buf: &mut Vec<u32>, mut f: F) {
        for v in lo..hi {
            f(v, self.list(v));
        }
    }
    #[inline]
    fn get<'b>(&self, v: usize, buf: &'b mut Vec<u32>) -> &'b [u32] {
        // copy out to be comparable with decoders (they materialize the list)
        buf.clear();
        buf.extend_from_slice(self.list(v));
        buf
    }
}

/// Node ranges with roughly `target` edges each (a single huge node gets its own chunk).
pub fn make_chunks(offsets: &[u64], target: u64) -> Vec<(usize, usize)> {
    let n = offsets.len() - 1;
    let m = offsets[n];
    let target = target.max(m / 8192).max(1);
    let mut v = Vec::new();
    let mut lo = 0;
    while lo < n {
        let goal = offsets[lo] + target;
        // first hi with offsets[hi] >= goal
        let mut hi = offsets.partition_point(|&o| o < goal).max(lo + 1).min(n);
        if hi - lo > 1 << 16 {
            hi = lo + (1 << 16);
        }
        v.push((lo, hi));
        lo = hi;
    }
    v
}

pub fn write_u64s(path: &str, v: &[u64]) -> Result<()> {
    let b: &[u8] = unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 8) };
    std::fs::write(path, b)?;
    Ok(())
}

pub fn read_u64s(path: &str) -> Result<Vec<u64>> {
    let b = std::fs::read(path)?;
    Ok(b.chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect())
}
