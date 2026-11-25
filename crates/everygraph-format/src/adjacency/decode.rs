// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Reading: the decoders, single lists, batches of consecutive lists, the whole graph.

use std::num::NonZeroUsize;
use std::ops::Range;

use everygraph_core::{NodeId, par};

use super::ef;
use super::index::AdjacencyIndex;
use super::simd::Decoder;
use super::svb::Corrupt;
use super::{DecodeError, FormatError};

/// Nodes per task when offsets are written in parallel.
const OFFSET_GRAIN: NonZeroUsize = NonZeroUsize::new(1 << 16).expect("non-zero");

/// A run of consecutive lists to decode.
pub(super) struct Job<'a> {
    data: &'a [u8],
    /// Byte offset of the first record.
    at: usize,
    /// Entry offsets, positioned at the first node.
    entries: ef::Iter<'a>,
    nodes: Range<u32>,
    /// Stop before a list that would take the output past this many entries, unless it
    /// is the first.
    limit: usize,
}

/// Where a [`run`] stopped.
pub(super) struct Done {
    /// The first node not decoded.
    node: u32,
    /// The byte past the last record read, which the caller compares with the index.
    at: usize,
    /// Entries written.
    entries: usize,
}

/// Decodes the lists of `job.nodes` one after the other into `out`, until the next one
/// would not fit in `job.limit` entries (the first list always does; `out` has room for
/// what can be written), and leaves the end of each list in `out` in `ends`.
///
/// Two passes: the list ends from the entry offsets, then the records. Keeping the
/// Elias–Fano iteration out of the decode loop leaves that loop its registers (lj scan,
/// one thread, exploratory, binaries alternated: 1.13 → 1.27 G entries/s; PR of #30).
#[inline(always)]
pub(super) fn run(
    kernel: impl Fn(&[u8], u32, usize, usize, &mut [u32]) -> Result<usize, Corrupt>,
    job: Job<'_>,
    out: &mut [u32],
    ends: &mut Vec<usize>,
) -> Result<Done, DecodeError> {
    let Job {
        data,
        mut at,
        mut entries,
        nodes,
        limit,
    } = job;
    ends.clear();
    let base = entries.next().unwrap_or(0);
    let mut stop = nodes.end;
    for v in nodes.clone() {
        let end = to_usize(entries.next().expect("an entry offset per node") - base);
        if v > nodes.start && end > limit {
            stop = v;
            break;
        }
        ends.push(end);
    }
    let mut s = 0;
    for (v, &end) in (nodes.start..).zip(ends.iter()) {
        if end > s {
            let corrupt = |_| DecodeError {
                first: v,
                end: v + 1,
            };
            let rec = data.get(at..).ok_or(Corrupt).map_err(corrupt)?;
            at += kernel(rec, v, end - s, end - s, &mut out[s..]).map_err(corrupt)?;
            s = end;
        }
    }
    Ok(Done {
        node: stop,
        at,
        entries: s,
    })
}

fn to_usize(x: u64) -> usize {
    usize::try_from(x).expect("an offset inside data held in memory")
}

/// Grows `buf` to at least `len` elements; never shrinks it, so a reused buffer is not
/// written twice.
fn grow(buf: &mut Vec<u32>, len: usize) {
    if buf.len() < len {
        buf.resize(len, 0);
    }
}

/// Decoded lists of consecutive nodes: the caller's reusable buffer for
/// [`Adjacency::decode_nodes`].
#[derive(Clone, Debug, Default)]
pub struct Batch {
    first: u32,
    entries: usize,
    /// At least `entries + 3`: the SIMD decoders' last group needs no scalar loop.
    targets: Vec<u32>,
    ends: Vec<usize>,
}

impl Batch {
    /// The first node decoded.
    #[must_use]
    pub fn first(&self) -> u32 {
        self.first
    }

    /// Number of lists.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ends.len()
    }

    /// Whether there are no lists.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ends.is_empty()
    }

    /// List `i` of the batch, that of node `first() + i`.
    ///
    /// # Panics
    ///
    /// If `i` is not below [`len`](Self::len).
    #[must_use]
    pub fn list(&self, i: usize) -> &[u32] {
        let start = if i == 0 { 0 } else { self.ends[i - 1] };
        &self.targets[start..self.ends[i]]
    }

    /// All entries, list after list.
    #[must_use]
    pub fn targets(&self) -> &[u32] {
        &self.targets[..self.entries]
    }

    /// The lists with their nodes.
    pub fn iter(&self) -> impl Iterator<Item = (NodeId, &[u32])> + '_ {
        (self.first..)
            .zip(0..self.len())
            .map(|(v, i)| (NodeId::new(v), self.list(i)))
    }
}

