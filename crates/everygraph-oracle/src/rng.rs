// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A small random number generator whose output is fixed forever.
//!
//! Generated graphs are identified by their seed in failure reports, golden files and the
//! corpus. A library generator (`rand`'s `StdRng`) may change its stream between releases,
//! which would silently change every graph; SplitMix64 is twelve lines and cannot.

/// SplitMix64 (Steele, Lea, Flood 2014). Not for cryptography.
#[derive(Clone, Debug)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    /// A generator seeded with `seed`.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// The next 64 random bits.
    pub const fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A uniform value in `0..bound`, without modulo bias. `bound` must be positive.
    ///
    /// # Panics
    ///
    /// If `bound` is zero.
    pub fn below(&mut self, bound: u64) -> u64 {
        assert!(bound > 0, "below(0)");
        // Rejection sampling on the largest multiple of `bound`.
        let zone = u64::MAX - u64::MAX % bound;
        loop {
            let x = self.next_u64();
            if x < zone {
                return x % bound;
            }
        }
    }

    /// A uniform `u32` in `0..bound`.
    ///
    /// # Panics
    ///
    /// If `bound` is zero.
    pub fn below_u32(&mut self, bound: u32) -> u32 {
        let x = self.below(u64::from(bound));
        u32::try_from(x).expect("below a u32 bound")
    }

    /// True with probability `permille / 1000`.
    pub fn chance(&mut self, permille: u32) -> bool {
        self.below(1000) < u64::from(permille)
    }

    /// A uniform double in `[0, 1)` with 53 random bits.
    ///
    /// # Panics
    ///
    /// Never: the halves are taken from 53 bits.
    pub fn unit(&mut self) -> f64 {
        let bits = self.next_u64() >> 11;
        // 2^-53: exact in f64.
        let x = u32::try_from(bits >> 32).expect("21 bits");
        let y = u32::try_from(bits & 0xFFFF_FFFF).expect("32 bits");
        (f64::from(x) * 4_294_967_296.0 + f64::from(y)) * (1.0 / 9_007_199_254_740_992.0)
    }

    /// A child generator: deterministic, and independent enough of the parent's stream.
    pub const fn fork(&mut self) -> Self {
        Self::new(self.next_u64())
    }
}

#[cfg(test)]
mod tests {
    use super::SplitMix64;

    #[test]
    fn stream_is_fixed() {
        // Reference values of SplitMix64 for seed 0. If this changes, every generated
        // graph, golden file and recorded failure seed changes with it.
        let mut r = SplitMix64::new(0);
        assert_eq!(r.next_u64(), 0xE220_A839_7B1D_CDAF);
        assert_eq!(r.next_u64(), 0x6E78_9E6A_A1B9_65F4);
    }

    #[test]
    fn below_is_in_range_and_unit_is_in_unit_interval() {
        let mut r = SplitMix64::new(7);
        for bound in 1..200 {
            assert!(r.below(bound) < bound);
            let u = r.unit();
            assert!((0.0..1.0).contains(&u));
        }
    }
}
