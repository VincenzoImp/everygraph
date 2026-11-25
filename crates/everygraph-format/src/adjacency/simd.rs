// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The SIMD record decoders — SSSE3 on x86 and x86-64, NEON on AArch64, `simd128` on wasm32 —
//! and [`Decoder`], the only way to reach them.
//!
//! **Why safe code is not enough.** Stream VByte decodes four words with one byte
//! shuffle, which only SIMD instructions provide; the shuffle and the prefix sum fused
//! into the same loop are what made the format faster than raw arrays on scans in the
//! lab (FINDINGS § C and § M1-13; the lab's scalar prefix-sum pass was 1.17–1.25× slower
//! on single-thread `PageRank`). Loads and stores of SIMD registers, and calls of
//! functions compiled for a CPU feature, are `unsafe`.
//!
//! **What keeps it sound.**
//! - A SIMD [`Decoder`] can only be constructed after its CPU feature was detected at
//!   run time (x86-64, AArch64) or when the feature is enabled at compile time (wasm32);
//!   every `unsafe` call of a feature function goes through one.
//! - Every 16-byte load is preceded, in the loop condition, by the check that 16 bytes
//!   remain in the slice; every 16-byte store by the check that four words remain in the
//!   output (`simd_groups`). What is left over goes through the scalar [`svb::tail`].
//! - Each decoder returns exactly what the scalar decoder [`svb::decode`] returns on the
//!   same input, valid or not (`tests/adjacency.rs`, and the fuzz target).
//!
//! The decoders may write past `out[k - 1]`, up to the end of the last group of four
//! (never past `out`), when `out` is long enough; callers pass that slack when they have
//! it, so that the last group needs no scalar loop (the lab's "pad" decoder).
//!
//! One loop, [`decode_with`], serves every instruction set; each provides the six
//! primitives of [`Lanes`]. The loop is `#[inline(always)]` and the primitives are small,
//! so both are inlined into the feature-enabled batch loop (checked in the disassembly:
//! no call per list).
#![allow(unsafe_code)]

use super::DecodeError;
use super::decode::{self, Done, Ends, Job};
use super::svb::{self, Corrupt};

/// Which record decoder runs. All of them return the same bytes; the SIMD ones are
/// faster. A SIMD decoder can only be obtained where it can run: through
/// [`best`](Self::best) or [`available`](Self::available), which detect the CPU's
/// features at run time on x86 and x86-64 (SSSE3) and on AArch64 (NEON); on wasm32 the
/// `simd128` decoder exists when the crate is compiled with that target feature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decoder(Kind);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Scalar,
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    Ssse3,
    #[cfg(target_arch = "aarch64")]
    Neon,
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    Simd128,
}

impl Decoder {
    /// The scalar decoder, the reference for the others. Runs everywhere.
    pub const SCALAR: Self = Self(Kind::Scalar);

    /// The fastest decoder this machine can run.
    #[must_use]
    pub fn best() -> Self {
        Self::available().pop().unwrap_or(Self::SCALAR)
    }

    /// Every decoder this machine can run, the scalar one first.
    #[must_use]
    pub fn available() -> Vec<Self> {
        #[allow(unused_mut, reason = "only targets with a SIMD decoder push")]
        let mut all = vec![Self::SCALAR];
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        if std::arch::is_x86_feature_detected!("ssse3") {
            all.push(Self(Kind::Ssse3));
        }
        #[cfg(target_arch = "aarch64")]
        if std::arch::is_aarch64_feature_detected!("neon") {
            all.push(Self(Kind::Neon));
        }
        #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
        all.push(Self(Kind::Simd128));
        all
    }

