// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Random numbers that do not depend on threads, chunks, budgets or the platform.
//!
//! [`draw`] is a counter-based generator: Philox4×32-10 (Salmon, Moraes, Dror, Shaw,
//! SC 2011), a pure function of a key and a counter, with no state and no sequence.
//! The same draw is computed wherever and whenever it is needed, so a result that uses
//! random numbers is the same at any thread count, in any execution regime and on
//! every platform (DESIGN D14, D15; rule R9 of `docs/research/sota-numerics.md`). It
//! uses only 32-bit integer multiplications, so its bits are the same everywhere.
//!
//! # The key schema
//!
//! Every draw is named by what it is for, never by where or when it is computed:
//!
//! - `seed`: the seed the user gave the operation (the provenance parameter `seed`).
//! - `purpose`: a [`Purpose`], one per use. Every purpose is a constant of [`Purpose`],
//!   listed in [`Purpose::ALL`], and a test checks that no two are equal; other crates
//!   cannot make one. A new use adds its constant there, with what its counter holds.
//! - `counter`: four words, `[entity, level, round, draw]`: the entity the draw belongs
//!   to (a node, a community, a sample index), the level, the iteration or round, and
//!   the index of the draw when one entity needs several. Unused words are 0. A 64-bit
//!   entity (an edge offset, a sample index of 2^32 or more) takes words 0 and 1, low
//!   word first: `[low, high, round, draw]`. A use that needs a level as well has one
//!   purpose per level instead.
//!
//! The Philox key is `seed XOR (tag · 0x9E37_79B9_7F4A_7C15)`, the tag read as a
//! little-endian `u32`. The multiplier is odd, so for one seed two purposes never share
//! a key, and the four counter words are left to the use.
//!
//! A thread id, a chunk index, a memory budget, an execution regime, a position in a
//! buffer or a count of earlier draws must never enter the key: each of those changes
//! with the schedule or the machine, and the draws would change with it. For the same
//! reason no generator here has state that advances.
//!
//! # From bits to numbers
//!
//! [`unit_f64`] turns a draw into a float in `[0, 1)` and [`below`] into an integer in
//! `0..n`. Both work on `u64`, never `usize`, whose width differs between targets. Do
//! not use `rand` distributions: their algorithms are not promised to stay the same
//! across versions and platforms (rand book, "Reproducibility").

/// Multipliers and key increments of Philox4×32 (Salmon et al. 2011).
const M0: u32 = 0xD251_1F53;
const M1: u32 = 0xCD9E_8D57;
const W0: u32 = 0x9E37_79B9;
const W1: u32 = 0xBB67_AE85;

/// The low 32 bits of `x`.
#[expect(
    clippy::cast_possible_truncation,
    reason = "dropping the high bits is the point"
)]
const fn low(x: u64) -> u32 {
    x as u32
}

/// The high and low words of `a · b`.
fn mul_hi_lo(a: u32, b: u32) -> (u32, u32) {
    let product = u64::from(a) * u64::from(b);
    (low(product >> 32), low(product))
}

/// Philox4×32 with 10 rounds: the block of four words for `counter` under `key`.
fn philox4x32_10(counter: [u32; 4], key: [u32; 2]) -> [u32; 4] {
    let (mut c, mut k) = (counter, key);
    for round in 0..10 {
        if round > 0 {
            k[0] = k[0].wrapping_add(W0);
            k[1] = k[1].wrapping_add(W1);
        }
        let (hi0, lo0) = mul_hi_lo(M0, c[0]);
        let (hi1, lo1) = mul_hi_lo(M1, c[2]);
        c = [hi1 ^ c[1] ^ k[0], lo1, hi0 ^ c[3] ^ k[1], lo0];
    }
    c
}

/// What a draw is for: the second part of the key. See the key schema above.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Purpose([u8; 4]);

impl Purpose {
    /// Tests and golden vectors of the core. Counter: `[index, stream, 0, 0]`.
    pub const TEST: Self = Self(*b"TEST");

    /// Components: the sampled neighbour of a node. Counter: `[node, 0, 0, 0]`.
    pub const COMPONENTS_NEIGHBOUR: Self = Self(*b"wcck");

    /// Components: the nodes whose labels find the giant component. Counter:
    /// `[sample index, 0, 0, 0]`.
    pub const COMPONENTS_GIANT: Self = Self(*b"wccg");

    /// Core numbers: whether peeling a node takes a sample of its neighbour in sample
    /// mode. Counter: `[neighbour, peeled node, 0, 0]`.
    pub const CORE_SAMPLE: Self = Self(*b"kcor");

    /// The block order of a sweep of Leiden's fast mode: the sort key of a block of 64
    /// nodes. Counter: `[block, level, iteration, stage]`.
    pub const LEIDEN_ORDER: Self = Self(*b"LORD");

    /// The sub-round of a group of nodes in the local moving of Leiden's deterministic
    /// mode. Counter: `[group, level, iteration, stage]`.
    pub const LEIDEN_ROUND: Self = Self(*b"LRND");

    /// The sub-round of a node in the refinement of Leiden's deterministic mode.
    /// Counter: `[node, level, iteration, stage]`.
    pub const LEIDEN_REFINE_ROUND: Self = Self(*b"LRRN");

