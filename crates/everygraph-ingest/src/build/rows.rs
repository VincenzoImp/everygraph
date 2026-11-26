// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The input, block by block: its keys for the identifier dictionary, then its rows as
//! pairs of node ids, then the items each row sends to the lists of its ends.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};

use everygraph_core::{ByteSource, Memory, MemoryError, MemoryKind, Reservation, par};

use super::{BuildError, GraphOptions, ONE, Phase, Progress, SelfLoops};
use crate::ids::{ColumnType, Dictionary, DictionaryError, Key, KeySource};
use crate::text::{Batch, IdType, Ids, Tally, TextReader};

/// The input as the dictionary reads it: the source and target keys of every row, in
/// input order. The position of a key is `(block << 32) | (2 * row + column)`, which
/// increases along the input because a block of 4 MiB holds fewer than 2^31 rows. It
/// keeps each block's tally, and reports the first full read as the `reading` phase.
pub(super) struct TextKeys<'r, S> {
    reader: &'r TextReader<S>,
    blocks: usize,
    tallies: Mutex<Vec<Option<Tally>>>,
    first_reads: AtomicUsize,
    progress: &'r dyn Progress,
}

impl<'r, S: ByteSource + Sync> TextKeys<'r, S> {
    pub(super) fn new(
        reader: &'r TextReader<S>,
        blocks: usize,
        progress: &'r dyn Progress,
    ) -> Self {
        Self {
            reader,
            blocks,
            tallies: Mutex::new((0..blocks).map(|_| None).collect()),
            first_reads: AtomicUsize::new(0),
            progress,
        }
    }

    /// The tally of every block, in block order.
    pub(super) fn into_tallies(self) -> Vec<Tally> {
        let tallies = self
            .tallies
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner);
        tallies.into_iter().flatten().collect()
    }
}

impl<S: ByteSource + Sync> KeySource for TextKeys<'_, S> {
    fn column(&self) -> ColumnType {
        match self.reader.layout().ids {
            IdType::String => ColumnType::Utf8,
            _ => ColumnType::UInt { bits: 64 },
        }
    }

    fn chunks(&self) -> usize {
        self.blocks
    }

    fn read_chunk(
        &self,
        chunk: usize,
        f: &mut dyn FnMut(u64, Key<'_>) -> Result<(), DictionaryError>,
    ) -> Result<(), DictionaryError> {
        let (batch, tally) = self
            .reader
            .read_block(chunk)
            .map_err(|e| DictionaryError::Input(Box::new(e)))?;
        let base = u64::try_from(chunk).unwrap_or(u64::MAX) << 32;
        for row in 0..batch.len() {
            let at = base | (u64::try_from(row).unwrap_or(u64::MAX) << 1);
            f(at, key(&batch.source, row))?;
            f(at | 1, key(&batch.target, row))?;
        }
        let first = {
            let mut tallies = self.tallies.lock().unwrap_or_else(PoisonError::into_inner);
            let first = tallies[chunk].is_none();
            tallies[chunk] = Some(tally);
            first
        };
        if first {
            self.progress.advance(Phase::Reading, 1);
            if self.first_reads.fetch_add(1, Ordering::Relaxed) + 1 == self.blocks {
                self.progress.begin(Phase::Identifiers, None);
            }
        }
        Ok(())
    }
}

fn key(ids: &Ids, row: usize) -> Key<'_> {
    match ids {
        Ids::Integer(values) => Key::UInt(values[row]),
        Ids::String(values) => Key::Bytes(values.get(row).unwrap_or_default()),
    }
}

/// What one pass over a block counted.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct BlockCounts {
    pub(super) rows: u64,
    /// Rows whose source or target is not of the dictionary's kind (left out).
    pub(super) rejected: u64,
    /// Loop rows, dropped under `self_loops="drop"`.
    pub(super) dropped_loops: u64,
}

/// The rows of the input as pairs of node ids, block by block.
pub(super) struct Rows<'a, 'd, S, D> {
    reader: &'a TextReader<S>,
    dictionary: &'a Dictionary<'d, D>,
    graph: GraphOptions,
    /// A digest of each block's ids, taken in the first pass and compared in the
    /// others: a block that changes between passes would send other items.
    fingerprints: Vec<AtomicU64>,
    /// Parse and mapping buffers of the blocks in flight, which live outside every
    /// other reservation.
    _buffers: Reservation,
    /// The mapped pairs of each block, kept by the first pass when the budget holds
    /// them, so that the next passes neither parse nor look up again; with what they
    /// hold reserved.
    cache: Option<(Vec<OnceLock<Vec<u64>>>, Reservation)>,
}

