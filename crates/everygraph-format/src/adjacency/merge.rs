// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The union of two adjacencies over the same nodes, list by list: the undirected
//! projection of a directed graph, from its out- and in-adjacency
//! (`docs/spec/graph-semantics.md`, "undirected projection").

use std::num::NonZeroUsize;
use std::ops::Range;

use everygraph_core::{InsufficientBudget, Memory, MemoryKind, NodeId, Reservation, par};

use super::decode::Adjacency;
use super::ef::EliasFano;
use super::encode::{Encoded, block_starts};
use super::index::AdjacencyIndex;
use super::stream::index_bytes_bound;
use super::{EncodeError, svb};

/// Nodes per task.
const GRAIN: usize = 1 << 12;

const PURPOSE: &str = "merged adjacency";

fn refused(e: InsufficientBudget) -> EncodeError {
    EncodeError::OutOfMemory { bytes: e.needed }
}

fn to_usize(x: u64) -> usize {
    usize::try_from(x).expect("checked: fits in memory")
}

/// The adjacency whose list of `v` is the union of the lists of `v` in `a` and in `b`,
/// encoded in memory, with the reservation that holds it. Two passes in parallel over
/// chunks of nodes, each decoding both lists of every node: the degrees and record
/// lengths, written into the offsets and summed in place; then the records, written in
/// place. Everything it holds is reserved against `memory`: the offsets, the data, the
/// index while it is built, and each task's three lists.
///
/// # Errors
///
/// [`EncodeError::Offsets`] if the two do not have the same nodes;
/// [`EncodeError::Decode`] for records that do not decode; [`EncodeError::NotIncreasing`]
/// for a list that does not increase; [`EncodeError::OutOfMemory`] if something does
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
    let n = same_nodes(a, b)?;
    let mut held = memory
        .reserve(PURPOSE, MemoryKind::Anonymous, 16 * (widen(n) + 1))
        .map_err(refused)?;
    // Entry and byte offsets: per node its degree and record length, then their sums.
    let mut offsets = Vec::new();
    offsets
        .try_reserve_exact(n + 1)
        .map_err(|_| EncodeError::OutOfMemory {
            bytes: 16 * (widen(n) + 1),
        })?;
    offsets.resize(n + 1, [0_u64; 2]);
    each_chunk(&mut offsets[1..], |first, chunk| {
        unions(a, b, first, memory, |i, v, list| {
            let len =
                svb::record_len(v, list).map_err(|_| EncodeError::NotIncreasing { node: v })?;
            chunk[i] = [widen(list.len()), widen(len)];
            Ok(())
        })
    })?;
    for v in 0..n {
        let [entries, bytes] = offsets[v];
        offsets[v + 1][0] += entries;
        offsets[v + 1][1] += bytes;
    }
    let [entries, data_bytes] = offsets[n];
    let index_bytes = index_bytes_bound(widen(n), entries, data_bytes)
        .ok_or(EncodeError::OutOfMemory { bytes: u64::MAX })?;
    held.try_grow(data_bytes.saturating_add(index_bytes))
        .map_err(refused)?;
    let mut data = Vec::new();
    data.try_reserve_exact(to_usize(data_bytes))
        .map_err(|_| EncodeError::OutOfMemory { bytes: data_bytes })?;
    data.resize(to_usize(data_bytes), 0);

    let at = |v: usize| to_usize(offsets[v][1]);
    let mut pieces = Vec::with_capacity(n.div_ceil(GRAIN));
    let mut rest = data.as_mut_slice();
    for start in (0..n).step_by(GRAIN) {
        let end = (start + GRAIN).min(n);
        let (piece, tail) = rest.split_at_mut(at(end) - at(start));
        pieces.push((start, piece));
        rest = tail;
    }
    let mut results: Vec<Result<(), EncodeError>> = pieces.iter().map(|_| Ok(())).collect();
    let mut jobs: Vec<_> = pieces.into_iter().zip(results.iter_mut()).collect();
    par::for_each_chunk_mut(&mut jobs, NonZeroUsize::MIN, |_, job| {
        let ((start, out), result) = &mut job[0];
        let (start, base) = (*start, at(*start));
        let count = (start + GRAIN).min(n) - start;
        **result = unions(a, b, start..start + count, memory, |_, v, list| {
            let v_at = to_usize(u64::from(v));
            svb::write_record(v, list, &mut out[at(v_at) - base..at(v_at + 1) - base]);
            Ok(())
        });
    });
    results.into_iter().collect::<Result<(), _>>()?;

    let nodes = u32::try_from(n).expect("the node count of an index");
    let degrees = offsets.windows(2).map(|w| w[1][0] - w[0][0]);
    let blocks = block_starts(n, degrees);
    let index = AdjacencyIndex::from_parts(
        false,
        nodes,
        entries,
        data_bytes,
        blocks,
        EliasFano::from_values(n + 1, data_bytes, offsets.iter().map(|o| o[1]))
            .ok_or(EncodeError::OutOfMemory { bytes: index_bytes })?,
        EliasFano::from_values(n + 1, entries, offsets.iter().map(|o| o[0]))
            .ok_or(EncodeError::OutOfMemory { bytes: index_bytes })?,
    );
    drop(offsets);
    let index = index.write();
    let kept = data_bytes + widen(index.len());
    held.shrink(held.bytes().saturating_sub(kept));
    Ok((Encoded { index, data }, held))
}

