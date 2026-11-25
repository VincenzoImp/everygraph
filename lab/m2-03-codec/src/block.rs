// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Variant B of the framing A/B: **block framing**. The same words as the production
//! records (per list: zigzag32(first − v), then gaps − 1), but one Stream VByte stream
//! per block — the control bytes of all the block's words, then their data — so that a
//! block decodes in one loop over groups of four with no per-list branch. List
//! boundaries are handled by a segmented prefix sum: a bitmap of list starts, built
//! from the degrees, selects per group which lanes restart from `v + unzigzag(w)`.
//!
//! Lab code (x86-64 with SSSE3 only, raw pointers); degrees and offsets are plain
//! arrays here, Elias–Fano in production, which favours B slightly.

use std::arch::x86_64::*;

pub struct Blocked {
    /// First node of each block, then n.
    pub starts: Vec<u32>,
    /// Byte offset of each block's stream, then the total.
    pub block_bytes: Vec<u64>,
    /// Entry offsets (n + 1).
    pub offsets: Vec<u64>,
    /// Byte offset, inside its block's data part, of each list's first word (random access).
    pub word_bytes: Vec<u32>,
    pub data: Vec<u8>,
}

fn zz(v: u32, x: u32) -> u32 {
    let d = x.wrapping_sub(v);
    (d << 1) ^ 0u32.wrapping_sub(d >> 31)
}

fn blen(w: u32) -> usize {
    match w {
        0..=0xff => 1,
        0x100..=0xffff => 2,
        0x1_0000..=0xff_ffff => 3,
        _ => 4,
    }
}

impl Blocked {
    pub fn encode(offsets: &[u64], targets: &[u32], starts: Vec<u32>) -> Blocked {
        let n = offsets.len() - 1;
        let mut data = Vec::new();
        let mut block_bytes = vec![0u64];
        let mut word_bytes = vec![0u32; n];
        let mut words = Vec::new();
        for b in starts.windows(2) {
            words.clear();
            let (lo, hi) = (b[0] as usize, b[1] as usize);
            let mut pos = 0usize;
            for v in lo..hi {
                let l = &targets[offsets[v] as usize..offsets[v + 1] as usize];
                word_bytes[v] = pos as u32;
                for (j, &x) in l.iter().enumerate() {
                    let w = if j == 0 { zz(v as u32, x) } else { x - l[j - 1] - 1 };
                    pos += blen(w);
                    words.push(w);
                }
            }
            let c0 = data.len();
            data.resize(c0 + words.len().div_ceil(4), 0);
            for (i, &w) in words.iter().enumerate() {
                let l = blen(w);
                data[c0 + i / 4] |= ((l - 1) as u8) << (2 * (i % 4));
                data.extend_from_slice(&w.to_le_bytes()[..l]);
            }
            block_bytes.push(data.len() as u64);
        }
        data.extend_from_slice(&[0u8; 64]); // over-read padding (lab)
        Blocked { starts, block_bytes, offsets: offsets.to_vec(), word_bytes, data }
    }

    pub fn bytes(&self) -> usize {
        self.data.len() - 64
    }

    /// Decodes block `b` into `out` (its entries + 3 slack); `bits` and `nodes` are
    /// scratch. Returns the entries.
    #[inline(never)]
    pub fn decode_block(&self, b: usize, out: &mut Vec<u32>, bits: &mut Vec<u64>, nodes: &mut Vec<u32>) -> usize {
        let (lo, hi) = (self.starts[b] as usize, self.starts[b + 1] as usize);
        let e0 = self.offsets[lo];
        let total = (self.offsets[hi] - e0) as usize;
        if out.len() < total + 4 {
            out.resize(total + 4, 0);
        }
        // Starts bitmap and the node of each non-empty list, branch-free.
        bits.clear();
        bits.resize(total.div_ceil(64) + 1, 0);
        nodes.clear();
        nodes.resize(hi - lo + 4, 0);
        let mut k = 0;
        for v in lo..hi {
            let p = (self.offsets[v] - e0) as usize;
            let nonempty = self.offsets[v + 1] > self.offsets[v];
            bits[p / 64] |= (nonempty as u64) << (p % 64);
            nodes[k] = v as u32;
            k += nonempty as usize;
        }
        unsafe { decode_ssse3(&self.data[self.block_bytes[b] as usize..], total, bits, nodes, out) };
        total
    }

