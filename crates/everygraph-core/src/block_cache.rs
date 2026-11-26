// SPDX-License-Identifier: Apache-2.0 OR MIT

//! [`BlockCache`]: a cache of fixed-size frames over a [`ByteSource`], held in memory
//! reserved from the budget. It is regime 2 of DESIGN D11 for random access whose working
//! set is larger than the budget (ADR 0029).
//!
//! The cache holds bytes, not results, so what a kernel computes does not change with it.
//! Its memory is anonymous and in the ledger, so the operating system cannot take it back
//! and the budget stays exact, unlike the page cache behind a memory map.
//!
//! - **Frames.** The source is cut into frames of [`CacheSettings::frame`] bytes. A read
//!   copies the bytes it needs out of their frames; a frame that is not held is read from
//!   the source first. Neighbouring frames that a read needs and the cache does not hold
//!   are read together, in one read of at most [`CacheSettings::max_read`] bytes.
//! - **Eviction** is CLOCK, per shard. Sixteen neighbouring frames share a shard, so a
//!   read of 64 KiB takes at most two locks.
//! - **Read-ahead.** [`AccessHint::WillNeed`] on a range claims its missing frames and
//!   hands them to a [`ReadAhead`]: reader threads outside the core (the native one is
//!   `everygraph_io::ReadPool`). A read that needs a claimed frame waits for it. Missing
//!   frames are merged into one read across frames that are held already, up to
//!   [`CacheSettings::max_read`] bytes, while the bytes read that nobody asked for stay
//!   within [`CacheSettings::max_extra_percent`] of the bytes asked for. Without a
//!   `ReadAhead` (wasm32), the advice is ignored and reads are synchronous.
//!
//! - **Phases.** The other hints are not passed on: `Sequential` on a buffered file would
//!   fill the page cache that the cache replaces. [`AccessHint::Sequential`] starts a
//!   scan phase, until [`AccessHint::Normal`] or [`AccessHint::Random`], whose bytes are
//!   counted apart ([`CacheStats::scan_bytes`]). Scans still read through the frames: a
//!   scan's body may read other lists by random access, as Leiden's aggregation does.

use std::fmt;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use crate::{
    AccessHint, AlignedBuf, ByteSource, Memory, MemoryError, MemoryKind, ReadError, Reservation,
    SEGMENT_ALIGN, SegVec, widen,
};

/// Frames per shard group: `2^4` frames in a row share a shard.
const GROUP_BITS: u32 = 4;
/// A frame that is not held, in a shard's map.
const EMPTY: u32 = u32::MAX;
/// A slot that holds no frame.
const NO_FRAME: u64 = u64::MAX;
/// Slot flags: CLOCK's reference bit, and "being read".
const REFERENCED: u8 = 1;
const PENDING: u8 = 2;
/// Most frames in one read: a fetch records which of its frames it claimed in a `u64`.
const MAX_FRAMES_PER_READ: usize = 64;

/// How a [`BlockCache`] is laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CacheSettings {
    /// Bytes of frames, rounded down to whole frames; at least one frame per shard.
    pub bytes: u64,
    /// Bytes per frame: a power of two, at least [`SEGMENT_ALIGN`]. 4 KiB by default:
    /// in m1-15, 16 KiB frames read 2.6× the bytes of 4 KiB frames for Leiden's local
    /// moving on Friendster (`lab/m1-15-mmap-hints-block-cache/results/README.md`).
    pub frame: usize,
    /// Longest read, in bytes: a multiple of the frame, at most 64 frames. 64 KiB by
    /// default: random 64 KiB reads gave the drive's highest random-read rate in m2-35
    /// (4.96 GB/s at queue depth 64, `lab/m2-35-random-read-probe/README.md`).
    pub max_read: usize,
    /// Most bytes a read-ahead may read over, between the frames it was asked for, as a
    /// percentage of the frames asked for. 25 by default, a choice: reading a held frame
    /// again costs about 1% of the time of a separate read of it (m2-35's model, 71 µs a
    /// read plus 0.74 µs per KiB), and bytes are what the budget's case is judged on.
    pub max_extra_percent: u32,
    /// Reads that callers make themselves at once, each with a buffer of
    /// [`max_read`](Self::max_read) bytes reserved with the cache; a caller beyond them
    /// waits for a buffer. The number of threads that read through the cache.
    pub readers: usize,
}

