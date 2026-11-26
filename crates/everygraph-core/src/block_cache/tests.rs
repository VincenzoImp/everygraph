// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::num::NonZeroUsize;
use std::sync::atomic::AtomicBool;

use super::*;
use crate::{Budget, MemorySource, par};

fn sample(len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13).to_le_bytes()[0])
        .collect()
}

/// A small generator: the tests must not depend on a crate the core does not have.
struct Draw(u64);

impl Draw {
    fn below(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % n.max(1)
    }
}

fn settings(frames: u64, frame: usize, read: usize) -> CacheSettings {
    CacheSettings {
        frame,
        max_read: read * frame,
        ..CacheSettings::new(frames * widen(frame), 2)
    }
}

/// Reads `count` random ranges and checks each against the bytes.
fn check_reads(cache: &BlockCache<'_>, bytes: &[u8], seed: u64, count: usize) {
    let mut draw = Draw(seed);
    let mut buf = vec![0_u8; 40_000];
    for _ in 0..count {
        let start = draw.below(widen(bytes.len()));
        let len = draw.below(20_000).min(widen(bytes.len()) - start);
        let (a, n) = (
            usize::try_from(start).unwrap(),
            usize::try_from(len).unwrap(),
        );
        cache.read_at(start, &mut buf[..n]).unwrap();
        assert_eq!(&buf[..n], &bytes[a..a + n], "{n} bytes at {start}");
    }
}

/// The oracle: whatever the frame, the read size and the eviction, a read returns the
/// source's bytes. The source is not a whole number of frames.
#[test]
fn reads_equal_the_source_under_eviction() {
    let bytes = sample(300_001);
    let source = MemorySource::new(bytes.as_slice());
    for (frames, frame, read) in [(8, 4096, 16), (3, 4096, 1), (40, 8192, 4), (1000, 4096, 64)] {
        let cache =
            BlockCache::new(&source, &Memory::unlimited(), settings(frames, frame, read)).unwrap();
        check_reads(&cache, &bytes, frames, 2000);
        let stats = cache.stats();
        assert!(stats.hits > 0 && stats.misses > 0, "{stats:?}");
    }
}

/// Many threads at once on a cache much smaller than the source.
#[test]
fn concurrent_reads_equal_the_source() {
    let bytes = sample(1 << 20);
    let source = MemorySource::new(bytes.as_slice());
    let cache = BlockCache::new(&source, &Memory::unlimited(), settings(64, 4096, 8)).unwrap();
    par::for_each_range(16, NonZeroUsize::MIN, |r| {
        for seed in r {
            check_reads(&cache, &bytes, widen(seed), 300);
        }
    });
}

/// With room for the whole source, a second pass reads nothing, and the first reads
/// each byte once, in reads of at most `max_read`.
#[test]
fn a_source_that_fits_is_read_once() {
    let bytes = sample(100_000);
    let source = MemorySource::new(bytes.as_slice());
    let cache = BlockCache::new(&source, &Memory::unlimited(), settings(64, 4096, 4)).unwrap();
    let mut all = vec![0_u8; bytes.len()];
    cache.read_at(0, &mut all).unwrap();
    assert_eq!(all, bytes);
    let first = cache.stats();
    assert_eq!(first.bytes_read, 100_000);
    assert_eq!(first.misses, 25);
    assert_eq!(first.reads, 7, "25 frames in reads of at most 4");
    cache.read_at(0, &mut all).unwrap();
    let second = cache.stats();
    assert_eq!((second.bytes_read, second.hits), (100_000, 25));
}

/// CLOCK keeps a frame that is read again between misses: three frames of room, frame 0
/// read before every new frame, which then always hits. A new frame earns its reference
/// bit only when it is read again.
#[test]
fn clock_keeps_the_frame_in_use() {
    let bytes = sample(64 * 4096);
    let source = MemorySource::new(bytes.as_slice());
    let cache = BlockCache::new(&source, &Memory::unlimited(), settings(3, 4096, 1)).unwrap();
    let mut buf = [0_u8; 8];
    for f in 1..64 {
        cache.read_at(0, &mut buf).unwrap();
        cache.read_at(f * 4096, &mut buf).unwrap();
    }
    let stats = cache.stats();
    assert_eq!(stats.misses, 64, "frame 0 once, every other frame once");
    assert_eq!(stats.hits, 62, "frame 0 from the second time on");
}