    /// The order among a node's candidate communities of equal gain in Leiden. Counter:
    /// `[node, level, iteration, stage]`.
    pub const LEIDEN_TIE: Self = Self(*b"LTIE");

    /// The choice of a node's community in Leiden's randomized refinement. Counter:
    /// `[node, level, iteration, stage]`.
    pub const LEIDEN_REFINE: Self = Self(*b"LREF");

    /// Every purpose, so that the test below can check that they are distinct.
    pub const ALL: [Self; 9] = [
        Self::TEST,
        Self::COMPONENTS_NEIGHBOUR,
        Self::COMPONENTS_GIANT,
        Self::CORE_SAMPLE,
        Self::LEIDEN_ORDER,
        Self::LEIDEN_ROUND,
        Self::LEIDEN_REFINE_ROUND,
        Self::LEIDEN_TIE,
        Self::LEIDEN_REFINE,
    ];
}

/// 64 random bits for (`seed`, `purpose`, `counter`). See the key schema above.
#[must_use]
pub fn draw(seed: u64, purpose: Purpose, counter: [u32; 4]) -> u64 {
    let tag = u64::from(u32::from_le_bytes(purpose.0));
    let key = seed ^ tag.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let block = philox4x32_10(counter, [low(key), low(key >> 32)]);
    u64::from(block[0]) | (u64::from(block[1]) << 32)
}

/// A float in `[0, 1)` from the top 53 bits of `bits`: every multiple of 2^-53 is
/// equally likely.
#[must_use]
pub fn unit_f64(bits: u64) -> f64 {
    const SCALE: f64 = 1.0 / (1_u64 << 53) as f64;
    (bits >> 11) as f64 * SCALE
}

/// An integer in `0..n` from `bits`, by multiply and shift (Lemire, ACM TOMACS 2019).
/// The probability of each value differs from 1 / n by less than 2^-64. Returns 0 when
/// `n` is 0.
#[must_use]
pub fn below(bits: u64, n: u64) -> u64 {
    ((u128::from(bits) * u128::from(n)) >> 64) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The known-answer vectors of Philox4×32-10 shipped with Random123 (the
    /// reference implementation by the authors of the paper, file `kat_vectors`).
    #[test]
    fn known_answers() {
        let cases = [
            (
                [0; 4],
                [0; 2],
                [0x6627_E8D5, 0xE169_C58D, 0xBC57_AC4C, 0x9B00_DBD8],
            ),
            (
                [u32::MAX; 4],
                [u32::MAX; 2],
                [0x408F_276D, 0x41C8_3B0E, 0xA20B_C7C6, 0x6D54_51FD],
            ),
            (
                [0x243F_6A88, 0x85A3_08D3, 0x1319_8A2E, 0x0370_7344],
                [0xA409_3822, 0x299F_31D0],
                [0xD16C_FE09, 0x94FD_CCEB, 0x5001_E420, 0x2412_6EA1],
            ),
        ];
        for (counter, key, expected) in cases {
            assert_eq!(philox4x32_10(counter, key), expected);
        }
    }

    #[test]
    fn purposes_are_distinct() {
        for (i, a) in Purpose::ALL.iter().enumerate() {
            assert!(Purpose::ALL[i + 1..].iter().all(|b| a != b), "{a:?} twice");
        }
    }

    #[test]
    fn every_part_of_the_key_changes_the_draw() {
        let other = Purpose(*b"TESU");
        let base = draw(1, Purpose::TEST, [2, 3, 4, 5]);
        assert_ne!(base, draw(2, Purpose::TEST, [2, 3, 4, 5]));
        assert_ne!(base, draw(1 << 32 | 1, Purpose::TEST, [2, 3, 4, 5]));
        assert_ne!(base, draw(1, other, [2, 3, 4, 5]));
        assert_ne!(base, draw(1, Purpose::TEST, [3, 3, 4, 5]));
        assert_ne!(base, draw(1, Purpose::TEST, [2, 4, 4, 5]));
        assert_ne!(base, draw(1, Purpose::TEST, [2, 3, 5, 5]));
        assert_ne!(base, draw(1, Purpose::TEST, [2, 3, 4, 6]));
        assert_eq!(base, draw(1, Purpose::TEST, [2, 3, 4, 5]));
    }

    #[test]
    fn conversions_stay_in_range() {
        assert_eq!(unit_f64(0), 0.0);
        assert_eq!(unit_f64(u64::MAX), 1.0 - f64::EPSILON / 2.0);
        assert_eq!(below(u64::MAX, 10), 9);
        assert_eq!(below(0, 10), 0);
        assert_eq!(below(1 << 63, 10), 5);
        assert_eq!(below(u64::MAX, 0), 0);
        let mut counts = [0_u32; 6];
        for i in 0..6_000 {
            let k = below(draw(9, Purpose::TEST, [i, 0, 0, 0]), 6);
            counts[usize::try_from(k).expect("below 6")] += 1;
        }
        // 6,000 draws over 6 values: 1,000 each, standard deviation about 29.
        assert!(
            counts.iter().all(|&c| c.abs_diff(1_000) < 150),
            "{counts:?}"
        );
    }
}
