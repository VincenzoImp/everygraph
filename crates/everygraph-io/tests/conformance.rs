// SPDX-License-Identifier: Apache-2.0 OR MIT

//! One conformance suite for every `ByteSource` backend, and the sinks checked by
//! reading what they wrote back through every backend.
//!
//! A new backend gets one line in `every_backend`. Files live under Cargo's target
//! directory (a disk file system, so that direct I/O is really used on Linux).

#![forbid(unsafe_code)]

#[path = "support/scratch.rs"]
mod scratch_dir;
#[path = "support/bytes.rs"]
mod support;

use std::fs::{self, File};
use std::io::Write;
use std::ops::Range;
use std::path::Path;

use everygraph_core::{
    AccessHint, AlignedBuf, ByteSource, MemorySource, ReadError, SEGMENT_ALIGN, SegmentReader,
    Segments,
};
use everygraph_io::{AppendSink, BlockWriter, FileSource, MmapSource, SpillLogs};
use everygraph_oracle::rng::SplitMix64;
use scratch_dir::scratch;
use support::bytes;
// Only the guard test uses it, and the guard is Unix only.
#[cfg(unix)]
use everygraph_io::FileSink;

const A: usize = SEGMENT_ALIGN;

/// Lengths around every boundary the backends care about: empty, one byte, one block
/// either side, and more than the 1 MiB bounce buffer of the direct backend.
const LENGTHS: [usize; 7] = [0, 1, A - 1, A, A + 1, 3 * A + 17, (2 << 20) + 333];

/// A number in `0..=n`.
fn up_to(rng: &mut SplitMix64, n: usize) -> usize {
    usize::try_from(rng.below(n as u64 + 1)).unwrap()
}

fn write_file(path: &Path, bytes: &[u8]) {
    File::create(path).unwrap().write_all(bytes).unwrap();
}

/// Reads `len` bytes at `offset` into a buffer whose address is either congruent to
/// `offset` modulo the block size (the direct backend's no-copy path) or deliberately
/// not (its bounce path).
fn read(
    source: &dyn ByteSource,
    offset: u64,
    len: usize,
    congruent: bool,
) -> Result<Vec<u8>, ReadError> {
    let mut backing = AlignedBuf::try_zeroed(len + 2 * A).unwrap();
    let shift = usize::try_from(offset % A as u64).unwrap() + usize::from(!congruent);
    let buf = &mut backing.as_mut_slice()[shift..shift + len];
    buf.fill(0xA5);
    source.read_at(offset, buf)?;
    Ok(buf.to_vec())
}

fn assert_out_of_bounds(
    result: Result<Vec<u8>, ReadError>,
    offset: u64,
    len: usize,
    source_len: u64,
) {
    match result {
        Err(ReadError::OutOfBounds {
            offset: o,
            len: l,
            source_len: s,
        }) => assert_eq!((o, l, s), (offset, len as u64, source_len)),
        other => panic!(
            "read of {len} at {offset}: expected OutOfBounds, got {:?}",
            other.map(|v| v.len())
        ),
    }
}