    /// Random access: the list of `v` (scalar prefix sum over shuffled words).
    pub fn list(&self, v: usize, out: &mut Vec<u32>) -> usize {
        let d = (self.offsets[v + 1] - self.offsets[v]) as usize;
        if d == 0 {
            return 0;
        }
        let b = self.starts.partition_point(|&s| s as usize <= v) - 1;
        let blk = &self.data[self.block_bytes[b] as usize..];
        let e0 = self.offsets[self.starts[b] as usize];
        let total = (self.offsets[self.starts[b + 1] as usize] - e0) as usize;
        let nc = total.div_ceil(4);
        let first = (self.offsets[v] - e0) as usize;
        if out.len() < d + 8 {
            out.resize(d + 8, 0);
        }
        let mut p = nc + self.word_bytes[v] as usize;
        let mut acc = 0u32;
        for j in 0..d {
            let i = first + j;
            let l = ((blk[i / 4] >> (2 * (i % 4))) & 3) as usize + 1;
            let mut w = 0u32;
            for (t, &byte) in blk[p..p + l].iter().enumerate() {
                w |= (byte as u32) << (8 * t);
            }
            p += l;
            acc = if j == 0 { (v as u32).wrapping_add((w >> 1) ^ 0u32.wrapping_sub(w & 1)) } else { acc + w + 1 };
            out[j] = acc;
        }
        d
    }
}

struct Tables {
    shuf: [[u8; 16]; 256],
    len: [u8; 256],
    /// Per 4-bit start mask: start lanes, keep after 1-lane shift, keep after 2-lane shift,
    /// carry lanes, and the shuffle placing start j's node in the lane of start j.
    start: [[u32; 4]; 16],
    keep1: [[u32; 4]; 16],
    keep2: [[u32; 4]; 16],
    carry: [[u32; 4]; 16],
    place: [[u8; 16]; 16],
}

fn tables() -> &'static Tables {
    static T: std::sync::OnceLock<Tables> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut t = Tables {
            shuf: [[0x80; 16]; 256],
            len: [0; 256],
            start: [[0; 4]; 16],
            keep1: [[0; 4]; 16],
            keep2: [[0; 4]; 16],
            carry: [[0; 4]; 16],
            place: [[0x80; 16]; 16],
        };
        for c in 0..256usize {
            let mut p = 0u8;
            for i in 0..4 {
                let l = ((c >> (2 * i)) & 3) as u8 + 1;
                for b in 0..l {
                    t.shuf[c][i * 4 + b as usize] = p + b;
                }
                p += l;
            }
            t.len[c] = p;
        }
        for m in 0..16usize {
            let s = |i: usize| (m >> i) & 1 == 1;
            let mut j = 0u8;
            for i in 0..4 {
                let on = |c: bool| if c { u32::MAX } else { 0 };
                t.start[m][i] = on(s(i));
                t.keep1[m][i] = on(i >= 1 && !s(i));
                t.keep2[m][i] = on(i >= 2 && !s(i) && !s(i - 1));
                t.carry[m][i] = on((0..=i).all(|q| !s(q)));
                if s(i) {
                    for b in 0..4 {
                        t.place[m][i * 4 + b] = j * 4 + b as u8;
                    }
                    j += 1;
                }
            }
        }
        t
    })
}

#[target_feature(enable = "ssse3")]
unsafe fn decode_ssse3(blk: &[u8], total: usize, bits: &[u64], nodes: &[u32], out: &mut [u32]) {
    unsafe {
        let t = tables();
        let nc = total.div_ceil(4);
        let d = blk.as_ptr();
        let mut p = nc;
        let mut k = 0usize; // starts consumed
        let mut prev = _mm_setzero_si128();
        let ones = _mm_set1_epi32(1);
        let ld = |a: &[u32; 4]| _mm_loadu_si128(a.as_ptr() as *const __m128i);
        for g in 0..nc {
            let c = *d.add(g) as usize;
            let raw = _mm_shuffle_epi8(_mm_loadu_si128(d.add(p) as *const __m128i), _mm_loadu_si128(t.shuf[c].as_ptr() as *const __m128i));
            p += t.len[c] as usize;
            let m = ((bits[g / 16] >> (4 * (g % 16))) & 15) as usize;
            let node = _mm_shuffle_epi8(
                _mm_loadu_si128(nodes.as_ptr().add(k) as *const __m128i),
                _mm_loadu_si128(t.place[m].as_ptr() as *const __m128i),
            );
            k += (m as u32).count_ones() as usize;
            // unzigzag: (w >> 1) ^ -(w & 1)
            let unzz = _mm_xor_si128(_mm_srli_epi32(raw, 1), _mm_sub_epi32(_mm_setzero_si128(), _mm_and_si128(raw, ones)));
            let first = _mm_add_epi32(node, unzz);
            let sm = ld(&t.start[m]);
            let x = _mm_or_si128(_mm_and_si128(sm, first), _mm_andnot_si128(sm, _mm_add_epi32(raw, ones)));
            let y = _mm_add_epi32(x, _mm_and_si128(_mm_slli_si128(x, 4), ld(&t.keep1[m])));
            let y = _mm_add_epi32(y, _mm_and_si128(_mm_slli_si128(y, 8), ld(&t.keep2[m])));
            let y = _mm_add_epi32(y, _mm_and_si128(prev, ld(&t.carry[m])));
            _mm_storeu_si128(out.as_mut_ptr().add(4 * g) as *mut __m128i, y);
            prev = _mm_shuffle_epi32(y, 0xff);
        }
    }
}
