// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The encoder: a CSR in memory to the two parts of a compressed adjacency.

use std::num::NonZeroUsize;

use everygraph_core::{EdgeOffset, par};

use super::ef::EliasFano;
use super::index::AdjacencyIndex;
use super::{EncodeError, svb};

/// A block closes before a list that would take it past this many entries...
pub(crate) const BLOCK_ENTRIES: u64 = 1 << 16;
/// ...or when it holds this many nodes (runs of isolated nodes).
pub(crate) const BLOCK_NODES: usize = 1 << 16;
/// Nodes per encoding task.
const GRAIN: NonZeroUsize = NonZeroUsize::new(1 << 14).expect("non-zero");

/// The two parts of a compressed adjacency, as [`encode`] writes them. They are stored
/// as two sections of a `.eg` file, or anywhere else; the reader takes them back as
/// byte slices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Encoded {
    /// The index: header, block directory, Elias–Fano byte and entry offsets. Small
    /// (0.2–0.8 bits per entry on the lab's social graphs); read whole at open.
    pub index: Vec<u8>,
    /// The records, one per node with a non-empty list, in node order.
    pub data: Vec<u8>,
}

/// A node index below `n`, which `encode` checked to fit `u32`.
fn node_id(v: usize) -> u32 {
    u32::try_from(v).expect("v <= n <= u32::MAX")
}

/// `len` zeros, or an error instead of an abort when the memory is not there.
fn zeroed<T: Copy + Default>(len: usize) -> Result<Vec<T>, EncodeError> {
    let mut v = Vec::new();
    let oom = || EncodeError::OutOfMemory {
        bytes: u64::try_from(len.saturating_mul(size_of::<T>())).unwrap_or(u64::MAX),
    };
    v.try_reserve_exact(len).map_err(|_| oom())?;
    v.resize(len, T::default());
    Ok(v)
}

fn to_index(offset: u64) -> usize {
    EdgeOffset::new(offset)
        .to_index()
        .expect("checked: offsets end at targets.len()")
}

/// Encodes a CSR: `offsets` has `n + 1` entries, from 0 to `targets.len()`, and the
/// list of node `v` is `targets[offsets[v]..offsets[v + 1]]`, strictly increasing.
///
/// Targets are any `u32`; the codec does not require them to be below `n`. Two passes
/// over the input (record lengths, then records written in place), in parallel over
/// chunks of nodes; memory beyond the output is `8 (n + 1)` bytes.
///
/// # Errors
///
/// [`EncodeError::Offsets`] if `offsets` is not a valid CSR offset array for `targets`
/// with at most `2^32 − 1` nodes, [`EncodeError::NotIncreasing`] at the first list
/// that is not strictly increasing, and [`EncodeError::OutOfMemory`] if the record
/// lengths (`8 (n + 1)` bytes) or the data part cannot be allocated.
///
/// The data part is one allocation, which DESIGN D2 (chunks of at most 2 GiB, budgeted
/// allocations) does not allow for large graphs: they use the streaming encoder
/// ([`append_record`](super::append_record), [`IndexBuilder`](super::IndexBuilder)),
/// which writes the same bytes list by list.
pub fn encode(offsets: &[u64], targets: &[u32]) -> Result<Encoded, EncodeError> {
    let Some((&last, _)) = offsets.split_last() else {
        return Err(EncodeError::Offsets(
            "no offsets: a graph with n nodes has n + 1",
        ));
    };
    let n = offsets.len() - 1;
    let nodes = u32::try_from(n).map_err(|_| EncodeError::Offsets("more than 2^32 - 1 nodes"))?;
    if offsets[0] != 0 || usize::try_from(last).ok() != Some(targets.len()) {
        return Err(EncodeError::Offsets(
            "offsets must run from 0 to targets.len()",
        ));
    }
    if offsets
        .windows(2)
        .any(|w| w[0] > w[1] || w[1] - w[0] > u64::from(u32::MAX))
    {
        return Err(EncodeError::Offsets(
            "offsets decrease, or a list has 2^32 entries or more",
        ));
    }
    let list = |v: usize| &targets[to_index(offsets[v])..to_index(offsets[v + 1])];

    // Pass 1: record lengths, u64::MAX marking a list that is not increasing.
    let mut byte_offsets = zeroed::<u64>(n + 1)?;
    par::for_each_chunk_mut(&mut byte_offsets[1..], GRAIN, |start, lens| {
        for (i, len) in lens.iter_mut().enumerate() {
            let v = start + i;
            *len = svb::record_len(node_id(v), list(v)).map_or(u64::MAX, |l| l as u64);
        }
    });
    if let Some(v) = byte_offsets[1..].iter().position(|&l| l == u64::MAX) {
        return Err(EncodeError::NotIncreasing { node: node_id(v) });
    }
    for v in 0..n {
        byte_offsets[v + 1] += byte_offsets[v];
    }

    // Pass 2: records written into disjoint parts of the output, chunk by chunk.
    let mut data = zeroed::<u8>(to_index(byte_offsets[n]))?;
    let mut pieces = Vec::with_capacity(n.div_ceil(GRAIN.get()));
    let mut rest = data.as_mut_slice();
    for start in (0..n).step_by(GRAIN.get()) {
        let end = (start + GRAIN.get()).min(n);
        let (piece, tail) = rest.split_at_mut(to_index(byte_offsets[end] - byte_offsets[start]));
        pieces.push((start..end, piece));
        rest = tail;
    }
    par::for_each_chunk_mut(&mut pieces, NonZeroUsize::MIN, |_, chunk| {
        for (nodes, out) in chunk.iter_mut() {
            let base = byte_offsets[nodes.start];
            for v in nodes.clone() {
                let at = to_index(byte_offsets[v] - base)..to_index(byte_offsets[v + 1] - base);
                if !at.is_empty() {
                    svb::write_record(node_id(v), list(v), &mut out[at]);
                }
            }
        }
    });

    let index = AdjacencyIndex::from_parts(
        false,
        nodes,
        last,
        byte_offsets[n],
        block_starts(n, offsets.windows(2).map(|w| w[1] - w[0])),
        EliasFano::new(&byte_offsets),
        EliasFano::new(offsets),
    );
    Ok(Encoded {
        index: index.write(),
        data,
    })
}

