// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Statistics of repeated measurements. All three methods are non-parametric or say
//! which assumption they make:
//!
//! - [`median_interval`]: the exact rank interval of the median. With `n` values,
//!   `[x(j), x(n+1-j)]` holds the population median with probability
//!   `1 - 2 P(B <= j-1)`, `B ~ Binomial(n, 1/2)`, for any continuous distribution
//!   (Le Boudec, *Performance Evaluation of Computer and Communication Systems*, 2010,
//!   Theorem 2.1; Hoefler and Belli, SC'15, rule 5). So `[min, max]` of 3 values is a
//!   75% interval, of 6 values a 96.9% one, and `[x(2), x(9)]` of 10 a 97.9% one.
//! - [`ratio_interval`]: the percentile bootstrap of the ratio of two medians, each
//!   sample resampled on its own (Efron and Tibshirani, *An Introduction to the
//!   Bootstrap*, 1993, § 13.3; a ratio with an interval is how Kalibera and Jones,
//!   ISMM'13, compare two systems). Seeded, so a report is reproducible. Its coverage
//!   is measured in `tests/stats.rs`: close to 95% from 6 values per side on lognormal
//!   data.
//! - [`repetitions_for`]: the repetitions after which the median's 95% interval is
//!   about `±h` of it. It uses the large-sample spread of a median,
//!   `sqrt(pi/2) * sigma / sqrt(n)` for normal data, with `sigma` estimated robustly as
//!   1.4826 times the median absolute deviation. An estimate, labelled so.

use super::extract::median;

/// The confidence level the harness asks of every interval.
pub const CONFIDENCE: f64 = 0.95;

/// The two-sided 95% quantile of the standard normal distribution.
const Z95: f64 = 1.959_963_984_540_054;

/// An interval and its confidence level.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Interval {
    /// Lower end.
    pub low: f64,
    /// Upper end.
    pub high: f64,
    /// The probability that the interval holds the true value (exact for the rank
    /// interval, nominal for the bootstrap).
    pub confidence: f64,
}

impl Interval {
    /// Whether the interval reaches [`CONFIDENCE`].
    #[must_use]
    pub fn is_95(&self) -> bool {
        self.confidence >= CONFIDENCE - 1e-12
    }
}

/// `P(B <= k)` for `B ~ Binomial(n, 1/2)`, summed in log space so that large `n` does
/// not underflow.
fn binomial_half_cdf(n: usize, k: usize) -> f64 {
    #[allow(clippy::cast_precision_loss, reason = "counts far below 2^52")]
    let nf = n as f64;
    let mut log_term = -nf * std::f64::consts::LN_2;
    let mut sum = log_term.exp();
    for i in 1..=k.min(n) {
        #[allow(clippy::cast_precision_loss, reason = "counts far below 2^52")]
        let (num, den) = ((n - i + 1) as f64, i as f64);
        log_term += (num / den).ln();
        sum += log_term.exp();
    }
    sum.min(1.0)
}

/// The confidence of the rank interval `[x(j), x(n+1-j)]` (1-based `j`).
#[must_use]
pub fn rank_confidence(n: usize, j: usize) -> f64 {
    if j == 0 || 2 * j > n + 1 {
        return 0.0;
    }
    (1.0 - 2.0 * binomial_half_cdf(n, j - 1)).max(0.0)
}

/// The `j` of [`median_interval`] for `n` values (`n > 0`).
fn interval_rank(n: usize, level: f64) -> usize {
    let mut j = 1;
    while rank_confidence(n, j + 1) >= level {
        j += 1;
    }
    j
}

/// The confidence of [`median_interval`] for `n` values at `level`.
#[must_use]
pub fn interval_confidence(n: usize, level: f64) -> f64 {
    if n == 0 {
        return 0.0;
    }
    rank_confidence(n, interval_rank(n, level))
}

