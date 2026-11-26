// SPDX-License-Identifier: Apache-2.0 OR MIT

//! [`CompressedStream`]: the compressed format read through a [`ByteSource`] in large
//! positional reads. This is the semi-external regime (DESIGN D11, FINDINGS § B1): the
//! kernel's O(n) state and the adjacency index are in memory; the records are not, and
//! no memory map holds them. Each reader holds a window of the data part, one aligned
//! buffer that a read fills.
//!
//! Each pass of a kernel costs about one sequential read of the data part. So a kernel
//! that is cheap in memory slows down by the ratio of the read time to its compute time:
//! components on Friendster took 0.34 s in memory and 3.55 s streamed under 8 GB, both
//! on raw arrays (FINDINGS § B1, lab). Iterative kernels such as `PageRank` lose less.
//!
//! - **Scans read ahead.** When a task needs a record outside the window, one read fills
//!   the window from that record, up to the window's size or the task's end. A task of
//!   about one window ([`CompressedStream::settings`]) then costs one read.
//! - **Only active lists.** In a block where at most half the nodes pass the filter,
//!   only the records of the nodes that pass are read, merged where they are less than
//!   16 KiB apart. So a kernel whose active set shrinks (bottom-up BFS, the finish of
//!   components) reads less as it goes (on-demand reads through the offsets: Seraph,
//!   FAST 2024).
//! - **Frontiers and random access** read the records asked for, merged in the same way.
//!
//! Whether reads go through the page cache or around it is the [`ByteSource`]'s choice:
//! a file opened for buffered or for direct reads. Reads start and end at multiples of
//! 4 KiB of the data part, which starts at a multiple of 4 KiB in a `.eg` file, and the
//! window is aligned, so a direct read needs no copy. The source advises
//! [`AccessHint::Sequential`] before scans and [`AccessHint::Normal`] before frontiers:
//! advice per phase (ADR 0029).

use std::fmt;
use std::ops::Range;
use std::sync::atomic::{AtomicU8, Ordering::Relaxed};

use everygraph_core::{
    AccessHint, AlignedBuf, ByteSource, DEFAULT_SEGMENT_SIZE, Memory, MemoryError, MemoryKind,
    NodeId, Reservation, SEGMENT_ALIGN, SegVec,
};
use everygraph_format::adjacency::{Adjacency, AdjacencyIndex, Batch, FormatError};

use super::source::{COALESCE_GAP, DecodeReader, advise_runs};
use super::{Error, FRONTIER_GRAIN, Settings, Source, block_nodes, ix, one_segment};

const ALIGN: u64 = SEGMENT_ALIGN as u64;

/// Largest window: 8 MiB, the lab's per-worker read (FINDINGS § B1).
const MAX_WINDOW: u64 = DEFAULT_SEGMENT_SIZE;

/// The last advice given to the data part.
const NONE: u8 = 0;
const SCAN: u8 = 1;
const BY_NODE: u8 = 2;

/// A graph in the compressed adjacency format whose data part is read through a
/// [`ByteSource`]: a file, or a section of one. Only the index is in memory.
pub struct CompressedStream<'a> {
    index: &'a AdjacencyIndex,
    data: &'a (dyn ByteSource + Sync),
    n: u32,
    /// Bytes per read ahead, a multiple of 4 KiB.
    window: u64,
    advice: AtomicU8,
}

impl fmt::Debug for CompressedStream<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CompressedStream")
            .field("nodes", &self.n)
            .field("window", &self.window)
            .finish_non_exhaustive()
    }
}

/// The reader of a [`CompressedStream`]: its window, its decode batch, and the nodes of
/// a block that pass a scan's filter.
#[derive(Debug)]
pub struct WindowReader {
    io: Window,
    /// Capacity reserved for `lists` nodes, so that it is one segment.
    active: SegVec<u32>,
    lists: usize,
}

#[derive(Debug)]
struct Window {
    decode: DecodeReader,
    buf: AlignedBuf,
    /// The bytes of the data part that the start of `buf` holds.
    bytes: Range<u64>,
    /// Invariant: `held.bytes()` is the heap of `buf`.
    held: Reservation,
    /// Where a scan reads ahead to: the end of the records of its task.
    task_end: u64,
}

