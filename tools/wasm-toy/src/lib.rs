// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A toy kernel for the wasm job: the degree of every node of a generated graph,
//! reduced to one hash.
//!
//! It goes through the public pieces of `everygraph-core` that a real kernel would use:
//! the edge list is bytes behind a [`ByteSource`], node ids are [`NodeId`], and the work
//! is split by [`par::map_ranges`] with a grain that depends on the data only, the
//! per-chunk counts folded in chunk order. Natively (feature `parallel`) the chunks run
//! on rayon; in wasm32 on the calling thread. The hash must be the same.

use std::num::NonZeroUsize;

use everygraph_core::{ByteSource, MemorySource, NodeId, par};

/// Bytes per edge in the generated edge list: two little-endian `u32` node ids.
const EDGE_BYTES: usize = 8;

/// Edges per chunk. A constant: a grain never depends on the machine.
const GRAIN: NonZeroUsize = NonZeroUsize::new(1 << 14).expect("non-zero");

/// The next value of the splitmix64 generator.
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A random edge list of `edges` edges over `nodes` nodes, as bytes.
fn generate(nodes: u32, edges: usize, seed: u64) -> Vec<u8> {
    let mut state = seed;
    let mut bytes = Vec::with_capacity(edges * EDGE_BYTES);
    for _ in 0..edges {
        let x = splitmix64(&mut state);
        let source = u32::try_from((x & 0xFFFF_FFFF) % u64::from(nodes)).expect("below nodes");
        let target = u32::try_from((x >> 32) % u64::from(nodes)).expect("below nodes");
        bytes.extend_from_slice(&source.to_le_bytes());
        bytes.extend_from_slice(&target.to_le_bytes());
    }
    bytes
}

/// FNV-1a over the degrees, each as four little-endian bytes.
fn fnv1a(degrees: &[u32]) -> u64 {
    let mut hash = 0xCBF2_9CE4_8422_2325_u64;
    for byte in degrees.iter().flat_map(|d| d.to_le_bytes()) {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01B3);
    }
    hash
}

/// The degree (out plus in) of every node of the generated graph.
///
/// # Panics
///
/// If `nodes` is zero.
#[must_use]
pub fn degrees(nodes: u32, edges: u32, seed: u64) -> Vec<u32> {
    assert!(nodes > 0, "a graph with edges needs nodes");
    let edges = usize::try_from(edges).expect("u32 fits in usize");
    let source = MemorySource::new(generate(nodes, edges, seed));
    let node_count = NodeId::new(nodes - 1).index() + 1;
    let partials = par::map_ranges(edges, GRAIN, |range| {
        let mut counts = vec![0_u32; node_count];
        let mut buf = vec![0_u8; range.len() * EDGE_BYTES];
        let offset = u64::try_from(range.start * EDGE_BYTES).expect("usize fits in u64");
        source.read_at(offset, &mut buf).expect("inside the source");
        for edge in buf.as_chunks::<EDGE_BYTES>().0 {
            for end in edge.as_chunks::<4>().0 {
                let id = NodeId::new(u32::from_le_bytes(*end));
                counts[id.index()] += 1;
            }
        }
        counts
    });
    let mut total = vec![0_u32; node_count];
    for partial in partials {
        for (sum, count) in total.iter_mut().zip(partial) {
            *sum += count;
        }
    }
    total
}

/// The hash of [`degrees`]: what the native run and the wasm32 run compare.
#[must_use]
pub fn degree_hash(nodes: u32, edges: u32, seed: u64) -> u64 {
    fnv1a(&degrees(nodes, edges, seed))
}

/// The entry point Node calls. A plain C ABI export, so that no bindings generator is
/// needed: three numbers in, one `u64` (a `BigInt` in JavaScript) out.
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn everygraph_toy_degree_hash(nodes: u32, edges: u32, seed: u64) -> u64 {
    degree_hash(nodes, edges, seed)
}