/// The block directory of `n` lists of these degrees: greedy, node-aligned blocks of at
/// most [`BLOCK_ENTRIES`] entries and [`BLOCK_NODES`] nodes; a longer list gets a block
/// of its own.
pub(crate) fn block_starts(n: usize, degrees: impl Iterator<Item = u64>) -> Vec<u32> {
    let mut starts = vec![0];
    let (mut first, mut in_block) = (0, 0);
    for (v, degree) in degrees.enumerate() {
        if v > first && (in_block + degree > BLOCK_ENTRIES || v - first == BLOCK_NODES) {
            starts.push(node_id(v));
            (first, in_block) = (v, 0);
        }
        in_block += degree;
    }
    if n > 0 {
        starts.push(node_id(n));
    }
    starts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_hold_whole_lists_and_a_long_list_alone() {
        let b = BLOCK_ENTRIES;
        // Degrees: 10, b (alone), 1, 1, b + 5 (alone), 0.
        let degrees = [10, b, 1, 1, b + 5, 0];
        let mut offsets = vec![0u64];
        for d in degrees {
            offsets.push(offsets.last().unwrap() + d);
        }
        let starts = |offsets: &[u64]| {
            block_starts(offsets.len() - 1, offsets.windows(2).map(|w| w[1] - w[0]))
        };
        assert_eq!(starts(&offsets), vec![0, 1, 2, 4, 5, 6]);
        // Isolated nodes: blocks of BLOCK_NODES nodes.
        let offsets = vec![0u64; 2 * BLOCK_NODES + 2];
        assert_eq!(starts(&offsets), vec![0, 1 << 16, 2 << 16, (2 << 16) + 1]);
        assert_eq!(starts(&[0]), vec![0]);
    }

    #[test]
    fn invalid_input_is_refused() {
        assert!(matches!(encode(&[], &[]), Err(EncodeError::Offsets(_))));
        assert!(matches!(encode(&[1], &[0]), Err(EncodeError::Offsets(_))));
        assert!(matches!(
            encode(&[0, 2, 1], &[0, 1]),
            Err(EncodeError::Offsets(_))
        ));
        assert!(matches!(
            encode(&[0, 1], &[0, 1]),
            Err(EncodeError::Offsets(_))
        ));
        assert_eq!(
            encode(&[0, 1, 3], &[5, 2, 2]),
            Err(EncodeError::NotIncreasing { node: 1 })
        );
    }
}
