// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The second pass: every item to its bucket. Items of resident buckets are scattered
//! into two columns in memory (`par::scatter`, stable, so the same bytes at any thread
//! count); items of the other buckets are appended to one spill file per round, in
//! chunks of 6-byte records.
//!
//! A chunk is `[bucket u32][count u32]` and `count` records `[local source u16][target
//! u32]`, little-endian; the source is local to its bucket, which holds at most 2^16
//! nodes. Chunks of different blocks interleave in an order that depends on the
//! schedule; a bucket's list order is restored when it is finished, so the output does
//! not depend on it.

use std::io::{self, BufWriter, Write};
use std::num::NonZeroUsize;
use std::sync::{Mutex, PoisonError};

use everygraph_core::{
    ByteSource, Memory, MemoryError, MemoryKind, Reservation, SegColumns, SegVec, par,
};

use super::layout::Layout;
use super::rows::{BlockCounts, Rows, each_block, high, low};
use super::{BuildError, Tuning};
use crate::ids::Scratch;

/// Write buffer in front of each spill file.
pub(super) const SPILL_BUFFER: u64 = 64 << 10;
/// Bytes of a spill record and of a chunk header.
const RECORD: usize = 6;
const CHUNK_HEADER: usize = 8;

/// The items of the resident buckets: targets, and sources local to their bucket.
pub(super) struct Resident {
    pub(super) targets: SegVec<u32>,
    pub(super) locals: SegVec<u16>,
}

/// One spill file per spilled round.
pub(super) struct Spills<W: Write> {
    files: Vec<Mutex<BufWriter<W>>>,
    _buffers: Reservation,
}

impl<W: Write> Spills<W> {
    fn create<Sc: Scratch<Writer = W>>(
        scratch: &Sc,
        memory: &Memory,
        rounds: usize,
        threads: usize,
        batch: usize,
    ) -> Result<Self, BuildError> {
        let per_thread = if rounds == 0 { 0 } else { widen(batch * 8) };
        let bytes = widen(rounds) * SPILL_BUFFER + widen(threads) * per_thread;
        let buffers = memory.reserve("spill buffers", MemoryKind::Anonymous, bytes)?;
        let files = (0..rounds)
            .map(|_| {
                let file = scratch.create("bucket spill")?;
                let capacity = usize::try_from(SPILL_BUFFER).expect("64 KiB");
                Ok(Mutex::new(BufWriter::with_capacity(capacity, file)))
            })
            .collect::<io::Result<_>>()?;
        Ok(Self {
            files,
            _buffers: buffers,
        })
    }

    /// Seals the files, in round order.
    pub(super) fn seal<Sc: Scratch<Writer = W>>(self, scratch: &Sc) -> io::Result<Vec<Sc::Source>> {
        self.files
            .into_iter()
            .map(|file| {
                let file = file
                    .into_inner()
                    .map_err(|_| io::Error::other("a spill writer panicked"))?;
                let file = file.into_inner().map_err(io::IntoInnerError::into_error)?;
                scratch.seal(file)
            })
            .collect()
    }
}

