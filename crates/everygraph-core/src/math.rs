// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Floating point that gives the same bits on every supported platform (DESIGN D14).
//!
//! The rules are those of `docs/research/sota-numerics.md` § 4 (R1–R15); this module
//! holds the pieces that code needs to follow them:
//!
//! - **Transcendentals.** `f64::sin` and its relatives call the platform's maths
//!   library, and those libraries return different bits (glibc, musl, Apple's libm and
//!   the `libm` crate on wasm32; glibc also chooses a code path by CPU at run time).
//!   [`sin`], [`cos`], [`exp`], [`ln`] and [`log2`] instead call the pure-Rust `libm`
//!   crate at the exact version [`LIBM_VERSION`], compiled from the same source for
//!   every target. They are not correctly rounded (`libm` stays below 1 ulp); identity
//!   needs one implementation, not the most accurate one. The crate's `pow` is left
//!   out on purpose: it comes from FreeBSD's `e_pow.c`, whose library shows 636 ulp
//!   (Gladman et al. 2026, Table 3). Write `x * x` for `powi(2)`, precompute constants
//!   such as `0.1^i`, and use `ilog2` or `leading_zeros` for integer decisions.
//!   `sqrt`, `mul_add`, `+ − × ÷ %` and `as` casts are exact IEEE 754 operations in
//!   Rust (RFC 3514) and need no wrapper. Clippy rejects the std transcendentals in the
//!   crates that produce results (`clippy.toml`).
//! - **Canonical bits.** Rust leaves the sign of a NaN open, x87 returns change its
//!   payload, and wasm32 engines need not keep its bits. [`canonical`] and
//!   [`canonical_f32`] map every NaN to the quiet NaN of the `.eg` specification and
//!   −0.0 to +0.0. Apply them to every value that is written, stored in a derived
//!   section or hashed.
//! - **Sums.** `Iterator::sum` on floats starts from −0.0, so an empty sum is −0.0;
//!   [`sum`] starts from +0.0. A sum whose order is not fixed by the data alone — a
//!   global scalar folded over chunks — uses [`ExactSum`](crate::ExactSum) instead.
//!   Where two values can be equal up to the sign of zero, or NaN, compare with
//!   `total_cmp`: `f64::max` may return either input.
//!
//! What a result was computed with goes into its provenance: [`NUMERICS`] and
//! [`LIBM_VERSION`], as the parameters `numerics` and `libm`
//! (`docs/spec/eg-container.md` § 6).

/// The version of the rule list of `docs/research/sota-numerics.md` § 4 that the
/// engine follows. Written into provenance as the parameter `numerics`.
pub const NUMERICS: u32 = 1;

/// The version of the `libm` crate behind the functions of this module, pinned with
/// `=` in `Cargo.toml`. Written into provenance as the parameter `libm`.
pub const LIBM_VERSION: &str = "0.2.16";

/// The NaN every result stores: quiet, positive, no payload (`docs/spec/eg-container.md` § 2).
pub const CANONICAL_NAN: f64 = f64::from_bits(0x7FF8_0000_0000_0000);

/// The `f32` NaN every result stores.
pub const CANONICAL_NAN_F32: f32 = f32::from_bits(0x7FC0_0000);

/// The sine of `x` (radians).
#[must_use]
pub fn sin(x: f64) -> f64 {
    libm::sin(x)
}

/// The cosine of `x` (radians).
#[must_use]
pub fn cos(x: f64) -> f64 {
    libm::cos(x)
}

/// `e` to the power `x`.
#[must_use]
pub fn exp(x: f64) -> f64 {
    libm::exp(x)
}

/// The natural logarithm of `x`.
#[must_use]
pub fn ln(x: f64) -> f64 {
    libm::log(x)
}

/// The base-2 logarithm of `x`.
#[must_use]
pub fn log2(x: f64) -> f64 {
    libm::log2(x)
}

/// `x` with every NaN replaced by [`CANONICAL_NAN`] and −0.0 by +0.0.
#[must_use]
pub fn canonical(x: f64) -> f64 {
    if x.is_nan() {
        CANONICAL_NAN
    } else if x == 0.0 {
        0.0
    } else {
        x
    }
}

/// `x` with every NaN replaced by [`CANONICAL_NAN_F32`] and −0.0 by +0.0.
#[must_use]
pub fn canonical_f32(x: f32) -> f32 {
    if x.is_nan() {
        CANONICAL_NAN_F32
    } else if x == 0.0 {
        0.0
    } else {
        x
    }
}

/// The sum of `values` added in order, starting from +0.0.
#[must_use]
pub fn sum(values: impl IntoIterator<Item = f64>) -> f64 {
    values.into_iter().fold(0.0, |total, x| total + x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_maps_every_nan_and_negative_zero() {
        let nans = [
            f64::NAN,
            -f64::NAN,
            f64::from_bits(0x7FF0_0000_0000_0001),
            f64::from_bits(0xFFF8_0000_0000_1234),
        ];
        let nans_f32 = [
            f32::NAN,
            -f32::NAN,
            f32::from_bits(0x7F80_0001),
            f32::from_bits(0xFFC0_1234),
        ];
        for (x, y) in nans.into_iter().zip(nans_f32) {
            assert_eq!(canonical(x).to_bits(), 0x7FF8_0000_0000_0000);
            assert_eq!(canonical_f32(y).to_bits(), 0x7FC0_0000);
        }
        assert_eq!(canonical(-0.0).to_bits(), 0);
        assert_eq!(canonical_f32(-0.0).to_bits(), 0);
        for x in [
            1.5,
            -2.0,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::MIN_POSITIVE / 4.0,
        ] {
            assert_eq!(canonical(x).to_bits(), x.to_bits());
        }
    }

    #[test]
    fn sum_starts_from_positive_zero() {
        assert_eq!(sum([]).to_bits(), 0);
        assert_eq!(sum([-0.0]).to_bits(), 0);
        assert_eq!(sum([0.5, 0.25, -1.0]), -0.25);
    }

    #[test]
    fn libm_version_matches_the_pin() {
        let manifest = include_str!("../Cargo.toml");
        let pin = format!("libm = {{ version = \"={LIBM_VERSION}\"");
        assert!(manifest.contains(&pin), "Cargo.toml must pin `{pin}`");
    }
}
