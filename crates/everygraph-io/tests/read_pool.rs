// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The block cache read ahead by a [`ReadPool`] over a file, direct and buffered: every
//! read returns the file's bytes while reader threads, read-ahead threads and eviction
//! race, every advised frame is read ahead, and the pool's memory is in the ledger.

#![forbid(unsafe_code)]

#[path = "support/scratch.rs"]
mod scratch_dir;
#[path = "support/bytes.rs"]
mod support;

use std::sync::Arc;

use everygraph_core::{AccessHint, BlockCache, Budget, ByteSource, CacheSettings, Memory};
use everygraph_io::{FileSource, PoolSettings, ReadPool};
use everygraph_oracle::rng::SplitMix64;
use scratch_dir::scratch;
use support::bytes;

const FRAME: u64 = 4096;
const FRAME_BYTES: usize = 4096;

fn pool(threads: usize) -> PoolSettings {
    PoolSettings {
        threads,
        queue: 64,
        ..PoolSettings::default()
    }
}

/// Eight threads each advise a range and then read random pieces of it, on a cache of a
/// quarter of the file, so that read-ahead, waiting and eviction all happen.
fn race(source: &FileSource, data: &[u8]) {
    let memory = Memory::unlimited();
    let len = data.len() as u64;
    ReadPool::run(source, &memory, pool(16), |ahead| {
        let cache =
            BlockCache::with_read_ahead(ahead, &memory, CacheSettings::new(len / 4, 8)).unwrap();
        std::thread::scope(|s| {
            for t in 0..8_u64 {
                let cache = &cache;
                s.spawn(move || {
                    let mut rng = SplitMix64::new(t);
                    let mut buf = vec![0_u8; 3 * FRAME_BYTES];
                    for _ in 0..400 {
                        let start = rng.next_u64() % (len - 64 * FRAME);
                        cache.advise(start..start + 64 * FRAME, AccessHint::WillNeed);
                        for _ in 0..4 {
                            let at = start + rng.next_u64() % (60 * FRAME);
                            let n = usize::try_from(rng.next_u64() % (3 * FRAME)).unwrap();
                            cache.read_at(at, &mut buf[..n]).unwrap();
                            let a = usize::try_from(at).unwrap();
                            assert_eq!(&buf[..n], &data[a..a + n], "{n} bytes at {at}");
                        }
                    }
                });
            }
        });
        let stats = cache.stats();
        assert!(stats.prefetched > 0 && stats.hits > 0, "{stats:?}");
    })
    .unwrap();
}

#[test]
fn reads_through_a_pool_equal_the_file() {
    let dir = scratch("race");
    let path = dir.join("data");
    // Not a whole number of frames.
    let data = bytes(7, (8 << 20) + 1234);
    std::fs::write(&path, &data).unwrap();
    race(&FileSource::open(&path).unwrap(), &data);
    race(&FileSource::open_direct(&path).unwrap(), &data);
}

/// Advised frames are read ahead, in reads of at most the pool's buffer, and the reads
/// that follow find them held: no byte is read twice.
#[test]
fn advised_frames_are_read_ahead_once() {
    let dir = scratch("ahead");
    let path = dir.join("data");
    let data = bytes(3, 4 << 20);
    std::fs::write(&path, &data).unwrap();
    let source = FileSource::open_direct(&path).unwrap();
    let memory = Memory::unlimited();
    ReadPool::run(&source, &memory, pool(4), |ahead| {
        let cache =
            BlockCache::with_read_ahead(ahead, &memory, CacheSettings::new(8 << 20, 1)).unwrap();
        // A quarter of the file, in 16 ranges: fewer frames than the half of each shard
        // that read-ahead may claim at once.
        for r in 0..16_u64 {
            cache.advise(r * 64 * FRAME..(r * 64 + 16) * FRAME, AccessHint::WillNeed);
        }
        let mut buf = vec![0_u8; 16 * FRAME_BYTES];
        for r in 0..16_u64 {
            let at = r * 64 * FRAME;
            cache.read_at(at, &mut buf).unwrap();
            let a = usize::try_from(at).unwrap();
            assert_eq!(buf, data[a..a + buf.len()]);
        }
        let stats = cache.stats();
        assert_eq!(stats.prefetched, 256, "{stats:?}");
        assert_eq!(stats.misses, 0, "{stats:?}");
        assert_eq!(stats.bytes_read, 256 * FRAME);
        assert_eq!(stats.reads, 16, "one read of 64 KiB per range");
    })
    .unwrap();
}

/// The threads' buffers and queue are reserved before anything starts, refused when
/// they do not fit, and given back when the pool stops.
#[test]
fn the_pool_is_in_the_ledger() {
    let dir = scratch("ledger");
    let path = dir.join("data");
    std::fs::write(&path, bytes(1, 1 << 16)).unwrap();
    let source = FileSource::open(&path).unwrap();
    let budget = Arc::new(Budget::new(1 << 20));
    let memory = Memory::from(Arc::clone(&budget));
    let seen = ReadPool::run(&source, &memory, pool(8), |_| budget.used()).unwrap();
    assert!(seen >= 8 * (64 << 10), "{seen}");
    assert_eq!(budget.used(), 0);
    assert!(ReadPool::run(&source, &memory, pool(64), |_| ()).is_err());
    assert_eq!(budget.used(), 0);
}