/// Read-ahead threads, run when the test says.
struct Deferred<'a> {
    source: &'a (dyn ByteSource + Sync),
    queue: Mutex<Vec<Fetch>>,
    capacity: usize,
}

impl ReadAhead for Deferred<'_> {
    fn source(&self) -> &(dyn ByteSource + Sync) {
        self.source
    }

    fn submit(&self, fetch: Fetch) -> Result<(), Fetch> {
        let mut queue = lock(&self.queue);
        if queue.len() == self.capacity {
            return Err(fetch);
        }
        queue.push(fetch);
        Ok(())
    }

    fn max_read(&self) -> usize {
        16 * 4096
    }
}

impl Deferred<'_> {
    fn run_all(&self) -> Vec<Range<u64>> {
        let mut buf = vec![0_u8; self.max_read()];
        let fetches = std::mem::take(&mut *lock(&self.queue));
        fetches
            .into_iter()
            .map(|fetch| {
                let range = fetch.range.clone();
                fetch.run(self.source, &mut buf).unwrap();
                range
            })
            .collect()
    }
}

/// Advice reads the missing frames ahead, merged over held frames within the extra
/// allowed, and the reads that follow hit.
#[test]
fn read_ahead_merges_within_the_caps() {
    let bytes = sample(64 * 4096);
    let source = MemorySource::new(bytes.as_slice());
    let ahead = Deferred {
        source: &source,
        queue: Mutex::new(Vec::new()),
        capacity: 100,
    };
    let cache =
        BlockCache::with_read_ahead(&ahead, &Memory::unlimited(), settings(64, 4096, 64)).unwrap();
    let mut buf = [0_u8; 1];
    // Hold frames 4 and 9; then frames 0..16 are advised.
    cache.read_at(4 * 4096, &mut buf).unwrap();
    cache.read_at(9 * 4096, &mut buf).unwrap();
    cache.advise(0..16 * 4096, AccessHint::WillNeed);
    // 14 frames asked for; 2 held in between cost 14% extra, within 25%. Reads are at
    // most 16 frames (the read-ahead's buffer).
    assert_eq!(
        ahead.run_all(),
        [Range {
            start: 0,
            end: 16 * 4096
        }]
    );
    // One frame in five held: reads merge runs across them while the held frames they
    // read over stay within 25% of the frames asked for.
    let held = |f: u64| f % 5 == 1;
    for f in (16..56).filter(|&f| held(f)) {
        cache.read_at(f * 4096, &mut buf).unwrap();
    }
    cache.advise(16 * 4096..56 * 4096, AccessHint::WillNeed);
    let reads = ahead.run_all();
    let mut covered = 0;
    for r in &reads {
        let frames: Vec<u64> = (r.start / 4096..r.end / 4096).collect();
        let over = widen(frames.iter().filter(|&&f| held(f)).count());
        let asked = widen(frames.len()) - over;
        assert!(frames.len() <= 16 && over * 100 <= 25 * asked, "{r:?}");
        assert!(!held(frames[0]) && !held(frames[frames.len() - 1]), "{r:?}");
        covered += asked;
    }
    assert_eq!(covered, 32);
    assert!(reads.len() < 8, "runs are merged: {reads:?}");
    assert_eq!(cache.stats().prefetched, 14 + 32);
    let before = cache.stats();
    let mut all = vec![0_u8; 56 * 4096];
    cache.read_at(0, &mut all).unwrap();
    assert_eq!(all, bytes[..56 * 4096]);
    let after = cache.stats();
    assert_eq!(after.misses, before.misses, "everything was read ahead");
    assert_eq!(after.bytes_read, before.bytes_read);
}

/// A fetch dropped without running (the queue full, or the pool gone) gives its frames
/// back: a later read reads them itself, and gets the right bytes.
#[test]
fn a_dropped_fetch_gives_its_frames_back() {
    let bytes = sample(32 * 4096);
    let source = MemorySource::new(bytes.as_slice());
    let ahead = Deferred {
        source: &source,
        queue: Mutex::new(Vec::new()),
        capacity: 1,
    };
    let cache =
        BlockCache::with_read_ahead(&ahead, &Memory::unlimited(), settings(16, 4096, 4)).unwrap();
    // Two reads of 4 frames: the first queued, the second refused.
    cache.advise(0..8 * 4096, AccessHint::WillNeed);
    assert_eq!(cache.stats().prefetched, 4);
    drop(std::mem::take(&mut *lock(&ahead.queue)));
    let mut all = vec![0_u8; 8 * 4096];
    cache.read_at(0, &mut all).unwrap();
    assert_eq!(all, bytes[..8 * 4096]);
    assert_eq!(cache.stats().misses, 8);
}

