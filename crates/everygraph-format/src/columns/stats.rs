// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Column statistics (`docs/spec/eg-columns.md` § 5, `graph-semantics.md` § 2.7), and
//! the exact sums they need: the sum of the values, rounded once to `f64`. Floats are
//! summed by `everygraph_core::ExactSum`; integers and decimals by [`WideSum`], an
//! exact integer sum whose one rounding applies the decimal scale.

use std::cmp::Ordering;

use everygraph_core::ExactSum;
use everygraph_core::le::{array, u32_at, u64_at};
use everygraph_core::math::CANONICAL_NAN;

use super::{ColumnError, ColumnLayout, ColumnType};

const MAGIC: &[u8; 8] = b"EGCOLSTA";
const VERSION: u32 = 1;
/// The length of a statistics section.
pub(super) const STATS_LEN: usize = 112;
/// Flag bit 0: `min` and `max` hold values.
const HAS_RANGE: u32 = 1;
/// 64-bit limbs of an integer sum: 2^64 values below 2^255 sum below 2^319, and the
/// rounding multiplies by at most 10^76 < 2^253 (or shifts to 370 bits): below 2^640.
const LIMBS: usize = 10;

/// An exact sum of integers, each added as a magnitude at a bit position, rounded once
/// when read.
#[derive(Clone, Debug)]
struct WideSum {
    positive: [u64; LIMBS],
    negative: [u64; LIMBS],
}

impl WideSum {
    fn new() -> Self {
        Self {
            positive: [0; LIMBS],
            negative: [0; LIMBS],
        }
    }

    /// Adds ±`v` × 2^`bit`.
    fn add(&mut self, negative: bool, v: u128, bit: u32) {
        let acc = if negative {
            &mut self.negative
        } else {
            &mut self.positive
        };
        let (limb, off) = ((bit / 64) as usize, bit % 64);
        #[allow(clippy::cast_possible_truncation)]
        let parts = if off == 0 {
            [v as u64, (v >> 64) as u64, 0]
        } else {
            [
                (v as u64) << off,
                (v >> (64 - off)) as u64,
                (v >> (128 - off)) as u64,
            ]
        };
        let mut carry = 0;
        for (i, slot) in acc[limb..].iter_mut().enumerate() {
            if i >= parts.len() && carry == 0 {
                break;
            }
            let (s, c1) = slot.overflowing_add(parts.get(i).copied().unwrap_or(0));
            let (s, c2) = s.overflowing_add(carry);
            *slot = s;
            carry = u64::from(c1 || c2);
        }
    }

    /// The sum times 10^−`scale`, rounded once to nearest, ties to even; +0.0 for a sum
    /// that rounds to zero.
    fn to_f64(&self, scale: i8) -> f64 {
        let (negative, mut m) = match cmp(&self.positive, &self.negative) {
            Ordering::Equal => return 0.0,
            Ordering::Greater => (false, sub(&self.positive, &self.negative)),
            Ordering::Less => (true, sub(&self.negative, &self.positive)),
        };
        // Below, the value is (m + f) × 2^e with f in [0, 1), non-zero exactly when
        // `sticky`.
        let mut e = 0i64;
        let mut sticky = false;
        if scale < 0 {
            for _ in 0..scale.unsigned_abs() {
                mul_small(&mut m, 10);
            }
        } else if scale > 0 {
            // Enough bits that 66 remain after dividing by 10^scale < 2^(4 scale).
            let k = (66 + 4 * u32::from(scale.unsigned_abs())).saturating_sub(bit_len(&m));
            shl(&mut m, k);
            e -= i64::from(k);
            for _ in 0..scale {
                sticky |= div_small(&mut m, 10) != 0;
            }
        }
        let len = bit_len(&m);
        let top = if len > 64 {
            let shift = len - 64;
            sticky |= low_bits_set(&m, shift);
            e += i64::from(shift);
            bits_at(&m, shift)
        } else {
            e -= i64::from(64 - len);
            m[0] << (64 - len)
        };
        compose(negative, top, sticky, e)
    }
}