impl CacheSettings {
    /// `bytes` of 4 KiB frames, reads of at most 64 KiB, 25% extra, `readers` buffers.
    #[must_use]
    pub const fn new(bytes: u64, readers: usize) -> Self {
        Self {
            bytes,
            frame: SEGMENT_ALIGN,
            max_read: 64 << 10,
            max_extra_percent: 25,
            readers,
        }
    }
}

/// What a [`BlockCache`] did, counted since it was made.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct CacheStats {
    /// Frames that a read found held.
    pub hits: u64,
    /// Frames that a read read itself.
    pub misses: u64,
    /// Frames that a read found being read ahead, and waited for.
    pub waits: u64,
    /// Frames read ahead.
    pub prefetched: u64,
    /// Reads of the source: by readers and ahead.
    pub reads: u64,
    /// Bytes read from the source, by readers and ahead.
    pub bytes_read: u64,
    /// The part of [`bytes_read`](Self::bytes_read) read in a scan phase: after
    /// [`AccessHint::Sequential`].
    pub scan_bytes: u64,
}

/// Reader threads that run a [`BlockCache`]'s reads ahead of need. The core has no
/// threads, so the native implementation lives in `everygraph-io` (`ReadPool`).
pub trait ReadAhead: Sync {
    /// The source the threads read. A cache made with
    /// [`BlockCache::with_read_ahead`] reads it too, so both read the same bytes.
    fn source(&self) -> &(dyn ByteSource + Sync);

    /// Queues `fetch` for a reader thread, which calls [`Fetch::run`] on it with
    /// [`source`](Self::source). Gives it back when the queue is full.
    ///
    /// # Errors
    ///
    /// The fetch, when it was not queued.
    fn submit(&self, fetch: Fetch) -> Result<(), Fetch>;

    /// The longest fetch a thread can take: the size of its buffer.
    fn max_read(&self) -> usize;
}

/// One read of a [`BlockCache`]: a range of its source and the frames waiting for it.
/// Dropped without being run, it gives its frames up, and a read that needs one of them
/// reads it itself.
pub struct Fetch {
    frames: Arc<Frames>,
    /// The first frame read.
    first: u64,
    /// Bytes of the source read.
    range: Range<u64>,
    /// Bit `i`: frame `first + i` was claimed by this fetch.
    mine: u64,
    done: bool,
}

impl fmt::Debug for Fetch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Fetch")
            .field("range", &self.range)
            .field("mine", &format_args!("{:#x}", self.mine))
            .finish_non_exhaustive()
    }
}

impl Fetch {
    /// Bytes to read: what [`run`](Self::run) needs of its buffer.
    #[must_use]
    pub fn len(&self) -> usize {
        usize::try_from(self.range.end - self.range.start).expect("at most 64 frames")
    }

    /// Whether the fetch reads nothing. It never does.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.range.is_empty()
    }

    /// Reads the fetch's bytes from `source` into `buf` (at least [`len`](Self::len)
    /// bytes, aligned to [`SEGMENT_ALIGN`] for direct reads) and fills the frames waiting
    /// for them. On an error the frames are given up.
    ///
    /// # Errors
    ///
    /// The source's.
    pub fn run(mut self, source: &(dyn ByteSource + Sync), buf: &mut [u8]) -> Result<(), ReadError> {
        let len = self.len();
        let buf = &mut buf[..len];
        source.read_at(self.range.start, buf)?;
        self.frames.stats.read(len);
        self.frames.install(self.first, self.mine, buf);
        self.done = true;
        Ok(())
    }
}

impl Drop for Fetch {
    fn drop(&mut self) {
        if !self.done {
            self.frames.give_up(self.first, self.mine);
        }
    }
}

