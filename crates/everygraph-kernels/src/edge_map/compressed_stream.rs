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
//!   the window from that record, up to the window's size or the task's end. The bytes
//!   the window already holds from there on are moved, not read again. So a scan reads
//!   about the data part once: at most 1.05 times it at the smallest window, counted at
//!   the source in `tests/stream.rs`.
//! - **Only active lists.** In a block where at most half the nodes pass the filter,
//!   only the records of the nodes that pass are read, merged where they are less than
//!   16 KiB apart. So a kernel whose active set shrinks (bottom-up BFS, the finish of
//!   components) reads less as it goes (on-demand reads through the offsets: Seraph,
//!   FAST 2024). Reads are whole 4 KiB pages, so this saves bytes only where the active
//!   nodes are sparser than about one per page of records.
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
use everygraph_format::adjacency::{Adjacency, AdjacencyIndex, Batch, FormatError, record_bound};

use super::source::{COALESCE_GAP, DecodeReader, advise_runs};
use super::{Error, FRONTIER_GRAIN, Settings, Source, block_nodes, ix, one_segment};

const ALIGN: u64 = SEGMENT_ALIGN as u64;

/// Largest window: 8 MiB, the lab's per-worker read (FINDINGS § B1).
const MAX_WINDOW: u64 = DEFAULT_SEGMENT_SIZE;

/// Fewest bytes of records in a task of [`CompressedStream::settings`]. A task's first
/// read starts at a 4 KiB boundary, so it may read again up to 4 KiB that the task
/// before it read; at 256 KiB a task, that is less than 2%.
const MIN_TASK: u64 = 256 << 10;

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
    /// Capacity reserved for the decode batch's lists, so that it is one segment.
    active: SegVec<u32>,
}

#[derive(Debug)]
struct Window {
    decode: DecodeReader,
    buf: AlignedBuf,
    /// The bytes of the data part that the start of `buf` holds.
    bytes: Range<u64>,
    /// Invariant: `held.bytes()` is the heap of `buf`.
    held: Reservation,
    /// The size `buf` keeps: a window and the records of one batch, so that a batch
    /// needs a larger buffer only when its first list alone does.
    usual: usize,
    /// Where a scan reads ahead to: the end of the records of its task.
    task_end: u64,
}

impl Window {
    /// Moves `buf[keep]` to the start of a buffer of at least `len` bytes, and returns
    /// the bytes kept. A larger buffer, of at least `usual` bytes, is reserved before it
    /// is made. A buffer that a long list grew is freed first and keeps nothing, so a
    /// reader never holds two grown buffers at once.
    fn shift(&mut self, len: usize, keep: Range<usize>) -> Result<usize, Error> {
        self.bytes = 0..0;
        if len <= self.buf.len() {
            let kept = keep.len();
            self.buf.as_mut_slice().copy_within(keep, 0);
            return Ok(kept);
        }
        let keep = if self.buf.len() > self.usual {
            self.buf = AlignedBuf::default();
            self.held.shrink(self.held.bytes());
            0..0
        } else {
            keep
        };
        let want = len.max(self.usual);
        // `AlignedBuf` holds its length and less than `ALIGN` bytes of padding.
        let heap = want as u64 + ALIGN;
        self.held.try_grow(heap)?;
        let Ok(mut buf) = AlignedBuf::try_zeroed(want) else {
            self.held.shrink(heap);
            return Err(Error::Memory(MemoryError::AllocationFailed {
                purpose: "edge-map stream window",
                bytes: heap,
            }));
        };
        let kept = keep.len();
        buf.as_mut_slice()[..kept].copy_from_slice(&self.buf.as_slice()[keep]);
        self.buf = buf;
        self.held.shrink(self.held.bytes() - heap);
        Ok(kept)
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
        let window = rounded(window);
        Ok(Self {
            index,
            data,
            n: index.nodes(),
            window,
            advice: AtomicU8::new(NONE),
        })
    }

    /// The window that gives `readers` readers at most half of `available` bytes,
    /// between 4 KiB and 8 MiB; the other half is left for their decode batches. At the
    /// 4 KiB floor the readers may need more than half: a planner checks
    /// [`reader_memory`](Self::reader_memory) against the budget.
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

