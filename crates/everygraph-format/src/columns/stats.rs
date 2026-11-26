// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Column statistics (`docs/spec/eg-columns.md` § 5, `graph-semantics.md` § 2.7) and the
//! exact sum they need (§ 4.3: the exact sum of the values, rounded once to `f64`).

use std::cmp::Ordering;

use super::{ColumnError, ColumnLayout, ColumnType};

const MAGIC: &[u8; 8] = b"EGCOLSTA";
const VERSION: u32 = 1;
/// The length of a statistics section.
pub(super) const STATS_LEN: usize = 112;
/// Flag bit 0: `min` and `max` hold values.
const HAS_RANGE: u32 = 1;

/// 64-bit limbs of the accumulator: 2,304 bits. A finite `f64` is an integer multiple of
/// 2^−1074 below 2^1024, so 2^64 of them sum, as such an integer, to less than 2^2162;
/// a decimal256 value is below 2^255, so after the 2^1074 scaling (§ 5) 2^64 of them
/// stay below 2^1393.
const LIMBS: usize = 36;
/// The binary point of the accumulator: every value is added as a multiple of 2^−1074.
const POINT: u32 = 1074;

/// The exact sum of `f64`, integer and decimal values, rounded once to `f64` (round to
/// nearest, ties to even) when asked: the result does not depend on the order of the
/// additions. Up to 2^64 values.
#[derive(Clone, Debug)]
pub struct ExactSum {
    positive: [u64; LIMBS],
    negative: [u64; LIMBS],
    pos_inf: bool,
    neg_inf: bool,
    nan: bool,
}

impl Default for ExactSum {
    fn default() -> Self {
        Self {
            positive: [0; LIMBS],
            negative: [0; LIMBS],
            pos_inf: false,
            neg_inf: false,
            nan: false,
        }
    }
}

impl ExactSum {
    /// An empty sum (0).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `x`. An infinity makes the sum infinite; NaN, or infinities of both signs,
    /// make it NaN.
    pub fn add_f64(&mut self, x: f64) {
        let bits = x.to_bits();
        let exponent = (bits >> 52) & 0x7FF;
        let fraction = bits & ((1 << 52) - 1);
        let negative = x.is_sign_negative();
        if exponent == 0x7FF {
            if fraction != 0 {
                self.nan = true;
            } else if negative {
                self.neg_inf = true;
            } else {
                self.pos_inf = true;
            }
            return;
        }
        // x = m × 2^(shift − 1074).
        let (m, shift) = if exponent == 0 {
            (fraction, 0)
        } else {
            (fraction | 1 << 52, u32::try_from(exponent - 1).unwrap_or(0))
        };
        self.add_at(negative, u128::from(m), shift);
    }

    /// Adds the integer `x` (for a decimal column, the stored integer: see
    /// [`to_f64_scaled`](Self::to_f64_scaled)).
    pub fn add_i128(&mut self, x: i128) {
        self.add_at(x < 0, x.unsigned_abs(), POINT);
    }

    /// Adds a 256-bit two's complement integer, little-endian: a decimal256 value.
    pub fn add_i256(&mut self, le: [u8; 32]) {
        let (mut lo, hi) = halves(le);
        let negative = hi < 0;
        let mut hi = hi.cast_unsigned();
        if negative {
            // Negate the 256-bit integer: invert and add one.
            lo = !lo;
            hi = !hi;
            let (l, carry) = lo.overflowing_add(1);
            lo = l;
            hi = hi.wrapping_add(u128::from(carry));
        }
        self.add_at(negative, lo, POINT);
        self.add_at(negative, hi, POINT + 128);
    }

    fn add_at(&mut self, negative: bool, v: u128, bit: u32) {
        let acc = if negative {
            &mut self.negative
        } else {
            &mut self.positive
        };
        let limb = (bit / 64) as usize;
        let off = bit % 64;
        // v << off, as three limbs.
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
            let add = parts.get(i).copied().unwrap_or(0);
            if i >= parts.len() && carry == 0 {
                break;
            }
            let (s, c1) = slot.overflowing_add(add);
            let (s, c2) = s.overflowing_add(carry);
            *slot = s;
            carry = u64::from(c1 || c2);
        }
    }

    /// The sum, rounded once to `f64`. Never −0.0: a sum that rounds to zero is +0.0.
    #[must_use]
    pub fn to_f64(&self) -> f64 {
        self.to_f64_scaled(0)
    }

    /// The sum times 10^−`scale`, rounded once to `f64`: the sum of a decimal column of
    /// that scale, from its stored integers.
    #[must_use]
    pub fn to_f64_scaled(&self, scale: i8) -> f64 {
        if self.nan || (self.pos_inf && self.neg_inf) {
            return f64::NAN;
        }
        if self.pos_inf {
            return f64::INFINITY;
        }
        if self.neg_inf {
            return f64::NEG_INFINITY;
        }
        let (negative, mut m) = match cmp(&self.positive, &self.negative) {
            Ordering::Equal => return 0.0,
            Ordering::Greater => (false, sub(&self.positive, &self.negative)),
            Ordering::Less => (true, sub(&self.negative, &self.positive)),
        };
        // The value is m × 2^e / 10^scale, with any discarded part recorded in `sticky`.
        let mut e = -i64::from(POINT);
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
    let lead = e + 63;
    if lead > 1023 {
        return f64::from_bits(sign | 0x7FF0_0000_0000_0000);
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
            return f64::from_bits(sign | 0x7FF0_0000_0000_0000);
        }
    }
    let exponent = u64::try_from(lead + 1023).unwrap_or(0);
    f64::from_bits(sign | exponent << 52 | (kept & ((1 << 52) - 1)))
}

