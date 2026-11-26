// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic section contents, shared by `tests/container.rs` and the example
//! `container_sample` (included with `#[path]`).

use everygraph_oracle::rng::SplitMix64;

/// `len` bytes that depend only on `seed`.
pub fn bytes(seed: u64, len: usize) -> Vec<u8> {
    let mut rng = SplitMix64::new(seed);
    (0..len).map(|_| rng.next_u64().to_le_bytes()[0]).collect()
}
