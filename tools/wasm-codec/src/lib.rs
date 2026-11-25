// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The wasm codec check (review of #30, S3): every decoder of `everygraph-format` the
//! platform has is compared with the scalar one, on every reading API, over generated
//! graphs — lists of any length (one longer than a block), targets anywhere in `u32` —
//! and over damaged copies of their data. In wasm32 built with `simd128` the second
//! decoder is the `simd128` one, which nothing else runs.
//!
//! [`check`] returns the number of decoders, the number of disagreements with the
//! scalar decoder, and a hash of what the scalar decoder returned; the native run and
//! the wasm32 run must print the same line (the decoder count included: 2 natively on
//! x86-64 with SSSE3 or on AArch64, 2 in wasm32 with `simd128`).

use everygraph_core::NodeId;
use everygraph_format::adjacency::{
    Adjacency, AdjacencyIndex, Batch, DecodeError, Decoder, encode,
};

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A CSR of `nodes` sorted lists: mostly short, a few hubs, one longer than a block;
/// targets near the node, anywhere, or near `u32::MAX`.
fn generate(nodes: u32, state: &mut u64) -> (Vec<u64>, Vec<u32>) {
    let mut offsets = vec![0u64];
    let mut targets = Vec::new();
    for v in 0..nodes {
        let r = splitmix64(state);
        let degree = match r % 100 {
            0 if v == nodes / 2 => 70_000,
            0..=2 => 200 + r % 2000,
            3..=30 => 0,
            _ => r % 40,
        };
        let mut x = match (r >> 8) % 3 {
            0 => v.saturating_sub(100),
            1 => u32::try_from(splitmix64(state) >> 33).unwrap_or(0),
            _ => u32::MAX - 1_000_000,
        };
        for _ in 0..degree {
            let step =
                1 + u32::try_from(splitmix64(state) % (1 << (splitmix64(state) % 26))).unwrap_or(0);
            let Some(next) = x.checked_add(step) else {
                break;
            };
            x = next;
            targets.push(x);
        }
        offsets.push(targets.len() as u64);
    }
    (offsets, targets)
}

type Outcome = Result<Vec<u32>, DecodeError>;

fn outcomes(adj: Adjacency<'_>) -> Vec<Outcome> {
    let index = adj.index();
    let mut out = Vec::new();
    let mut buf = Vec::new();
    for v in 0..index.nodes() {
        let id = NodeId::new(v);
        out.push(adj.neighbors(id, &mut buf).map(<[u32]>::to_vec));
        for k in [1, 3, 5] {
            out.push(adj.neighbors_prefix(id, k, &mut buf).map(<[u32]>::to_vec));
        }
    }
    let mut batch = Batch::default();
    for b in 0..index.blocks() {
        let nodes = index.block_nodes(b);
        for budget in [7, usize::MAX] {
            let mut at = nodes.start;
            while at < nodes.end {
                match adj.decode_nodes(at..nodes.end, budget, &mut batch) {
                    Ok(next) => {
                        out.push(Ok(batch.targets().to_vec()));
                        at = next;
                    }
                    Err(e) => {
                        out.push(Err(e));
                        break;
                    }
                }
            }
        }
    }
    let mut offsets = vec![0; index.nodes() as usize + 1];
    let mut targets = vec![0; usize::try_from(index.entries()).expect("small graphs")];
    out.push(adj.decode_all(&mut offsets, &mut targets).map(|()| targets));
    out
}

fn fnv1a(hash: &mut u64, words: impl IntoIterator<Item = u32>) {
    for w in words {
        for byte in w.to_le_bytes() {
            *hash = (*hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01B3);
        }
    }
}

/// What the check found.
#[derive(Debug, PartialEq, Eq)]
pub struct Report {
    /// Decoders this build can run (the scalar one included).
    pub decoders: u32,
    /// Outcomes of a non-scalar decoder that differ from the scalar decoder's.
    pub mismatches: u32,
    /// FNV-1a of the scalar outcomes (each list's words; errors as their node).
    pub hash: u64,
}

/// Generates `graphs` graphs of `nodes` nodes from `seed`, and checks each clean and
/// with a few damaged data bytes.
///
/// # Panics
///
/// If a generated graph does not encode, which would be a bug of the generator.
#[must_use]
pub fn check(seed: u64, graphs: u32, nodes: u32) -> Report {
    let mut state = seed;
    let decoders = Decoder::available();
    let mut report = Report {
        decoders: u32::try_from(decoders.len()).unwrap_or(u32::MAX),
        mismatches: 0,
        hash: 0xCBF2_9CE4_8422_2325,
    };
    for _ in 0..graphs {
        let (offsets, targets) = generate(nodes, &mut state);
        let encoded = encode(&offsets, &targets).expect("a valid CSR");
        let index = AdjacencyIndex::parse(&encoded.index).expect("an index the encoder wrote");
        let mut damaged = encoded.data.clone();
        for _ in 0..4 {
            if !damaged.is_empty() {
                let i = usize::try_from(splitmix64(&mut state) % damaged.len() as u64).unwrap_or(0);
                damaged[i] ^= 1 << (splitmix64(&mut state) % 8);
            }
        }
        for data in [&encoded.data, &damaged] {
            let adj = Adjacency::new(&index, data).expect("the data the index describes");
            let reference = outcomes(adj.with_decoder(Decoder::SCALAR));
            for o in &reference {
                match o {
                    Ok(words) => fnv1a(&mut report.hash, words.iter().copied()),
                    Err(e) => fnv1a(&mut report.hash, [u32::MAX, e.first]),
                }
            }
            for &d in &decoders[1..] {
                let got = outcomes(adj.with_decoder(d));
                let differ = got.iter().zip(&reference).filter(|(a, b)| a != b).count()
                    + got.len().abs_diff(reference.len());
                report.mismatches += u32::try_from(differ).unwrap_or(u32::MAX);
            }
        }
    }
    report
}

/// The entry point Node calls: a plain C ABI export, so that no bindings generator is
/// needed. Returns the hash; the two counts come from the exports below, which repeat
/// the run (it is deterministic and short).
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn everygraph_codec_hash(seed: u64, graphs: u32, nodes: u32) -> u64 {
    check(seed, graphs, nodes).hash
}

/// Decoders available, and mismatches, packed as `decoders << 32 | mismatches`.
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn everygraph_codec_counts(seed: u64, graphs: u32, nodes: u32) -> u64 {
    let r = check(seed, graphs, nodes);
    u64::from(r.decoders) << 32 | u64::from(r.mismatches)
}