/// The value `(top + f) × 2^e`, `top` in [2^63, 2^64), `f` in [0, 1) and non-zero
/// exactly when `sticky`, rounded to the nearest `f64`, ties to even.
fn compose(negative: bool, top: u64, sticky: bool, e: i64) -> f64 {
    let sign = if negative { 1u64 << 63 } else { 0 };
    let infinity = f64::from_bits(sign | 0x7FF0_0000_0000_0000);
    let lead = e + 63;
    if lead > 1023 {
        return infinity;
    }
    // Bits of `top` below the last one kept: 11 for a normal result, more below 2^−1022.
    let drop = 11 + (-1022 - lead).max(0);
    if drop >= 128 {
        return 0.0;
    }
    let drop = u32::try_from(drop).unwrap_or(127);
    let x = u128::from(top);
    let mut kept = x >> drop;
    let rest = x & ((1u128 << drop) - 1);
    let half = 1u128 << (drop - 1);
    if rest > half || (rest == half && (sticky || kept & 1 == 1)) {
        kept += 1;
    }
    #[allow(clippy::cast_possible_truncation)]
    let mut kept = kept as u64;
    if kept == 0 {
        return 0.0;
    }
    if lead < -1022 {
        // Subnormal, or the smallest normal when the rounding carried into bit 52.
        return f64::from_bits(sign | kept);
    }
    let mut lead = lead;
    if kept == 1 << 53 {
        kept >>= 1;
        lead += 1;
        if lead > 1023 {
            return infinity;
        }
    }
    let exponent = u64::try_from(lead + 1023).unwrap_or(0);
    f64::from_bits(sign | exponent << 52 | (kept & ((1 << 52) - 1)))
}

/// A 256-bit two's complement integer, little-endian, as its sign and its magnitude
/// (high 128 bits, low 128 bits).
fn signed_magnitude(le: [u8; 32]) -> (bool, (u128, u128)) {
    let (hi, lo) = halves(le);
    if hi >> 127 == 0 {
        return (false, (hi, lo));
    }
    let (lo, carry) = (!lo).overflowing_add(1);
    (true, (!hi + u128::from(carry), lo))
}

/// The magnitude of a 256-bit two's complement integer, little-endian.
pub(super) fn magnitude(le: [u8; 32]) -> (u128, u128) {
    signed_magnitude(le).1
}

/// 10^p as a 256-bit integer (high 128 bits, low 128 bits); for p ≤ 76, below 2^253.
pub(super) fn pow10(p: u8) -> (u128, u128) {
    let mut m = [1u64, 0, 0, 0];
    for _ in 0..p {
        mul_small(&mut m, 10);
    }
    let join = |lo: u64, hi: u64| u128::from(hi) << 64 | u128::from(lo);
    (join(m[2], m[3]), join(m[0], m[1]))
}

fn halves(le: [u8; 32]) -> (u128, u128) {
    let (lo, hi) = le.split_at(16);
    let half = |b: &[u8]| u128::from_le_bytes(b.try_into().unwrap_or_default());
    (half(hi), half(lo))
}

/// Orders two 256-bit two's complement little-endian integers.
fn cmp_i256(a: [u8; 32], b: [u8; 32]) -> Ordering {
    let key = |x: [u8; 32]| {
        let (hi, lo) = halves(x);
        (hi.cast_signed(), lo)
    };
    key(a).cmp(&key(b))
}

fn cmp(a: &[u64], b: &[u64]) -> Ordering {
    a.iter().rev().cmp(b.iter().rev())
}