    /// Settings for this source: tasks of about one window of records (and at least
    /// 256 KiB), and every second scan in reverse order ([`Settings::alternate`]).
    #[must_use]
    pub fn settings(&self) -> Settings {
        let window = ix(self.window);
        Self::settings_for(self.index.entries(), self.index.data_bytes(), window)
    }

    /// [`settings`](Self::settings) of a stream, made with `window`, over an adjacency of
    /// `entries` entries and `data_bytes` bytes of records: for a planner, before the
    /// source exists.
    #[must_use]
    pub fn settings_for(entries: u64, data_bytes: u64, window: usize) -> Settings {
        let task = u128::from(rounded(window).max(MIN_TASK));
        let entries = task * u128::from(entries) / u128::from(data_bytes.max(1));
        Settings {
            chunk_entries: u64::try_from(entries).unwrap_or(u64::MAX).max(1),
            alternate: true,
            ..Settings::default()
        }
    }

    /// The most memory that the readers of a plan over this source reserve with
    /// `settings`, when `workers` workers run its drivers: two readers per worker (one
    /// for the driver, one for [`Lists::get`](super::Lists::get)), each as
    /// [`reader_bytes`](Self::reader_bytes) says. Reads the degrees once.
    #[must_use]
    pub fn reader_memory(&self, settings: &Settings, workers: usize) -> u64 {
        let i = self.index;
        let longest = i.degrees(0..self.n).max().unwrap_or(0);
        let each = Self::reader_bytes(
            self.n,
            i.entries(),
            i.data_bytes(),
            longest,
            ix(self.window),
            settings.batch_entries,
        );
        each.saturating_mul(2 * workers as u64)
    }

    /// The most memory one reader of a stream made with `window` reserves, over an
    /// adjacency of `nodes` nodes, `entries` entries, `data_bytes` bytes of records and
    /// a longest list of `longest` entries, with batches of about `batch` entries: its
    /// buffer and its decode batch, each while it grows from its usual size to the
    /// longest list's.
    #[must_use]
    pub fn reader_bytes(
        nodes: u32,
        entries: u64,
        data_bytes: u64,
        longest: u32,
        window: usize,
        batch: usize,
    ) -> u64 {
        let lists = ix(block_nodes(nodes).into()).max(FRONTIER_GRAIN);
        // A batch holds no more entries than the graph.
        let most = entries.saturating_add(8);
        let batch = (batch.max(1) as u64).min(most);
        let long = u64::from(longest).max(batch).min(most);
        let targets = |e: u64| (e + 8).saturating_mul(size_of::<u32>() as u64);
        let ends = Batch::bytes_for(0, lists) as u64;
        let decode = 2 * ends + targets(batch) + targets(long);
        let usual = usual(rounded(window), data_bytes, ix(batch)) as u64 + ALIGN;
        let record = record_bound(u64::from(longest)).min(data_bytes) + 3 * ALIGN;
        let buffer = if record > usual {
            usual + record
        } else {
            usual
        };
        buffer + decode + (lists * size_of::<u32>()) as u64
    }

    /// The offset in the data part of the record of `v`, for `v` in `0..=nodes`.
    fn byte_at(&self, v: u32) -> u64 {
        self.index.data_range(v..v).start
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
        // On a miss `need.end` is past `held.end`, so `held.end` is not the end of the
        // data part and is a multiple of 4 KiB.
        let keep = if held.contains(&start) {
            ix(start - held.start)..ix(held.end - held.start)
        } else {
            0..0
        };
        let kept = io.shift(len, keep)?;
        let read = &mut io.buf.as_mut_slice()[kept..len];
        self.data.read_at(start + kept as u64, read)?;
        io.bytes = start..end;
        Ok(())
    }

    /// Decodes the lists of `nodes` from the window into the batch, and checks them;
    /// returns the first node not decoded. The window must hold their records
    /// ([`fill`](Self::fill)).
    fn decode(&self, io: &mut Window, nodes: Range<u32>) -> Result<u32, Error> {
        let window = self.window_of(io.buf.as_slice(), &io.bytes, nodes.clone())?;
        io.decode.decode(&window, nodes)
    }

