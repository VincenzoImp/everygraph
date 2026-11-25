// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The hash of a key and the integer functions derived from it. Everything here is part
//! of the on-disk format (module documentation, "Hashing"): changing a constant changes
//! every dictionary.

/// The 128-bit hash of a normalized key, split in two halves.
///
/// `hi` chooses the partition, the MPHF part and the bucket; `lo` chooses the slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct KeyHash {
    pub(crate) hi: u64,
    pub(crate) lo: u64,
}

impl KeyHash {
    /// XXH3-128 of `bytes` with `seed`.
    pub(crate) fn of(bytes: &[u8], seed: u64) -> Self {
        let h = xxhash_rust::xxh3::xxh3_128_with_seed(bytes, seed);
        Self {
            hi: high_half(h),
            lo: low_half(h),
        }
    }

    /// The fine partition of the spilling dedupe: the top [`FINE_BITS`] bits of `hi`.
    pub(crate) fn fine(self) -> usize {
        top_bits(self.hi, FINE_BITS)
    }
}

/// Bits of the hash that name a fine partition of the distinct-key pass. Partitions are
/// the unit in which spilled runs are indexed; there are `2^FINE_BITS` of them.
pub(crate) const FINE_BITS: u32 = 10;
/// Number of fine partitions.
pub(crate) const FINE: usize = 1 << FINE_BITS;

/// The top `bits` bits of `x` as an index (0 when `bits` is 0).
pub(crate) fn top_bits(x: u64, bits: u32) -> usize {
    if bits == 0 {
        0
    } else {
        index(x >> (64 - bits))
    }
}

/// A value already known to be below 2^32 (or a small count) as an index.
pub(crate) fn index(x: u64) -> usize {
    usize::try_from(x).expect("an index below 2^32 fits in usize on every supported target")
}

const fn high_half(x: u128) -> u64 {
    (x >> 64) as u64
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "taking the low half of a u128 is the point"
)]
const fn low_half(x: u128) -> u64 {
    x as u64
}

/// `floor(a * b / 2^64)`: maps a uniform `a` onto `0..b`.
pub(crate) const fn mul_high(a: u64, b: u64) -> u64 {
    high_half((a as u128) * (b as u128))
}

/// The splitmix64 finalizer: a bijective mix of 64 bits.
pub(crate) const fn mix64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Odd constant of the slot function (the 64-bit golden ratio).
pub(crate) const SLOT_MULTIPLIER: u64 = 0x9E37_79B9_7F4A_7C15;

/// The skewed bucket function of `PtrHash` ("cubic-eps"): `x³(1+x)/2` scaled by
/// 255/256, plus `x/256`, on 64-bit fixed point. Larger buckets come first, which is what
/// lets the pilot search place them while the table is still empty.
pub(crate) const fn skew(x: u64) -> u64 {
    mul_high(mul_high(x, x), (x >> 1) | (1 << 63)) / 256 * 255 + x / 256
}

/// The bucket of a key in a part with `buckets` buckets and the part's `seed`.
pub(crate) const fn bucket(hi: u64, seed: u32, buckets: u32) -> u64 {
    let x = mix64(hi ^ (seed as u64).wrapping_mul(0xD6E8_FEB8_6659_FD93));
    mul_high(skew(x), buckets as u64)
}

/// What a pilot contributes to the slot of a key, for a part's `seed`.
pub(crate) const fn pilot_hash(pilot: u8, seed: u32) -> u64 {
    mix64((pilot as u64) | ((seed as u64) << 8))
}

/// The slot of a key inside a part with `slots` slots, for a pilot's hash.
pub(crate) const fn slot(lo: u64, pilot_hash: u64, slots: u32) -> u64 {
    mul_high(
        (lo ^ pilot_hash).wrapping_mul(SLOT_MULTIPLIER),
        slots as u64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The hash is XXH3-128 of the specification: two published test vectors.
    #[test]
    fn the_hash_is_xxh3_128() {
        // XXH3_128bits of the empty input with seed 0 (xxHash test suite).
        let empty = KeyHash::of(b"", 0);
        assert_eq!(empty.hi, 0x99AA_06D3_0147_98D8);
        assert_eq!(empty.lo, 0x6001_C324_468D_497F);
        // Different seeds give different hashes.
        assert_ne!(KeyHash::of(b"abc", 0), KeyHash::of(b"abc", 1));
    }

    #[test]
    fn skew_is_monotone_and_covers_the_range() {
        let mut last = 0;
        for i in 0..=1000_u64 {
            let x = i.wrapping_mul(u64::MAX / 1000);
            let y = skew(x);
            assert!(y >= last);
            last = y;
        }
        assert!(skew(u64::MAX) > u64::MAX - (u64::MAX >> 7));
        assert_eq!(skew(0), 0);
    }

    #[test]
    fn buckets_and_slots_stay_in_range() {
        for i in 0..10_000_u64 {
            let h = KeyHash::of(&i.to_le_bytes(), 0);
            assert!(bucket(h.hi, 3, 7) < 7);
            assert!(slot(h.lo, pilot_hash(200, 3), 101) < 101);
        }
        assert_eq!(top_bits(u64::MAX, 0), 0);
        assert_eq!(top_bits(u64::MAX, FINE_BITS), FINE - 1);
    }
}
