// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Golden vectors of the portable numerics (rules R15c and R15d of
//! `docs/research/sota-numerics.md`): each function of `everygraph_core::math` on
//! 100,000 inputs, 100,000 draws of `everygraph_core::rng`, and NaN and −0.0 made at
//! run time, each reduced to one digest and compared with a frozen value.
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

use everygraph_core::{math, rng};

/// Inputs per function and draws of the generator.
const COUNT: u32 = 100_000;

/// The frozen digests, computed on x86-64.
pub const EXPECTED: [(&str, u64); 7] = [
    ("sin", 0x9809_C6C7_94A0_2BDD),
    ("cos", 0x4F55_77B7_16BD_F310),
    ("exp", 0xD582_8CC7_8D48_D1B7),
    ("ln", 0xD4F2_2E1E_585B_171C),
    ("log2", 0x429E_FCA0_2886_0B30),
    ("rng", 0x44D8_F08C_6849_92DD),
    ("canonical", 0xCEDC_C921_8CFF_6465),
];

/// Input `i`: a quarter each of values in [−800, 800) (the useful range of `exp`), of
/// either sign with magnitudes 2^−40 to 2^40, of any bits (subnormals, infinities and
/// NaNs included), and of any positive bits (large arguments of `sin` and `cos`).
fn input(i: u32) -> f64 {
    let bits = rng::draw(0, *b"GOLD", [i, 0, 0]);
    match i % 4 {
        0 => (rng::unit_f64(bits) - 0.5) * 1600.0,
        1 => {
            let exponent = 1023 - 40 + rng::below(bits >> 52, 81);
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

fn of_function(f: fn(f64) -> f64) -> u64 {
    digest((0..COUNT).map(|i| math::canonical(f(input(i))).to_bits()))
}

/// Draws whose keys vary every part of the key.
fn of_rng() -> u64 {
    digest((0..COUNT).map(|i| {
        let seed = u64::from(i).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        rng::draw(seed, i.to_le_bytes(), [i, i ^ 0xFFFF, i.rotate_left(16)])
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
pub fn digests() -> [(&'static str, u64); 7] {
    [
        ("sin", of_function(math::sin)),
        ("cos", of_function(math::cos)),
        ("exp", of_function(math::exp)),
        ("ln", of_function(math::ln)),
        ("log2", of_function(math::log2)),
        ("rng", of_rng()),
        ("canonical", of_canonical()),
    ]
}

#[test]
fn golden_vectors() {
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
