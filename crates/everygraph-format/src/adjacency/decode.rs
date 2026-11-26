// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Reading: the decoders, single lists, batches of consecutive lists, the whole graph.

use std::num::NonZeroUsize;
use std::ops::Range;

use everygraph_core::{NodeId, par};

use super::ef::EliasFano;
use super::index::AdjacencyIndex;
use super::simd::Decoder;
use super::svb::Corrupt;
use super::{DecodeError, FormatError};

/// Nodes per task when offsets are written in parallel.
const OFFSET_GRAIN: NonZeroUsize = NonZeroUsize::new(1 << 16).expect("non-zero");

/// A run of consecutive lists to decode.
pub(super) struct Job<'a> {
    data: &'a [u8],
    /// Offset in the data part of `data[0]`.
    base: u64,
    /// Entry offsets.
    entries: &'a EliasFano,
    /// Byte offsets.
    bytes: &'a EliasFano,
    nodes: Range<u32>,
    /// Stop before a list that would take the output past this many entries, unless it
    /// is the first.
    limit: usize,
    /// Entries decoded of each list, at most: its prefix.
    prefix: usize,
}

/// Where a [`run`] stopped.
pub(super) struct Done {
    /// The first node not decoded.
    node: u32,
    /// Entries written.
    entries: usize,
}

/// Where each list of a run ends: in the output (`entries`, relative to the run's first
/// entry) and in the data (`bytes`, from the start of the job's data). Reused from run to
/// run.
#[derive(Clone, Debug, Default)]
pub(super) struct Ends {
    pub(super) entries: Vec<usize>,
    pub(super) bytes: Vec<usize>,
}