/// `a − b` for `a ≥ b`.
fn sub(a: &[u64; LIMBS], b: &[u64; LIMBS]) -> [u64; LIMBS] {
    let mut out = [0; LIMBS];
    let mut borrow = false;
    for ((o, &x), &y) in out.iter_mut().zip(a).zip(b) {
        let (d, b1) = x.overflowing_sub(y);
        let (d, b2) = d.overflowing_sub(u64::from(borrow));
        *o = d;
        borrow = b1 || b2;
    }
    out
}

fn bit_len(m: &[u64]) -> u32 {
    m.iter().rposition(|&l| l != 0).map_or(0, |i| {
        64 * u32::try_from(i).unwrap_or(0) + 64 - m[i].leading_zeros()
    })
}

fn shl(m: &mut [u64], k: u32) {
    let (limbs, bits) = ((k / 64) as usize, k % 64);
    for i in (0..m.len()).rev() {
        let hi = i.checked_sub(limbs).map_or(0, |j| m[j]);
        let lo = i.checked_sub(limbs + 1).map_or(0, |j| m[j]);
        m[i] = if bits == 0 {
            hi
        } else {
            hi << bits | lo >> (64 - bits)
        };
    }
}

#[allow(clippy::cast_possible_truncation)]
fn mul_small(m: &mut [u64], k: u64) {
    let mut carry = 0u128;
    for limb in m.iter_mut() {
        let x = u128::from(*limb) * u128::from(k) + carry;
        *limb = x as u64;
        carry = x >> 64;
    }
}

/// Divides `m` by `k` in place; returns the remainder.
#[allow(clippy::cast_possible_truncation)]
fn div_small(m: &mut [u64], k: u64) -> u64 {
    let mut rem = 0u128;
    for limb in m.iter_mut().rev() {
        let x = rem << 64 | u128::from(*limb);
        *limb = (x / u128::from(k)) as u64;
        rem = x % u128::from(k);
    }
    rem as u64
}

/// Whether any of the lowest `n` bits of `m` is set.
fn low_bits_set(m: &[u64], n: u32) -> bool {
    let (limbs, bits) = ((n / 64) as usize, n % 64);
    m[..limbs].iter().any(|&l| l != 0) || (bits > 0 && m[limbs] & ((1 << bits) - 1) != 0)
}

/// The 64 bits of `m` from bit `at` up.
fn bits_at(m: &[u64], at: u32) -> u64 {
    let (limb, bits) = ((at / 64) as usize, at % 64);
    let hi = if bits == 0 {
        0
    } else {
        m.get(limb + 1).map_or(0, |&h| h << (64 - bits))
    };
    m[limb] >> bits | hi
}

/// The statistics of a numeric column (`graph-semantics.md` § 2.7), over its stored
/// values.
#[derive(Clone, Debug, PartialEq)]
pub struct ColumnStats {
    /// Values that are not null (NaN included).
    pub count: u64,
    /// Nulls.
    pub null_count: u64,
    /// NaN values (0 for every type but floats).
    pub nan_count: u64,
    /// The exact sum of the values that are neither null nor NaN, rounded once.
    pub sum: f64,
    /// The smallest and largest of those values, as the column stores them
    /// (little-endian, its width, zero bytes to 32); `None` when there is none.
    pub range: Option<([u8; 32], [u8; 32])>,
}

impl ColumnStats {
    /// Whether every operation may use the column as a weight or a cost: no NaN, no
    /// negative value, a finite sum (`graph-semantics.md` § 2.7). `ty` is its type.
    #[must_use]
    pub fn is_clean(&self, ty: ColumnType) -> bool {
        let non_negative = self.range.is_none_or(|(min, _)| match ty {
            ColumnType::Float32 | ColumnType::Float64 => as_f64(ty, min) >= 0.0,
            _ => !is_signed(ty) || min[top_byte(ty)] & 0x80 == 0,
        });
        self.nan_count == 0 && non_negative && self.sum.is_finite()
    }