impl Window {
    /// Makes `buf` at least `len` bytes long, and at least `usual`; reserved first.
    fn make_room(&mut self, len: usize, usual: usize) -> Result<(), Error> {
        if len <= self.buf.len() {
            return Ok(());
        }
        let want = len.max(usual);
        // `AlignedBuf` holds its length and less than `ALIGN` bytes of padding.
        let heap = want as u64 + ALIGN;
        // The old buffer is freed only after the new one exists.
        self.held.try_grow(heap)?;
        self.bytes = 0..0;
        match AlignedBuf::try_zeroed(want) {
            Ok(buf) => self.buf = buf,
            Err(_) => {
                self.held.shrink(heap);
                return Err(Error::Memory(MemoryError::AllocationFailed {
                    purpose: "edge-map stream window",
                    bytes: heap,
                }));
            }
        }
        self.held.shrink(self.held.bytes() - heap);
        Ok(())
    }
}

impl<'a> CompressedStream<'a> {
    /// The adjacency of `index` whose data part `data` holds, read in windows of
    /// `window` bytes (rounded up to a multiple of 4 KiB). Lists are checked as they
    /// are decoded.
    ///
    /// # Errors
    ///
    /// [`Error::Format`] if `data` does not have the length of the data part.
    pub fn new(
        index: &'a AdjacencyIndex,
        data: &'a (dyn ByteSource + Sync),
        window: usize,
    ) -> Result<Self, Error> {
        if data.len() != index.data_bytes() {
            return Err(Error::Format(FormatError::Length {
                expected: index.data_bytes(),
                actual: data.len(),
            }));
        }
        let window = (window as u64)
            .clamp(ALIGN, MAX_WINDOW)
            .next_multiple_of(ALIGN);
        Ok(Self {
            index,
            data,
            n: index.nodes(),
            window,
            advice: AtomicU8::new(NONE),
        })
    }

    /// The window that lets `readers` readers take at most half of `available` bytes,
    /// between 4 KiB and 8 MiB. The other half is left for the readers' decode batches
    /// and for lists longer than a window.
    #[must_use]
    pub fn window_for(available: u64, readers: usize) -> usize {
        let share = available / 2 / (readers.max(1) as u64);
        ix(share.clamp(ALIGN, MAX_WINDOW) / ALIGN * ALIGN)
    }

    /// Bytes per read ahead.
    #[must_use]
    pub fn window(&self) -> usize {
        ix(self.window)
    }

    /// Settings for this source: tasks of about one window of records, and every second
    /// scan in reverse order ([`Settings::alternate`]).
    #[must_use]
    pub fn settings(&self) -> Settings {
        let data = u128::from(self.index.data_bytes().max(1));
        let entries = u128::from(self.window) * u128::from(self.index.entries()) / data;
        Settings {
            chunk_entries: u64::try_from(entries).unwrap_or(u64::MAX).max(1),
            alternate: true,
            ..Settings::default()
        }
    }

    /// The memory that the readers of a plan over this source reserve with `settings`,
    /// when `workers` workers run its drivers: two readers per worker (one for the
    /// driver, one for [`Lists::get`](super::Lists::get)). A list longer than the window
    /// or than a batch makes its reader take more while it is read.
    #[must_use]
    pub fn reader_memory(&self, settings: &Settings, workers: usize) -> u64 {
        let lists = ix(block_nodes(self.n).into()).max(FRONTIER_GRAIN);
        let batch = settings.batch_entries.max(1).saturating_add(8);
        let reader = self.window
            + ALIGN
            + Batch::bytes_for(batch, lists) as u64
            + (lists * size_of::<u32>()) as u64;
        reader.saturating_mul(2 * workers as u64)
    }

    /// The offset in the data part of the record of `v`, for `v` in `0..=nodes`.
    fn byte_at(&self, v: u32) -> u64 {
        if v == self.n {
            self.index.data_bytes()
        } else {
            self.index.record_range(NodeId::new(v)).start
        }
    }

    fn advise(&self, phase: u8, hint: AccessHint) {
        if self.advice.swap(phase, Relaxed) != phase {
            self.data.advise(0..self.data.len(), hint);
        }
    }