/// The contract of `ByteSource`, checked against the bytes the source must hold.
fn check(name: &str, source: &(dyn ByteSource + Sync), expected: &[u8]) {
    let n = expected.len();
    let len = n as u64;
    assert_eq!(source.len(), len, "{name}: len");
    assert_eq!(source.is_empty(), n == 0, "{name}: is_empty");
    if let Some(all) = source.as_slice() {
        assert_eq!(all, expected, "{name}: as_slice");
    }

    // Every pair of interesting offset and length, in both buffer placements.
    let points = [
        0,
        1,
        A - 1,
        A,
        A + 1,
        n / 2,
        n.saturating_sub(A + 1),
        n.saturating_sub(1),
        n,
        n + 1,
    ];
    let lengths = [0, 1, 7, A - 1, A, A + 1, 2 * A, 3 * A + 5, n, n + 1];
    for &offset in &points {
        for &want in &lengths {
            for congruent in [true, false] {
                let result = read(source, offset as u64, want, congruent);
                if let Some(bytes) = source.slice(offset as u64..(offset + want) as u64) {
                    assert_eq!(Some(bytes), expected.get(offset..offset + want), "{name}");
                }
                match expected.get(offset..offset + want) {
                    Some(bytes) => assert_eq!(
                        result.unwrap_or_else(|e| panic!("{name}: {want} at {offset}: {e}")),
                        bytes,
                        "{name}: {want} at {offset}"
                    ),
                    None => assert_out_of_bounds(result, offset as u64, want, len),
                }
            }
        }
    }
    // Offsets that would wrap or truncate.
    for offset in [1 << 32, u64::MAX - 3, u64::MAX] {
        assert_out_of_bounds(read(source, offset, 4, true), offset, 4, len);
    }

    // Advice of every kind, over any range, changes nothing.
    for hint in [
        AccessHint::Normal,
        AccessHint::Sequential,
        AccessHint::Random,
        AccessHint::WillNeed,
    ] {
        for range in [
            0..len,
            0..u64::MAX,
            len / 3..len / 2,
            len..len + 10,
            Range { start: 7, end: 3 },
            u64::MAX - 1..u64::MAX,
        ] {
            source.advise(range, hint);
        }
    }

    // Segments reassemble the source, twice in a row (re-readable).
    for size in [1, A as u64, 3 * A as u64 + 5, 1 << 20] {
        for _pass in 0..2 {
            let mut reader = SegmentReader::new();
            let mut out = Vec::with_capacity(n);
            for range in Segments::of(source, size) {
                out.extend_from_slice(reader.read(source, range).unwrap());
            }
            assert!(out == expected, "{name}: segments of {size}");
        }
    }

    // Random reads from several threads at once.
    std::thread::scope(|s| {
        for t in 0..4_u64 {
            s.spawn(move || {
                let mut rng = SplitMix64::new(0x9E37_79B9 + t);
                for _ in 0..200 {
                    let offset = up_to(&mut rng, n);
                    let want = up_to(&mut rng, n - offset).min(3 * A + 1);
                    let got = read(source, offset as u64, want, rng.chance(500)).unwrap();
                    assert!(
                        got == expected[offset..offset + want],
                        "{name}: thread {t}, {want} at {offset}"
                    );
                }
            });
        }
    });
}

/// Runs the suite on every backend over a file holding `bytes`.
fn every_backend(dir: &Path, bytes: &[u8]) {
    let path = dir.join(format!("source-{}", bytes.len()));
    write_file(&path, bytes);

    check("memory", &MemorySource::new(bytes.to_vec()), bytes);
    check("pread", &FileSource::open(&path).unwrap(), bytes);
    check("mmap", &MmapSource::open(&path).unwrap(), bytes);
    let direct = FileSource::open_direct(&path).unwrap();
    if cfg!(any(target_os = "linux", target_os = "macos")) {
        assert!(
            direct.is_direct(),
            "direct I/O was refused under {}",
            dir.display()
        );
    }
    check("direct", &direct, bytes);
    check(
        "pread (from a File)",
        &FileSource::from_file(File::open(&path).unwrap()).unwrap(),
        bytes,
    );
    check(
        "mmap (from a File)",
        &MmapSource::from_file(&File::open(&path).unwrap()).unwrap(),
        bytes,
    );
    // Range maps: the whole file, an unaligned middle, nothing, each half.
    let n = bytes.len() as u64;
    for range in [0..n, n / 3..n - n / 5, n / 2..n / 2, 0..n / 2, n / 2..n] {
        let map = MmapSource::open_range(&path, range.clone()).unwrap();
        let (start, end) = (to_usize(range.start), to_usize(range.end));
        assert_eq!(map.mapped(), &bytes[start..end]);
        assert_eq!(map.slice(range.clone()), Some(&bytes[start..end]));
        check(&format!("mmap of {range:?}"), &map, bytes);
    }
    let too_far = MmapSource::open_range(&path, 0..n + 1).unwrap_err();
    assert_eq!(too_far.kind(), std::io::ErrorKind::InvalidInput);
}

