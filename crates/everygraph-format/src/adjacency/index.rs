// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The index part of an adjacency (`docs/spec/eg-adjacency.md`, "Index"): header, block
//! directory, and the two Elias–Fano sequences.

use std::num::NonZeroUsize;
use std::ops::Range;

use everygraph_core::{EdgeOffset, NodeId, par};

use super::FormatError;
use super::ef::{self, EliasFano};

pub(crate) const MAGIC: [u8; 8] = *b"EGADJIDX";
pub(crate) const VERSION: u32 = 1;
pub(crate) const HEADER_BYTES: usize = 48;
/// The only block codec of version 1: records as written by this crate.
pub(crate) const CODEC_RECORDS: u8 = 0;

/// Nodes per task when the index is checked in parallel.
const CHECK_GRAIN: NonZeroUsize = NonZeroUsize::new(1 << 16).expect("non-zero");

/// The in-memory index of a compressed adjacency: everything but the records.
///
/// Built by [`AdjacencyIndex::parse`] from the index bytes written by
/// [`encode`](super::encode). It answers degrees and entry offsets without touching the
/// records, and tells where each node's record lies.
#[derive(Clone, Debug)]
pub struct AdjacencyIndex {
    nodes: u32,
    entries: u64,
    data_bytes: u64,
    /// `blocks + 1` first nodes, the last one `nodes`.
    block_starts: Vec<u32>,
    /// `nodes + 1` byte offsets of the records in the data part.
    byte_offsets: EliasFano,
    /// `nodes + 1` entry offsets (cumulative degrees).
    entry_offsets: EliasFano,
}

fn pad8(n: usize) -> usize {
    n.div_ceil(8) * 8
}

fn read_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("four bytes"))
}

fn read_u64(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().expect("eight bytes"))
}

/// The block count and the byte lengths of the four parts after the header, or `None`
/// if they do not fit in memory here.
fn part_lengths(
    nodes: u64,
    entries: u64,
    data_bytes: u64,
    blocks: u64,
) -> Option<(usize, [usize; 4])> {
    let blocks = usize::try_from(blocks).ok()?;
    let starts = pad8(blocks.checked_add(1)?.checked_mul(4)?);
    let codecs = pad8(blocks);
    let count = nodes + 1;
    let (lo, hi) = ef::stored_words(count, data_bytes)?;
    let bytes_ef = lo.checked_add(hi)?.checked_mul(8)?;
    let (lo, hi) = ef::stored_words(count, entries)?;
    let entries_ef = lo.checked_add(hi)?.checked_mul(8)?;
    Some((blocks, [starts, codecs, bytes_ef, entries_ef]))
}

impl AdjacencyIndex {
    pub(crate) fn from_parts(
        nodes: u32,
        entries: u64,
        data_bytes: u64,
        block_starts: Vec<u32>,
        byte_offsets: EliasFano,
        entry_offsets: EliasFano,
    ) -> Self {
        Self {
            nodes,
            entries,
            data_bytes,
            block_starts,
            byte_offsets,
            entry_offsets,
        }
    }