/// A compressed adjacency: its index and its data part, in memory (a heap buffer or a
/// memory map). Cheap to create and to copy; every method takes `&self`, so one value
/// can be shared by many threads.
#[derive(Clone, Copy, Debug)]
pub struct Adjacency<'a> {
    index: &'a AdjacencyIndex,
    data: &'a [u8],
    decoder: Decoder,
}

impl<'a> Adjacency<'a> {
    /// Pairs an index with its data part, using [`Decoder::best`].
    ///
    /// # Errors
    ///
    /// [`FormatError::Length`] if `data` is not as long as the index says.
    pub fn new(index: &'a AdjacencyIndex, data: &'a [u8]) -> Result<Self, FormatError> {
        let actual = u64::try_from(data.len()).unwrap_or(u64::MAX);
        if actual != index.data_bytes() {
            return Err(FormatError::Length {
                expected: index.data_bytes(),
                actual,
            });
        }
        Ok(Self {
            index,
            data,
            decoder: Decoder::best(),
        })
    }

    /// The same adjacency read with another decoder.
    #[must_use]
    pub fn with_decoder(self, decoder: Decoder) -> Self {
        Self { decoder, ..self }
    }

    /// The decoder in use.
    #[must_use]
    pub fn decoder(&self) -> Decoder {
        self.decoder
    }