    /// A short name: `scalar`, `ssse3`, `neon` or `simd128`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self.0 {
            Kind::Scalar => "scalar",
            #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
            Kind::Ssse3 => "ssse3",
            #[cfg(target_arch = "aarch64")]
            Kind::Neon => "neon",
            #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
            Kind::Simd128 => "simd128",
        }
    }

    /// One record; see [`svb::decode`].
    pub(super) fn list(
        self,
        bytes: &[u8],
        v: u32,
        total: usize,
        k: usize,
        out: &mut [u32],
    ) -> Result<usize, Corrupt> {
        match self.0 {
            Kind::Scalar => svb::decode(bytes, v, total, k, out),
            #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
            // SAFETY: a decoder of this kind is only made after SSSE3 was detected.
            Kind::Ssse3 => unsafe { x86::decode(bytes, v, total, k, out) },
            #[cfg(target_arch = "aarch64")]
            // SAFETY: a decoder of this kind is only made after NEON was detected.
            Kind::Neon => unsafe { neon::decode(bytes, v, total, k, out) },
            #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
            // SAFETY: `simd128` is enabled at compile time.
            Kind::Simd128 => unsafe { decode_with::<wasm::Simd128>(bytes, v, total, k, out) },
        }
    }

    /// Consecutive records; see [`decode::run`].
    pub(super) fn run(
        self,
        job: Job<'_>,
        out: &mut [u32],
        ends: &mut Ends,
    ) -> Result<Done, DecodeError> {
        match self.0 {
            Kind::Scalar => decode::run(svb::decode, job, out, ends),
            #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
            // SAFETY: a decoder of this kind is only made after SSSE3 was detected.
            Kind::Ssse3 => unsafe { x86::run(job, out, ends) },
            #[cfg(target_arch = "aarch64")]
            // SAFETY: a decoder of this kind is only made after NEON was detected.
            Kind::Neon => unsafe { neon::run(job, out, ends) },
            #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
            Kind::Simd128 => decode::run(
                // SAFETY: `simd128` is enabled at compile time.
                |b, v, t, k, o| unsafe { decode_with::<wasm::Simd128>(b, v, t, k, o) },
                job,
                out,
                ends,
            ),
        }
    }
}

/// The primitives of one instruction set. Every method may only be called where the
/// implementation's CPU feature is available; that is each method's safety condition,
/// besides the bounds stated on `load` and `store`.
#[cfg_attr(
    not(any(
        target_arch = "x86",
        target_arch = "x86_64",
        target_arch = "aarch64",
        all(target_arch = "wasm32", target_feature = "simd128")
    )),
    allow(dead_code, reason = "no SIMD decoder on this target")
)]
trait Lanes {
    /// Four 32-bit lanes.
    type V: Copy;
    /// `x` in every lane.
    unsafe fn splat(x: u32) -> Self::V;
    /// The 16 bytes at `bytes[at..at + 16]`, which must be inside `bytes`.
    unsafe fn load(bytes: &[u8], at: usize) -> Self::V;
    /// The shuffle of control byte `c`.
    unsafe fn mask(c: u8) -> Self::V;
    /// Shuffle, add one to each word, prefix sum, add the running value `prev`; returns
    /// the four values and the last one in every lane.
    unsafe fn group(data: Self::V, mask: Self::V, prev: Self::V) -> (Self::V, Self::V);
    /// Writes the four lanes to `out[at..at + 4]`, which must be inside `out`.
    unsafe fn store(out: &mut [u32], at: usize, x: Self::V);
    /// Lane 0.
    unsafe fn first(x: Self::V) -> u32;
}

/// As [`svb::decode`], with the groups of four through `L`.
///
/// # Safety
///
/// `L`'s CPU feature is available.
#[inline(always)]
#[cfg_attr(
    not(any(
        target_arch = "x86",
        target_arch = "x86_64",
        target_arch = "aarch64",
        all(target_arch = "wasm32", target_feature = "simd128")
    )),
    allow(dead_code, reason = "no SIMD decoder on this target")
)]
unsafe fn decode_with<L: Lanes>(
    bytes: &[u8],
    v: u32,
    total: usize,
    k: usize,
    out: &mut [u32],
) -> Result<usize, Corrupt> {
    // The bound every store below relies on, checked here rather than trusted from the
    // callers (review of #30, S1).
    assert!(k <= out.len(), "output of {} words for {k}", out.len());
    let (control, base) = svb::prologue(bytes, v, total, k)?;
    let groups = simd_groups(k, out.len());
    let mut at = control;
    let mut g = 0;
    // SAFETY: `L`'s feature is available (caller).
    let mut prev = unsafe { L::splat(base) };
    while g < groups && bytes.len() - at >= 16 {
        let c = bytes[g];
        // SAFETY: feature (caller); `at + 16 <= bytes.len()` (loop condition).
        let data = unsafe { L::load(bytes, at) };
        // SAFETY: feature (caller).
        let mask = unsafe { L::mask(c) };
        // SAFETY: feature (caller).
        let (x, last) = unsafe { L::group(data, mask, prev) };
        // SAFETY: feature (caller); `4 * g + 4 <= 4 * groups <= out.len()`: `simd_groups`
        // returns `⌈k/4⌉` only if `out` has that many groups, else `⌊k/4⌋`, and
        // `4 * ⌊k/4⌋ <= k <= out.len()` (asserted above).
        unsafe { L::store(out, 4 * g, x) };
        prev = last;
        at += usize::from(svb::GROUP_LEN[usize::from(c)]);
        g += 1;
    }
    // SAFETY: feature (caller).
    let acc = unsafe { L::first(prev) };
    if 4 * g >= k {
        // The loop wrote every word; the last group may have had unused fields, which
        // its length counted. Count the used ones only (meaningful when `k == total`).
        let c = bytes[g - 1];
        let used = k - 4 * (g - 1);
        let mask = u8::try_from((1u32 << (2 * used)) - 1).unwrap_or(u8::MAX);
        Ok(at - usize::from(svb::GROUP_LEN[usize::from(c)])
            + usize::from(svb::GROUP_LEN[usize::from(c & mask)])
            - (4 - used))
    } else {
        svb::tail(bytes, 4 * g, at, acc, k, out)
    }
}