/// The frames of one shard: which it holds, and their bytes.
struct Slots {
    /// For each frame of the source that maps to this shard (by its local index): its
    /// slot, or [`EMPTY`].
    slot_of: SegVec<u32>,
    /// For each slot: its frame, or [`NO_FRAME`].
    frame_of: SegVec<u64>,
    flags: SegVec<u8>,
    /// The frames' bytes, slot after slot.
    bytes: SegVec<u8>,
    hand: usize,
    pending: usize,
}

struct Shard {
    slots: Mutex<Slots>,
    /// Signalled when a pending frame of the shard is filled or given up.
    ready: Condvar,
}

#[derive(Default)]
struct Stats {
    hits: AtomicU64,
    misses: AtomicU64,
    waits: AtomicU64,
    prefetched: AtomicU64,
    reads: AtomicU64,
    bytes_read: AtomicU64,
    scan_bytes: AtomicU64,
    /// Whether the last phase hint was [`AccessHint::Sequential`].
    scanning: AtomicBool,
}

impl Stats {
    fn read(&self, bytes: usize) {
        self.reads.fetch_add(1, Relaxed);
        self.bytes_read.fetch_add(widen(bytes), Relaxed);
        if self.scanning.load(Relaxed) {
            self.scan_bytes.fetch_add(widen(bytes), Relaxed);
        }
    }
}

/// The state shared with fetches in flight.
struct Frames {
    shift: u32,
    frame: usize,
    len: u64,
    shards: Box<[Shard]>,
    /// Slots per shard.
    per: usize,
    stats: Stats,
    /// Buffers for the reads that callers make themselves.
    spare: Mutex<Vec<AlignedBuf>>,
    /// Signalled when a buffer goes back to `spare`.
    returned: Condvar,
    /// Pays for `shards` and the buffers.
    _held: Reservation,
}

/// A frame and what a read does with it, decided under its shard's lock.
enum Found {
    /// Held: the bytes were copied.
    Copied,
    /// Being read: wait.
    Pending,
    /// Not held, now claimed by the caller.
    Claimed,
    /// Not held, and every slot of its shard is being read.
    Full,
}

/// A buffer of [`Frames::spare`], given back when dropped.
struct Spare<'f> {
    frames: &'f Frames,
    buf: Option<AlignedBuf>,
}

impl Spare<'_> {
    fn bytes(&mut self) -> &mut AlignedBuf {
        self.buf.as_mut().expect("held until dropped")
    }
}

