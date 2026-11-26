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
/// Flag bit 1: the adjacency has a `.events` section (`docs/spec/eg-adjacency.md` § 3.1).
/// Bit 0 is reserved for segments and refused, as every other bit.
pub(crate) const FLAG_EVENTS: u32 = 1 << 1;

/// Nodes per task when the index is checked in parallel.
const CHECK_GRAIN: NonZeroUsize = NonZeroUsize::new(1 << 16).expect("non-zero");

/// The in-memory index of a compressed adjacency: everything but the records.
///
/// Built by [`AdjacencyIndex::parse`] from the index bytes written by
/// [`encode`](super::encode). It answers degrees and entry offsets without touching the
/// records, and tells where each node's record lies.
#[derive(Clone, Debug)]
pub struct AdjacencyIndex {
    events: bool,
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
        events: bool,
        nodes: u32,
        entries: u64,
        data_bytes: u64,
        block_starts: Vec<u32>,
        byte_offsets: EliasFano,
        entry_offsets: EliasFano,
    ) -> Self {
        Self {
            events,
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
        let parts = part_lengths(
            u64::from(self.nodes),
            self.entries,
            self.data_bytes,
            blocks as u64,
        )
        .map_or(0, |(_, lengths)| lengths.iter().sum::<usize>());
        let mut out = Vec::with_capacity(HEADER_BYTES + parts);
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        let flags = if self.events { FLAG_EVENTS } else { 0 };
        out.extend_from_slice(&flags.to_le_bytes());
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
        if flags & !FLAG_EVENTS != 0 {
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
            flags == FLAG_EVENTS,
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

    /// Whether the adjacency has a `.events` section (flag bit 1): some pair has more
    /// than one stored event.
    #[must_use]
    pub fn has_events(&self) -> bool {
        self.events
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
    /// [`entries`](Self::entries)). Per-pair columns are indexed by it; per-event
    /// columns by the event offset of [`AdjacencyEvents`](super::AdjacencyEvents),
    /// which is the same when the adjacency has no `.events` section.
    ///
    /// # Panics
    ///
    /// If `v` is above [`nodes`](Self::nodes).
    #[must_use]
    pub fn entry_offset(&self, v: NodeId) -> EdgeOffset {
        EdgeOffset::new(self.entry_offsets.get(v.index()))
    }

    /// The degrees of `nodes`, in order, from one sequential pass over the entry
    /// offsets: a few nanoseconds a node, where [`degree`](Self::degree) pays a select
    /// for each.
    ///
    /// # Panics
    ///
    /// If `nodes` is not a range within `0..nodes()`.
    pub fn degrees(&self, nodes: Range<u32>) -> impl Iterator<Item = u32> + '_ {
        assert!(
            nodes.start <= nodes.end && nodes.end <= self.nodes,
            "nodes {nodes:?} outside 0..{}",
            self.nodes
        );
        let count = nodes.len();
        let mut offsets = self
            .entry_offsets
            .iter_from(usize::try_from(nodes.start).expect("usize holds u32"));
        let mut prev = offsets.next().unwrap_or(0);
        offsets.take(count).map(move |next| {
            let d = next - prev;
            prev = next;
            u32::try_from(d).expect("checked by parse")
        })
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

    /// The bytes of the data part that hold the records of `nodes`.
    ///
    /// # Panics
    ///
    /// If `nodes` is not a range within `0..nodes()`.
    #[must_use]
    pub fn data_range(&self, nodes: Range<u32>) -> Range<u64> {
        assert!(nodes.start <= nodes.end, "nodes {nodes:?}");
        let at = |v: u32| self.byte_offsets.get(usize::try_from(v).expect("32 bits"));
        at(nodes.start)..at(nodes.end)
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

#[cfg(test)]
mod tests {
    //! Hostile indexes: each breaks one rule of `parse`, and each
    //! must be refused. The good index is built field by field, so that the header
    //! may disagree with the parts.

    use super::*;

    /// An index as fields; `bytes()` lays them out as the specification says.
    #[derive(Clone)]
    struct Raw {
        magic: [u8; 8],
        version: u32,
        flags: u32,
        nodes: u64,
        entries: u64,
        data_bytes: u64,
        blocks: u64,
        starts: Vec<u32>,
        codecs: Vec<u8>,
        byte_offsets: Vec<u64>,
        entry_offsets: Vec<u64>,
    }

    /// Three nodes with degrees 4, 0, 4: records of 5 bytes each; `l` is 1 in both
    /// sequences, so a low bit can make one decrease.
    fn good() -> Raw {
        Raw {
            magic: MAGIC,
            version: VERSION,
            flags: 0,
            nodes: 3,
            entries: 8,
            data_bytes: 10,
            blocks: 1,
            starts: vec![0, 3],
            codecs: vec![0],
            byte_offsets: vec![0, 5, 5, 10],
            entry_offsets: vec![0, 4, 4, 8],
        }
    }

    impl Raw {
        fn bytes(&self) -> Vec<u8> {
            let mut out = Vec::new();
            out.extend_from_slice(&self.magic);
            out.extend_from_slice(&self.version.to_le_bytes());
            out.extend_from_slice(&self.flags.to_le_bytes());
            for x in [self.nodes, self.entries, self.data_bytes, self.blocks] {
                out.extend_from_slice(&x.to_le_bytes());
            }
            for s in &self.starts {
                out.extend_from_slice(&s.to_le_bytes());
            }
            out.resize(pad8(out.len()), 0);
            out.extend_from_slice(&self.codecs);
            out.resize(pad8(out.len()), 0);
            EliasFano::with_universe(&self.byte_offsets, self.data_bytes).write(&mut out);
            EliasFano::with_universe(&self.entry_offsets, self.entries).write(&mut out);
            out
        }

        fn with(&self, change: impl FnOnce(&mut Self)) -> Vec<u8> {
            let mut raw = self.clone();
            change(&mut raw);
            raw.bytes()
        }
    }

    fn refused(bytes: &[u8]) -> FormatError {
        AdjacencyIndex::parse(bytes).expect_err("a hostile index must be refused")
    }

    #[test]
    fn the_good_index_is_what_the_encoder_writes() {
        let encoded = super::super::encode(&[0, 4, 4, 8], &[1, 2, 3, 4, 0, 1, 3, 5]).unwrap();
        assert_eq!(encoded.data.len(), 10);
        assert_eq!(good().bytes(), encoded.index);
        assert!(AdjacencyIndex::parse(&good().bytes()).is_ok());
    }

    #[test]
    fn hostile_headers_are_refused() {
        let g = good();
        let b = g.bytes();
        assert_eq!(refused(&b[..HEADER_BYTES - 1]), FormatError::Truncated);
        assert_eq!(refused(&g.with(|r| r.magic[0] ^= 1)), FormatError::BadMagic);
        assert_eq!(
            refused(&g.with(|r| r.version = 2)),
            FormatError::UnsupportedVersion(2)
        );
        for flags in [1, 3, 4, 1 << 31] {
            assert_eq!(
                refused(&g.with(|r| r.flags = flags)),
                FormatError::UnsupportedFlags(flags)
            );
        }
        let events = AdjacencyIndex::parse(&g.with(|r| r.flags = FLAG_EVENTS)).unwrap();
        assert!(events.has_events());
        assert_eq!(events.write(), g.with(|r| r.flags = FLAG_EVENTS));
        let mut longer = b.clone();
        longer.extend_from_slice(&[0; 8]);
        assert!(matches!(refused(&longer), FormatError::Length { .. }));
        assert!(matches!(
            refused(&b[..b.len() - 8]),
            FormatError::Length { .. }
        ));
        // 2^32 nodes; 2^32 - 1 nodes with the body of three.
        assert!(matches!(
            refused(&g.with(|r| r.nodes = 1 << 32)),
            FormatError::Invalid(_)
        ));
        assert!(matches!(
            refused(&g.with(|r| r.nodes = u64::from(u32::MAX))),
            FormatError::Length { .. }
        ));
        // Totals at u64::MAX: the parts no longer have the lengths of the body.
        let either =
            |e: FormatError| matches!(e, FormatError::Length { .. } | FormatError::Invalid(_));
        assert!(either(refused(&g.with(|r| r.entries = u64::MAX))));
        assert!(either(refused(&g.with(|r| r.data_bytes = u64::MAX))));
    }

    #[test]
    fn hostile_block_directories_are_refused() {
        let g = good();
        let invalid = |b: Vec<u8>| assert!(matches!(refused(&b), FormatError::Invalid(_)));
        invalid(g.with(|r| r.blocks = 0));
        invalid(g.with(|r| r.blocks = 4));
        invalid(g.with(|r| r.blocks = u64::MAX));
        invalid(g.with(|r| r.starts = vec![1, 3]));
        invalid(g.with(|r| r.starts = vec![0, 2]));
        invalid(g.with(|r| {
            r.blocks = 3;
            r.starts = vec![0, 1, 1, 3];
            r.codecs = vec![0; 3];
        }));
        invalid(g.with(|r| {
            r.blocks = 3;
            r.starts = vec![0, 2, 1, 3];
            r.codecs = vec![0; 3];
        }));
        assert_eq!(
            refused(&g.with(|r| r.codecs = vec![1])),
            FormatError::UnsupportedBlockCodec { block: 0, codec: 1 }
        );
        // Padding of the codecs (byte 1 of their 8), and of the starts with two blocks
        // (bytes 12..16 of their 16).
        let codecs_at = HEADER_BYTES + 8;
        let mut b = g.bytes();
        b[codecs_at + 1] = 1;
        invalid(b);
        let two = g.with(|r| {
            r.blocks = 2;
            r.starts = vec![0, 1, 3];
            r.codecs = vec![0; 2];
        });
        assert!(AdjacencyIndex::parse(&two).is_ok());
        let mut b = two;
        b[HEADER_BYTES + 12] = 1;
        invalid(b);
    }

    #[test]
    fn hostile_offsets_are_refused() {
        let g = good();
        let invalid = |b: Vec<u8>| assert!(matches!(refused(&b), FormatError::Invalid(_)));
        // Elias–Fano structure: the entry offsets are the last 16 bytes (one low word,
        // one high word). Their high bits are at 0, 3, 4 and 7; bit 1 is a fifth one,
        // bit 63 is padding.
        let b = g.bytes();
        let high = b.len() - 8;
        let mut extra_one = b.clone();
        extra_one[high] |= 1 << 1;
        invalid(extra_one);
        let mut padding = b.clone();
        padding[high + 7] |= 1 << 7;
        invalid(padding);
        // Values: not starting at 0, not ending at the total, decreasing (a low bit:
        // 5 then 4 keeps every high bit, so the ones count is right).
        invalid(g.with(|r| r.entry_offsets = vec![1, 4, 4, 8]));
        invalid(g.with(|r| {
            r.data_bytes = 11;
            r.byte_offsets = vec![0, 5, 5, 10];
        }));
        invalid(g.with(|r| r.byte_offsets = vec![0, 5, 4, 10]));
        invalid(g.with(|r| r.entry_offsets = vec![0, 5, 4, 8]));
        // Record lengths against degrees: each bound on its own.
        // Too short: 4 bytes for degree 4 (at least 1 + 4); node 2 stays possible (6).
        invalid(g.with(|r| r.byte_offsets = vec![0, 4, 4, 10]));
        // Too long: degrees 1, 0, 2; 6 bytes for degree 1 (at most 1 + 4); node 2 has 3.
        invalid(g.with(|r| {
            r.entries = 3;
            r.entry_offsets = vec![0, 1, 1, 3];
            r.data_bytes = 9;
            r.byte_offsets = vec![0, 6, 6, 9];
        }));
        // A record for an empty list: node 1 has degree 0 and 1 byte.
        invalid(g.with(|r| {
            r.data_bytes = 11;
            r.byte_offsets = vec![0, 5, 6, 11];
        }));
        // A degree of 2^33, with a record length that would fit it.
        invalid(
            Raw {
                nodes: 1,
                entries: 1 << 33,
                data_bytes: (1 << 31) + (1 << 33),
                starts: vec![0, 1],
                byte_offsets: vec![0, (1 << 31) + (1 << 33)],
                entry_offsets: vec![0, 1 << 33],
                ..good()
            }
            .bytes(),
        );
    }

    #[test]
    fn the_length_bounds_of_a_record_are_exact() {
        // Degree 4: 5 to 17 bytes are possible, 4 and 18 are not.
        let at = |len: u64| {
            Raw {
                nodes: 1,
                entries: 4,
                data_bytes: len,
                starts: vec![0, 1],
                byte_offsets: vec![0, len],
                entry_offsets: vec![0, 4],
                ..good()
            }
            .bytes()
        };
        for len in [5, 17] {
            assert!(AdjacencyIndex::parse(&at(len)).is_ok(), "{len}");
        }
        for len in [4, 18] {
            assert!(
                matches!(refused(&at(len)), FormatError::Invalid(_)),
                "{len}"
            );
        }
    }
}
