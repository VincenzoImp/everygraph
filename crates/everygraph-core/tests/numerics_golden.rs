// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Golden vectors of the portable numerics (rules R15c and R15d of
//! `docs/research/sota-numerics.md`): each function of `everygraph_core::math`, `sqrt`
//! and `mul_add` on 100,000 inputs, `ExactSum` over the same inputs, 100,000 draws of
//! `everygraph_core::rng`, and NaN and −0.0 made at run time, each reduced to one digest
//! and compared with a frozen value. Results are hashed as raw bits, so the sign of a
//! zero counts; only NaNs are made canonical, because their bits are not specified.
//!
//! The same file is compiled into the core's tests on every target CI tests (x86-64,
//! i686, aarch64, macOS arm64, Windows) and into `tools/wasm-toy`, which runs it as
//! wasm32 in Node (`tools/check_wasm_toy.py`). A digest that changes on one target is
//! a platform difference; one that changes everywhere is a change of the pinned `libm`
//! or of the generator, and needs a new `NUMERICS` version.

#![forbid(
    unsafe_code,
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]

use std::hint::black_box;

use everygraph_core::rng::Purpose;
use everygraph_core::{ExactSum, math, rng};

/// Inputs per function and draws of the generator.
const COUNT: u32 = 100_000;

/// The frozen digests, computed on x86-64.
pub const EXPECTED: [(&str, u64); 10] = [
    ("sin", 0x3939_011F_FAC3_7251),
    ("cos", 0x6482_B616_360D_FECF),
    ("exp", 0xBCBD_159A_E70F_3F26),
    ("ln", 0x4EDD_F0B9_7888_A8C2),
    ("log2", 0xF5E2_D2F2_DEF6_66CE),
    ("sqrt", 0xAD71_E90F_94E2_53E3),
    ("mul_add", 0x342D_D7B1_FCCA_2CD3),
    ("exact_sum", 0x2809_B097_C2A6_54F3),
    ("rng", 0xF919_7365_F787_4972),
    ("canonical", 0xCEDC_C921_8CFF_6465),
];

/// Input `i`: a quarter each of values in [−800, 800) (the useful range of `exp`), of
/// either sign with magnitudes 2^−40 to 2^40, of any bits (subnormals, infinities and
/// NaNs included), and of any positive bits (large arguments of `sin` and `cos`).
fn input(i: u32) -> f64 {
    let bits = rng::draw(0, Purpose::TEST, [i, 0, 0, 0]);
    match i % 4 {
        0 => (rng::unit_f64(bits) - 0.5) * 1600.0,
        1 => {
            let draw = rng::draw(0, Purpose::TEST, [i, 1, 0, 0]);
            let exponent = 1023 - 40 + rng::below(draw, 81);
            f64::from_bits((bits & 0x800F_FFFF_FFFF_FFFF) | (exponent << 52))
        }
        2 => f64::from_bits(bits),
        _ => f64::from_bits(bits & 0x7FFF_FFFF_FFFF_FFFF),
    }
}

/// FNV-1a over 64-bit words.
fn digest(words: impl Iterator<Item = u64>) -> u64 {
    words.fold(0xCBF2_9CE4_8422_2325, |hash, word| {
        (hash ^ word).wrapping_mul(0x0000_0100_0000_01B3)
    })
}

/// The bits of `x`, with a NaN made canonical.
fn bits(x: f64) -> u64 {
    if x.is_nan() { math::CANONICAL_NAN } else { x }.to_bits()
}

fn of_function(f: fn(f64) -> f64) -> u64 {
    digest((0..COUNT).map(|i| bits(f(input(i)))))
}

/// `mul_add` calls the C library's `fma` on i686 and on x86-64 without the `fma`
/// feature, and a software `fma` on wasm32.
fn of_mul_add() -> u64 {
    let three = |i| (input(i), input(i + COUNT), input(i + 2 * COUNT));
    digest((0..COUNT).map(three).map(|(x, y, z)| bits(x.mul_add(y, z))))
}

/// Sums of 1,000 groups of 100 inputs, then of all groups merged in reverse order.
/// The rounding goes through `u128 as f64`, a software routine on wasm32 and i686.
fn of_exact_sum() -> u64 {
    let groups: Vec<ExactSum> = (0..COUNT / 100)
        .map(|g| (g * 100..(g + 1) * 100).map(input).collect())
        .collect();
    let mut total = ExactSum::new();
    groups.iter().rev().for_each(|group| total.merge(group));
    digest(groups.iter().chain([&total]).map(|sum| bits(sum.value())))
}

/// Draws whose seeds and counter words all vary.
fn of_rng() -> u64 {
    digest((0..COUNT).map(|i| {
        let seed = u64::from(i).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        rng::draw(seed, Purpose::TEST, [i, i ^ 0xFFFF, i.rotate_left(16), !i])
    }))
}

/// Values that are NaN or −0.0 on some platforms, made at run time so that the
/// platform's arithmetic makes them, then canonicalized.
fn of_canonical() -> u64 {
    let [zero, one, infinity] = [0.0_f64, 1.0, f64::INFINITY].map(black_box);
    let values = [
        -zero,
        zero * -one,
        -1e-300 * black_box(1e-300),
        zero / black_box(0.0),
        -(zero / black_box(0.0)),
        infinity - black_box(f64::INFINITY),
        zero * infinity,
        black_box(-1.0_f64).sqrt(),
        math::ln(-1.0),
        math::sin(infinity),
        math::sum([-zero]),
    ];
    let [zero, infinity] = [0.0_f32, f32::INFINITY].map(black_box);
    let narrow = [
        -zero,
        zero / black_box(0.0),
        -(zero / black_box(0.0)),
        infinity - black_box(f32::INFINITY),
        black_box(-1.0_f32).sqrt(),
    ];
    let wide = values.iter().map(|&x| math::canonical(x).to_bits());
    let narrow = narrow
        .iter()
        .map(|&x| u64::from(math::canonical_f32(x).to_bits()));
    digest(wide.chain(narrow))
}

/// Every digest, in the order of [`EXPECTED`].
pub fn digests() -> [(&'static str, u64); 10] {
    [
        ("sin", of_function(math::sin)),
        ("cos", of_function(math::cos)),
        ("exp", of_function(math::exp)),
        ("ln", of_function(math::ln)),
        ("log2", of_function(math::log2)),
        ("sqrt", of_function(f64::sqrt)),
        ("mul_add", of_mul_add()),
        ("exact_sum", of_exact_sum()),
        ("rng", of_rng()),
        ("canonical", of_canonical()),
    ]
}

#[test]
fn golden_vectors_full_size() {
    let found = digests();
    let show = |digests: &[(&str, u64)]| {
        let lines: Vec<String> = digests
            .iter()
            .map(|(n, d)| format!("(\"{n}\", {d:#018x}),"))
            .collect();
        lines.join("\n")
    };
    assert!(
        found == EXPECTED,
        "found:\n{}\nexpected:\n{}",
        show(&found),
        show(&EXPECTED)
    );
}