impl Drop for Spare<'_> {
    fn drop(&mut self) {
        if let Some(buf) = self.buf.take() {
            lock(&self.frames.spare).push(buf);
            self.frames.returned.notify_one();
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Frames {
    fn shard_of(&self, f: u64) -> (&Shard, usize) {
        let n = widen(self.shards.len());
        let group = f >> GROUP_BITS;
        let local = ((group / n) << GROUP_BITS) | (f & ((1 << GROUP_BITS) - 1));
        let s = usize::try_from(group % n).expect("below the shard count");
        (&self.shards[s], usize::try_from(local).expect("a frame index"))
    }

    /// The bytes of frame `f` in the source.
    fn frame_range(&self, f: u64) -> Range<u64> {
        let start = f << self.shift;
        start..(start + widen(self.frame)).min(self.len)
    }

    /// CLOCK: the first slot not referenced since the hand last passed it, and not being
    /// read; its old frame is forgotten. `None` if every slot is being read.
    fn take_slot(&self, s: &mut Slots) -> Option<usize> {
        for _ in 0..2 * self.per {
            let h = s.hand;
            s.hand = (h + 1) % self.per;
            let flags = &mut s.flags.as_mut_slice().expect("one segment")[h];
            if *flags & PENDING != 0 {
                continue;
            }
            if *flags & REFERENCED != 0 {
                *flags &= !REFERENCED;
                continue;
            }
            let old = s.frame_of.as_slice().expect("one segment")[h];
            if old != NO_FRAME {
                let (_, local) = self.shard_of(old);
                s.slot_of.as_mut_slice().expect("one segment")[local] = EMPTY;
            }
            return Some(h);
        }
        None
    }

    /// Claims frame `f`, held in `s` at `local`, for a read: a slot, marked pending.
    fn claim(&self, s: &mut Slots, f: u64, local: usize) -> bool {
        let Some(slot) = self.take_slot(s) else {
            return false;
        };
        s.slot_of.as_mut_slice().expect("one segment")[local] =
            u32::try_from(slot).expect("slots fit in u32");
        s.frame_of.as_mut_slice().expect("one segment")[slot] = f;
        s.flags.as_mut_slice().expect("one segment")[slot] = PENDING;
        s.pending += 1;
        true
    }

    /// Looks up frame `f` for a read of `out` at `pos` (both within `f`), and copies the
    /// bytes if it is held. Claims it if it is not.
    fn find(&self, f: u64, pos: u64, out: &mut [u8]) -> Found {
        let (shard, local) = self.shard_of(f);
        let mut s = lock(&shard.slots);
        let slot = s.slot_of.as_slice().expect("one segment")[local];
        if slot == EMPTY {
            return if self.claim(&mut s, f, local) {
                Found::Claimed
            } else {
                Found::Full
            };
        }
        let slot = usize::try_from(slot).expect("a slot index");
        let flags = &mut s.flags.as_mut_slice().expect("one segment")[slot];
        if *flags & PENDING != 0 {
            return Found::Pending;
        }
        *flags |= REFERENCED;
        let at = slot * self.frame + usize::try_from(pos - (f << self.shift)).expect("in a frame");
        out.copy_from_slice(&s.bytes.as_slice().expect("one segment")[at..at + out.len()]);
        Found::Copied
    }

    /// Waits until frame `f` is no longer being read.
    fn wait(&self, f: u64) {
        let (shard, local) = self.shard_of(f);
        let mut s = lock(&shard.slots);
        loop {
            let slot = s.slot_of.as_slice().expect("one segment")[local];
            let pending = slot != EMPTY
                && s.flags.as_slice().expect("one segment")
                    [usize::try_from(slot).expect("a slot index")]
                    & PENDING
                    != 0;
            if !pending {
                return;
            }
            s = shard
                .ready
                .wait(s)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// Calls `f(shard, slots, frame, slot)` for each frame `first + i` with bit `i` of
    /// `mine`, under its shard's lock, and wakes the shard's waiters.
    fn each_mine(&self, first: u64, mine: u64, mut f: impl FnMut(&mut Slots, u64, usize)) {
        let mut bits = mine;
        while bits != 0 {
            let i = bits.trailing_zeros();
            bits &= bits - 1;
            let frame = first + u64::from(i);
            let (shard, local) = self.shard_of(frame);
            let mut s = lock(&shard.slots);
            let slot = s.slot_of.as_slice().expect("one segment")[local];
            debug_assert_ne!(slot, EMPTY, "a claimed frame has a slot");
            f(&mut s, frame, usize::try_from(slot).expect("a slot index"));
            drop(s);
            shard.ready.notify_all();
        }
    }

    /// Fills the claimed frames of a read of `first..` with `bytes` (the read's bytes,
    /// from the start of frame `first`).
    fn install(&self, first: u64, mine: u64, bytes: &[u8]) {
        self.each_mine(first, mine, |s, frame, slot| {
            let from = usize::try_from((frame - first) << self.shift).expect("in a read");
            let take = (bytes.len() - from).min(self.frame);
            let to = &mut s.bytes.as_mut_slice().expect("one segment")
                [slot * self.frame..(slot + 1) * self.frame];
            to[..take].copy_from_slice(&bytes[from..from + take]);
            to[take..].fill(0);
            // Unreferenced: a frame earns its reference bit when it is read again.
            s.flags.as_mut_slice().expect("one segment")[slot] = 0;
            s.pending -= 1;
        });
    }

    /// Gives up the claimed frames of a read of `first..`.
    fn give_up(&self, first: u64, mine: u64) {
        self.each_mine(first, mine, |s, frame, slot| {
            let (_, local) = self.shard_of(frame);
            s.slot_of.as_mut_slice().expect("one segment")[local] = EMPTY;
            s.frame_of.as_mut_slice().expect("one segment")[slot] = NO_FRAME;
            s.flags.as_mut_slice().expect("one segment")[slot] = 0;
            s.pending -= 1;
        });
    }

    /// A buffer for a caller's own read, when one is free.
    fn spare(&self) -> Spare<'_> {
        let mut spare = lock(&self.spare);
        loop {
            if let Some(buf) = spare.pop() {
                return Spare {
                    frames: self,
                    buf: Some(buf),
                };
            }
            spare = self
                .returned
                .wait(spare)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

/// A budgeted cache of fixed-size frames over a [`ByteSource`]; a `ByteSource` itself
/// (module documentation).
pub struct BlockCache<'s> {
    source: &'s (dyn ByteSource + Sync),
    ahead: Option<&'s dyn ReadAhead>,
    frames: Arc<Frames>,
    /// Frames per read.
    read_frames: usize,
    max_extra_percent: u64,
}

impl fmt::Debug for BlockCache<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BlockCache")
            .field("len", &self.frames.len)
            .field("frame", &self.frames.frame)
            .field("shards", &self.frames.shards.len())
            .field("slots_per_shard", &self.frames.per)
            .field("read_ahead", &self.ahead.is_some())
            .finish_non_exhaustive()
    }
}

impl<'s> BlockCache<'s> {
    /// A cache over `source` that reads only when a read needs the bytes.
    ///
    /// # Errors
    ///
    /// [`MemoryError`] if its frames and maps do not fit in `memory`; nothing is held then.
    ///
    /// # Panics
    ///
    /// If `settings.frame` is not a power of two of at least [`SEGMENT_ALIGN`] bytes, or
    /// `settings.max_read` is not a multiple of it of at most 64 frames.
    pub fn new(
        source: &'s (dyn ByteSource + Sync),
        memory: &Memory,
        settings: CacheSettings,
    ) -> Result<Self, MemoryError> {
        let frame = settings.frame;
        assert!(
            frame.is_power_of_two() && frame >= SEGMENT_ALIGN,
            "a frame is a power of two of at least {SEGMENT_ALIGN} bytes"
        );
        let read_frames = settings.max_read / frame;
        assert!(
            settings.max_read.is_multiple_of(frame) && (1..=MAX_FRAMES_PER_READ).contains(&read_frames),
            "a read is 1 to {MAX_FRAMES_PER_READ} frames"
        );
        let shift = frame.trailing_zeros();
        let len = source.len();
        let groups = len.div_ceil(widen(frame)).div_ceil(1 << GROUP_BITS).max(1);
        let slots = usize::try_from(settings.bytes / widen(frame)).unwrap_or(usize::MAX);
        // About 64 slots a shard, and no more shards than groups of frames.
        let shards = (slots / 64).clamp(1, 4096).min(usize::try_from(groups).unwrap_or(usize::MAX));
        let per = (slots / shards).max(1);
        let locals = usize::try_from(groups.div_ceil(widen(shards)) << GROUP_BITS)
            .map_err(|_| MemoryError::AllocationFailed {
                purpose: "block cache map",
                bytes: u64::MAX,
            })?;
        let readers = settings.readers.max(1);
        let buffer = widen(settings.max_read + SEGMENT_ALIGN);
        let held = memory.reserve(
            "block cache buffers",
            MemoryKind::Anonymous,
            widen(shards)
                .saturating_mul(widen(size_of::<Shard>()))
                .saturating_add(widen(readers).saturating_mul(buffer)),
        )?;
        let failed = |bytes| MemoryError::AllocationFailed {
            purpose: "block cache buffers",
            bytes,
        };
        let spare = (0..readers)
            .map(|_| AlignedBuf::try_zeroed(settings.max_read).map_err(|_| failed(buffer)))
            .collect::<Result<Vec<_>, _>>()?;
        let mut all = Vec::with_capacity(shards);
        for _ in 0..shards {
            all.push(Shard {
                slots: Mutex::new(Slots {
                    slot_of: SegVec::try_with_len(memory, "block cache map", locals, EMPTY)?,
                    frame_of: SegVec::try_with_len(memory, "block cache map", per, NO_FRAME)?,
                    flags: SegVec::try_with_len(memory, "block cache map", per, 0)?,
                    bytes: SegVec::try_with_len(
                        memory,
                        "block cache frames",
                        per.saturating_mul(frame),
                        0,
                    )?,
                    hand: 0,
                    pending: 0,
                }),
                ready: Condvar::new(),
            });
        }
        Ok(Self {
            source,
            ahead: None,
            frames: Arc::new(Frames {
                shift,
                frame,
                len,
                shards: all.into_boxed_slice(),
                per,
                stats: Stats::default(),
                spare: Mutex::new(spare),
                returned: Condvar::new(),
                _held: held,
            }),
            read_frames,
            max_extra_percent: u64::from(settings.max_extra_percent),
        })
    }

    /// A cache over the source of `ahead`, whose threads read the ranges advised with
    /// [`AccessHint::WillNeed`] before they are needed. Reads are at most
    /// `ahead.max_read()` bytes.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    ///
    /// # Panics
    ///
    /// As [`new`](Self::new).
    pub fn with_read_ahead(
        ahead: &'s dyn ReadAhead,
        memory: &Memory,
        settings: CacheSettings,
    ) -> Result<Self, MemoryError> {
        let mut cache = Self::new(ahead.source(), memory, settings)?;
        cache.read_frames = cache.read_frames.min(ahead.max_read() / settings.frame).max(1);
        cache.ahead = Some(ahead);
        Ok(cache)
    }

    /// Bytes the frames hold.
    #[must_use]
    pub fn frame_bytes(&self) -> u64 {
        widen(self.frames.shards.len() * self.frames.per * self.frames.frame)
    }

    /// What the cache did so far.
    #[must_use]
    pub fn stats(&self) -> CacheStats {
        let s = &self.frames.stats;
        CacheStats {
            hits: s.hits.load(Relaxed),
            misses: s.misses.load(Relaxed),
            waits: s.waits.load(Relaxed),
            prefetched: s.prefetched.load(Relaxed),
            reads: s.reads.load(Relaxed),
            bytes_read: s.bytes_read.load(Relaxed),
            scan_bytes: s.scan_bytes.load(Relaxed),
        }
    }

    fn fetch(&self, first: u64, end: u64, mine: u64) -> Fetch {
        let range = self.frames.frame_range(first).start..self.frames.frame_range(end - 1).end;
        Fetch {
            frames: Arc::clone(&self.frames),
            first,
            range,
            mine,
            done: false,
        }
    }

    /// Reads frames `first..` that the caller claimed (as many as fit one read and are
    /// missing, up to frame `last`) and copies what `out` (the bytes from `pos`) needs.
    /// Returns where the copy ended.
    fn read_missing(&self, first: u64, last: u64, pos: u64, out: &mut [u8]) -> Result<u64, ReadError> {
        let f = &self.frames;
        let mut end = first + 1;
        while end <= last && end - first < widen(self.read_frames) {
            if !self.claim_missing(end, false) {
                break;
            }
            end += 1;
        }
        let mine = u64::MAX >> (64 - (end - first));
        let fetch = self.fetch(first, end, mine);
        let range = fetch.range.clone();
        let mut spare = f.spare();
        f.stats.misses.fetch_add(end - first, Relaxed);
        fetch.run(self.source, spare.bytes().as_mut_slice())?;
        let stop = range.end.min(pos + widen(out.len()));
        let from = usize::try_from(pos - range.start).expect("in a read");
        let n = usize::try_from(stop - pos).expect("in a read");
        out[..n].copy_from_slice(&spare.bytes().as_slice()[from..from + n]);
        Ok(stop)
    }

    /// Reads frame `f` around the cache, when its shard has no slot free.
    fn read_around(&self, f: u64, pos: u64, out: &mut [u8]) -> Result<(), ReadError> {
        let stats = &self.frames.stats;
        let range = self.frames.frame_range(f);
        let len = usize::try_from(range.end - range.start).expect("one frame");
        let mut spare = self.frames.spare();
        let buf = spare.bytes().as_mut_slice();
        stats.misses.fetch_add(1, Relaxed);
        self.source.read_at(range.start, &mut buf[..len])?;
        stats.read(len);
        let from = usize::try_from(pos - range.start).expect("in a frame");
        out.copy_from_slice(&buf[from..from + out.len()]);
        Ok(())
    }

    /// Claims the missing frames of `range` and queues them for the read-ahead threads,
    /// merged into reads within the caps.
    fn read_ahead(&self, ahead: &dyn ReadAhead, range: Range<u64>) {
        let f = &self.frames;
        let range = range.start.min(f.len)..range.end.min(f.len);
        if range.is_empty() {
            return;
        }
        let (first, last) = (range.start >> f.shift, (range.end - 1) >> f.shift);
        // The read being built: frames `start..end`, `wanted` of them claimed (`mine`).
        let mut open: Option<(u64, u64, u64, u64)> = None;
        let flush = |open: Option<(u64, u64, u64, u64)>| {
            if let Some((start, end, _, mine)) = open {
                let fetch = self.fetch(start, end, mine);
                let frames = u64::from(mine.count_ones());
                if ahead.submit(fetch).is_ok() {
                    f.stats.prefetched.fetch_add(frames, Relaxed);
                }
            }
        };
        for frame in first..=last {
            if !self.claim_missing(frame, true) {
                continue;
            }
            open = match open {
                Some((start, end, wanted, mine)) if self.joins(start, end, wanted, frame) => {
                    Some((start, frame + 1, wanted + 1, mine | 1 << (frame - start)))
                }
                other => {
                    flush(other);
                    Some((frame, frame + 1, 1, 1))
                }
            };
        }
        flush(open);
    }

    /// Whether claimed frame `frame` can join the read of `start..end` with `wanted`
    /// claimed frames: within one read, and within the extra bytes allowed.
    fn joins(&self, start: u64, end: u64, wanted: u64, frame: u64) -> bool {
        let span = frame + 1 - start;
        let extra = (frame - end) + (end - start - wanted);
        span <= widen(self.read_frames) && extra * 100 <= self.max_extra_percent * (wanted + 1)
    }

    /// Claims frame `frame` if it is missing and, for a read `ahead`, fewer than half of
    /// its shard's slots are being read.
    fn claim_missing(&self, frame: u64, ahead: bool) -> bool {
        let f = &self.frames;
        let (shard, local) = f.shard_of(frame);
        let mut s = lock(&shard.slots);
        s.slot_of.as_slice().expect("one segment")[local] == EMPTY
            && (!ahead || 2 * s.pending < f.per)
            && f.claim(&mut s, frame, local)
    }
}

impl ByteSource for BlockCache<'_> {
    fn len(&self) -> u64 {
        self.frames.len
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), ReadError> {
        let f = &self.frames;
        let end = offset
            .checked_add(widen(buf.len()))
            .filter(|&end| end <= f.len)
            .ok_or(ReadError::OutOfBounds {
                offset,
                len: widen(buf.len()),
                source_len: f.len,
            })?;
        let last = if end > offset { (end - 1) >> f.shift } else { 0 };
        let mut pos = offset;
        while pos < end {
            let frame = pos >> f.shift;
            let stop = f.frame_range(frame).end.min(end);
            let at = usize::try_from(pos - offset).expect("in the buffer");
            let out = &mut buf[at..at + usize::try_from(stop - pos).expect("in a frame")];
            match f.find(frame, pos, out) {
                Found::Copied => {
                    f.stats.hits.fetch_add(1, Relaxed);
                    pos = stop;
                }
                Found::Pending => {
                    f.stats.waits.fetch_add(1, Relaxed);
                    f.wait(frame);
                }
                Found::Claimed => {
                    let out = &mut buf[at..];
                    pos = self.read_missing(frame, last, pos, out)?;
                }
                Found::Full => {
                    self.read_around(frame, pos, out)?;
                    pos = stop;
                }
            }
        }
        Ok(())
    }

    fn advise(&self, range: Range<u64>, hint: AccessHint) {
        match hint {
            AccessHint::Sequential => self.frames.stats.scanning.store(true, Relaxed),
            AccessHint::Normal | AccessHint::Random => {
                self.frames.stats.scanning.store(false, Relaxed);
            }
            AccessHint::WillNeed => {
                if let Some(ahead) = self.ahead {
                    self.read_ahead(ahead, range);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
