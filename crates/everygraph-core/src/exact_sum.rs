// SPDX-License-Identifier: Apache-2.0 OR MIT

//! An exact sum of `f64` values: the same bits for any order and any split into parts.

use crate::math::CANONICAL_NAN;

/// Bits per digit of the accumulator.
const DIGIT_BITS: u32 = 32;
/// Digits 0..=65 cover every bit an `f64` can have (2^-1074 to 2^1023); the two above
/// them take the carries of up to 2^64 additions of `f64::MAX`.
const DIGITS: usize = 68;
/// The weight of bit 0 of digit 0: 2^-1074, the smallest subnormal.
const LOWEST_EXPONENT: i32 = -1074;
/// Additions between two carry passes. Each adds less than 2^32 to a digit, so a digit
/// stays below 2^63 in magnitude.
const PENDING_LIMIT: u32 = 1 << 30;

/// The exact sum of the `f64` values added to it, rounded to nearest only when read.
///
/// For global scalars — a total weight, a modularity, the rank held by dangling
/// nodes, a residual — whose parts are summed per chunk: each chunk adds its values into its own
/// `ExactSum`, and the parts are merged in any order. The value does not depend on the
/// chunk size, the thread count or the merge order (rule R6 of
/// `docs/research/sota-numerics.md`), so a reduction tree does not have to be fixed.
///
/// It is a long fixed-point accumulator, like the "small superaccumulator" of Neal
/// (arXiv 1505.05571): 68 signed digits of 32 bits, with carries propagated every 2^30
/// additions and before a merge or a read. An addition costs a few integer operations.
///
/// The value is the exact sum rounded once to the nearest `f64` (ties to even), so it
/// is infinite only if that rounded value is: `f64::MAX + f64::MAX - f64::MAX` is
/// `f64::MAX`. An exact zero is +0.0. Infinite inputs give an infinity of their sign;
/// a NaN, or infinities of both signs, give [`CANONICAL_NAN`].
#[derive(Clone, Debug)]
pub struct ExactSum {
    digits: [i64; DIGITS],
    /// Below `PENDING_LIMIT`; every digit but the last is smaller in magnitude than
    /// `(pending + 1) * 2^32`.
    pending: u32,
    positive_infinity: bool,
    negative_infinity: bool,
    nan: bool,
}

impl Default for ExactSum {
    fn default() -> Self {
        Self::new()
    }
}

impl ExactSum {
    /// An empty sum, whose value is +0.0.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            digits: [0; DIGITS],
            pending: 0,
            positive_infinity: false,
            negative_infinity: false,
            nan: false,
        }
    }

    /// Adds `x`.
    #[expect(clippy::missing_panics_doc, reason = "every conversion is in range")]
    pub fn add(&mut self, x: f64) {
        if !x.is_finite() {
            if x.is_nan() {
                self.nan = true;
            } else if x > 0.0 {
                self.positive_infinity = true;
            } else {
                self.negative_infinity = true;
            }
            return;
        }
        let bits = x.to_bits();
        let biased_exponent = (bits >> 52) & 0x7FF;
        let fraction = bits & ((1 << 52) - 1);
        // x = mantissa · 2^(LOWEST_EXPONENT + position)
        let (mantissa, position) = match biased_exponent {
            0 => (fraction, 0),
            e => (fraction | (1 << 52), e - 1),
        };
        if mantissa == 0 {
            return;
        }
        let index = usize::try_from(position / u64::from(DIGIT_BITS)).expect("below 64");
        let shifted = u128::from(mantissa) << (position % u64::from(DIGIT_BITS));
        let negative = bits >> 63 == 1;
        for (k, digit) in self.digits[index..index + 3].iter_mut().enumerate() {
            let piece = low_digit(shifted >> (32 * k));
            if negative {
                *digit -= piece;
            } else {
                *digit += piece;
            }
        }
        self.pending += 1;
        if self.pending == PENDING_LIMIT {
            carry(&mut self.digits);
            self.pending = 0;
        }
    }

    /// Adds every value that `other` holds.
    pub fn merge(&mut self, other: &Self) {
        let mut theirs = other.digits;
        carry(&mut theirs);
        carry(&mut self.digits);
        for (digit, their) in self.digits.iter_mut().zip(theirs) {
            *digit += their;
        }
        self.pending = 1;
        self.positive_infinity |= other.positive_infinity;
        self.negative_infinity |= other.negative_infinity;
        self.nan |= other.nan;
    }

    /// The sum, rounded to the nearest `f64`.
    #[must_use]
    #[expect(clippy::missing_panics_doc, reason = "every conversion is in range")]
    pub fn value(&self) -> f64 {
        match (self.nan, self.positive_infinity, self.negative_infinity) {
            (true, _, _) | (_, true, true) => return CANONICAL_NAN,
            (_, true, false) => return f64::INFINITY,
            (_, false, true) => return f64::NEG_INFINITY,
            (false, false, false) => {}
        }
        let mut digits = self.digits;
        carry(&mut digits);
        let negative = digits[DIGITS - 1] < 0;
        if negative {
            for digit in &mut digits {
                *digit = -*digit;
            }
            carry(&mut digits);
        }
        let Some(top) = digits.iter().rposition(|&d| d != 0) else {
            return 0.0;
        };
        // The top three digits hold at least 65 significant bits when there are digits
        // below them, so a sticky bit at bit 0 makes the one rounding of `as f64`
        // (to nearest, ties to even) round the whole sum correctly.
        let low = top.saturating_sub(2);
        let mut head = 0_u128;
        for &digit in digits[low..=top].iter().rev() {
            head = (head << DIGIT_BITS) | u128::from(digit.unsigned_abs());
        }
        if digits[..low].iter().any(|&d| d != 0) {
            head |= 1;
        }
        let exponent = LOWEST_EXPONENT
            + i32::try_from(low).expect("below 68") * i32::try_from(DIGIT_BITS).expect("32");
        let magnitude = scale(head as f64, exponent);
        if negative { -magnitude } else { magnitude }
    }
}

