// SPDX-License-Identifier: Apache-2.0 OR MIT

//! What the unit tests of this crate share.

/// A small deterministic generator, so that the tests need no dependency.
pub(crate) struct Lcg(pub(crate) u64);

impl Lcg {
    pub(crate) fn below(&mut self, bound: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        usize::try_from((self.0 >> 33) % crate::widen(bound)).unwrap()
    }
}