/// Half of a shard's slots at most are claimed ahead, so reads always find a slot.
#[test]
fn read_ahead_leaves_half_the_slots() {
    let bytes = sample(64 * 4096);
    let source = MemorySource::new(bytes.as_slice());
    let ahead = Deferred {
        source: &source,
        queue: Mutex::new(Vec::new()),
        capacity: 100,
    };
    let cache =
        BlockCache::with_read_ahead(&ahead, &Memory::unlimited(), settings(8, 4096, 4)).unwrap();
    cache.advise(0..64 * 4096, AccessHint::WillNeed);
    assert_eq!(cache.stats().prefetched, 4);
    let mut buf = vec![0_u8; 8192];
    cache.read_at(40 * 4096, &mut buf).unwrap();
    assert_eq!(buf, bytes[40 * 4096..41 * 4096 + 4096]);
    ahead.run_all();
}

/// Fails every read while `fail` is set.
struct Flaky<'a> {
    bytes: &'a [u8],
    fail: AtomicBool,
}

impl ByteSource for Flaky<'_> {
    fn len(&self) -> u64 {
        widen(self.bytes.len())
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), ReadError> {
        if self.fail.load(Relaxed) {
            return Err(ReadError::Backend("injected".into()));
        }
        MemorySource::new(self.bytes).read_at(offset, buf)
    }
}

/// A failed read is reported and leaves no frame half claimed: the same read succeeds
/// once the source does.
#[test]
fn a_failed_read_leaves_the_cache_usable() {
    let bytes = sample(20_000);
    let source = Flaky {
        bytes: &bytes,
        fail: AtomicBool::new(true),
    };
    let cache = BlockCache::new(&source, &Memory::unlimited(), settings(4, 4096, 2)).unwrap();
    let mut buf = vec![0_u8; 10_000];
    assert!(matches!(
        cache.read_at(5000, &mut buf),
        Err(ReadError::Backend(_))
    ));
    source.fail.store(false, Relaxed);
    cache.read_at(5000, &mut buf).unwrap();
    assert_eq!(buf, bytes[5000..15_000]);
    let past = cache.read_at(19_999, &mut [0, 0]).unwrap_err();
    assert!(matches!(
        past,
        ReadError::OutOfBounds {
            offset: 19_999,
            len: 2,
            source_len: 20_000
        }
    ));
    cache.read_at(20_000, &mut []).unwrap();
}

/// Everything the cache holds is reserved before it is allocated, a cache that does not
/// fit is refused, and dropping one gives it all back.
#[test]
fn the_cache_is_in_the_ledger() {
    let bytes = sample(1 << 20);
    let source = MemorySource::new(bytes.as_slice());
    let budget = Arc::new(Budget::new(1 << 20));
    let memory = Memory::from(Arc::clone(&budget));
    let cache = BlockCache::new(&source, &memory, settings(128, 4096, 16)).unwrap();
    let frames = cache.frame_bytes();
    assert_eq!(frames, 128 * 4096);
    assert!(
        budget.used() > frames && budget.used() < frames + 200_000,
        "{}",
        budget.used()
    );
    let refused = BlockCache::new(&source, &memory, settings(128, 4096, 16)).unwrap_err();
    assert!(matches!(refused, MemoryError::Budget(_)));
    drop(cache);
    assert_eq!(budget.used(), 0);
}

/// The bytes read after `Sequential` are counted as a scan's, until `Normal`.
#[test]
fn scan_bytes_are_counted_apart() {
    let bytes = sample(64 * 4096);
    let source = MemorySource::new(bytes.as_slice());
    let cache = BlockCache::new(&source, &Memory::unlimited(), settings(8, 4096, 1)).unwrap();
    let mut buf = vec![0_u8; 100];
    cache.read_at(0, &mut buf).unwrap();
    cache.advise(0..1, AccessHint::Sequential);
    let mut all = vec![0_u8; bytes.len()];
    cache.read_at(0, &mut all).unwrap();
    assert_eq!(all, bytes);
    cache.advise(0..1, AccessHint::Normal);
    cache.read_at(0, &mut buf).unwrap();
    let stats = cache.stats();
    assert_eq!(
        stats.bytes_read,
        4096 * (1 + 63 + 1),
        "frame 0 was held, then pushed out by the scan"
    );
    assert_eq!(stats.scan_bytes, 4096 * 63);
}
