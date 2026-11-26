// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The check of a per-entry weight or cost column before a kernel uses it, and the
//! power of two that keeps a weighted power iteration inside its state's range.

use std::num::NonZeroUsize;

use everygraph_core::par;

use crate::edge_map::Error;

const GRAIN: NonZeroUsize = NonZeroUsize::new(1 << 16).expect("non-zero");

/// The binary exponents of the smallest positive and of the largest value of a column:
/// `2^lo ≤ min` and `max < 2^(hi + 1)`. `None` if no value is positive.
pub(crate) type Range = Option<(i32, i32)>;

/// `Ok` with the column's [`Range`] if `column` has one value per entry of a source with
/// `entries` entries, and every value is finite and not negative (zero is allowed).
/// Otherwise [`Error::Invalid`], or [`Error::Weight`] with the first bad value
/// (`docs/spec/graph-semantics.md` § 4.4).
pub(crate) fn check(column: &[f64], entries: u64) -> Result<Range, Error> {
    if column.len() as u64 != entries {
        return Err(Error::Invalid(
            "a per-entry column whose length is not the number of entries",
        ));
    }
    // Per chunk: the first bad position, else the smallest positive and the largest
    // value. Minimum and maximum do not depend on the order they are taken in.
    let parts = par::map_ranges(column.len(), GRAIN, |r| {
        let start = r.start;
        let values = &column[r];
        match values.iter().position(|w| !(w.is_finite() && *w >= 0.0)) {
            Some(i) => Err(start + i),
            None => Ok(values.iter().fold((f64::INFINITY, 0.0_f64), |(lo, hi), &w| {
                (if w > 0.0 && w < lo { w } else { lo }, if w > hi { w } else { hi })
            })),
        }
    });
    let (mut lo, mut hi) = (f64::INFINITY, 0.0_f64);
    for part in parts {
        match part {
            Err(i) => {
                return Err(Error::Weight {
                    entry: i as u64,
                    value: column[i],
                });
            }
            Ok((l, h)) => {
                lo = if l < lo { l } else { lo };
                hi = if h > hi { h } else { hi };
            }
        }
    }
    Ok((hi > 0.0).then(|| (exponent(lo), exponent(hi))))
}

/// `⌊log₂ x⌋` of a positive finite `x`, from its bits (subnormals included).
fn exponent(x: f64) -> i32 {
    let bits = x.to_bits();
    let biased = i32::try_from((bits >> 52) & 0x7FF).expect("11 bits");
    if biased == 0 {
        // Subnormal: 2^-1074 times the fraction, whose top bit gives the exponent.
        let top = 63 - i32::try_from(bits.leading_zeros()).expect("below 64");
        -1074 + top
    } else {
        biased - 1023
    }
}

/// The power of two `2^k` by which a power iteration multiplies the weights of a column
/// of range `range` over `entries` entries, so that every scaled weight is at least
/// `2^-bound` and every sum of them (a strength, at most `2·entries` weights) is below
/// `2^bound`. Then a contribution `x / s` (with `x ≤ 1`) and a strength stay finite and
/// normal in a state whose exponents reach `bound` (`Real::EXPONENTS`). `k` is 0 when
/// the column already fits, so ordinary weights keep their bits; scaling by a power of
/// two is exact, and `PageRank` and the eigenvector do not change when every weight is
/// scaled alike.
///
/// # Errors
///
/// [`Error::Invalid`] if no single power of two can do both: the weights span more than
/// the state's range.
pub(crate) fn scale(range: Range, entries: u64, bound: i32) -> Result<f64, Error> {
    let Some((lo, hi)) = range else {
        return Ok(1.0);
    };
    // A sum of at most `2·entries` values below `2^(hi + k + 1)` is below this power.
    let terms = i32::try_from(64 - (2 * entries.max(1) - 1).leading_zeros()).expect("below 65");
    let (least, most) = (-bound - lo, bound - hi - 1 - terms);
    if least > most {
        return Err(Error::Invalid(
            "weights: the smallest and the largest positive weight are too far apart for \
             the state's precision",
        ));
    }
    Ok(pow2(0.clamp(least, most)))
}

/// `2^k`, exactly, for `|k| ≤ 2,046`.
fn pow2(k: i32) -> f64 {
    let one = |j: i32| f64::from_bits(u64::try_from(j + 1023).expect("in range") << 52);
    let half = k / 2;
    one(half) * one(k - half)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exponents_and_powers_of_two_are_exact() {
        for x in [1.0, 1.5, 2.0, 1e308, f64::MAX, 1e-310, 5e-324, f64::MIN_POSITIVE] {
            let e = exponent(x);
            assert!(pow2(e) <= x && x < 2.0 * pow2(e), "{x}");
        }
        assert_eq!(pow2(0), 1.0);
        assert_eq!(pow2(-1074), 5e-324);
        assert_eq!(pow2(1023).to_bits(), 0x7FE0_0000_0000_0000);
    }

    #[test]
    fn a_column_that_fits_keeps_its_scale() {
        let range = check(&[0.0, 1.0, 3.0, 1e-9], 4).unwrap();
        assert_eq!(scale(range, 4, 120).unwrap(), 1.0);
        assert_eq!(scale(None, 4, 120).unwrap(), 1.0);
        let wide = check(&[1e-300, 1e300], 2).unwrap();
        assert!(scale(wide, 2, 120).is_err());
        assert!(scale(wide, 2, 1000).is_ok());
    }
}