fn to_usize(n: u64) -> usize {
    usize::try_from(n).unwrap()
}

#[test]
fn every_backend_conforms() {
    let dir = scratch("conformance");
    for (seed, &len) in LENGTHS.iter().enumerate() {
        every_backend(&dir, &bytes(seed as u64, len));
    }
    fs::remove_dir_all(dir).unwrap();
}

/// A file that shrinks under a reader is a backend error, never a panic or stale bytes.
/// (Not for the memory map: there the lost pages raise SIGBUS, as documented.)
#[test]
fn a_file_that_shrinks_gives_a_backend_error() {
    let dir = scratch("shrink");
    let path = dir.join("shrinking");
    let bytes = bytes(5, 4 * A);
    write_file(&path, &bytes);
    let sources = [
        FileSource::open(&path).unwrap(),
        FileSource::open_direct(&path).unwrap(),
    ];
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(A as u64)
        .unwrap();
    for source in &sources {
        assert_eq!(source.len(), 4 * A as u64, "the length is the one at open");
        assert_eq!(read(source, 0, A, true).unwrap(), &bytes[..A]);
        for congruent in [true, false] {
            match read(source, A as u64 - 1, 2 * A, congruent) {
                Err(ReadError::Backend(_)) => {}
                other => panic!("expected a backend error, got {:?}", other.map(|v| v.len())),
            }
        }
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn an_append_sink_writes_what_it_is_given_and_counts_it() {
    let dir = scratch("append");
    let path = dir.join("appended");
    let bytes = bytes(9, (3 << 20) + 11);
    let mut sink = AppendSink::create(&path).unwrap();
    let mut rng = SplitMix64::new(3);
    let mut written = 0;
    while written < bytes.len() {
        let piece = up_to(&mut rng, bytes.len() - written).clamp(1, 300_000);
        sink.write_all(&bytes[written..written + piece]).unwrap();
        written += piece;
        assert_eq!(sink.position(), written as u64);
    }
    sink.finish().unwrap().sync_all().unwrap();
    check(
        "pread of an append sink",
        &FileSource::open(&path).unwrap(),
        &bytes,
    );
    fs::remove_dir_all(dir).unwrap();
}

/// Many threads fill a preallocated file block by block, from buffers aligned in memory
/// and not, with and without direct I/O; every backend then reads the same bytes.
#[test]
fn a_block_writer_assembles_the_file_from_parallel_writes() {
    let dir = scratch("blocks");
    for direct in [false, true] {
        for len in [0, 1, A, 5 * A + 123, (2 << 20) + A + 1] {
            let path = dir.join(format!("blocks-{direct}-{len}"));
            let bytes = bytes(len as u64 + 1, len);
            let writer = BlockWriter::create(&path, len as u64, direct).unwrap();
            assert_eq!(writer.len(), len as u64);
            assert_eq!(writer.is_empty(), len == 0);
            if direct && cfg!(any(target_os = "linux", target_os = "macos")) {
                assert!(writer.is_direct());
            }
            // `fallocate` really allocated the blocks: no silent fall-back to a sparse
            // `set_len` (the target directory is on a file system that supports it).
            #[cfg(target_os = "linux")]
            {
                use std::os::unix::fs::MetadataExt;
                let meta = fs::metadata(&path).unwrap();
                assert_eq!(meta.len(), len as u64);
                assert!(
                    meta.blocks() * 512 >= len as u64,
                    "{} blocks for {len} bytes",
                    meta.blocks()
                );
            }
            // Pieces of 1, 2 and 3 blocks round-robin over 3 threads.
            let mut pieces = Vec::new();
            let mut start = 0;
            for k in (1..=3).cycle() {
                if start >= len {
                    break;
                }
                let end = (start + k * A).min(len);
                pieces.push(start..end);
                start = end;
            }
            std::thread::scope(|s| {
                for t in 0..3 {
                    let (writer, pieces, bytes) = (&writer, &pieces, &bytes);
                    s.spawn(move || {
                        for (i, range) in pieces.iter().enumerate().skip(t).step_by(3) {
                            let data = &bytes[range.clone()];
                            if i % 2 == 0 {
                                let mut aligned = AlignedBuf::try_zeroed(data.len()).unwrap();
                                aligned.as_mut_slice().copy_from_slice(data);
                                writer
                                    .write_at(range.start as u64, aligned.as_slice())
                                    .unwrap();
                            } else {
                                let mut shifted = vec![0; data.len() + 1];
                                shifted[1..].copy_from_slice(data);
                                writer.write_at(range.start as u64, &shifted[1..]).unwrap();
                            }
                        }
                    });
                }
            });
            let file = writer.finish().unwrap();
            assert_eq!(file.metadata().unwrap().len(), len as u64);
            drop(file);
            check(
                "pread of a block writer",
                &FileSource::open(&path).unwrap(),
                &bytes,
            );
            check(
                "mmap of a block writer",
                &MmapSource::open(&path).unwrap(),
                &bytes,
            );
        }
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_block_writer_refuses_writes_that_break_the_block_rule() {
    let dir = scratch("block-rules");
    for direct in [false, true] {
        let len = 3 * A as u64 + 10;
        let writer = BlockWriter::create(dir.join(format!("rules-{direct}")), len, direct).unwrap();
        let block = vec![1_u8; A];
        for (offset, size) in [
            (1, A),               // offset not on a block boundary
            (0, A + 1),           // not whole blocks and not the end of the file
            (A as u64, 10),       // the same, shorter
            (3 * A as u64, 11),   // past the end
            (4 * A as u64, 0),    // starts past the end
            (u64::MAX - 4095, A), // would wrap
        ] {
            let error = writer
                .write_at(offset, &block.repeat(2)[..size])
                .unwrap_err();
            assert_eq!(
                error.kind(),
                std::io::ErrorKind::InvalidInput,
                "{size} at {offset}"
            );
        }
        writer.write_at(3 * A as u64, &block[..10]).unwrap();
        writer.write_at(0, &block).unwrap();
        writer.write_at(len, &[]).unwrap_err(); // not a block boundary
    }
    fs::remove_dir_all(dir).unwrap();
}

/// Threads append records to shards; each shard reads back exactly its records, and on
/// Unix the files are gone from the directory from the start.
#[test]
fn spill_logs_read_back_per_shard_and_leave_nothing_behind() {
    let dir = scratch("spill");
    let shards = 5;
    let logs = SpillLogs::create(&dir, shards).unwrap();
    assert_eq!(logs.shards(), shards);
    if cfg!(unix) {
        assert_eq!(
            fs::read_dir(&dir).unwrap().count(),
            0,
            "spill files are unlinked at creation"
        );
    }
    std::thread::scope(|s| {
        for t in 0..4_u32 {
            let logs = &logs;
            s.spawn(move || {
                for i in 0..2000_u32 {
                    let record = [t.to_le_bytes(), i.to_le_bytes()].concat();
                    logs.append(usize::try_from(i).unwrap() % shards, &record)
                        .unwrap();
                }
            });
        }
    });
    let sources = logs.finish().unwrap();
    assert_eq!(sources.len(), shards);
    for (shard, source) in sources.iter().enumerate() {
        let mut reader = SegmentReader::new();
        let bytes = reader.read(source, 0..source.len()).unwrap();
        let mut records: Vec<(u32, u32)> = bytes
            .chunks_exact(8)
            .map(|r| {
                let word = |k: usize| u32::from_le_bytes(r[k..k + 4].try_into().unwrap());
                (word(0), word(4))
            })
            .collect();
        records.sort_unstable();
        let expected: Vec<(u32, u32)> = (0..4)
            .flat_map(|t| {
                (0..2000)
                    .filter(move |i| usize::try_from(i % 5).unwrap() == shard)
                    .map(move |i| (t, i))
            })
            .collect();
        assert_eq!(records, expected, "shard {shard}");
    }
    drop(sources);
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
    fs::remove_dir_all(dir).unwrap();
}

/// A file is mapped and its slice borrowed, then this crate's sinks
/// are pointed at it, under its own path and through a hard link. Each is refused
/// before anything changes, so the borrowed bytes stay the same; once the map is gone,
/// the same calls succeed. The other order is refused too: a file being written cannot
/// be mapped.
#[cfg(unix)]
#[test]
fn sinks_refuse_a_mapped_file_and_maps_refuse_a_file_being_written() {
    use std::io::ErrorKind::ResourceBusy;

    use everygraph_core::ByteSink;

    let dir = scratch("guard");
    let path = dir.join("mapped");
    let link = dir.join("hard-link");
    write_file(&path, &[7; 3 * A]);
    fs::hard_link(&path, &link).unwrap();

    let map = MmapSource::open(&path).unwrap();
    let bytes = map.as_slice().unwrap();
    for target in [&path, &link] {
        for direct in [false, true] {
            let refused = BlockWriter::create(target, 2 * A as u64, direct).unwrap_err();
            assert_eq!(refused.kind(), ResourceBusy);
        }
        assert_eq!(AppendSink::create(target).unwrap_err().kind(), ResourceBusy);
        assert_eq!(FileSink::create(target).unwrap_err().kind(), ResourceBusy);
        // A FileSink opens, and is refused each write into the map.
        let mut sink = FileSink::open(target).unwrap();
        assert_eq!(kind(sink.write_at(0, &[1])), ResourceBusy);
        assert_eq!(kind(sink.set_len(0)), ResourceBusy);
        drop(sink);
        let file = File::options().write(true).open(target).unwrap();
        assert_eq!(
            AppendSink::from_file(file).unwrap_err().kind(),
            ResourceBusy
        );
    }
    assert_eq!(
        fs::metadata(&path).unwrap().len(),
        3 * A as u64,
        "nothing was truncated"
    );
    assert!(
        bytes.iter().all(|&b| b == 7),
        "the borrowed bytes did not change"
    );
    // A second map of a mapped file is fine.
    let again = MmapSource::open(&link).unwrap();
    drop((map, again));

    // The maps are gone: writers may open the file, and while they hold it, maps are
    // refused.
    let writer = BlockWriter::create(&path, A as u64, false).unwrap();
    assert_eq!(MmapSource::open(&link).unwrap_err().kind(), ResourceBusy);
    writer.write_at(0, &[9; A]).unwrap();
    writer.finish().unwrap();
    let mut sink = AppendSink::create(&link).unwrap();
    assert_eq!(MmapSource::open(&path).unwrap_err().kind(), ResourceBusy);
    sink.write_all(&[5; 10]).unwrap();
    drop(sink);
    let map = MmapSource::open(&path).unwrap();
    assert_eq!(map.as_slice().unwrap(), &[5; 10]);
    drop(map);
    // A FileSink holds the file alone, but it may be mapped beside it.
    let sink = FileSink::open(&link).unwrap();
    assert_eq!(AppendSink::create(&path).unwrap_err().kind(), ResourceBusy);
    MmapSource::open(&path).unwrap();
    drop(sink.into_file());
    fs::remove_dir_all(dir).unwrap();
}

/// The kind of the I/O error a `FileSink` returned.
#[cfg(unix)]
fn kind<T: std::fmt::Debug>(result: Result<T, everygraph_core::WriteError>) -> std::io::ErrorKind {
    match result {
        Err(everygraph_core::WriteError::Backend(e)) => {
            e.downcast_ref::<std::io::Error>().unwrap().kind()
        }
        other => panic!("expected an I/O error, got {other:?}"),
    }
}

/// A `FileSink` beside a range map: every write and length change that would reach a
/// mapped byte is refused, everything else goes through, and the mapped bytes never
/// change. A map taken while the sink is open is checked the same way.
#[cfg(unix)]
#[test]
fn a_file_sink_writes_beside_a_range_map_and_never_into_it() {
    use std::io::ErrorKind::ResourceBusy;

    use everygraph_core::ByteSink;

    let dir = scratch("range-guard");
    let path = dir.join("file");
    let a = A as u64;
    write_file(&path, &[7; 4 * A]);
    let (start, end) = (a + 100, 3 * a - 5);
    let map = MmapSource::open_range(&path, start..end).unwrap();
    let mut sink = FileSink::open(&path).unwrap();
    for (offset, len) in [(start - 1, 2), (end - 1, 1), (0, 4 * A), (start + 7, 1)] {
        assert_eq!(
            kind(sink.write_at(offset, &vec![1; len])),
            ResourceBusy,
            "{len} at {offset}"
        );
    }
    for (offset, len) in [(0, 100), (start - 1, 1), (end, 1), (4 * a, 10)] {
        sink.write_at(offset, &vec![2; len]).unwrap();
    }
    assert_eq!(kind(sink.set_len(end - 1)), ResourceBusy);
    assert_eq!(kind(sink.set_len(0)), ResourceBusy);
    sink.set_len(end).unwrap();
    sink.set_len(10 * a).unwrap();
    assert!(map.mapped().iter().all(|&b| b == 7));
    assert_eq!(map.mapped().len() as u64, end - start);

    // A second map, taken while the sink is open, guards its range too.
    let head = MmapSource::open_range(&path, 0..10).unwrap();
    assert_eq!(kind(sink.write_at(5, &[3])), ResourceBusy);
    drop((map, head));
    sink.write_at(start, &[4]).unwrap();
    sink.set_len(0).unwrap();
    fs::remove_dir_all(dir).unwrap();
}

/// A map reads the file's length and registers its range in one step, so a `FileSink`
/// of another thread that shortens the file can never leave a map past its end. Before
/// that, this loop ended the process with `SIGBUS` within seconds (review of PR #216).
#[cfg(unix)]
#[test]
fn a_map_never_reaches_past_the_end_while_another_thread_shortens_the_file() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    use everygraph_core::ByteSink;

    let dir = scratch("map-race");
    let path = dir.join("file");
    write_file(&path, &[7; 16 * A]);
    let stop = AtomicBool::new(false);
    let mut maps = 0_u64;
    std::thread::scope(|s| {
        s.spawn(|| {
            let mut sink = FileSink::open(&path).unwrap();
            while !stop.load(Ordering::Relaxed) {
                // Refused while a map covers the bytes cut off; that is the point.
                let _ = sink.set_len(16 * A as u64);
                let _ = sink.set_len(A as u64);
            }
        });
        // Set however this thread ends, so that a failed assertion ends the test
        // instead of leaving the other thread to spin.
        struct Stop<'a>(&'a AtomicBool);
        impl Drop for Stop<'_> {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Relaxed);
            }
        }
        let _stop = Stop(&stop);
        let until = Instant::now() + Duration::from_secs(2);
        while Instant::now() < until {
            let map = MmapSource::from_file(&File::open(&path).unwrap()).unwrap();
            if let Some(&last) = map.mapped().last() {
                assert!(last == 7 || last == 0);
            }
            maps += 1;
        }
    });
    assert!(maps > 0);
    fs::remove_dir_all(dir).unwrap();
}
