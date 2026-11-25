//! Elias–Fano encoding of a non-decreasing sequence of u64 (the byte offsets of the adjacency
//! records). Own implementation (~150 lines, no dependencies, so it also works for wasm32):
//!   low bits : n values of `l` bits, packed
//!   high bits: unary, value i sets bit (v_i >> l) + i
//!   select   : position of every 64th one bit (rebuilt at open, not stored)
//! Lab C used sux's EfSeq for the same purpose; sizes are the same up to the select samples.

pub const SAMPLE_LOG: u32 = 6;

pub struct EliasFano {
    pub n: usize,
    pub universe: u64,
    l: u32,
    low: Vec<u8>,      // packed, 8 bytes of padding
    high: Vec<u64>,    // one padding word
    samples: Vec<u64>, // bit position of the (k << SAMPLE_LOG)-th one
}

#[inline(always)]
fn select_in_word(w: u64, r: u32) -> u32 {
    #[cfg(target_feature = "bmi2")]
    unsafe {
        std::arch::x86_64::_pdep_u64(1u64 << r, w).trailing_zeros()
    }
    #[cfg(not(target_feature = "bmi2"))]
    {
        let mut w = w;
        for _ in 0..r {
            w &= w - 1;
        }
        w.trailing_zeros()
    }
}

impl EliasFano {
    /// `vals` must be non-decreasing; `universe` >= last value.
    pub fn build(vals: &[u64], universe: u64) -> EliasFano {
        let n = vals.len();
        let l = if n == 0 || universe / (n as u64) < 2 { 0 } else { 63 - (universe / n as u64).leading_zeros() };
        assert!(l <= 56);
        let mut low = vec![0u8; (n * l as usize).div_ceil(8) + 8];
        let hbits = n + (universe >> l) as usize + 1;
        let mut high = vec![0u64; hbits.div_ceil(64) + 1];
        let mask = if l == 0 { 0 } else { (1u64 << l) - 1 };
        let mut prev = 0u64;
        for (i, &v) in vals.iter().enumerate() {
            debug_assert!(v >= prev && v <= universe);
            prev = v;
            if l > 0 {
                let bit = i * l as usize;
                let p = unsafe { low.as_mut_ptr().add(bit >> 3) as *mut u64 };
                unsafe { p.write_unaligned(p.read_unaligned() | ((v & mask) << (bit & 7))) };
            }
            let h = (v >> l) as usize + i;
            high[h >> 6] |= 1u64 << (h & 63);
        }
        let mut ef = EliasFano { n, universe, l, low, high, samples: Vec::new() };
        ef.build_samples();
        ef
    }

    fn build_samples(&mut self) {
        let mut samples = Vec::with_capacity((self.n >> SAMPLE_LOG) + 1);
        let mut ones = 0usize;
        for (wi, &w) in self.high.iter().enumerate() {
            let c = w.count_ones() as usize;
            // does a multiple of 2^SAMPLE_LOG fall in [ones, ones + c)?
            let mut next = (ones + (1 << SAMPLE_LOG) - 1) >> SAMPLE_LOG << SAMPLE_LOG;
            while next < ones + c {
                samples.push((wi as u64) << 6 | select_in_word(w, (next - ones) as u32) as u64);
                next += 1 << SAMPLE_LOG;
            }
            ones += c;
        }
        debug_assert_eq!(ones, self.n);
        self.samples = samples;
    }

    /// Bytes of the stored representation (low + high bits, without the select samples).
    pub fn stored_bytes(&self) -> usize {
        (self.n * self.l as usize).div_ceil(8) + (self.n + (self.universe >> self.l) as usize + 1).div_ceil(8)
    }
    /// Bytes held in RAM (including the select samples and padding).
    pub fn ram_bytes(&self) -> usize {
        self.low.len() + self.high.len() * 8 + self.samples.len() * 8
    }