    /// The end of the batch that starts at `at`, as [`Adjacency::decode_nodes`] cuts
    /// it: the first list, then lists while they fit in `max` entries, up to `end`.
    fn batch_end(&self, at: u32, end: u32, max: usize) -> u32 {
        let (mut stop, mut entries) = (at, 0_u64);
        for d in self.index.degrees(at..end) {
            entries += u64::from(d);
            if stop > at && entries > max as u64 {
                break;
            }
            stop += 1;
        }
        stop
    }

    /// Where a read of the records of `nodes[..i]` should end: the end of the records of
    /// `nodes[i - 1]` (`end`), extended over the runs of consecutive nodes that follow
    /// while each starts less than [`COALESCE_GAP`] bytes after the last, up to `cap`.
    fn run_end(&self, nodes: &[u32], mut i: usize, mut end: u64, cap: u64) -> u64 {
        while i < nodes.len() && end < cap && nodes[i] > nodes[i - 1] {
            if self.byte_at(nodes[i]) > end + COALESCE_GAP {
                break;
            }
            let mut e = i + 1;
            while e < nodes.len() && nodes[e] == nodes[e - 1] + 1 {
                e += 1;
            }
            end = self.byte_at(nodes[e - 1] + 1);
            i = e;
        }
        end
    }

    /// Makes the window hold `need`. On a miss, the window becomes `need.start` (rounded
    /// down to 4 KiB) to at least `need.end` and at most one window, or to `limit(cap)`
    /// if that is less, where `cap` is the end of one window. The bytes it already holds
    /// from there on are moved, not read again, so a scan reads each byte once.
    fn fill(
        &self,
        io: &mut Window,
        need: Range<u64>,
        limit: impl FnOnce(u64) -> u64,
    ) -> Result<(), Error> {
        if need.is_empty() || (io.bytes.start <= need.start && need.end <= io.bytes.end) {
            return Ok(());
        }
        let start = need.start / ALIGN * ALIGN;
        let cap = start.saturating_add(self.window);
        let end = limit(cap).min(cap).max(need.end);
        let end = end
            .checked_next_multiple_of(ALIGN)
            .unwrap_or(u64::MAX)
            .min(self.data.len());
        let len = ix(end - start);
        let held = io.bytes.clone();
        let keep = if len <= io.buf.len() && held.contains(&start) {
            // `held.end` is a multiple of 4 KiB: it is below `need.end`, so not the end
            // of the data part.
            let from = ix(start - held.start);
            io.buf
                .as_mut_slice()
                .copy_within(from..ix(held.end - held.start), 0);
            held.end - start
        } else {
            io.make_room(len, ix(self.window))?;
            0
        };
        io.bytes = 0..0;
        let read = &mut io.buf.as_mut_slice()[ix(keep)..len];
        self.data.read_at(start + keep, read)?;
        io.bytes = start..end;
        Ok(())
    }

    /// Decodes the lists of `nodes` from the window into the batch, and checks them;
    /// returns the first node not decoded.
    fn decode(&self, io: &mut Window, nodes: Range<u32>) -> Result<u32, Error> {
        let held = &io.buf.as_slice()[..ix(io.bytes.end - io.bytes.start)];
        let part = Adjacency::part(self.index, io.bytes.clone(), held).map_err(Error::Format)?;
        io.decode.decode(&part, nodes)
    }

    /// Calls `f` for each node of `nodes` with its list, in order. Runs of consecutive
    /// nodes are decoded together, a batch at a time. A scan reads ahead (`ahead`) to the
    /// end of its task; other reads end with the nodes they serve.
    fn read_lists(
        &self,
        io: &mut Window,
        nodes: &[u32],
        ahead: bool,
        mut f: impl FnMut(u32, u64, &[u32]),
    ) -> Result<(), Error> {
        let mut i = 0;
        while i < nodes.len() {
            let mut e = i + 1;
            while e < nodes.len() && nodes[e] == nodes[e - 1] + 1 {
                e += 1;
            }
            let end = nodes[e - 1] + 1;
            let mut at = nodes[i];
            while at < end {
                let stop = self.batch_end(at, end, io.decode.max_entries());
                let need = self.byte_at(at)..self.byte_at(stop);
                let task_end = io.task_end;
                self.fill(io, need, |cap| {
                    if ahead {
                        task_end
                    } else {
                        self.run_end(nodes, e, self.byte_at(end), cap)
                    }
                })?;
                at = self.decode(io, at..stop)?;
                io.decode.hand_out(self.index, &mut f);
            }
            i = e;
        }
        Ok(())
    }
}

