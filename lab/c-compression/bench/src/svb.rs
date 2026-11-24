//! Minimal Stream VByte (Lemire et al.) with SSSE3 pshufb decoder (stable Rust, std::arch).
//! Layout: ceil(k/4) control bytes (2 bits per value: byte length-1), then data bytes.
//! Decoder may over-read up to 16 bytes past the end of the data: blob must be padded.
use std::sync::OnceLock;

struct Tables {
    shuf: [[u8; 16]; 256],
    len: [u8; 256],
}
fn tables() -> &'static Tables {
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
    if x < 1 << 8 { 1 } else if x < 1 << 16 { 2 } else if x < 1 << 24 { 3 } else { 4 }
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

/// Decodes k values from d into out[..k] (out.len() >= k rounded up to 4). Returns bytes consumed.
#[inline(always)]
pub fn decode(d: &[u8], k: usize, out: &mut [u32]) -> usize {
    use std::arch::x86_64::*;
    let t = tables();
    let nc = k.div_ceil(4);
    let full = k / 4;
    let ctrl = d.as_ptr();
    let mut p = nc;
    unsafe {
        for g in 0..full {
            let c = *ctrl.add(g) as usize;
            let v = _mm_loadu_si128(d.as_ptr().add(p) as *const __m128i);
            let s = _mm_loadu_si128(t.shuf[c].as_ptr() as *const __m128i);
            _mm_storeu_si128(out.as_mut_ptr().add(g * 4) as *mut __m128i, _mm_shuffle_epi8(v, s));
            p += t.len[c] as usize;
        }
        let rem = k - full * 4;
        if rem > 0 {
            let c = *ctrl.add(full) as usize;
            for i in 0..rem {
                let l = ((c >> (2 * i)) & 3) + 1;
                let mut x = 0u32;
                for b in 0..l {
                    x |= (*d.get_unchecked(p + b) as u32) << (8 * b);
                }
                out[full * 4 + i] = x;
                p += l;
            }
        }
    }
    p
}
