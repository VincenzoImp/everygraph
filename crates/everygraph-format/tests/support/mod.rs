// SPDX-License-Identifier: Apache-2.0 OR MIT

//! What the format's integration tests share.

#![allow(dead_code, reason = "each test crate uses a part of it")]

use everygraph_oracle::rng::SplitMix64;
use proptest::prelude::ProptestConfig;

/// Miri interprets the tests about 1000 times slower: it runs a sample.
pub const MIRI: bool = cfg!(miri);

/// Few cases under Miri, and no failure file (Miri's isolation hides the file system).
pub fn config() -> ProptestConfig {
    ProptestConfig {
        cases: if MIRI { 2 } else { 256 },
        failure_persistence: if MIRI {
            None
        } else {
            ProptestConfig::default().failure_persistence
        },
        ..ProptestConfig::default()
    }
}

/// `len` bytes that depend only on `seed`.
pub fn bytes(seed: u64, len: usize) -> Vec<u8> {
    let mut rng = SplitMix64::new(seed);
    (0..len).map(|_| rng.next_u64().to_le_bytes()[0]).collect()
}