    /// The index bytes, as [`parse`](Self::parse) reads them.
    pub(crate) fn write(&self) -> Vec<u8> {
        let blocks = self.blocks();
        let mut out = Vec::new();
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&u64::from(self.nodes).to_le_bytes());
        out.extend_from_slice(&self.entries.to_le_bytes());
        out.extend_from_slice(&self.data_bytes.to_le_bytes());
        out.extend_from_slice(&(blocks as u64).to_le_bytes());
        for s in &self.block_starts {
            out.extend_from_slice(&s.to_le_bytes());
        }
        out.resize(pad8(out.len()), 0);
        out.resize(pad8(out.len() + blocks), CODEC_RECORDS);
        self.byte_offsets.write(&mut out);
        self.entry_offsets.write(&mut out);
        out
    }

    /// Reads and checks an index.
    ///
    /// Everything that later lookups rely on is checked here, so that they cannot fail:
    /// the header, the block directory, the structure of both Elias–Fano sequences,
    /// that both offsets start at 0, never decrease and end at the totals, and that
    /// each node's record length is possible for its degree (zero for an empty list;
    /// between `⌈d/4⌉ + d` and `⌈d/4⌉ + 4d` bytes otherwise). The records themselves are
    /// checked as they are decoded.
    ///
    /// # Errors
    ///
    /// A [`FormatError`] naming the first problem found.
    pub fn parse(bytes: &[u8]) -> Result<Self, FormatError> {
        let header = bytes.get(..HEADER_BYTES).ok_or(FormatError::Truncated)?;
        if header[..8] != MAGIC {
            return Err(FormatError::BadMagic);
        }
        let version = read_u32(header, 8);
        if version != VERSION {
            return Err(FormatError::UnsupportedVersion(version));
        }
        let flags = read_u32(header, 12);
        if flags != 0 {
            return Err(FormatError::UnsupportedFlags(flags));
        }
        let (nodes, entries, data_bytes, blocks) = (
            read_u64(header, 16),
            read_u64(header, 24),
            read_u64(header, 32),
            read_u64(header, 40),
        );
        let nodes =
            u32::try_from(nodes).map_err(|_| FormatError::Invalid("more than 2^32 - 1 nodes"))?;
        if blocks > u64::from(nodes) || (nodes > 0) != (blocks > 0) {
            return Err(FormatError::Invalid("block count"));
        }
        let (blocks, lengths) = part_lengths(u64::from(nodes), entries, data_bytes, blocks)
            .ok_or(FormatError::Invalid("index too large for this target"))?;
        let expected = lengths
            .iter()
            .try_fold(HEADER_BYTES, |a, &b| a.checked_add(b));
        if expected != Some(bytes.len()) {
            return Err(FormatError::Length {
                expected: expected.map_or(u64::MAX, |e| e as u64),
                actual: bytes.len() as u64,
            });
        }
        let mut at = HEADER_BYTES;
        let mut take = |len: usize| {
            let part = &bytes[at..at + len];
            at += len;
            part
        };
        let (starts, codecs, bytes_ef, entries_ef) = (
            take(lengths[0]),
            take(lengths[1]),
            take(lengths[2]),
            take(lengths[3]),
        );

        let block_starts: Vec<u32> = (0..=blocks).map(|b| read_u32(starts, 4 * b)).collect();
        if starts[4 * (blocks + 1)..].iter().any(|&b| b != 0) {
            return Err(FormatError::Invalid("block directory padding"));
        }
        if block_starts[0] != 0
            || block_starts[blocks] != nodes
            || block_starts.windows(2).any(|w| w[0] >= w[1])
        {
            return Err(FormatError::Invalid("block directory"));
        }
        if let Some(b) = codecs[..blocks].iter().position(|&c| c != CODEC_RECORDS) {
            return Err(FormatError::UnsupportedBlockCodec {
                block: b as u64,
                codec: codecs[b],
            });
        }
        if codecs[blocks..].iter().any(|&b| b != 0) {
            return Err(FormatError::Invalid("block codec padding"));
        }
        let count = u64::from(nodes) + 1;
        let byte_offsets =
            EliasFano::read(bytes_ef, count, data_bytes).map_err(FormatError::Invalid)?;
        let entry_offsets =
            EliasFano::read(entries_ef, count, entries).map_err(FormatError::Invalid)?;
        let index = Self::from_parts(
            nodes,
            entries,
            data_bytes,
            block_starts,
            byte_offsets,
            entry_offsets,
        );
        index.check_offsets()?;
        Ok(index)
    }

    /// The offset checks of [`parse`](Self::parse), in parallel over chunks of nodes.
    fn check_offsets(&self) -> Result<(), FormatError> {
        let n = self.byte_offsets.len() - 1;
        if self.byte_offsets.get(0) != 0 || self.entry_offsets.get(0) != 0 {
            return Err(FormatError::Invalid("offsets do not start at 0"));
        }
        if self.byte_offsets.get(n) != self.data_bytes || self.entry_offsets.get(n) != self.entries
        {
            return Err(FormatError::Invalid("offsets do not end at the totals"));
        }
        let bad = par::map_ranges(n, CHECK_GRAIN, |nodes| {
            let mut bytes = self.byte_offsets.iter_from(nodes.start);
            let mut entries = self.entry_offsets.iter_from(nodes.start);
            let (mut b0, mut e0) = (bytes.next()?, entries.next()?);
            for _ in nodes {
                let (b1, e1) = (bytes.next()?, entries.next()?);
                let (len, d) = (b1.checked_sub(b0)?, e1.checked_sub(e0)?);
                let control = d.div_ceil(4);
                if d > u64::from(u32::MAX) || len < control + d || len > control + 4 * d {
                    return None;
                }
                (b0, e0) = (b1, e1);
            }
            Some(())
        });
        if bad.iter().any(Option::is_none) {
            return Err(FormatError::Invalid(
                "a record length does not fit its degree",
            ));
        }
        Ok(())
    }

    /// Number of nodes (lists).
    #[must_use]
    pub fn nodes(&self) -> u32 {
        self.nodes
    }

    /// Number of adjacency entries (the sum of the degrees).
    #[must_use]
    pub fn entries(&self) -> u64 {
        self.entries
    }

    /// Length in bytes of the data part.
    #[must_use]
    pub fn data_bytes(&self) -> u64 {
        self.data_bytes
    }

    /// Degree of `v`, from the entry offsets alone.
    ///
    /// # Panics
    ///
    /// If `v` is not below [`nodes`](Self::nodes).
    #[must_use]
    pub fn degree(&self, v: NodeId) -> u32 {
        let (a, b) = self.entry_offsets.get_pair(v.index());
        u32::try_from(b - a).expect("checked by parse")
    }

    /// Number of entries before the list of `v`, for `v` in `0..=nodes` (`nodes` gives
    /// [`entries`](Self::entries)). Edge-indexed columns are indexed by it.
    ///
    /// # Panics
    ///
    /// If `v` is above [`nodes`](Self::nodes).
    #[must_use]
    pub fn entry_offset(&self, v: NodeId) -> EdgeOffset {
        EdgeOffset::new(self.entry_offsets.get(v.index()))
    }

    /// The bytes of the data part that hold the record of `v`.
    ///
    /// # Panics
    ///
    /// If `v` is not below [`nodes`](Self::nodes).
    #[must_use]
    pub fn record_range(&self, v: NodeId) -> Range<u64> {
        let (a, b) = self.byte_offsets.get_pair(v.index());
        a..b
    }

    /// Number of blocks: node-aligned runs of lists, the unit of parallel work and of
    /// I/O (about 64 Ki entries each; a list longer than that has a block of its own).
    #[must_use]
    pub fn blocks(&self) -> usize {
        self.block_starts.len() - 1
    }

    /// The nodes of block `b`.
    ///
    /// # Panics
    ///
    /// If `b` is not below [`blocks`](Self::blocks).
    #[must_use]
    pub fn block_nodes(&self, b: usize) -> Range<u32> {
        self.block_starts[b]..self.block_starts[b + 1]
    }

    /// Bytes this index holds in memory.
    #[must_use]
    pub fn heap_bytes(&self) -> usize {
        self.block_starts.len() * 4
            + self.byte_offsets.heap_bytes()
            + self.entry_offsets.heap_bytes()
    }

    pub(crate) fn byte_offsets(&self) -> &EliasFano {
        &self.byte_offsets
    }

    pub(crate) fn entry_offsets(&self) -> &EliasFano {
        &self.entry_offsets
    }
}
