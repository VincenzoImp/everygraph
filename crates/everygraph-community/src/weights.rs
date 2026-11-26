// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Real edge weights as fixed-point integers (`docs/research/README.md` N1;
//! `docs/research/sota-numerics.md` § 3.3, option B).
//!
//! Every weight `w` becomes the integer `round(w·2^s)`. The scale `s` is chosen once,
//! from the exact total `T` of the weight columns (`everygraph_core::ExactSum`): the
//! largest `s` with `T·2^s < 2^60`. So the total of `A` (a loop twice, and the rounding
//! of every entry) stays below 2^62, as `level::strengths` requires. Every sum of
//! weights (strengths, community volumes, quotient weights, `L(c)`, `S(c)`) is then an
//! integer, exact in any order: the result does not depend on threads, chunking, the
//! budget or the source.
//!
//! The cost is quantization: each weight is a multiple of `2^-s`, and its error is at
//! most `2^-(s+1)`, below `2^-60·T`. A weight below `2^-(s+1)` counts as 0, like a
//! zero weight; that happens only when `T` over the smallest non-zero weight exceeds
//! 2^60. Integer weights with `T < 2^60` get `s ≥ 0` and are exact. Multiplying every
//! weight by a power of two changes no comparison of a gain and no modularity (every
//! product scales exactly), so weights that are all 1.0 give the bits of an unweighted
//! run.

use everygraph_core::{ExactSum, par};
use everygraph_kernels::edge_map::{Error, Source};

use crate::column::GRAIN;
use crate::level::Weights;

/// The weights of a graph's entries, in the order of its lists.
#[derive(Clone, Copy, Debug, Default)]
pub enum EdgeWeights<'a> {
    /// Every pair weighs 1.
    #[default]
    Unit,
    /// One weight per entry of an undirected graph: its pair's weight (a per-pair
    /// column, `docs/spec/eg-columns.md` § 6), the same in both lists of the pair. A
    /// kept loop's entry holds `w`; it counts `2w` in its node's strength.
    Pairs(&'a [f64]),
    /// The undirected projection of a directed graph: for the entry of `v` in the list
    /// of `u`, `out` holds the weight of `(u, v)` and `inn` that of `(v, u)`, 0 for an
    /// arc that does not exist. A kept loop's entry holds its weight in both. Its pair
    /// weighs `out + inn` (`docs/spec/graph-semantics.md`, Q9), and `use_direction`
    /// reads the two apart.
    Directed {
        /// The weight of each entry's arc from the list's node.
        out: &'a [f64],
        /// The weight of each entry's arc to the list's node.
        inn: &'a [f64],
    },
}

/// The smallest scale: below it, `2^-s` is not a normal float.
const MIN_SCALE: i32 = -1022;
/// The largest scale, for the same reason.
const MAX_SCALE: i32 = 1022;

/// `2^s`, for a scale in range.
pub(crate) fn power_of_two(s: i32) -> f64 {
    debug_assert!((MIN_SCALE..=MAX_SCALE).contains(&s));
    let biased = u64::try_from(s + 1023).expect("a scale in range");
    f64::from_bits(biased << 52)
}

/// `round(w·unit)`, ties away from zero, for `0 ≤ w·unit < 2^62`.
#[inline]
pub(crate) fn quantize(w: f64, unit: f64) -> u64 {
    let x = w * unit;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "x is in [0, 2^62)"
    )]
    let t = x as u64;
    // Exact: `t` is the integer part of a float, so it is one.
    t + u64::from(x - t as f64 >= 0.5)
}

/// The integer weights of `weights` on `source`, and their scale `s` (`None` for
/// [`EdgeWeights::Unit`]).
///
/// # Errors
///
/// [`Error::Weight`] for the first negative, NaN or infinite weight;
/// [`Error::Invalid`] for a column whose length is not the entry count, or a total that
/// is infinite or below 2^-963.
pub(crate) fn scaled<'a, S: Source>(
    source: &S,
    weights: EdgeWeights<'a>,
) -> Result<(Weights<'a>, Option<i32>), Error> {
    let (first, second) = match weights {
        EdgeWeights::Unit => return Ok((Weights::Unit, None)),
        EdgeWeights::Pairs(w) => (w, None),
        EdgeWeights::Directed { out, inn } => (out, Some(inn)),
    };
    let mut total = ExactSum::new();
    for column in std::iter::once(first).chain(second) {
        if column.len() as u64 != source.entries() {
            return Err(Error::Invalid("weights: not one per entry"));
        }
        let parts = par::map_ranges(column.len(), GRAIN, |r| {
            let mut sum = ExactSum::new();
            for e in r {
                let w = column[e];
                if !(w >= 0.0 && w.is_finite()) {
                    return Err(e);
                }
                sum.add(w);
            }
            Ok(sum)
        });
        for part in parts {
            let sum = part.map_err(|e| Error::Weight {
                entry: e as u64,
                value: column[e],
            })?;
            total.merge(&sum);
        }
    }
    let t = total.value();
    if !t.is_finite() {
        return Err(Error::Invalid("weights: a total above the largest float"));
    }
    let s = if t == 0.0 {
        0
    } else {
        // The exponent field: floor(log2 T) for a normal T, so 2^e ≤ T < 2^(e + 1).
        let e = i32::try_from((t.to_bits() >> 52) & 0x7ff).expect("11 bits") - 1023;
        59 - e
    };
    if s > MAX_SCALE {
        return Err(Error::Invalid("weights: a total below 2^-963"));
    }
    let unit = power_of_two(s);
    let w = match second {
        None => Weights::Scaled { w: first, unit },
        Some(inn) => Weights::Directed {
            out: first,
            inn,
            unit,
        },
    };
    Ok((w, Some(s)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantize_rounds_to_nearest() {
        assert_eq!(quantize(1.0, 1.0), 1);
        assert_eq!(quantize(0.49, 1.0), 0);
        assert_eq!(quantize(0.5, 1.0), 1);
        assert_eq!(quantize(2.5, 1.0), 3);
        assert_eq!(quantize(0.3, 1024.0), 307);
        assert_eq!(quantize(3.0, power_of_two(40)), 3 << 40);
        let odd = (1_u64 << 52) + 1;
        assert_eq!(quantize(odd as f64, 1.0), odd);
        assert_eq!(power_of_two(-1022), f64::MIN_POSITIVE);
        assert_eq!(power_of_two(0), 1.0);
    }
}