/// Decodes the lists of `job.nodes` one after the other into `out`, until the next one
/// would not fit in `job.limit` entries (the first list always does; `out` has room for
/// what can be written), and leaves where each list ends in `ends`. Of each list only the
/// first `job.prefix` entries are decoded; a record decoded in full is checked, a prefix
/// is not.
///
/// Two passes: where every list ends, in entries and in bytes, from the two Elias–Fano
/// sequences; then the records, each checked to end exactly where the index says
/// (`docs/spec/eg-adjacency.md` § 5), so that a damaged byte is an error at its own
/// node and every API agrees on the same bytes. Keeping the Elias–Fano iteration out of
/// the decode loop leaves that loop its registers (lj scan, one thread, exploratory,
/// binaries alternated, not recorded: 1.13 → 1.27 G entries/s). Checking every record, not
/// only the batch's end, costs the second sequence: lj 1.28 → 1.10 and orkut 2.37 →
/// 2.17 G entries/s (exploratory, interleaved; `lab/m2-03-codec/results/record-check.jsonl`).
#[inline(always)]
pub(super) fn run(
    kernel: impl Fn(&[u8], u32, usize, usize, &mut [u32]) -> Result<usize, Corrupt>,
    job: Job<'_>,
    out: &mut [u32],
    ends: &mut Ends,
) -> Result<Done, DecodeError> {
    let Job {
        data,
        base: data_base,
        entries,
        bytes,
        nodes,
        limit,
        prefix,
    } = job;
    ends.entries.clear();
    ends.bytes.clear();
    let first = to_usize(u64::from(nodes.start));
    let count = nodes.len();
    // Each sequence in a loop of its own, which keeps each loop in registers: the entry
    // ends up to the budget (the first list always), then as many byte ends. Value
    // `first` of each is where the run starts.
    let (mut prev, mut end, mut started) = (0, 0, false);
    entries.scan_from(first, |x| {
        if !started {
            (prev, started) = (x, true);
            return true;
        }
        let next = end + to_usize(x - prev).min(prefix);
        let lists = ends.entries.len();
        if lists == count || (lists > 0 && next > limit) {
            return false;
        }
        (prev, end) = (x, next);
        ends.entries.push(end);
        true
    });
    let lists = ends.entries.len();
    let (mut at, mut started) = (0, false);
    bytes.scan_from(first, |x| {
        if !started {
            (at, started) = (to_usize(x - data_base), true);
            return true;
        }
        if ends.bytes.len() == lists {
            return false;
        }
        ends.bytes.push(to_usize(x - data_base));
        true
    });
    let stop = nodes.start + u32::try_from(lists).expect("at most one list per node");
    let mut s = 0;
    let lists = ends.entries.iter().zip(&ends.bytes);
    // Decodes the first `end - s` of the `total` entries of the list of `v`.
    let mut record = |v: u32, at: usize, s: usize, end: usize, total: usize, record_end| {
        let corrupt = |_| DecodeError {
            first: v,
            end: v + 1,
        };
        let rec = data.get(at..).ok_or(Corrupt).map_err(corrupt)?;
        let used = kernel(rec, v, total, end - s, &mut out[s..]).map_err(corrupt)?;
        if end - s == total && at + used != record_end {
            return Err(corrupt(Corrupt));
        }
        Ok(())
    };
    if prefix == usize::MAX {
        for (v, (&end, &record_end)) in (nodes.start..).zip(lists) {
            if end > s {
                record(v, at, s, end, end - s, record_end)?;
                s = end;
            }
            at = record_end;
        }
    } else {
        // The degrees are needed only here: whole lists keep the loop above free of a
        // third Elias–Fano iteration.
        let mut offsets = entries.iter_from(first);
        let mut from = offsets.next().unwrap_or(0);
        for ((v, (&end, &record_end)), to) in (nodes.start..).zip(lists).zip(offsets) {
            if end > s {
                record(v, at, s, end, to_usize(to - from), record_end)?;
                s = end;
            }
            (at, from) = (record_end, to);
        }
    }
    Ok(Done {
        node: stop,
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

/// Entries the SIMD decoders may write past the last list: their last group needs no
/// scalar loop.
const SLACK: usize = 3;

/// Decoded lists of consecutive nodes: the caller's reusable buffer for
/// [`Adjacency::decode_nodes`].
///
/// A batch grows as it decodes. A caller that counts its memory sizes it first:
/// [`Adjacency::batch_entries`] says how many entries a call may need, [`reserve`](Self::reserve)
/// makes room for them, and the call then does not allocate.
#[derive(Clone, Debug, Default)]
pub struct Batch {
    first: u32,
    entries: usize,
    /// At least `entries + SLACK`.
    targets: Vec<u32>,
    ends: Ends,
}

impl Batch {
    /// Heap bytes a batch holds after [`reserve`](Self::reserve) of `entries` and
    /// `lists` from empty.
    #[must_use]
    pub const fn bytes_for(entries: usize, lists: usize) -> usize {
        entries * size_of::<u32>() + lists * 2 * size_of::<usize>()
    }

    /// Entries and lists the batch has room for without allocating.
    #[must_use]
    pub fn capacity(&self) -> (usize, usize) {
        let lists = self.ends.entries.capacity().min(self.ends.bytes.capacity());
        (self.targets.capacity(), lists)
    }

    /// Heap bytes the batch holds.
    #[must_use]
    pub fn heap_bytes(&self) -> usize {
        self.targets.capacity() * size_of::<u32>()
            + (self.ends.entries.capacity() + self.ends.bytes.capacity()) * size_of::<usize>()
    }

    /// Makes room, without spare capacity, for a decode of `entries` entries (the value
    /// of [`Adjacency::batch_entries`]) into at most `lists` lists, so that the decode
    /// does not allocate.
    pub fn reserve(&mut self, entries: usize, lists: usize) {
        self.targets
            .reserve_exact(entries.saturating_sub(self.targets.len()));
        for ends in [&mut self.ends.entries, &mut self.ends.bytes] {
            ends.reserve_exact(lists.saturating_sub(ends.len()));
        }
    }

    /// Empties the batch and frees memory beyond `entries` entries and `lists` lists.
    pub fn shrink_to(&mut self, entries: usize, lists: usize) {
        self.clear();
        self.targets.truncate(entries);
        self.targets.shrink_to(entries);
        self.ends.entries.shrink_to(lists);
        self.ends.bytes.shrink_to(lists);
    }

    /// Empties the batch: no lists. The memory is kept.
    pub fn clear(&mut self) {
        self.entries = 0;
        self.ends.entries.clear();
        self.ends.bytes.clear();
    }

    /// The first node decoded.
    #[must_use]
    pub fn first(&self) -> u32 {
        self.first
    }

    /// Number of lists.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ends.entries.len()
    }

    /// Whether there are no lists.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ends.entries.is_empty()
    }

    /// List `i` of the batch, that of node `first() + i`.
    ///
    /// # Panics
    ///
    /// If `i` is not below [`len`](Self::len).
    #[must_use]
    pub fn list(&self, i: usize) -> &[u32] {
        let ends = &self.ends.entries;
        let start = if i == 0 { 0 } else { ends[i - 1] };
        &self.targets[start..ends[i]]
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
///
/// A [`window`](Self::window) holds the records of a range of nodes only, for a data
/// part that is read piece by piece.
#[derive(Clone, Copy, Debug)]
pub struct Adjacency<'a> {
    index: &'a AdjacencyIndex,
    data: &'a [u8],
    /// The nodes whose records `data` holds: `first..end`.
    first: u32,
    end: u32,
    /// Offset in the data part of `data[0]`.
    base: u64,
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
            first: 0,
            end: index.nodes(),
            base: 0,
            decoder: Decoder::best(),
        })
    }

    /// The lists of `nodes` alone, from `records`: the bytes of the data part from the
    /// start of the record of `nodes.start` to the end of the record of `nodes.end − 1`.
    /// Only nodes of the window may be decoded.
    ///
    /// # Errors
    ///
    /// [`FormatError::Length`] if `records` is not as long as the index says.
    ///
    /// # Panics
    ///
    /// If `nodes` is not a range within `0..nodes()`.
    pub fn window(
        index: &'a AdjacencyIndex,
        nodes: Range<u32>,
        records: &'a [u8],
    ) -> Result<Self, FormatError> {
        assert!(
            nodes.start <= nodes.end && nodes.end <= index.nodes(),
            "nodes {nodes:?} outside 0..{}",
            index.nodes()
        );
        let bytes = index.data_range(nodes.clone());
        let actual = u64::try_from(records.len()).unwrap_or(u64::MAX);
        if actual != bytes.end - bytes.start {
            return Err(FormatError::Length {
                expected: bytes.end - bytes.start,
                actual,
            });
        }
        Ok(Self {
            index,
            data: records,
            first: nodes.start,
            end: nodes.end,
            base: bytes.start,
            decoder: Decoder::best(),
        })
    }

    /// The same adjacency read with another decoder.
    #[must_use]
    pub fn with_decoder(self, decoder: Decoder) -> Self {
        Self { decoder, ..self }
    }

    /// The nodes whose lists can be decoded: all of them, or those of a
    /// [`window`](Self::window).
    #[must_use]
    pub fn nodes(&self) -> Range<u32> {
        self.first..self.end
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
    /// # Untrusted input
    ///
    /// Every record is checked to end where the index says, but a damaged or hostile
    /// file can still hold records that decode to lists that are not increasing or that
    /// name targets `>= nodes()`. Code that indexes arrays with decoded targets checks
    /// them first (#44).
    ///
    /// # Errors
    ///
    /// [`DecodeError`] if the record of `v` is corrupt.
    ///
    /// # Panics
    ///
    /// If `v` is not a node of [`nodes`](Self::nodes).
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
    /// If `v` is not a node of [`nodes`](Self::nodes).
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
        assert!(
            (self.first..self.end).contains(&v.get()),
            "node {v} outside the window"
        );
        let range = self.index.record_range(v);
        grow(buf, k + SLACK);
        let used = self
            .decoder
            .list(
                &self.data[to_usize(range.start - self.base)..],
                v.get(),
                d,
                k,
                buf,
            )
            .map_err(|_| corrupt)?;
        if k == d && used as u64 != range.end - range.start {
            return Err(corrupt);
        }
        Ok(k)
    }

    /// Decodes from `nodes.start` into `out` while the lists fit in `limit` entries,
    /// each record checked against the index; returns where it stopped.
    fn decode_into(
        &self,
        nodes: Range<u32>,
        limit: usize,
        prefix: usize,
        out: &mut [u32],
        ends: &mut Ends,
    ) -> Result<Done, DecodeError> {
        let job = Job {
            data: self.data,
            base: self.base,
            entries: self.index.entry_offsets(),
            bytes: self.index.byte_offsets(),
            nodes,
            limit,
            prefix,
        };
        self.decoder.run(job, out, ends)
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
    /// # Untrusted input
    ///
    /// Every record is checked to end where the index says, but a damaged or hostile
    /// file can still hold records that decode to lists that are not increasing or that
    /// name targets `>= nodes()`. Code that indexes arrays with decoded targets checks
    /// them first (#44).
    ///
    /// # Errors
    ///
    /// [`DecodeError`] if a record in the range is corrupt; the batch is then empty.
    ///
    /// # Panics
    ///
    /// If `nodes` is not a range within [`nodes`](Self::nodes), or the batch does not fit in
    /// memory on this target.
    pub fn decode_nodes(
        &self,
        nodes: Range<u32>,
        max_entries: usize,
        batch: &mut Batch,
    ) -> Result<u32, DecodeError> {
        self.decode_prefixes(nodes, usize::MAX, max_entries, batch)
    }

    /// As [`decode_nodes`](Self::decode_nodes), but of each list only its first `prefix`
    /// entries (all of them if it has fewer): list `i` of the batch is then that prefix,
    /// and `max_entries` counts prefix entries. A reader that needs only the start of
    /// each list does not decode the rest (bottom-up BFS stops at the first parent).
    ///
    /// A record decoded in full is checked to end where the index says; a prefix is not,
    /// so a damaged byte after it goes unseen.
    ///
    /// # Errors
    ///
    /// As [`decode_nodes`](Self::decode_nodes).
    ///
    /// # Panics
    ///
    /// As [`decode_nodes`](Self::decode_nodes).
    pub fn decode_prefixes(
        &self,
        nodes: Range<u32>,
        prefix: usize,
        max_entries: usize,
        batch: &mut Batch,
    ) -> Result<u32, DecodeError> {
        assert!(
            self.first <= nodes.start && nodes.start <= nodes.end && nodes.end <= self.end,
            "nodes {nodes:?} outside {:?}",
            self.first..self.end
        );
        batch.first = nodes.start;
        batch.clear();
        if nodes.is_empty() {
            return Ok(nodes.end);
        }
        grow(
            &mut batch.targets,
            self.prefix_entries(nodes.clone(), prefix, max_entries),
        );
        let (targets, ends) = (&mut batch.targets, &mut batch.ends);
        match self.decode_into(nodes, max_entries, prefix, targets, ends) {
            Ok(done) => {
                batch.entries = done.entries;
                Ok(done.node)
            }
            Err(e) => {
                batch.ends.entries.clear();
                Err(e)
            }
        }
    }

    /// The length [`decode_nodes`](Self::decode_nodes) with these arguments grows a
    /// batch's buffer to, at most: the entries it may decode (the first list always, then
    /// lists up to `max_entries`), and the decoders' slack. For [`Batch::reserve`].
    ///
    /// # Panics
    ///
    /// If `nodes` is not a range within `0..nodes()`.
    #[must_use]
    pub fn batch_entries(&self, nodes: Range<u32>, max_entries: usize) -> usize {
        if nodes.is_empty() {
            return 0;
        }
        let ef = self.index.entry_offsets();
        let (start, end) = (to_usize(nodes.start.into()), to_usize(nodes.end.into()));
        let (first, second) = ef.get_pair(start);
        let span = to_usize(ef.get(end) - first);
        max_entries.max(to_usize(second - first)).min(span) + SLACK
    }

    /// As [`batch_entries`](Self::batch_entries), for
    /// [`decode_prefixes`](Self::decode_prefixes) with `prefix`: no more than the first
    /// list's prefix or `max_entries`.
    ///
    /// # Panics
    ///
    /// If `nodes` is not a range within `0..nodes()`.
    #[must_use]
    pub fn prefix_entries(&self, nodes: Range<u32>, prefix: usize, max_entries: usize) -> usize {
        let all = self.batch_entries(nodes, max_entries);
        all.min(max_entries.max(prefix).saturating_add(SLACK))
    }

    /// Decodes the whole adjacency into a raw CSR: `offsets` (`nodes + 1` entries) and
    /// `targets` (`entries`), which the caller allocates (against its memory budget).
    /// Runs in parallel over blocks.
    ///
    /// # Untrusted input
    ///
    /// Every record is checked to end where the index says, but a damaged or hostile
    /// file can still hold records that decode to lists that are not increasing or that
    /// name targets `>= nodes()`. Code that indexes arrays with decoded targets checks
    /// them first (#44).
    ///
    /// # Errors
    ///
    /// [`DecodeError`] for the first corrupt block, in node order.
    ///
    /// # Panics
    ///
    /// If the two slices do not have those lengths, or the adjacency is a
    /// [`window`](Self::window) of some of the nodes.
    pub fn decode_all(&self, offsets: &mut [u64], targets: &mut [u32]) -> Result<(), DecodeError> {
        let index = self.index;
        assert!(
            self.first == 0 && self.end == index.nodes(),
            "decode_all of a window"
        );
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
                    .decode_into(
                        nodes.clone(),
                        usize::MAX,
                        usize::MAX,
                        out,
                        &mut Ends::default(),
                    )
                    .map(|_| ());
            }
        });
        pieces.into_iter().try_for_each(|p| p.2)
    }
}