    /// The adjacency of `nodes` alone, over their records in `buf`, which holds the
    /// bytes `held` of the data part.
    fn window_of<'w>(
        &'w self,
        buf: &'w [u8],
        held: &Range<u64>,
        nodes: Range<u32>,
    ) -> Result<Adjacency<'w>, Error> {
        let bytes = self.index.data_range(nodes.clone());
        let records = if bytes.is_empty() {
            Some(&[][..])
        } else {
            let at = |x: u64| usize::try_from(x.checked_sub(held.start)?).ok();
            let (a, b) = (at(bytes.start), at(bytes.end));
            a.zip(b).and_then(|(a, b)| buf.get(a..b))
        };
        let records = records.ok_or(Error::Invalid("records outside the read window"))?;
        Adjacency::window(self.index, nodes, records).map_err(Error::Format)
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
                let need = self.index.data_range(at..stop);
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

/// `window` as a stream uses it: between 4 KiB and 8 MiB, a multiple of 4 KiB.
fn rounded(window: usize) -> u64 {
    (window as u64)
        .clamp(ALIGN, MAX_WINDOW)
        .next_multiple_of(ALIGN)
}

/// The buffer a reader keeps for batches of about `batch` entries: one window and the
/// records of one batch (at most `⌈e/4⌉ + 4e` bytes for `e` entries, and one more control
/// byte for each list, and no more than the `data_bytes` of the data part), across 4 KiB
/// boundaries on both sides.
fn usual(window: u64, data_bytes: u64, batch: usize) -> usize {
    let entries = batch as u64;
    let records = record_bound(entries)
        .saturating_add(entries)
        .min(data_bytes);
    let bytes = window.saturating_add(records).saturating_add(2 * ALIGN);
    let bytes = bytes.checked_next_multiple_of(ALIGN).unwrap_or(u64::MAX);
    usize::try_from(bytes).unwrap_or(usize::MAX)
}

// SAFETY: every list handed out comes from the reader's batch after `check` accepted it
// in the same call (`DecodeReader::decode` or `decode_prefix`): the batch holds the lists asked for, they lie
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

    /// The window, taken at the first read, and beside it a buffer for the whole record
    /// when it is longer than the window (`Window::shift`), and the decode batch of the
    /// prefix.
    fn list_bytes(&self, entries: u64, prefix: u64, batch: usize) -> u64 {
        let usual = usual(self.window, self.index.data_bytes(), batch) as u64 + ALIGN;
        let record = (record_bound(entries) + 1).min(self.index.data_bytes()) + 3 * ALIGN;
        let buffer = if record > usual {
            usual + record
        } else {
            usual
        };
        buffer.saturating_add(DecodeReader::batch_bytes(prefix))
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
                usual: usual(self.window, self.index.data_bytes(), batch),
                task_end: 0,
            },
            active,
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
        let piece = u32::try_from(r.io.decode.lists().max(1)).unwrap_or(u32::MAX);
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

    fn list_prefix<'r>(
        &'r self,
        r: &'r mut WindowReader,
        v: u32,
        k: usize,
    ) -> Result<(u64, &'r [u32]), Error> {
        assert!(v < self.n, "node {v} outside the graph");
        let need = self.index.record_range(NodeId::new(v));
        self.fill(&mut r.io, need, |_| 0)?;
        let io = &mut r.io;
        let window = self.window_of(io.buf.as_slice(), &io.bytes, v..v + 1)?;
        io.decode.decode_prefix(&window, v, k)?;
        Ok((self.entry_offset(v), r.io.decode.only()))
    }

    fn prefetch(&self, nodes: &[u32]) {
        advise_runs(self.data, nodes, |v| {
            self.index.record_range(NodeId::new(v))
        });
    }

    fn idle(&self, r: &mut WindowReader) {
        r.io.decode.release();
        if r.io.buf.len() > r.io.usual {
            r.io.buf = AlignedBuf::default();
            r.io.bytes = 0..0;
            r.io.held.shrink(r.io.held.bytes());
        }
    }
}