// SAFETY: every list handed out comes from the reader's batch after `check` accepted it
// in the same call (`DecodeReader::decode`): the batch holds the lists asked for, they lie
// inside `batch.targets()`, and every target there is below `n`. The index is borrowed,
// so `n` does not change. `scan` calls `filter` for the nodes of `nodes` and `f` for
// those that passed; `lists` hands out runs of consecutive elements of `nodes`.
unsafe impl Source for CompressedStream<'_> {
    type Reader = WindowReader;

    fn nodes(&self) -> u32 {
        self.n
    }

    fn entries(&self) -> u64 {
        self.index.entries()
    }

    fn entry_offset(&self, v: u32) -> u64 {
        self.index.entry_offset(NodeId::new(v)).get()
    }

    fn degree(&self, v: u32) -> u32 {
        self.index.degree(NodeId::new(v))
    }

    #[inline]
    fn degrees(&self, nodes: Range<u32>, mut f: impl FnMut(u32, u32)) {
        for (v, d) in nodes.clone().zip(self.index.degrees(nodes)) {
            f(v, d);
        }
    }

    fn reader(&self, memory: &Memory, batch: usize, lists: usize) -> Result<WindowReader, Error> {
        let mut active = SegVec::new(memory, "edge-map stream nodes");
        active.try_reserve(lists)?;
        Ok(WindowReader {
            io: Window {
                decode: DecodeReader::new(memory, batch, lists)?,
                buf: AlignedBuf::default(),
                bytes: 0..0,
                held: Reservation::empty(memory, "edge-map stream window", MemoryKind::Anonymous),
                task_end: 0,
            },
            active,
            lists: lists.max(1),
        })
    }

    fn begin_task(&self, r: &mut WindowReader, nodes: Range<u32>) {
        self.advise(SCAN, AccessHint::Sequential);
        r.io.task_end = self.byte_at(nodes.end);
    }

    fn scan(
        &self,
        r: &mut WindowReader,
        nodes: Range<u32>,
        filter: impl Fn(u32) -> bool,
        mut f: impl FnMut(u32, u64, &[u32]),
    ) -> Result<(), Error> {
        let piece = u32::try_from(r.lists).unwrap_or(u32::MAX);
        let mut at = nodes.start;
        while at < nodes.end {
            let stop = at.saturating_add(piece).min(nodes.end);
            r.active.clear();
            for v in (at..stop).filter(|&v| filter(v)) {
                r.active.try_push(v)?;
            }
            let active = one_segment(&r.active)?;
            let dense = 2 * active.len() >= ix((stop - at).into());
            self.read_lists(&mut r.io, active, dense, &mut f)?;
            at = stop;
        }
        Ok(())
    }

    fn lists(
        &self,
        r: &mut WindowReader,
        nodes: &[u32],
        f: impl FnMut(u32, u64, &[u32]),
    ) -> Result<(), Error> {
        self.advise(BY_NODE, AccessHint::Normal);
        self.read_lists(&mut r.io, nodes, false, f)
    }

    fn list<'r>(&'r self, r: &'r mut WindowReader, v: u32) -> Result<(u64, &'r [u32]), Error> {
        assert!(v < self.n, "node {v} outside the graph");
        let need = self.index.record_range(NodeId::new(v));
        self.fill(&mut r.io, need, |_| 0)?;
        self.decode(&mut r.io, v..v + 1)?;
        Ok((self.entry_offset(v), r.io.decode.only()))
    }

    fn prefetch(&self, nodes: &[u32]) {
        advise_runs(self.data, nodes, |v| {
            self.index.record_range(NodeId::new(v))
        });
    }

    fn idle(&self, r: &mut WindowReader) {
        r.io.decode.release();
        if r.io.buf.len() as u64 > self.window {
            r.io.buf = AlignedBuf::default();
            r.io.bytes = 0..0;
            r.io.held.shrink(r.io.held.bytes());
        }
    }
}
