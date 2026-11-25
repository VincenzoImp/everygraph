//! Stream VByte (Lemire et al.) with the SSSE3 pshufb decoder from lab/c-compression/bench/src/svb.rs.
//! Layout: ceil(k/4) control bytes (2 bits per value: byte length - 1), then the data bytes.
//! The decoders may over-read up to 16 bytes past the end of the data: the blob must be padded.
//!
//! Two decoders over the same bytes:
//!  * `decode`        — the lab decoder, unchanged: raw values out, the caller does the prefix sum
//!                      in a second scalar pass (this is what lab C measured);
//!  * `decode_delta`  — same shuffle, prefix sum fused into the loop with SSE2 adds (one pass).
use std::sync::OnceLock;

pub struct Tables {
    pub shuf: [[u8; 16]; 256],
    pub len: [u8; 256],
}
#[inline(always)]
pub fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(|| {
        let mut shuf = [[0x80u8; 16]; 256];
        let mut len = [0u8; 256];
        for c in 0..256usize {
            let mut p = 0u8;
            for i in 0..4 {
                let l = ((c >> (2 * i)) & 3) as u8 + 1;
                for b in 0..l {
                    shuf[c][i * 4 + b as usize] = p + b;
                }
                p += l;
            }
            len[c] = p;
        }
        Tables { shuf, len }
    })
}

#[inline(always)]
fn blen(x: u32) -> usize {
    if x < 1 << 8 {
        1
    } else if x < 1 << 16 {
        2
    } else if x < 1 << 24 {
        3
    } else {
        4
    }
}

pub fn encode(vals: &[u32], out: &mut Vec<u8>) {
    let k = vals.len();
    let cstart = out.len();
    let nc = k.div_ceil(4);
    out.resize(cstart + nc, 0);
    for (i, &x) in vals.iter().enumerate() {
        let l = blen(x);
        out[cstart + i / 4] |= ((l - 1) as u8) << (2 * (i % 4));
        out.extend_from_slice(&x.to_le_bytes()[..l]);
    }
}

/// Lab decoder. Decodes the first `k` of `total` values from `d` into out[..k]. Returns the number of
/// bytes consumed (control + data) — meaningful as "record length" only when k == total.
///
/// # Safety
/// `d` must hold a valid stream of `total` values followed by >= 16 readable bytes; out.len() >= k.
#[inline(always)]
pub unsafe fn decode(d: *const u8, total: usize, k: usize, out: *mut u32) -> usize {
    use std::arch::x86_64::*;
    let t = tables();
    let nc = total.div_ceil(4);
    let full = k / 4;
    let mut p = nc;
    for g in 0..full {
        let c = *d.add(g) as usize;
        let v = _mm_loadu_si128(d.add(p) as *const __m128i);
        let s = _mm_loadu_si128(t.shuf.get_unchecked(c).as_ptr() as *const __m128i);
        _mm_storeu_si128(out.add(g * 4) as *mut __m128i, _mm_shuffle_epi8(v, s));
        p += *t.len.get_unchecked(c) as usize;
    }
    let rem = k - full * 4;
    if rem > 0 {
        let c = *d.add(full) as usize;
        for i in 0..rem {
            let l = ((c >> (2 * i)) & 3) + 1;
            let mut x = 0u32;
            for b in 0..l {
                x |= (*d.add(p + b) as u32) << (8 * b);
            }
            *out.add(full * 4 + i) = x;
            p += l;
        }
    }
    p
}