/// The degree of each node in the union of `a` and `b`, a node's own entry counted
/// twice: the `degree` of a directed graph, from its out- and in-adjacency
/// (`docs/spec/graph-semantics.md` § 3), without building the union. `degrees` has one
/// slot per node.
///
/// # Errors
///
/// As [`merge`], but for the result's memory, which the caller holds.
///
/// # Panics
///
/// If `degrees` does not have one slot per node, or `a` or `b` is a window.
pub fn union_degrees(
    a: &Adjacency<'_>,
    b: &Adjacency<'_>,
    memory: &Memory,
    degrees: &mut [u32],
) -> Result<(), EncodeError> {
    let n = same_nodes(a, b)?;
    assert_eq!(degrees.len(), n, "one degree per node");
    each_chunk(degrees, |first, chunk| {
        unions(a, b, first, memory, |i, v, list| {
            let own = u32::from(list.binary_search(&v).is_ok());
            chunk[i] = u32::try_from(list.len())
                .ok()
                .and_then(|d| d.checked_add(own))
                .ok_or(EncodeError::Offsets("a degree of 2^32 or more"))?;
            Ok(())
        })
    })
}

fn same_nodes(a: &Adjacency<'_>, b: &Adjacency<'_>) -> Result<usize, EncodeError> {
    let nodes = a.index().nodes();
    if b.index().nodes() != nodes {
        return Err(EncodeError::Offsets("the two adjacencies differ in nodes"));
    }
    assert!(
        a.nodes() == (0..nodes) && b.nodes() == (0..nodes),
        "merge of a window"
    );
    Ok(to_usize(u64::from(nodes)))
}

/// Calls `f(start, chunk)` for chunks of [`GRAIN`] slots of `slots` in parallel, slot
/// `i` of a chunk being node `start + i`; the first error in node order.
fn each_chunk<T: Send>(
    slots: &mut [T],
    f: impl Fn(Range<usize>, &mut [T]) -> Result<(), EncodeError> + Sync,
) -> Result<(), EncodeError> {
    let chunks = slots.len().div_ceil(GRAIN);
    let mut results: Vec<Result<(), EncodeError>> = (0..chunks).map(|_| Ok(())).collect();
    let mut jobs: Vec<_> = slots.chunks_mut(GRAIN).zip(results.iter_mut()).collect();
    par::for_each_chunk_mut(&mut jobs, NonZeroUsize::MIN, |k, job| {
        let (chunk, result) = &mut job[0];
        let start = k * GRAIN;
        **result = f(start..start + chunk.len(), chunk);
    });
    results.into_iter().collect()
}

/// Calls `f(i, v, union)` for the `i`-th node `v` of `nodes`, with the sorted union of
/// its lists in `a` and `b`. The three lists are held under `memory`.
fn unions(
    a: &Adjacency<'_>,
    b: &Adjacency<'_>,
    nodes: Range<usize>,
    memory: &Memory,
    mut f: impl FnMut(usize, u32, &[u32]) -> Result<(), EncodeError>,
) -> Result<(), EncodeError> {
    let mut held = Reservation::empty(memory, PURPOSE, MemoryKind::Anonymous);
    let (mut x, mut y, mut union) = (Vec::new(), Vec::new(), Vec::new());
    for (i, v) in nodes.enumerate() {
        let id = NodeId::new(u32::try_from(v).expect("a node of an index"));
        let degrees = [a.index().degree(id), b.index().degree(id)].map(u64::from);
        // Each list with the decoders' slack, and their union.
        let need = (degrees[0] + degrees[1]) * 8 + 32;
        if need > held.bytes() {
            held.try_resize(need).map_err(refused)?;
        }
        let xa = a.neighbors(id, &mut x).map_err(EncodeError::Decode)?;
        let yb = b.neighbors(id, &mut y).map_err(EncodeError::Decode)?;
        union.clear();
        let (mut p, mut q) = (0, 0);
        while p < xa.len() || q < yb.len() {
            let next = match (xa.get(p), yb.get(q)) {
                (Some(&s), Some(&t)) if s == t => {
                    (p, q) = (p + 1, q + 1);
                    s
                }
                (Some(&s), Some(&t)) if s < t => {
                    p += 1;
                    s
                }
                (Some(&s), None) => {
                    p += 1;
                    s
                }
                (_, Some(&t)) => {
                    q += 1;
                    t
                }
                (None, None) => unreachable!("the loop runs while a list has entries"),
            };
            union.push(next);
        }
        f(i, id.get(), &union)?;
    }
    Ok(())
}

fn widen(x: usize) -> u64 {
    u64::try_from(x).expect("usize has at most 64 bits")
}