/// The shuffle that spreads the data bytes of one control byte over four 32-bit lanes;
/// 0x80 marks a zero byte (`pshufb`, `tbl` and `swizzle` all give 0 for it).
#[cfg_attr(
    not(any(
        target_arch = "x86",
        target_arch = "x86_64",
        target_arch = "aarch64",
        all(target_arch = "wasm32", target_feature = "simd128")
    )),
    allow(dead_code, reason = "no SIMD decoder on this target")
)]
static SHUFFLE: [[u8; 16]; 256] = {
    let mut t = [[0x80u8; 16]; 256];
    let mut c = 0;
    while c < 256 {
        let mut src = 0u8;
        let mut lane = 0;
        while lane < 4 {
            let len = ((c >> (2 * lane)) & 3) + 1;
            let mut b = 0;
            while b < len {
                t[c][lane * 4 + b] = src;
                src += 1;
                b += 1;
            }
            lane += 1;
        }
        c += 1;
    }
    t
};

/// Groups of four the SIMD loop may write: all of the first `k` words, and the last
/// partial group too when `out` has room for it.
#[inline]
#[cfg_attr(
    not(any(
        target_arch = "x86",
        target_arch = "x86_64",
        target_arch = "aarch64",
        all(target_arch = "wasm32", target_feature = "simd128")
    )),
    allow(dead_code, reason = "no SIMD decoder on this target")
)]
fn simd_groups(k: usize, out_len: usize) -> usize {
    if out_len >= k.div_ceil(4) * 4 {
        k.div_ceil(4)
    } else {
        k / 4
    }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod x86 {
    #[cfg(target_arch = "x86")]
    use core::arch::x86::{
        __m128i, _mm_add_epi32, _mm_cvtsi128_si32, _mm_loadu_si128, _mm_set1_epi32,
        _mm_shuffle_epi8, _mm_shuffle_epi32, _mm_slli_si128, _mm_storeu_si128,
    };
    #[cfg(target_arch = "x86_64")]
    use core::arch::x86_64::{
        __m128i, _mm_add_epi32, _mm_cvtsi128_si32, _mm_loadu_si128, _mm_set1_epi32,
        _mm_shuffle_epi8, _mm_shuffle_epi32, _mm_slli_si128, _mm_storeu_si128,
    };

    use super::{Corrupt, DecodeError, Done, Ends, Job, Lanes, SHUFFLE, decode, decode_with};

    pub(super) struct Ssse3;

    impl Lanes for Ssse3 {
        type V = __m128i;

        #[target_feature(enable = "ssse3")]
        #[inline]
        #[expect(clippy::cast_possible_wrap, reason = "the bits are what matters")]
        unsafe fn splat(x: u32) -> __m128i {
            _mm_set1_epi32(x as i32)
        }

        #[target_feature(enable = "ssse3")]
        #[inline]
        unsafe fn load(bytes: &[u8], at: usize) -> __m128i {
            // SAFETY: `bytes[at..at + 16]` is inside `bytes` (the method's contract).
            unsafe { _mm_loadu_si128(bytes.as_ptr().wrapping_add(at).cast()) }
        }

        #[target_feature(enable = "ssse3")]
        #[inline]
        unsafe fn mask(c: u8) -> __m128i {
            // SAFETY: each row of the table is 16 bytes.
            unsafe { _mm_loadu_si128(SHUFFLE[usize::from(c)].as_ptr().cast()) }
        }

        #[target_feature(enable = "ssse3")]
        #[inline]
        unsafe fn group(data: __m128i, mask: __m128i, prev: __m128i) -> (__m128i, __m128i) {
            let x = _mm_add_epi32(_mm_shuffle_epi8(data, mask), _mm_set1_epi32(1));
            let x = _mm_add_epi32(x, _mm_slli_si128::<4>(x));
            let x = _mm_add_epi32(x, _mm_slli_si128::<8>(x));
            let x = _mm_add_epi32(x, prev);
            (x, _mm_shuffle_epi32::<0xff>(x))
        }

        #[target_feature(enable = "ssse3")]
        #[inline]
        unsafe fn store(out: &mut [u32], at: usize, x: __m128i) {
            // SAFETY: `out[at..at + 4]` is inside `out` (the method's contract).
            unsafe { _mm_storeu_si128(out.as_mut_ptr().wrapping_add(at).cast(), x) }
        }

        #[target_feature(enable = "ssse3")]
        #[inline]
        #[expect(clippy::cast_sign_loss, reason = "the bits are what matters")]
        unsafe fn first(x: __m128i) -> u32 {
            _mm_cvtsi128_si32(x) as u32
        }
    }

    /// One record, for random access.
    ///
    /// # Safety
    ///
    /// The CPU supports SSSE3.
    #[target_feature(enable = "ssse3")]
    pub(super) unsafe fn decode(
        bytes: &[u8],
        v: u32,
        total: usize,
        k: usize,
        out: &mut [u32],
    ) -> Result<usize, Corrupt> {
        // SAFETY: SSSE3 is available (this function's contract).
        unsafe { decode_with::<Ssse3>(bytes, v, total, k, out) }
    }

    /// The batch loop compiled for SSSE3, with the record decoder inlined into it.
    ///
    /// # Safety
    ///
    /// The CPU supports SSSE3.
    #[target_feature(enable = "ssse3")]
    pub(super) unsafe fn run(
        job: Job<'_>,
        out: &mut [u32],
        ends: &mut Ends,
    ) -> Result<Done, DecodeError> {
        decode::run(
            // SAFETY: SSSE3 is available (this function's contract).
            |b, v, t, k, o| unsafe { decode_with::<Ssse3>(b, v, t, k, o) },
            job,
            out,
            ends,
        )
    }
}

#[cfg(target_arch = "aarch64")]
mod neon {
    use core::arch::aarch64::{
        uint32x4_t, vaddq_u32, vdupq_laneq_u32, vdupq_n_u32, vextq_u32, vgetq_lane_u32, vld1q_u8,
        vqtbl1q_u8, vreinterpretq_u8_u32, vreinterpretq_u32_u8, vst1q_u32,
    };

    use super::{Corrupt, DecodeError, Done, Ends, Job, Lanes, SHUFFLE, decode, decode_with};

    pub(super) struct Neon;

    impl Lanes for Neon {
        type V = uint32x4_t;

        #[target_feature(enable = "neon")]
        #[inline]
        unsafe fn splat(x: u32) -> uint32x4_t {
            vdupq_n_u32(x)
        }

        #[target_feature(enable = "neon")]
        #[inline]
        unsafe fn load(bytes: &[u8], at: usize) -> uint32x4_t {
            // SAFETY: `bytes[at..at + 16]` is inside `bytes` (the method's contract).
            vreinterpretq_u32_u8(unsafe { vld1q_u8(bytes.as_ptr().wrapping_add(at)) })
        }

        #[target_feature(enable = "neon")]
        #[inline]
        unsafe fn mask(c: u8) -> uint32x4_t {
            // SAFETY: each row of the table is 16 bytes.
            vreinterpretq_u32_u8(unsafe { vld1q_u8(SHUFFLE[usize::from(c)].as_ptr()) })
        }

        #[target_feature(enable = "neon")]
        #[inline]
        unsafe fn group(
            data: uint32x4_t,
            mask: uint32x4_t,
            prev: uint32x4_t,
        ) -> (uint32x4_t, uint32x4_t) {
            let zero = vdupq_n_u32(0);
            let bytes = vqtbl1q_u8(vreinterpretq_u8_u32(data), vreinterpretq_u8_u32(mask));
            let x = vaddq_u32(vreinterpretq_u32_u8(bytes), vdupq_n_u32(1));
            let x = vaddq_u32(x, vextq_u32::<3>(zero, x));
            let x = vaddq_u32(x, vextq_u32::<2>(zero, x));
            let x = vaddq_u32(x, prev);
            (x, vdupq_laneq_u32::<3>(x))
        }

        #[target_feature(enable = "neon")]
        #[inline]
        unsafe fn store(out: &mut [u32], at: usize, x: uint32x4_t) {
            // SAFETY: `out[at..at + 4]` is inside `out` (the method's contract).
            unsafe { vst1q_u32(out.as_mut_ptr().wrapping_add(at), x) }
        }

        #[target_feature(enable = "neon")]
        #[inline]
        unsafe fn first(x: uint32x4_t) -> u32 {
            vgetq_lane_u32::<0>(x)
        }
    }

    /// One record, for random access.
    ///
    /// # Safety
    ///
    /// The CPU supports NEON.
    #[target_feature(enable = "neon")]
    pub(super) unsafe fn decode(
        bytes: &[u8],
        v: u32,
        total: usize,
        k: usize,
        out: &mut [u32],
    ) -> Result<usize, Corrupt> {
        // SAFETY: NEON is available (this function's contract).
        unsafe { decode_with::<Neon>(bytes, v, total, k, out) }
    }

    /// The batch loop compiled for NEON, with the record decoder inlined into it.
    ///
    /// # Safety
    ///
    /// The CPU supports NEON.
    #[target_feature(enable = "neon")]
    pub(super) unsafe fn run(
        job: Job<'_>,
        out: &mut [u32],
        ends: &mut Ends,
    ) -> Result<Done, DecodeError> {
        decode::run(
            // SAFETY: NEON is available (this function's contract).
            |b, v, t, k, o| unsafe { decode_with::<Neon>(b, v, t, k, o) },
            job,
            out,
            ends,
        )
    }
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
mod wasm {
    use core::arch::wasm32::{
        i8x16_swizzle, i32x4_add, i32x4_shuffle, u32x4_extract_lane, u32x4_splat, v128, v128_load,
        v128_store,
    };

    use super::{Lanes, SHUFFLE};

    /// `simd128`, enabled at compile time: its value intrinsics are safe functions.
    pub(super) struct Simd128;

    impl Lanes for Simd128 {
        type V = v128;

        #[inline]
        unsafe fn splat(x: u32) -> v128 {
            u32x4_splat(x)
        }

        #[inline]
        unsafe fn load(bytes: &[u8], at: usize) -> v128 {
            // SAFETY: `bytes[at..at + 16]` is inside `bytes` (the method's contract);
            // `v128_load` reads unaligned.
            unsafe { v128_load(bytes.as_ptr().wrapping_add(at).cast()) }
        }

        #[inline]
        unsafe fn mask(c: u8) -> v128 {
            // SAFETY: each row of the table is 16 bytes.
            unsafe { v128_load(SHUFFLE[usize::from(c)].as_ptr().cast()) }
        }

        #[inline]
        unsafe fn group(data: v128, mask: v128, prev: v128) -> (v128, v128) {
            let zero = u32x4_splat(0);
            let x = i32x4_add(i8x16_swizzle(data, mask), u32x4_splat(1));
            let x = i32x4_add(x, i32x4_shuffle::<0, 4, 5, 6>(zero, x));
            let x = i32x4_add(x, i32x4_shuffle::<0, 1, 4, 5>(zero, x));
            let x = i32x4_add(x, prev);
            (x, i32x4_shuffle::<3, 3, 3, 3>(x, x))
        }

        #[inline]
        unsafe fn store(out: &mut [u32], at: usize, x: v128) {
            // SAFETY: `out[at..at + 4]` is inside `out` (the method's contract).
            unsafe { v128_store(out.as_mut_ptr().wrapping_add(at).cast(), x) }
        }

        #[inline]
        unsafe fn first(x: v128) -> u32 {
            u32x4_extract_lane::<0>(x)
        }
    }
}
