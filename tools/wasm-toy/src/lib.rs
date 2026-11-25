// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A toy kernel for the wasm job: the degree of every node of a generated graph,
//! reduced to one hash.
//!
//! It goes through the public pieces of `everygraph-core` that a real kernel would use:
//! the edge list is bytes behind a [`ByteSource`], node ids are [`NodeId`], and the work
//! is split by [`par::map_ranges`] with a grain that depends on the data only, the
//! per-chunk counts folded in chunk order. Natively (feature `parallel`) the chunks run
//! on rayon; in wasm32 on the calling thread. The hash must be the same.
//!
//! [`kernels_hash`] runs the proof kernels of the edge-map framework (#44) — `PageRank`
//! with f64 state, WCC, BFS — on the same graph made symmetric, over the raw and the
//! compressed source, and hashes their results: the same kernels must compile to wasm32
//! and give the native bytes.

use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

use everygraph_core::{ByteSource, Memory, MemorySource, NodeId, par};
use everygraph_format::adjacency::{Adjacency, AdjacencyIndex, encode};
use everygraph_kernels::edge_map::{Compressed, Csr, Plan, Settings, Source};
use everygraph_kernels::proof;

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

/// FNV-1a over 64-bit words.
fn fnv1a64(words: impl Iterator<Item = u64>) -> u64 {
    let mut hash = 0xCBF2_9CE4_8422_2325_u64;
    for byte in words.flat_map(u64::to_le_bytes) {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01B3);
    }
    hash
}

/// The proof kernels on one source: the hash of the `PageRank` bits, the component
/// labels and the BFS depths from node 0. Small tasks and batches, so that the graph
/// is cut into many of each.
fn kernels_on<S: Source>(source: &S) -> u64 {
    let memory = Memory::unlimited();
    let settings = Settings {
        chunk_entries: 1 << 12,
        batch_entries: 1 << 10,
        ..Settings::default()
    };
    Plan::with(source, &memory, settings, |plan| {
        let rank = proof::pagerank::<_, f64>(&plan, &memory, 0.85, 20).expect("pagerank");
        let (labels, _) = proof::wcc(&plan, &memory).expect("wcc");
        let (depth, _) = proof::bfs(&plan, &memory, 0, true).expect("bfs");
        let load = |a: &AtomicU32| u64::from(a.load(Relaxed));
        let words = rank.iter().map(|r| r.to_bits());
        fnv1a64(words.chain(labels.iter().map(load)).chain(depth.iter().map(load)))
    })
}

/// The hash of the proof kernels' results on the generated graph, made symmetric and
/// without loops, over the raw and the compressed source; 0 if the two sources disagree.
///
/// # Panics
///
/// If `nodes` is zero.
#[must_use]
pub fn kernels_hash(nodes: u32, edges: u32, seed: u64) -> u64 {
    assert!(nodes > 0, "a graph with edges needs nodes");
    let edges = usize::try_from(edges).expect("u32 fits in usize");
    let bytes = generate(nodes, edges, seed);
    let mut pairs: Vec<(u32, u32)> = bytes
        .as_chunks::<EDGE_BYTES>()
        .0
        .iter()
        .flat_map(|e| {
            let (u, v) = e.split_at(4);
            let u = u32::from_le_bytes(u.try_into().expect("four bytes"));
            let v = u32::from_le_bytes(v.try_into().expect("four bytes"));
            [(u, v), (v, u)]
        })
        .filter(|(u, v)| u != v)
        .collect();
    pairs.sort_unstable();
    pairs.dedup();
    let n = NodeId::new(nodes - 1).index() + 1;
    let mut offsets = vec![0_u64; n + 1];
    for &(u, _) in &pairs {
        offsets[NodeId::new(u).index() + 1] += 1;
    }
    for v in 0..n {
        offsets[v + 1] += offsets[v];
    }
    let targets: Vec<u32> = pairs.iter().map(|p| p.1).collect();
    let raw = kernels_on(&Csr::new(&offsets, &targets).expect("a CSR"));
    let encoded = encode(&offsets, &targets).expect("encode");
    let index = AdjacencyIndex::parse(&encoded.index).expect("an index");
    let adjacency = Adjacency::new(&index, &encoded.data).expect("the data part");
    let compressed = kernels_on(&Compressed::new(adjacency));
    if raw == compressed { raw } else { 0 }
}

/// The entry point Node calls for [`kernels_hash`].
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn everygraph_toy_kernels_hash(nodes: u32, edges: u32, seed: u64) -> u64 {
    kernels_hash(nodes, edges, seed)
}

/// The entry point Node calls. A plain C ABI export, so that no bindings generator is
/// needed: three numbers in, one `u64` (a `BigInt` in JavaScript) out.
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn everygraph_toy_degree_hash(nodes: u32, edges: u32, seed: u64) -> u64 {
    degree_hash(nodes, edges, seed)
}