    pub fn write(&self, path: &std::path::Path) -> std::io::Result<()> {
        use std::io::Write;
        let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
        f.write_all(&(self.n as u64).to_le_bytes())?;
        f.write_all(&self.universe.to_le_bytes())?;
        f.write_all(&(self.l as u64).to_le_bytes())?;
        f.write_all(&(self.low.len() as u64).to_le_bytes())?;
        f.write_all(&self.low)?;
        f.write_all(&(self.high.len() as u64).to_le_bytes())?;
        for w in &self.high {
            f.write_all(&w.to_le_bytes())?;
        }
        f.flush()
    }
    pub fn read(path: &std::path::Path) -> std::io::Result<EliasFano> {
        let b = std::fs::read(path)?;
        let u = |o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap());
        let (n, universe, l, ll) = (u(0) as usize, u(8), u(16) as u32, u(24) as usize);
        let low = b[32..32 + ll].to_vec();
        let hl = u(32 + ll) as usize;
        let high: Vec<u64> = (0..hl).map(|i| u(40 + ll + 8 * i)).collect();
        let mut ef = EliasFano { n, universe, l, low, high, samples: Vec::new() };
        ef.build_samples();
        Ok(ef)
    }

    #[inline(always)]
    fn low_at(&self, i: usize) -> u64 {
        if self.l == 0 {
            return 0;
        }
        let bit = i * self.l as usize;
        let w = unsafe { (self.low.as_ptr().add(bit >> 3) as *const u64).read_unaligned() };
        (w >> (bit & 7)) & ((1u64 << self.l) - 1)
    }

    /// Bit position of the i-th one in the high bits.
    #[inline(always)]
    fn select(&self, i: usize) -> usize {
        unsafe {
            let s = *self.samples.get_unchecked(i >> SAMPLE_LOG) as usize;
            let mut r = (i & ((1 << SAMPLE_LOG) - 1)) as u32;
            let mut wi = s >> 6;
            let mut w = *self.high.get_unchecked(wi) & (!0u64 << (s & 63));
            loop {
                let c = w.count_ones();
                if r < c {
                    return wi << 6 | select_in_word(w, r) as usize;
                }
                r -= c;
                wi += 1;
                w = *self.high.get_unchecked(wi);
            }
        }
    }

    #[inline(always)]
    pub fn get(&self, i: usize) -> u64 {
        debug_assert!(i < self.n);
        (((self.select(i) - i) as u64) << self.l) | self.low_at(i)
    }

    /// Values i and i+1 (the second by scanning forward from the first one bit).
    #[inline(always)]
    pub fn get2(&self, i: usize) -> (u64, u64) {
        debug_assert!(i + 1 < self.n);
        let p = self.select(i);
        let a = (((p - i) as u64) << self.l) | self.low_at(i);
        unsafe {
            let mut wi = p >> 6;
            // bits strictly above p in the same word
            let mut w = *self.high.get_unchecked(wi) & (!0u64 << (p & 63)) & !(1u64 << (p & 63));
            while w == 0 {
                wi += 1;
                w = *self.high.get_unchecked(wi);
            }
            let q = wi << 6 | w.trailing_zeros() as usize;
            (a, (((q - i - 1) as u64) << self.l) | self.low_at(i + 1))
        }
    }

    /// Sequential access starting at index i.
    #[inline(always)]
    pub fn iter_from(&self, i: usize) -> EfIter<'_> {
        if i >= self.n {
            return EfIter { ef: self, i, wi: 0, w: 0 };
        }
        let p = self.select(i);
        let wi = p >> 6;
        let w = unsafe { *self.high.get_unchecked(wi) } & (!0u64 << (p & 63));
        EfIter { ef: self, i, wi, w }
    }
}

pub struct EfIter<'a> {
    ef: &'a EliasFano,
    i: usize,
    wi: usize,
    w: u64,
}
impl EfIter<'_> {
    /// Next value; the caller must not read past the end.
    #[inline(always)]
    pub fn next(&mut self) -> u64 {
        unsafe {
            while self.w == 0 {
                self.wi += 1;
                self.w = *self.ef.high.get_unchecked(self.wi);
            }
        }
        let p = self.wi << 6 | self.w.trailing_zeros() as usize;
        self.w &= self.w - 1;
        let v = (((p - self.i) as u64) << self.ef.l) | self.ef.low_at(self.i);
        self.i += 1;
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip() {
        let mut x = 0x9E3779B97F4A7C15u64;
        for &(n, step) in &[(1usize, 5u64), (2, 1), (1000, 1), (1000, 300), (70_000, 17), (70_000, 100_000), (5, 0)] {
            let mut v = Vec::with_capacity(n);
            let mut acc = 0u64;
            for _ in 0..n {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                acc += if step == 0 { 0 } else { x % (2 * step) };
                v.push(acc);
            }
            let ef = EliasFano::build(&v, acc);
            for i in 0..n {
                assert_eq!(ef.get(i), v[i], "get n={n} step={step} i={i}");
                if i + 1 < n {
                    assert_eq!(ef.get2(i), (v[i], v[i + 1]));
                }
            }
            for start in [0, n / 3, n - 1] {
                let mut it = ef.iter_from(start);
                for i in start..n {
                    assert_eq!(it.next(), v[i]);
                }
            }
        }
    }
}