    pub(super) fn encode(&self) -> [u8; STATS_LEN] {
        let mut out = [0; STATS_LEN];
        out[..8].copy_from_slice(MAGIC);
        out[8..12].copy_from_slice(&VERSION.to_le_bytes());
        let flags = if self.range.is_some() { HAS_RANGE } else { 0 };
        out[12..16].copy_from_slice(&flags.to_le_bytes());
        out[16..24].copy_from_slice(&self.count.to_le_bytes());
        out[24..32].copy_from_slice(&self.null_count.to_le_bytes());
        out[32..40].copy_from_slice(&self.nan_count.to_le_bytes());
        let sum = if self.sum.is_nan() {
            CANONICAL_NAN
        } else {
            self.sum
        };
        out[40..48].copy_from_slice(&sum.to_bits().to_le_bytes());
        if let Some((min, max)) = &self.range {
            out[48..80].copy_from_slice(min);
            out[80..112].copy_from_slice(max);
        }
        out
    }

    /// Parses the statistics section of the column whose layout is `layout`.
    ///
    /// # Errors
    ///
    /// [`ColumnError::Unsupported`] for a version or flags this reader does not know;
    /// [`ColumnError::Invalid`] when the record breaks the specification or does not
    /// fit the column (counts that do not add up to its rows, a range out of order, a
    /// sum of −0.0, or a non-zero sum of no value).
    pub fn parse(bytes: &[u8], layout: &ColumnLayout) -> Result<Self, ColumnError> {
        let invalid = ColumnError::Invalid;
        let ty = layout.ty();
        if !ty.is_numeric() {
            return Err(invalid("statistics of a column that is not numeric"));
        }
        if bytes.len() != STATS_LEN || &bytes[..8] != MAGIC {
            return Err(invalid("not a column statistics record"));
        }
        if u32_at(bytes, 8) != VERSION {
            return Err(ColumnError::Unsupported(
                "a statistics version other than 1",
            ));
        }
        if u32_at(bytes, 12) & !HAS_RANGE != 0 {
            return Err(ColumnError::Unsupported("statistics flags"));
        }
        let (min, max) = (array(bytes, 48), array(bytes, 80));
        let stats = Self {
            count: u64_at(bytes, 16),
            null_count: u64_at(bytes, 24),
            nan_count: u64_at(bytes, 32),
            sum: f64::from_bits(u64_at(bytes, 40)),
            range: (u32_at(bytes, 12) & HAS_RANGE != 0).then_some((min, max)),
        };
        let has_values = stats.nan_count < stats.count;
        if stats.count.checked_add(stats.null_count) != Some(layout.rows())
            || stats.nan_count > stats.count
            || (stats.nan_count > 0 && !matches!(ty, ColumnType::Float32 | ColumnType::Float64))
            || stats.range.is_some() != has_values
            || (stats.sum == 0.0 && stats.sum.is_sign_negative())
            || (!has_values && stats.sum.to_bits() != 0)
        {
            return Err(invalid("the statistics do not fit the column"));
        }
        if let Some((min, max)) = stats.range {
            if !in_order(ty, min, max) {
                return Err(invalid(
                    "the statistics' minimum is above their maximum, or not a value",
                ));
            }
        }
        if stats.encode()[..] != *bytes {
            return Err(invalid("the statistics are not stored canonically"));
        }
        Ok(stats)
    }
}

fn as_f64(ty: ColumnType, b: [u8; 32]) -> f64 {
    match ty {
        ColumnType::Float32 => f64::from(f32::from_le_bytes(array(&b, 0))),
        _ => f64::from_le_bytes(array(&b, 0)),
    }
}

fn is_signed(ty: ColumnType) -> bool {
    !matches!(
        ty,
        ColumnType::UInt8 | ColumnType::UInt16 | ColumnType::UInt32 | ColumnType::UInt64
    )
}

fn top_byte(ty: ColumnType) -> usize {
    usize::try_from(ty.width()).unwrap_or(32).clamp(1, 32) - 1
}