impl FromIterator<f64> for ExactSum {
    fn from_iter<I: IntoIterator<Item = f64>>(values: I) -> Self {
        let mut sum = Self::new();
        values.into_iter().for_each(|x| sum.add(x));
        sum
    }
}

/// The low 32 bits of `x`.
fn low_digit(x: u128) -> i64 {
    i64::try_from(x & 0xFFFF_FFFF).expect("32 bits")
}

/// Moves carries up, leaving every digit but the last in `0..2^32`. The value is
/// unchanged.
fn carry(digits: &mut [i64; DIGITS]) {
    for i in 0..DIGITS - 1 {
        let up = digits[i] >> DIGIT_BITS;
        digits[i] -= up << DIGIT_BITS;
        digits[i + 1] += up;
    }
}

/// `x · 2^exponent` for `exponent` in `-1074..=1006`. Each factor is a normal power of
/// two, so each product is exact unless it overflows; the caller guarantees that the
/// final value is representable or infinite.
fn scale(x: f64, exponent: i32) -> f64 {
    let half = exponent / 2;
    x * power_of_two(half) * power_of_two(exponent - half)
}

/// 2^`e` for `e` in `-1022..=1023`.
fn power_of_two(e: i32) -> f64 {
    f64::from_bits(u64::try_from(e + 1023).expect("normal exponent") << 52)
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use super::*;
    use crate::{par, rng};

    /// A value of either sign whose magnitude is spread over 2^-1074 to 2^1023, with
    /// some exact zeros and subnormals.
    fn wide(bits: u64) -> f64 {
        let magnitude = f64::from_bits(bits & 0x7FEF_FFFF_FFFF_FFFF);
        if bits >> 63 == 1 {
            -magnitude
        } else {
            magnitude
        }
    }

    fn values(n: u32, f: impl Fn(u64) -> f64) -> Vec<f64> {
        (0..n)
            .map(|i| f(rng::draw(7, *b"TEST", [i, 0, 0])))
            .collect()
    }

    /// The sum by the `xsum` crate. xsum 0.1.6 rounds some negative totals one unit
    /// away from zero (the single value with bits `0xE329_C06B_253C_76B9` comes back
    /// one unit larger in magnitude, its negation exactly), so a negative total is
    /// taken from the negated values.
    fn xsum(values: &[f64]) -> f64 {
        use xsum::Xsum;
        let plain = |values: &[f64]| {
            let mut sum = xsum::XsumLarge::new();
            sum.add_list(values);
            sum.sum()
        };
        let total = plain(values);
        if total < 0.0 {
            -plain(&values.iter().map(|x| -x).collect::<Vec<_>>())
        } else {
            total
        }
    }

    fn exact(values: &[f64]) -> f64 {
        values.iter().copied().collect::<ExactSum>().value()
    }

    #[test]
    fn agrees_with_xsum() {
        let ranks = values(3_000, |x| rng::unit_f64(x) * 1e-4);
        let wide_range = values(3_000, wide);
        let tiny = values(3_000, |x| f64::from_bits(x & 0x83FF_FFFF_FFFF_FFFF));
        let mut cancelling = values(1_000, |x| rng::unit_f64(x) * 1e300);
        cancelling.extend(cancelling.clone().iter().map(|x| -x));
        cancelling.push(f64::MIN_POSITIVE / 3.0);
        for case in [ranks, wide_range, tiny, cancelling] {
            for n in [0, 1, 2, 17, case.len()] {
                let part = &case[..n];
                // xsum returns -0.0 for an empty sum; ours is +0.0.
                let expected = crate::math::canonical(xsum(part));
                assert_eq!(exact(part).to_bits(), expected.to_bits(), "{part:?}");
                let negated: Vec<f64> = part.iter().map(|x| -x).collect();
                let expected = crate::math::canonical(-expected);
                assert_eq!(exact(&negated).to_bits(), expected.to_bits(), "{part:?}");
            }
        }
    }

    #[test]
    fn rounds_once_at_the_edges() {
        let cases: [(&[f64], f64); 8] = [
            (&[], 0.0),
            (&[-0.0, -0.0], 0.0),
            (&[1e308, 1.0, -1e308], 1.0),
            (&[f64::MAX, f64::MAX, -f64::MAX], f64::MAX),
            (&[f64::MAX, f64::MAX], f64::INFINITY),
            (&[-f64::MAX, -f64::MAX], f64::NEG_INFINITY),
            (&[f64::from_bits(1), f64::from_bits(1)], f64::from_bits(2)),
            // 1 + 2^-53 + 2^-1074 is just above the tie, so it rounds up.
            (
                &[1.0, f64::EPSILON / 2.0, f64::from_bits(1)],
                1.0 + f64::EPSILON,
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(exact(input).to_bits(), expected.to_bits(), "{input:?}");
        }
        assert_eq!(
            exact(&[1.0, f64::EPSILON / 2.0]),
            1.0,
            "a tie rounds to even"
        );
        assert_eq!(exact(&[f64::INFINITY, 1.0]), f64::INFINITY);
        assert_eq!(exact(&[f64::NEG_INFINITY, 1.0]), f64::NEG_INFINITY);
        let nan = CANONICAL_NAN.to_bits();
        assert_eq!(exact(&[f64::INFINITY, f64::NEG_INFINITY]).to_bits(), nan);
        assert_eq!(exact(&[1.0, -f64::NAN]).to_bits(), nan);
    }

    #[test]
    fn carries_keep_the_value() {
        let mut sum = ExactSum::new();
        let mut reference = ExactSum::new();
        for _ in 0..3 {
            sum.add(-f64::MAX);
            reference.add(-f64::MAX);
        }
        sum.pending = PENDING_LIMIT - 1;
        sum.add(-1.0);
        assert_eq!(sum.pending, 0, "the addition carried");
        reference.add(-1.0);
        assert_eq!(sum.value().to_bits(), reference.value().to_bits());
    }

    /// The acceptance test of issue #187: the rank held by dangling nodes (here, a
    /// random third of the nodes) has the same bits for chunks of 64 and
    /// 65,536 and for one sequential pass, while a plain `f64` fold over the same
    /// chunks does not.
    #[test]
    fn dangling_mass_is_the_same_for_any_chunking() {
        let ranks = values(70_000, |x| {
            if x % 3 == 0 {
                rng::unit_f64(x) / 7e4
            } else {
                0.0
            }
        });
        let sequential = exact(&ranks);
        let chunked = |grain| {
            let grain = NonZeroUsize::new(grain).expect("non-zero");
            par::map_ranges(ranks.len(), grain, |range| {
                ranks[range].iter().copied().collect::<ExactSum>()
            })
        };
        let plain = |grain| {
            let grain = NonZeroUsize::new(grain).expect("non-zero");
            let parts = par::map_ranges(ranks.len(), grain, |range| {
                crate::math::sum(ranks[range].iter().copied())
            });
            crate::math::sum(parts)
        };
        for grain in [64, 65_536] {
            let parts = chunked(grain);
            let mut forward = ExactSum::new();
            parts.iter().for_each(|part| forward.merge(part));
            let mut backward = ExactSum::new();
            parts.iter().rev().for_each(|part| backward.merge(part));
            assert_eq!(forward.value().to_bits(), sequential.to_bits());
            assert_eq!(backward.value().to_bits(), sequential.to_bits());
        }
        assert_ne!(
            plain(64).to_bits(),
            plain(65_536).to_bits(),
            "the test would see a difference"
        );
    }
}