/// Sends every item of the input to its bucket. Returns the resident columns, the
/// spill files and the counts of the pass (rows, dropped loops).
pub(super) fn fill<S, D, Sc>(
    rows: &Rows<'_, '_, S, D>,
    layout: &Layout,
    memory: &Memory,
    scratch: &Sc,
    tuning: &Tuning,
    threads: usize,
) -> Result<(Resident, Vec<Sc::Source>, u64), BuildError>
where
    S: ByteSource + Sync,
    D: ByteSource + Sync,
    Sc: Scratch,
{
    let resident = layout.resident();
    let spilled_rounds = layout.round_count() - layout.resident_rounds;
    let items = usize::try_from(layout.resident_items()).map_err(|_| too_large())?;
    let mut targets = SegVec::with_segment_bits(memory, "resident targets", tuning.segment_bits);
    targets.try_resize(items, 0)?;
    let mut locals = SegVec::with_segment_bits(memory, "resident sources", tuning.segment_bits);
    locals.try_resize(items, 0)?;
    let batch_len = tuning.spill_batch;
    let spills = Spills::create(scratch, memory, spilled_rounds, threads, batch_len)?;
    let spill_bytes = Mutex::new(0_u64);

    // Items of spilled buckets, gathered per block and written in chunks.
    let spill = |batch: &mut Vec<u64>| -> Result<(), BuildError> {
        let bytes = write_spills(batch, layout, &spills)?;
        *spill_bytes.lock().unwrap_or_else(PoisonError::into_inner) += bytes;
        Ok(())
    };
    let new_batch = || {
        let mut batch = Vec::new();
        batch
            .try_reserve_exact(batch_len)
            .map_err(|_| MemoryError::AllocationFailed {
                purpose: "spill buffers",
                bytes: widen(batch_len * 8),
            })?;
        Ok::<_, BuildError>(batch)
    };

    if let Some(width) = NonZeroUsize::new(resident) {
        let blocks = rows.blocks();
        let group = layout.group;
        let emit = |scatter_block: usize, sink: &mut par::Sink<'_, (&mut [u32], &mut [u16])>| {
            let mut batch = if sink.is_counting() || spilled_rounds == 0 {
                Vec::new()
            } else {
                new_batch()?
            };
            let first = scatter_block * group;
            for block in first..blocks.min(first + group) {
                let mut failed = None;
                rows.items(block, |u, v| {
                    let b = layout.bucket_of(u);
                    if b < resident {
                        let local =
                            u16::try_from(u - layout.starts[b]).expect("buckets hold 2^16 nodes");
                        sink.push(b, (v, local));
                    } else if !sink.is_counting() {
                        batch.push(u64::from(u) << 32 | u64::from(v));
                        if batch.len() == batch_len && failed.is_none() {
                            failed = spill(&mut batch).err();
                        }
                    }
                })?;
                if let Some(e) = failed {
                    return Err(e);
                }
            }
            if !batch.is_empty() {
                spill(&mut batch)?;
            }
            Ok(())
        };
        let mut columns = (&mut targets, &mut locals);
        let parts = columns.parts(0..items);
        let offsets = par::scatter(
            memory,
            "scatter table",
            blocks.div_ceil(group),
            width,
            emit,
            parts,
        )?;
        debug_assert!(
            offsets
                .iter()
                .zip(&layout.offsets)
                .all(|(&a, &b)| widen(a) == b)
        );
    } else {
        each_block(rows.blocks(), |block| {
            let mut batch = new_batch()?;
            let mut failed = None;
            rows.items(block, |u, v| {
                batch.push(u64::from(u) << 32 | u64::from(v));
                if batch.len() == batch_len && failed.is_none() {
                    failed = spill(&mut batch).err();
                }
            })?;
            if let Some(e) = failed {
                return Err(e);
            }
            spill(&mut batch)?;
            Ok(BlockCounts::default())
        })?;
    }
    let sources = spills.seal(scratch)?;
    let spilled = spill_bytes
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner);
    Ok((Resident { targets, locals }, sources, spilled))
}

/// Writes `batch` (items `source << 32 | target` of spilled buckets) to the spill files
/// as one chunk per bucket, and empties it. Returns the bytes written.
fn write_spills<W: Write>(
    batch: &mut Vec<u64>,
    layout: &Layout,
    spills: &Spills<W>,
) -> Result<u64, BuildError> {
    batch.sort_unstable();
    let first_spilled_round = layout.resident_rounds;
    let mut written = 0;
    let mut chunk = [0_u8; 4096];
    let mut at = 0;
    while at < batch.len() {
        let b = layout.bucket_of(high(batch[at]));
        let end = at + batch[at..].partition_point(|&x| high(x) < layout.starts[b + 1]);
        let round = layout.rounds.partition_point(|&r| r <= b) - 1;
        let mut file = spills.files[round - first_spilled_round]
            .lock()
            .map_err(|_| io::Error::other("a spill writer panicked"))?;
        let count = u32::try_from(end - at).expect("a batch holds fewer than 2^32 items");
        file.write_all(&u32::try_from(b).map_err(|_| too_large())?.to_le_bytes())?;
        file.write_all(&count.to_le_bytes())?;
        let mut filled = 0;
        for &x in &batch[at..end] {
            let local = u16::try_from(high(x) - layout.starts[b]).expect("buckets hold 2^16 nodes");
            chunk[filled..filled + 2].copy_from_slice(&local.to_le_bytes());
            chunk[filled + 2..filled + RECORD].copy_from_slice(&low(x).to_le_bytes());
            filled += RECORD;
            if filled + RECORD > chunk.len() {
                file.write_all(&chunk[..filled])?;
                filled = 0;
            }
        }
        file.write_all(&chunk[..filled])?;
        written += widen(CHUNK_HEADER + RECORD * (end - at));
        at = end;
    }
    batch.clear();
    Ok(written)
}