/// Fused decoder for gap lists: out[i] = base + sum_{j<=i} (val[j] + 1), all arithmetic wrapping.
/// The caller picks `base` so that the first value comes out right (see store.rs).
///
/// # Safety
/// Same as `decode`.
#[inline(always)]
pub unsafe fn decode_delta(d: *const u8, total: usize, k: usize, base: u32, out: *mut u32) -> usize {
    use std::arch::x86_64::*;
    let t = tables();
    let nc = total.div_ceil(4);
    let full = k / 4;
    let mut p = nc;
    let ones = _mm_set1_epi32(1);
    let mut prev = _mm_set1_epi32(base as i32);
    for g in 0..full {
        let c = *d.add(g) as usize;
        let v = _mm_loadu_si128(d.add(p) as *const __m128i);
        let s = _mm_loadu_si128(t.shuf.get_unchecked(c).as_ptr() as *const __m128i);
        let mut x = _mm_add_epi32(_mm_shuffle_epi8(v, s), ones);
        x = _mm_add_epi32(x, _mm_slli_si128(x, 4));
        x = _mm_add_epi32(x, _mm_slli_si128(x, 8));
        x = _mm_add_epi32(x, prev);
        _mm_storeu_si128(out.add(g * 4) as *mut __m128i, x);
        prev = _mm_shuffle_epi32(x, 0xFF);
        p += *t.len.get_unchecked(c) as usize;
    }
    let rem = k - full * 4;
    if rem > 0 {
        let mut acc = _mm_cvtsi128_si32(prev) as u32;
        let c = *d.add(full) as usize;
        for i in 0..rem {
            let l = ((c >> (2 * i)) & 3) + 1;
            let mut x = 0u32;
            for b in 0..l {
                x |= (*d.add(p + b) as u32) << (8 * b);
            }
            acc = acc.wrapping_add(x).wrapping_add(1);
            *out.add(full * 4 + i) = acc;
            p += l;
        }
    }
    p
}

/// As `decode_delta`, but without the scalar loop for the last 1-3 values: the last group is
/// decoded with the same shuffle as the others, so the loop has no data-dependent branch.
/// May write up to 3 values past out[k-1] (garbage): the caller's buffer needs that slack.
/// The return value is the stream length only if k == total.
///
/// # Safety
/// Same as `decode`, and out must have room for k rounded up to a multiple of 4.
#[inline(always)]
pub unsafe fn decode_delta_pad(d: *const u8, total: usize, k: usize, base: u32, out: *mut u32) -> usize {
    use std::arch::x86_64::*;
    let t = tables();
    let nc = total.div_ceil(4);
    let groups = k.div_ceil(4);
    let mut p = nc;
    let ones = _mm_set1_epi32(1);
    let mut prev = _mm_set1_epi32(base as i32);
    for g in 0..groups {
        let c = *d.add(g) as usize;
        let v = _mm_loadu_si128(d.add(p) as *const __m128i);
        let s = _mm_loadu_si128(t.shuf.get_unchecked(c).as_ptr() as *const __m128i);
        let mut x = _mm_add_epi32(_mm_shuffle_epi8(v, s), ones);
        x = _mm_add_epi32(x, _mm_slli_si128(x, 4));
        x = _mm_add_epi32(x, _mm_slli_si128(x, 8));
        x = _mm_add_epi32(x, prev);
        _mm_storeu_si128(out.add(g * 4) as *mut __m128i, x);
        prev = _mm_shuffle_epi32(x, 0xFF);
        p += *t.len.get_unchecked(c) as usize;
    }
    // the unused 2-bit fields of the last control byte are zero, i.e. counted as one byte each
    p - (groups * 4 - k)
}

/// First value of a stream of `total` values (scalar peek; does not advance anything).
///
/// # Safety
/// Same as `decode`; total >= 1.
#[inline(always)]
pub unsafe fn peek_first(d: *const u8, total: usize) -> u32 {
    let nc = total.div_ceil(4);
    let l = (*d & 3) as u32 + 1;
    let w = (d.add(nc) as *const u32).read_unaligned();
    if l == 4 {
        w
    } else {
        w & ((1u32 << (8 * l)) - 1)
    }
}

/// Byte length of a stream of `total` values without decoding it (sums the control-byte lengths).
///
/// # Safety
/// Same as `decode`.
#[inline(always)]
pub unsafe fn stream_len(d: *const u8, total: usize) -> usize {
    let t = tables();
    let nc = total.div_ceil(4);
    let full = total / 4;
    let mut p = nc;
    for g in 0..full {
        p += *t.len.get_unchecked(*d.add(g) as usize) as usize;
    }
    let rem = total - full * 4;
    if rem > 0 {
        let c = *d.add(full) as usize;
        for i in 0..rem {
            p += ((c >> (2 * i)) & 3) + 1;
        }
    }
    p
}