    /// The index.
    #[must_use]
    pub fn index(&self) -> &'a AdjacencyIndex {
        self.index
    }

    /// The neighbours of `v`, decoded into `buf` (which grows as needed and is never
    /// shrunk, so reusing it costs nothing).
    ///
    /// # Errors
    ///
    /// [`DecodeError`] if the record of `v` is corrupt.
    ///
    /// # Panics
    ///
    /// If `v` is not below the number of nodes.
    pub fn neighbors<'b>(
        &self,
        v: NodeId,
        buf: &'b mut Vec<u32>,
    ) -> Result<&'b [u32], DecodeError> {
        let d = self.index.degree(v);
        let d = usize::try_from(d).expect("usize has at least 32 bits");
        let k = self.prefix(v, d, d, buf)?;
        Ok(&buf[..k])
    }

    /// The first `k` neighbours of `v` (all of them if it has fewer), decoded into
    /// `buf`, without decoding the rest of the list. The degree is
    /// [`AdjacencyIndex::degree`].
    ///
    /// # Errors
    ///
    /// [`DecodeError`] if the record of `v` is corrupt where it was read.
    ///
    /// # Panics
    ///
    /// If `v` is not below the number of nodes.
    pub fn neighbors_prefix<'b>(
        &self,
        v: NodeId,
        k: usize,
        buf: &'b mut Vec<u32>,
    ) -> Result<&'b [u32], DecodeError> {
        let d = usize::try_from(self.index.degree(v)).expect("usize has at least 32 bits");
        let k = self.prefix(v, d, k.min(d), buf)?;
        Ok(&buf[..k])
    }

    fn prefix(
        &self,
        v: NodeId,
        d: usize,
        k: usize,
        buf: &mut Vec<u32>,
    ) -> Result<usize, DecodeError> {
        if k == 0 {
            return Ok(0);
        }
        let corrupt = DecodeError {
            first: v.get(),
            end: v.get() + 1,
        };
        let range = self.index.record_range(v);
        grow(buf, k + 3);
        let used = self
            .decoder
            .list(&self.data[to_usize(range.start)..], v.get(), d, k, buf)
            .map_err(|_| corrupt)?;
        if k == d && used as u64 != range.end - range.start {
            return Err(corrupt);
        }
        Ok(k)
    }

    /// Decodes from `nodes.start` into `out` while the lists fit in `limit` entries,
    /// checking that the records end where the index says; returns where it stopped.
    fn decode_into(
        &self,
        nodes: Range<u32>,
        limit: usize,
        out: &mut [u32],
        ends: &mut Vec<usize>,
    ) -> Result<Done, DecodeError> {
        let start = to_usize(u64::from(nodes.start));
        let job = Job {
            data: self.data,
            at: to_usize(self.index.byte_offsets().get(start)),
            entries: self.index.entry_offsets().iter_from(start),
            nodes: nodes.clone(),
            limit,
        };
        let done = self.decoder.run(job, out, ends)?;
        let end = self
            .index
            .byte_offsets()
            .get(to_usize(u64::from(done.node)));
        if done.at as u64 == end {
            Ok(done)
        } else {
            Err(DecodeError {
                first: nodes.start,
                end: done.node,
            })
        }
    }

    /// Decodes consecutive lists into `batch`, replacing what it held: from
    /// `nodes.start`, while the next list still fits in `max_entries` entries (the
    /// first list always goes in, whatever its length), up to `nodes.end`. Returns the
    /// first node not decoded, where the next batch starts.
    ///
    /// This is how sequential access decodes: a batch at a time, before the work on it
    /// (ADR 0004, condition 2); batches of a few thousand entries stay in the L1 cache
    /// (the lab used 8,192). A block (see [`AdjacencyIndex::block_nodes`]) with
    /// `usize::MAX` is a batch too.
    ///
    /// # Errors
    ///
    /// [`DecodeError`] if a record in the range is corrupt; the batch is then empty.
    ///
    /// # Panics
    ///
    /// If `nodes` is not a range within `0..nodes()`, or the batch does not fit in
    /// memory on this target.
    pub fn decode_nodes(
        &self,
        nodes: Range<u32>,
        max_entries: usize,
        batch: &mut Batch,
    ) -> Result<u32, DecodeError> {
        assert!(
            nodes.start <= nodes.end && nodes.end <= self.index.nodes(),
            "nodes {nodes:?} outside 0..{}",
            self.index.nodes()
        );
        batch.first = nodes.start;
        batch.entries = 0;
        batch.ends.clear();
        if nodes.is_empty() {
            return Ok(nodes.end);
        }
        let ef = self.index.entry_offsets();
        let (start, end) = (to_usize(nodes.start.into()), to_usize(nodes.end.into()));
        let (first, second) = ef.get_pair(start);
        let span = to_usize(ef.get(end) - first);
        let room = max_entries.max(to_usize(second - first)).min(span);
        grow(&mut batch.targets, room + 3);
        match self.decode_into(nodes, max_entries, &mut batch.targets, &mut batch.ends) {
            Ok(done) => {
                batch.entries = done.entries;
                Ok(done.node)
            }
            Err(e) => {
                batch.ends.clear();
                Err(e)
            }
        }
    }

    /// Decodes the whole adjacency into a raw CSR: `offsets` (`nodes + 1` entries) and
    /// `targets` (`entries`), which the caller allocates (against its memory budget).
    /// Runs in parallel over blocks.
    ///
    /// # Errors
    ///
    /// [`DecodeError`] for the first corrupt block, in node order.
    ///
    /// # Panics
    ///
    /// If the two slices do not have those lengths.
    pub fn decode_all(&self, offsets: &mut [u64], targets: &mut [u32]) -> Result<(), DecodeError> {
        let index = self.index;
        assert_eq!(
            offsets.len() as u64,
            u64::from(index.nodes()) + 1,
            "offsets length"
        );
        assert_eq!(targets.len() as u64, index.entries(), "targets length");
        par::for_each_chunk_mut(offsets, OFFSET_GRAIN, |start, chunk| {
            let mut values = index.entry_offsets().iter_from(start);
            for o in chunk {
                *o = values.next().expect("an entry offset per node");
            }
        });
        let mut pieces = Vec::with_capacity(index.blocks());
        let mut rest = targets;
        for b in 0..index.blocks() {
            let nodes = index.block_nodes(b);
            let len = to_usize(
                offsets[to_usize(nodes.end.into())] - offsets[to_usize(nodes.start.into())],
            );
            let (piece, tail) = rest.split_at_mut(len);
            pieces.push((nodes, piece, Ok(())));
            rest = tail;
        }
        par::for_each_chunk_mut(&mut pieces, NonZeroUsize::MIN, |_, chunk| {
            for (nodes, out, result) in chunk.iter_mut() {
                *result = self
                    .decode_into(nodes.clone(), usize::MAX, out, &mut Vec::new())
                    .map(|_| ());
            }
        });
        pieces.into_iter().try_for_each(|p| p.2)
    }
}