/// Loads spilled round `round` from its file into two columns, bucket by bucket.
pub(super) fn load_round<R: ByteSource>(
    source: &R,
    layout: &Layout,
    round: usize,
    memory: &Memory,
    tuning: &Tuning,
) -> Result<Resident, BuildError> {
    let (first, end) = (layout.rounds[round], layout.rounds[round + 1]);
    let base = layout.offsets[first];
    let items = usize::try_from(layout.offsets[end] - base).map_err(|_| too_large())?;
    let mut targets = SegVec::with_segment_bits(memory, "spilled targets", tuning.segment_bits);
    targets.try_resize(items, 0)?;
    let mut locals = SegVec::with_segment_bits(memory, "spilled sources", tuning.segment_bits);
    locals.try_resize(items, 0)?;
    let mut next: Vec<u64> = layout.offsets[first..end]
        .iter()
        .map(|o| o - base)
        .collect();
    let nodes: Vec<u32> = (first..end)
        .map(|b| layout.starts[b + 1] - layout.starts[b])
        .collect();
    let graph_nodes = layout.starts.last().copied().unwrap_or(0);
    let corrupt = || {
        BuildError::Scratch(io::Error::other(
            "a spill file does not hold what was written",
        ))
    };
    let mut reader = Chunked::new(source, memory)?;
    while let Some(header) = reader.take(CHUNK_HEADER)? {
        let bucket = u32::from_le_bytes(header[..4].try_into().expect("4 bytes"));
        let count = u32::from_le_bytes(header[4..].try_into().expect("4 bytes"));
        let b = usize::try_from(bucket).map_err(|_| corrupt())?;
        let slot = b
            .checked_sub(first)
            .filter(|_| b < end)
            .ok_or_else(corrupt)?;
        let mut left = usize::try_from(count).map_err(|_| corrupt())?;
        while left > 0 {
            let take = left.min(4096 / RECORD);
            let records = reader.take(take * RECORD)?.ok_or_else(corrupt)?;
            for record in records.chunks_exact(RECORD) {
                let at = usize::try_from(next[slot]).map_err(|_| corrupt())?;
                let local = u16::from_le_bytes(record[..2].try_into().expect("2 bytes"));
                let target = u32::from_le_bytes(record[2..].try_into().expect("4 bytes"));
                if widen(at) >= layout.offsets[b + 1] - base
                    || nodes[slot] <= u32::from(local)
                    || target >= graph_nodes
                {
                    return Err(corrupt());
                }
                locals[at] = local;
                targets[at] = target;
                next[slot] += 1;
            }
            left -= take;
        }
    }
    if (first..end).any(|b| next[b - first] != layout.offsets[b + 1] - base) {
        return Err(corrupt());
    }
    Ok(Resident { targets, locals })
}

/// Reads a source front to back through a reserved buffer.
pub(super) struct Chunked<'s, R> {
    source: &'s R,
    at: u64,
    buf: Vec<u8>,
    start: usize,
    _reservation: Reservation,
}

const READ_BUFFER: usize = 1 << 20;

impl<'s, R: ByteSource> Chunked<'s, R> {
    pub(super) fn new(source: &'s R, memory: &Memory) -> Result<Self, BuildError> {
        let reservation =
            memory.reserve("spill reader", MemoryKind::Anonymous, widen(READ_BUFFER))?;
        Ok(Self {
            source,
            at: 0,
            buf: Vec::new(),
            start: 0,
            _reservation: reservation,
        })
    }

    /// The next `n` bytes (at most 4 KiB), `None` at the end, or an error if the source
    /// ends inside them.
    pub(super) fn take(&mut self, n: usize) -> Result<Option<&[u8]>, BuildError> {
        if self.buf.len() - self.start < n {
            let left = self.source.len() - self.at;
            if left == 0 && self.buf.len() == self.start {
                return Ok(None);
            }
            self.buf.drain(..self.start);
            self.start = 0;
            let more =
                usize::try_from(left.min(widen(READ_BUFFER - self.buf.len()))).expect("small");
            let old = self.buf.len();
            self.buf.resize(old + more, 0);
            self.source.read_at(self.at, &mut self.buf[old..])?;
            self.at += widen(more);
            if self.buf.len() < n {
                return Err(BuildError::Scratch(io::Error::other(
                    "a spill file ends inside a record",
                )));
            }
        }
        let piece = &self.buf[self.start..self.start + n];
        self.start += n;
        Ok(Some(piece))
    }
}

fn too_large() -> BuildError {
    BuildError::Memory(MemoryError::AllocationFailed {
        purpose: "resident items",
        bytes: u64::MAX,
    })
}

fn widen(x: usize) -> u64 {
    u64::try_from(x).unwrap_or(u64::MAX)
}
