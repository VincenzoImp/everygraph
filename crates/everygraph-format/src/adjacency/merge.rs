// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The union of two adjacencies over the same nodes, list by list: the undirected
//! projection of a directed graph, from its out- and in-adjacency
//! (`docs/spec/graph-semantics.md`, "undirected projection").

use std::num::NonZeroUsize;

use everygraph_core::{Memory, MemoryError, MemoryKind, Reservation, par};

use super::decode::{Adjacency, Batch};
use super::ef::EliasFano;
use super::encode::{Encoded, block_starts};
use super::index::AdjacencyIndex;
use super::{EncodeError, svb};

/// Nodes per task.
const GRAIN: NonZeroUsize = NonZeroUsize::new(1 << 12).expect("non-zero");

const PURPOSE: &str = "merged adjacency";

/// The adjacency whose list of `v` is the union of the lists of `v` in `a` and in `b`,
/// encoded in memory, with the reservation that holds it. Two passes in parallel over
/// chunks of nodes, each decoding both lists of every node: the record lengths, then the
/// records written in place. Besides the result, `16 (n + 1)` bytes and the batches of
/// the tasks in flight.
///
/// # Errors
///
/// [`EncodeError::Offsets`] if the two do not have the same nodes;
/// [`EncodeError::Decode`] for records that do not decode; [`EncodeError::NotIncreasing`]
/// for a list that does not increase; [`EncodeError::OutOfMemory`] if the result does
/// not fit `memory`.
///
/// # Panics
///
/// If `a` or `b` is a [`window`](Adjacency::window) of some of the nodes.
pub fn merge(
    a: &Adjacency<'_>,
    b: &Adjacency<'_>,
    memory: &Memory,
) -> Result<(Encoded, Reservation), EncodeError> {
    let nodes = a.index().nodes();
    if b.index().nodes() != nodes {
        return Err(EncodeError::Offsets("the two adjacencies differ in nodes"));
    }
    assert!(
        a.nodes() == (0..nodes) && b.nodes() == (0..nodes),
        "merge of a window"
    );
    let n = usize::try_from(nodes).expect("usize has at least 32 bits");
    let oom = |e: MemoryError| EncodeError::OutOfMemory {
        bytes: match e {
            MemoryError::Budget(refused) => refused.needed,
            _ => u64::MAX,
        },
    };
    let offsets_bytes = 16 * (u64::from(nodes) + 1);
    let mut held = memory
        .reserve(PURPOSE, MemoryKind::Anonymous, offsets_bytes)
        .map_err(|e| oom(e.into()))?;

    // Pass 1: each list's degree and record length.
    let chunks = par::map_ranges(n, GRAIN, |range| {
        let mut sizes = Vec::with_capacity(range.len());
        each_union(a, b, range, |v, list| {
            let len =
                svb::record_len(v, list).map_err(|_| EncodeError::NotIncreasing { node: v })?;
            sizes.push((list.len() as u64, len as u64));
            Ok(())
        })?;
        Ok::<_, EncodeError>(sizes)
    });
    let mut entry_offsets = Vec::with_capacity(n + 1);
    let mut byte_offsets = Vec::with_capacity(n + 1);
    entry_offsets.push(0);
    byte_offsets.push(0);
    for chunk in chunks {
        for (degree, len) in chunk? {
            entry_offsets.push(entry_offsets[entry_offsets.len() - 1] + degree);
            byte_offsets.push(byte_offsets[byte_offsets.len() - 1] + len);
        }
    }
    let (entries, data_bytes) = (entry_offsets[n], byte_offsets[n]);
    held.try_grow(data_bytes).map_err(|e| oom(e.into()))?;
    let mut data = Vec::new();
    let len =
        usize::try_from(data_bytes).map_err(|_| EncodeError::OutOfMemory { bytes: data_bytes })?;
    data.try_reserve_exact(len)
        .map_err(|_| EncodeError::OutOfMemory { bytes: data_bytes })?;
    data.resize(len, 0);

    // Pass 2: the records, each chunk into its own part of the data.
    let at = |v: usize| usize::try_from(byte_offsets[v]).expect("checked: data_bytes fits");
    let mut pieces = Vec::with_capacity(n.div_ceil(GRAIN.get()));
    let mut rest = data.as_mut_slice();
    for start in (0..n).step_by(GRAIN.get()) {
        let end = (start + GRAIN.get()).min(n);
        let (piece, tail) = rest.split_at_mut(at(end) - at(start));
        pieces.push((start..end, piece, Ok(())));
        rest = tail;
    }
    par::for_each_chunk_mut(&mut pieces, NonZeroUsize::MIN, |_, chunk| {
        for (range, out, result) in chunk.iter_mut() {
            let base = at(range.start);
            *result = each_union(a, b, range.clone(), |v, list| {
                let v_at = usize::try_from(v).expect("32 bits");
                svb::write_record(v, list, &mut out[at(v_at) - base..at(v_at + 1) - base]);
                Ok(())
            });
        }
    });
    pieces.into_iter().try_for_each(|p| p.2)?;

    let degrees = entry_offsets.windows(2).map(|w| w[1] - w[0]);
    let index = AdjacencyIndex::from_parts(
        false,
        nodes,
        entries,
        data_bytes,
        block_starts(n, degrees),
        EliasFano::new(&byte_offsets),
        EliasFano::new(&entry_offsets),
    );
    drop((entry_offsets, byte_offsets));
    let index = index.write();
    let total = data_bytes + u64::try_from(index.len()).unwrap_or(u64::MAX);
    let _ = held.try_resize(total);
    Ok((Encoded { index, data }, held))
}

/// Calls `f(v, union)` for each node of `range` with the sorted union of its lists in
/// `a` and `b`.
fn each_union(
    a: &Adjacency<'_>,
    b: &Adjacency<'_>,
    range: std::ops::Range<usize>,
    mut f: impl FnMut(u32, &[u32]) -> Result<(), EncodeError>,
) -> Result<(), EncodeError> {
    let nodes =
        u32::try_from(range.start).expect("32 bits")..u32::try_from(range.end).expect("32 bits");
    let (mut from_a, mut from_b) = (Batch::default(), Batch::default());
    a.decode_nodes(nodes.clone(), usize::MAX, &mut from_a)
        .map_err(EncodeError::Decode)?;
    b.decode_nodes(nodes, usize::MAX, &mut from_b)
        .map_err(EncodeError::Decode)?;
    let mut union = Vec::new();
    for ((v, x), (_, y)) in from_a.iter().zip(from_b.iter()) {
        union.clear();
        let (mut i, mut j) = (0, 0);
        while i < x.len() || j < y.len() {
            let next = match (x.get(i), y.get(j)) {
                (Some(&p), Some(&q)) if p == q => {
                    (i, j) = (i + 1, j + 1);
                    p
                }
                (Some(&p), Some(&q)) if p < q => {
                    i += 1;
                    p
                }
                (Some(&p), None) => {
                    i += 1;
                    p
                }
                (_, Some(&q)) => {
                    j += 1;
                    q
                }
                (None, None) => unreachable!("the loop runs while a list has entries"),
            };
            union.push(next);
        }
        f(v.get(), &union)?;
    }
    Ok(())
}