/// A stored integer of `ty` (an integer, a timestamp or a decimal) as a 256-bit two's
/// complement integer: sign-extended for signed types.
fn widen(b: [u8; 32], ty: ColumnType) -> [u8; 32] {
    let mut x = b;
    if is_signed(ty) && b[top_byte(ty)] & 0x80 != 0 {
        x[top_byte(ty) + 1..].fill(0xFF);
    }
    x
}

/// Whether `min ≤ max` as values of `ty`, each of them canonical (§ 5): no NaN, no
/// −0.0, zero bytes past the type's width.
fn in_order(ty: ColumnType, min: [u8; 32], max: [u8; 32]) -> bool {
    let width = top_byte(ty) + 1;
    if min[width..].iter().chain(&max[width..]).any(|&b| b != 0) {
        return false;
    }
    if matches!(ty, ColumnType::Float32 | ColumnType::Float64) {
        let (lo, hi) = (as_f64(ty, min), as_f64(ty, max));
        let canonical = |x: f64| !x.is_nan() && !(x == 0.0 && x.is_sign_negative());
        return canonical(lo) && canonical(hi) && lo <= hi;
    }
    cmp_i256(widen(min, ty), widen(max, ty)) != Ordering::Greater
}

/// Statistics being gathered by the writer, value by value.
#[derive(Debug)]
pub(super) struct Accumulator {
    ty: ColumnType,
    count: u64,
    null_count: u64,
    nan_count: u64,
    floats: ExactSum,
    ints: WideSum,
    range: Option<([u8; 32], [u8; 32])>,
}

impl Accumulator {
    pub(super) fn new(ty: ColumnType) -> Self {
        Self {
            ty,
            count: 0,
            null_count: 0,
            nan_count: 0,
            floats: ExactSum::new(),
            ints: WideSum::new(),
            range: None,
        }
    }

    pub(super) fn null(&mut self, n: u64) {
        self.null_count += n;
    }

    /// One float value, already canonical.
    pub(super) fn float(&mut self, x: f64, stored: [u8; 32]) {
        self.count += 1;
        if x.is_nan() {
            self.nan_count += 1;
            return;
        }
        self.floats.add(x);
        let ty = self.ty;
        extend(&mut self.range, stored, |a, b| {
            as_f64(ty, a) < as_f64(ty, b)
        });
    }

    /// One integer value (an integer, a timestamp or a decimal128).
    pub(super) fn int(&mut self, x: i128, stored: [u8; 32]) {
        self.count += 1;
        self.ints.add(x < 0, x.unsigned_abs(), 0);
        let ty = self.ty;
        extend(&mut self.range, stored, |a, b| {
            cmp_i256(widen(a, ty), widen(b, ty)) == Ordering::Less
        });
    }

    /// One decimal256 value.
    pub(super) fn wide(&mut self, x: [u8; 32]) {
        self.count += 1;
        let (negative, (hi, lo)) = signed_magnitude(x);
        self.ints.add(negative, lo, 0);
        self.ints.add(negative, hi, 128);
        extend(&mut self.range, x, |a, b| cmp_i256(a, b) == Ordering::Less);
    }

    pub(super) fn finish(self) -> ColumnStats {
        let sum = match self.ty {
            ColumnType::Float32 | ColumnType::Float64 => self.floats.value(),
            ty => self.ints.to_f64(ty.scale()),
        };
        ColumnStats {
            count: self.count,
            null_count: self.null_count,
            nan_count: self.nan_count,
            sum,
            range: self.range,
        }
    }
}

fn extend(
    range: &mut Option<([u8; 32], [u8; 32])>,
    x: [u8; 32],
    less: impl Fn([u8; 32], [u8; 32]) -> bool,
) {
    *range = Some(match *range {
        None => (x, x),
        Some((lo, hi)) => (
            if less(x, lo) { x } else { lo },
            if less(hi, x) { x } else { hi },
        ),
    });
}