/// Bytes the cache of [`Rows`] holds per row.
pub(super) const CACHE_BYTES_PER_ROW: u64 = 8;

/// Memory one block in flight holds per row: two ids (or two string ends) in its batch
/// and a mapped pair. Besides, it holds its text twice: the reader's chunk and the
/// bytes of string ids.
const BLOCK_BYTES_PER_ROW: u64 = 16 + 8;

impl<'a, 'd, S: ByteSource + Sync, D: ByteSource + Sync> Rows<'a, 'd, S, D> {
    pub(super) fn new(
        reader: &'a TextReader<S>,
        dictionary: &'a Dictionary<'d, D>,
        graph: GraphOptions,
        blocks: usize,
        memory: &Memory,
        threads: usize,
        block_bytes: u64,
        most_rows: u64,
        cached_rows: Option<u64>,
    ) -> Result<Self, MemoryError> {
        let per_block = 2 * block_bytes + most_rows.saturating_mul(BLOCK_BYTES_PER_ROW);
        let bytes = per_block.saturating_mul(u64::try_from(threads).unwrap_or(u64::MAX));
        let buffers = memory.reserve("text blocks in flight", MemoryKind::Anonymous, bytes)?;
        let cache = match cached_rows {
            Some(rows) => {
                let bytes = rows.saturating_mul(CACHE_BYTES_PER_ROW);
                let reservation = memory.reserve("mapped rows", MemoryKind::Anonymous, bytes)?;
                Some(((0..blocks).map(|_| OnceLock::new()).collect(), reservation))
            }
            None => None,
        };
        Ok(Self {
            reader,
            dictionary,
            graph,
            fingerprints: (0..blocks).map(|_| AtomicU64::new(0)).collect(),
            _buffers: buffers,
            cache,
        })
    }

    /// Frees the mapped pairs kept by the first pass.
    pub(super) fn release_cache(&mut self) {
        self.cache = None;
    }

    /// Number of blocks.
    pub(super) fn blocks(&self) -> usize {
        self.fingerprints.len()
    }

    /// Calls `f(source, target)` for every item of block `block`: one per loop row kept,
    /// one per other row on a directed graph, and one in each direction on an
    /// undirected one, in input order.
    pub(super) fn items(
        &self,
        block: usize,
        f: impl FnMut(u32, u32),
    ) -> Result<BlockCounts, BuildError> {
        let cached = self.cache.as_ref().map(|(blocks, _)| &blocks[block]);
        if let Some(pairs) = cached.and_then(OnceLock::get) {
            let mut counts = BlockCounts::default();
            self.send(pairs, &mut counts, f);
            return Ok(counts);
        }
        let (batch, _) = self.reader.read_block(block)?;
        let pairs = self.map(&batch)?;
        let mut digest = (u64::try_from(pairs.len()).unwrap_or(u64::MAX) + 1)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15);
        for &pair in &pairs {
            digest = (digest.rotate_left(23) ^ pair).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        }
        let digest = digest | 1;
        let seen = &self.fingerprints[block];
        match seen.compare_exchange(0, digest, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => {}
            Err(earlier) if earlier == digest => {}
            Err(_) => return Err(BuildError::InputChanged),
        }
        let mut counts = BlockCounts {
            rows: u64::try_from(batch.len()).unwrap_or(u64::MAX),
            ..BlockCounts::default()
        };
        counts.rejected = counts.rows - u64::try_from(pairs.len()).unwrap_or(u64::MAX);
        self.send(&pairs, &mut counts, f);
        if let Some(slot) = cached {
            let _ = slot.set(pairs);
        }
        Ok(counts)
    }

    fn send(&self, pairs: &[u64], counts: &mut BlockCounts, mut f: impl FnMut(u32, u32)) {
        for &pair in pairs {
            let (u, v) = (high(pair), low(pair));
            if u == v {
                match self.graph.self_loops {
                    SelfLoops::Keep => f(u, u),
                    SelfLoops::Drop => counts.dropped_loops += 1,
                }
            } else {
                f(u, v);
                if !self.graph.directed {
                    f(v, u);
                }
            }
        }
    }

    /// The rows of `batch` whose two keys are in the dictionary, as `source << 32 | target`.
    fn map(&self, batch: &Batch) -> Result<Vec<u64>, BuildError> {
        let mut pairs = Vec::with_capacity(batch.len());
        for row in 0..batch.len() {
            let source = self.dictionary.id_of_member(key(&batch.source, row))?;
            let target = self.dictionary.id_of_member(key(&batch.target, row))?;
            if let (Some(u), Some(v)) = (source, target) {
                pairs.push(u64::from(u.get()) << 32 | u64::from(v.get()));
            }
        }
        Ok(pairs)
    }
}

