// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Counter-based random bits for the kernels' sampling: a draw is a function of its key
//! (a seed, a purpose and the node or edge it is for), never of a thread or of the
//! order of the draws (`docs/research/sota-kernels.md`, challenge 10). The kernels that
//! sample here give the same answer whatever the bits; only their work depends on them.

/// 64 bits for (`seed`, `purpose`, `counter`): `SplitMix64`'s finalizer (Steele, Lea and
/// Flood, OOPSLA 2014) applied to each word of the key in turn.
#[must_use]
pub(crate) fn draw(seed: u64, purpose: [u8; 4], counter: [u32; 3]) -> u64 {
    let [c0, c1, c2] = counter;
    let words = [
        u64::from(u32::from_le_bytes(purpose)) << 32 | u64::from(c0),
        u64::from(c1) << 32 | u64::from(c2),
    ];
    words.iter().fold(mix(seed), |h, &w| mix(h ^ w))
}

fn mix(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// An integer in `0..n` from `bits`, by multiply and shift (Lemire, ACM TOMACS 2019).
#[must_use]
pub(crate) fn below(bits: u64, n: u64) -> u64 {
    ((u128::from(bits) * u128::from(n)) >> 64) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_depend_on_every_word_of_the_key() {
        let base = draw(1, *b"test", [2, 3, 4]);
        assert_eq!(base, draw(1, *b"test", [2, 3, 4]));
        for other in [
            draw(0, *b"test", [2, 3, 4]),
            draw(1, *b"tess", [2, 3, 4]),
            draw(1, *b"test", [3, 3, 4]),
            draw(1, *b"test", [2, 2, 4]),
            draw(1, *b"test", [2, 3, 5]),
        ] {
            assert_ne!(base, other);
        }
    }

    #[test]
    fn below_stays_below_and_covers_the_range() {
        let mut seen = [0_u32; 7];
        for i in 0..7000 {
            let x = below(draw(9, *b"test", [i, 0, 0]), 7);
            seen[usize::try_from(x).unwrap()] += 1;
        }
        assert!(seen.iter().all(|&c| (800..1200).contains(&c)), "{seen:?}");
        assert_eq!(below(u64::MAX, 0), 0);
    }
}