fn halves(le: [u8; 32]) -> (u128, i128) {
    let mut lo = [0; 16];
    let mut hi = [0; 16];
    lo.copy_from_slice(&le[..16]);
    hi.copy_from_slice(&le[16..]);
    (u128::from_le_bytes(lo), i128::from_le_bytes(hi))
}

/// Orders two 256-bit two's complement little-endian integers.
pub(super) fn cmp_i256(a: [u8; 32], b: [u8; 32]) -> Ordering {
    let (alo, ahi) = halves(a);
    let (blo, bhi) = halves(b);
    (ahi, alo).cmp(&(bhi, blo))
}

fn cmp(a: &[u64; LIMBS], b: &[u64; LIMBS]) -> Ordering {
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

fn bit_len(m: &[u64; LIMBS]) -> u32 {
    m.iter().rposition(|&l| l != 0).map_or(0, |i| {
        64 * u32::try_from(i).unwrap_or(0) + 64 - m[i].leading_zeros()
    })
}

fn shl(m: &mut [u64; LIMBS], k: u32) {
    let (limbs, bits) = ((k / 64) as usize, k % 64);
    for i in (0..LIMBS).rev() {
        let hi = i.checked_sub(limbs).map_or(0, |j| m[j]);
        let lo = i.checked_sub(limbs + 1).map_or(0, |j| m[j]);
        m[i] = if bits == 0 {
            hi
        } else {
            hi << bits | lo >> (64 - bits)
        };
    }
}

fn mul_small(m: &mut [u64; LIMBS], k: u64) {
    let mut carry = 0u128;
    for limb in m.iter_mut() {
        let x = u128::from(*limb) * u128::from(k) + carry;
        #[allow(clippy::cast_possible_truncation)]
        {
            *limb = x as u64;
        }
        carry = x >> 64;
    }
}

/// Divides `m` by `k` in place; returns the remainder.
fn div_small(m: &mut [u64; LIMBS], k: u64) -> u64 {
    let mut rem = 0u128;
    for limb in m.iter_mut().rev() {
        let x = rem << 64 | u128::from(*limb);
        #[allow(clippy::cast_possible_truncation)]
        {
            *limb = (x / u128::from(k)) as u64;
        }
        rem = x % u128::from(k);
    }
    #[allow(clippy::cast_possible_truncation)]
    let rem = rem as u64;
    rem
}

/// Whether any of the lowest `n` bits of `m` is set.
fn low_bits_set(m: &[u64; LIMBS], n: u32) -> bool {
    let (limbs, bits) = ((n / 64) as usize, n % 64);
    m[..limbs].iter().any(|&l| l != 0) || (bits > 0 && m[limbs] & ((1 << bits) - 1) != 0)
}

/// The 64 bits of `m` from bit `at` up.
fn bits_at(m: &[u64; LIMBS], at: u32) -> u64 {
    let (limb, bits) = ((at / 64) as usize, at % 64);
    let lo = m[limb] >> bits;
    let hi = if bits == 0 {
        0
    } else {
        m.get(limb + 1).map_or(0, |&h| h << (64 - bits))
    };
    lo | hi
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
            ColumnType::Float32 => f32::from_le_bytes(take(min)) >= 0.0,
            ColumnType::Float64 => f64::from_le_bytes(take(min)) >= 0.0,
            ColumnType::UInt8 | ColumnType::UInt16 | ColumnType::UInt32 | ColumnType::UInt64 => {
                true
            }
            // Signed integers, timestamps and decimals: the sign bit of the top byte.
            _ => min[usize::try_from(ty.width()).unwrap_or(1) - 1] & 0x80 == 0,
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
        out[40..48].copy_from_slice(&canonical_f64(self.sum).to_le_bytes());
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
    /// fit the column (counts that do not add up to its rows, a range out of order).
    pub fn parse(bytes: &[u8], layout: &ColumnLayout) -> Result<Self, ColumnError> {
        let invalid = ColumnError::Invalid;
        let ty = layout.ty();
        if !ty.is_numeric() {
            return Err(invalid("statistics of a column that is not numeric"));
        }
        if bytes.len() != STATS_LEN || &bytes[..8] != MAGIC {
            return Err(invalid("not a column statistics record"));
        }
        let u64_at =
            |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap_or_default());
        let u32_at =
            |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap_or_default());
        if u32_at(8) != VERSION {
            return Err(ColumnError::Unsupported(
                "a statistics version other than 1",
            ));
        }
        if u32_at(12) & !HAS_RANGE != 0 {
            return Err(ColumnError::Unsupported("statistics flags"));
        }
        let mut min = [0; 32];
        let mut max = [0; 32];
        min.copy_from_slice(&bytes[48..80]);
        max.copy_from_slice(&bytes[80..112]);
        let stats = Self {
            count: u64_at(16),
            null_count: u64_at(24),
            nan_count: u64_at(32),
            sum: f64::from_bits(u64_at(40)),
            range: (u32_at(12) & HAS_RANGE != 0).then_some((min, max)),
        };
        let has_values = stats.nan_count < stats.count;
        if stats.count.checked_add(stats.null_count) != Some(layout.rows())
            || stats.nan_count > stats.count
            || (stats.nan_count > 0 && !matches!(ty, ColumnType::Float32 | ColumnType::Float64))
            || stats.range.is_some() != has_values
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

fn take<const N: usize>(b: [u8; 32]) -> [u8; N] {
    let mut out = [0; N];
    out.copy_from_slice(&b[..N]);
    out
}

/// Whether `min ≤ max` as values of `ty`, each of them canonical (§ 5): no NaN, no
/// −0.0, zero bytes past the type's width.
fn in_order(ty: ColumnType, min: [u8; 32], max: [u8; 32]) -> bool {
    let width = usize::try_from(ty.width()).unwrap_or(32).min(32);
    if min[width..].iter().chain(&max[width..]).any(|&b| b != 0) {
        return false;
    }
    let float = |b: [u8; 32]| match ty {
        ColumnType::Float32 => Some(f64::from(f32::from_le_bytes(take(b)))),
        ColumnType::Float64 => Some(f64::from_le_bytes(take(b))),
        _ => None,
    };
    if let (Some(lo), Some(hi)) = (float(min), float(max)) {
        let canonical = |x: f64| !x.is_nan() && !(x == 0.0 && x.is_sign_negative());
        return canonical(lo) && canonical(hi) && lo <= hi;
    }
    // Every other numeric type orders as a sign-extended integer of its width.
    let extend = |b: [u8; 32]| {
        let signed = !matches!(
            ty,
            ColumnType::UInt8 | ColumnType::UInt16 | ColumnType::UInt32 | ColumnType::UInt64
        );
        let mut x = b;
        if signed && b[width - 1] & 0x80 != 0 {
            x[width..].fill(0xFF);
        }
        x
    };
    cmp_i256(extend(min), extend(max)) != Ordering::Greater
}

/// NaN as the canonical quiet NaN (eg-container § 1).
fn canonical_f64(x: f64) -> u64 {
    if x.is_nan() {
        0x7FF8_0000_0000_0000
    } else {
        x.to_bits()
    }
}

/// Statistics being gathered by the writer, value by value.
#[derive(Debug)]
pub(super) struct Accumulator {
    ty: ColumnType,
    count: u64,
    null_count: u64,
    nan_count: u64,
    sum: ExactSum,
    range: Option<([u8; 32], [u8; 32])>,
}

impl Accumulator {
    pub(super) fn new(ty: ColumnType) -> Self {
        Self {
            ty,
            count: 0,
            null_count: 0,
            nan_count: 0,
            sum: ExactSum::new(),
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
        self.sum.add_f64(x);
        let ty = self.ty;
        extend(&mut self.range, stored, |a, b| {
            as_f64(ty, a) < as_f64(ty, b)
        });
    }

    /// One integer value (an integer, a timestamp or a decimal128).
    pub(super) fn int(&mut self, x: i128, stored: [u8; 32]) {
        self.count += 1;
        self.sum.add_i128(x);
        let ty = self.ty;
        extend(&mut self.range, stored, |a, b| {
            sign_extend(a, ty) < sign_extend(b, ty)
        });
    }

    /// One decimal256 value.
    pub(super) fn wide(&mut self, x: [u8; 32]) {
        self.count += 1;
        self.sum.add_i256(x);
        extend(&mut self.range, x, |a, b| cmp_i256(a, b) == Ordering::Less);
    }

    pub(super) fn finish(self) -> ColumnStats {
        let scale = self.ty.scale();
        ColumnStats {
            count: self.count,
            null_count: self.null_count,
            nan_count: self.nan_count,
            sum: self.sum.to_f64_scaled(scale),
            range: self.range,
        }
    }
}

fn as_f64(ty: ColumnType, b: [u8; 32]) -> f64 {
    match ty {
        ColumnType::Float32 => f64::from(f32::from_le_bytes(take(b))),
        _ => f64::from_le_bytes(take(b)),
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

/// A stored integer of `ty`'s width as an `i128` (unsigned types stay positive).
fn sign_extend(b: [u8; 32], ty: ColumnType) -> i128 {
    let width = usize::try_from(ty.width()).unwrap_or(16).min(16);
    let signed = !matches!(
        ty,
        ColumnType::UInt8 | ColumnType::UInt16 | ColumnType::UInt32 | ColumnType::UInt64
    );
    let mut x = [0u8; 16];
    x[..width].copy_from_slice(&b[..width]);
    if signed && b[width - 1] & 0x80 != 0 {
        x[width..].fill(0xFF);
    }
    i128::from_le_bytes(x)
}