/// The narrowest rank interval of the median whose confidence reaches `level`, or the
/// widest one, `[min, max]`, with its lower confidence when no interval reaches it.
/// `None` for no values.
#[must_use]
pub fn median_interval(values: &[f64], level: f64) -> Option<Interval> {
    let n = values.len();
    if n == 0 {
        return None;
    }
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    let j = interval_rank(n, level);
    Some(Interval {
        low: v[j - 1],
        high: v[n - j],
        confidence: rank_confidence(n, j),
    })
}

/// The fewest values whose `[min, max]` is a median interval at `level`: 6 for 95%.
#[must_use]
pub fn min_repetitions(level: f64) -> u32 {
    let mut n = 1;
    while rank_confidence(n, 1) < level {
        n += 1;
    }
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// `SplitMix64` (Steele, Lea and Flood, OOPSLA'14): small, seedable, and good enough to
/// resample and to shuffle.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    /// A generator from a seed.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A uniform index below `n` (`n > 0`), by Lemire's multiply-shift; the bias is
    /// below `n / 2^64`.
    pub fn below(&mut self, n: usize) -> usize {
        let wide = u128::from(self.next_u64()) * n as u128;
        usize::try_from(wide >> 64).unwrap_or(0)
    }

    /// A uniform number in `[0, 1)`.
    pub fn unit(&mut self) -> f64 {
        #[allow(clippy::cast_precision_loss, reason = "53 bits fit exactly")]
        let x = (self.next_u64() >> 11) as f64;
        x / (1u64 << 53) as f64
    }

    /// Fisher–Yates.
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.below(i + 1);
            items.swap(i, j);
        }
    }
}

/// Bootstrap resamples behind a ratio interval.
pub const RESAMPLES: usize = 10_000;

/// The ratio `median(b) / median(a)` with its percentile-bootstrap interval at
/// `level`. `None` when a side is empty or a median of `a` can be 0.
#[must_use]
pub fn ratio_interval(a: &[f64], b: &[f64], level: f64, seed: u64) -> Option<(f64, Interval)> {
    if a.is_empty() || b.is_empty() || a.contains(&0.0) {
        return None;
    }
    let ratio = median(b) / median(a);
    let mut rng = Rng::new(seed);
    let (mut ra, mut rb) = (vec![0.0; a.len()], vec![0.0; b.len()]);
    let mut ratios: Vec<f64> = (0..RESAMPLES)
        .map(|_| {
            for x in &mut ra {
                *x = a[rng.below(a.len())];
            }
            for x in &mut rb {
                *x = b[rng.below(b.len())];
            }
            median(&rb) / median(&ra)
        })
        .collect();
    ratios.sort_by(f64::total_cmp);
    let tail = (1.0 - level) / 2.0;
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "an index into RESAMPLES values"
    )]
    let at = |q: f64| ratios[((q * RESAMPLES as f64).floor() as usize).min(RESAMPLES - 1)];
    Some((
        ratio,
        Interval {
            low: at(tail),
            high: at(1.0 - tail),
            confidence: level,
        },
    ))
}

/// The estimated repetitions after which the median's 95% interval is about `±h` of
/// the median (`h` relative, 0.02 for ±2%), from a pilot sample; at least
/// [`min_repetitions`]. `None` for fewer than 3 values or a zero median.
#[must_use]
pub fn repetitions_for(values: &[f64], h: f64) -> Option<u32> {
    if values.len() < 3 || h <= 0.0 {
        return None;
    }
    let m = median(values);
    if m == 0.0 {
        return None;
    }
    let deviations: Vec<f64> = values.iter().map(|x| (x - m).abs()).collect();
    let sigma = 1.4826 * median(&deviations);
    let need = (Z95 * (std::f64::consts::PI / 2.0).sqrt() * sigma / (h * m.abs())).powi(2);
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to the u32 range first"
    )]
    let need = need.ceil().clamp(0.0, f64::from(u32::MAX)) as u32;
    Some(need.max(min_repetitions(CONFIDENCE)))
}