pub(super) fn high(x: u64) -> u32 {
    u32::try_from(x >> 32).expect("32 bits")
}

pub(super) fn low(x: u64) -> u32 {
    u32::try_from(x & u64::from(u32::MAX)).expect("32 bits")
}

/// Runs `f` on every block, in parallel, and adds up what it counted; the first error
/// in block order is returned.
pub(super) fn each_block(
    blocks: usize,
    f: impl Fn(usize) -> Result<BlockCounts, BuildError> + Send + Sync,
) -> Result<BlockCounts, BuildError> {
    let results = par::map_ranges(blocks, ONE, |range| f(range.start));
    let mut total = BlockCounts::default();
    for result in results {
        let counts = result?;
        total.rows += counts.rows;
        total.rejected += counts.rejected;
        total.dropped_loops += counts.dropped_loops;
    }
    Ok(total)
}

/// The number of items of each node, its list's length before pairs are merged, and
/// on a directed graph whether it is the target of an item.
pub(super) struct ItemCounts {
    segments: Vec<Vec<AtomicU32>>,
    targets: Vec<AtomicU64>,
    overflow: AtomicBool,
    _reservation: Reservation,
}

/// Counts per segment of [`ItemCounts`]: 1 GiB of `u32`.
const COUNT_SEGMENT: usize = 1 << 28;

impl ItemCounts {
    pub(super) fn new(memory: &Memory, nodes: u32, directed: bool) -> Result<Self, MemoryError> {
        let n = usize::try_from(nodes).expect("usize has at least 32 bits");
        let words = if directed { n.div_ceil(64) } else { 0 };
        let bytes = u64::from(nodes) * 4 + u64::try_from(words).unwrap_or(u64::MAX) * 8;
        let reservation = memory.reserve("item counts", MemoryKind::Anonymous, bytes)?;
        let failed = || MemoryError::AllocationFailed {
            purpose: "item counts",
            bytes,
        };
        let mut segments = Vec::new();
        segments
            .try_reserve_exact(n.div_ceil(COUNT_SEGMENT))
            .map_err(|_| failed())?;
        for start in (0..n).step_by(COUNT_SEGMENT) {
            let len = COUNT_SEGMENT.min(n - start);
            let mut segment = Vec::new();
            segment.try_reserve_exact(len).map_err(|_| failed())?;
            segment.resize_with(len, || AtomicU32::new(0));
            segments.push(segment);
        }
        let mut targets = Vec::new();
        targets.try_reserve_exact(words).map_err(|_| failed())?;
        targets.resize_with(words, || AtomicU64::new(0));
        Ok(Self {
            segments,
            targets,
            overflow: AtomicBool::new(false),
            _reservation: reservation,
        })
    }

    fn cell(&self, v: u32) -> &AtomicU32 {
        let v = usize::try_from(v).expect("usize has at least 32 bits");
        &self.segments[v / COUNT_SEGMENT][v % COUNT_SEGMENT]
    }

    pub(super) fn add(&self, v: u32) {
        if self.cell(v).fetch_add(1, Ordering::Relaxed) == u32::MAX {
            self.overflow.store(true, Ordering::Relaxed);
        }
    }

    /// Bytes held.
    pub(super) fn bytes(&self) -> u64 {
        self._reservation.bytes()
    }

    /// Marks `v` as the target of an item (directed graphs only).
    pub(super) fn target(&self, v: u32) {
        let v = usize::try_from(v).expect("usize has at least 32 bits");
        self.targets[v / 64].fetch_or(1 << (v % 64), Ordering::Relaxed);
    }

    /// Nodes that no item starts or, on a directed graph, ends at.
    pub(super) fn isolates(&self) -> u64 {
        let mut isolates = 0;
        for (v, c) in self.segments.iter().flatten().enumerate() {
            let target = self
                .targets
                .get(v / 64)
                .is_some_and(|w| w.load(Ordering::Relaxed) >> (v % 64) & 1 == 1);
            isolates += u64::from(c.load(Ordering::Relaxed) == 0 && !target);
        }
        isolates
    }

    /// The counts in node order; `None` if a node has 2^32 items or more.
    pub(super) fn values(&self) -> Option<impl Iterator<Item = u32> + '_> {
        (!self.overflow.load(Ordering::Relaxed)).then(|| {
            self.segments
                .iter()
                .flatten()
                .map(|c| c.load(Ordering::Relaxed))
        })
    }
}
