// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Bounded, aligned chunks of a [`ByteSource`]: what readers and kernels loop over.
//!
//! [`Segments`] cuts `0..len` into ranges of a fixed size; [`SegmentReader`] reads one
//! range at a time into a buffer it reuses, or hands out the bytes in place when the
//! source is already in memory. Segment boundaries and the reader's buffer both sit on
//! multiples of [`SEGMENT_ALIGN`], which is what lets a direct-I/O backend (`O_DIRECT`)
//! read a segment straight into the buffer, without a second copy.
//!
//! Segment boundaries depend only on the length and the size passed in, never on the
//! machine, like the chunks of [`par`](crate::par). A parallel scan gives each worker
//! its own reader and distributes segment indices; the lab's direct reader used 8 MiB
//! per worker ([`DEFAULT_SEGMENT_SIZE`]).

use std::collections::TryReserveError;
use std::ops::Range;

use crate::{ByteSource, ReadError};

/// Alignment of segment boundaries and of [`AlignedBuf`], in bytes. 4 KiB is the page
/// size of the common platforms and a multiple of every logical block size that direct
/// I/O requires on them.
pub const SEGMENT_ALIGN: usize = 4096;

/// The segment size the lab's direct reader used per worker: 8 MiB.
pub const DEFAULT_SEGMENT_SIZE: u64 = 8 << 20;

const ALIGN: u64 = SEGMENT_ALIGN as u64;

/// A zero-filled heap buffer whose first byte is at a multiple of [`SEGMENT_ALIGN`].
///
/// It is built from a `Vec<u8>` a little longer than asked, without `unsafe`, using
/// `align_offset`, which is allowed to report that alignment is impossible (no real
/// allocator does). The buffer is then unaligned. For a buffer a caller reads into,
/// that costs speed only: a direct-I/O backend copies through its own bounce buffer.
/// The bounce buffers themselves are `AlignedBuf`s, though, and an unaligned one would
/// make every direct read fail with an error.
#[derive(Debug, Default)]
pub struct AlignedBuf {
    raw: Vec<u8>,
    start: usize,
    len: usize,
}

impl AlignedBuf {
    /// Allocates `len` zero bytes, aligned.
    ///
    /// # Errors
    ///
    /// The allocation failed, or `len` plus the alignment slack overflows `usize`.
    /// Nothing is allocated in that case.
    pub fn try_zeroed(len: usize) -> Result<Self, TryReserveError> {
        let mut raw = Vec::new();
        // `usize::MAX` makes `try_reserve_exact` fail with a capacity overflow, which is
        // the error the caller should see for a length that cannot be padded.
        raw.try_reserve_exact(len.saturating_add(SEGMENT_ALIGN - 1))?;
        raw.resize(raw.capacity(), 0);
        let offset = raw.as_ptr().align_offset(SEGMENT_ALIGN);
        let start = if offset < SEGMENT_ALIGN { offset } else { 0 };
        Ok(Self { raw, start, len })
    }

    /// Length in bytes, as asked for.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether the buffer has no bytes.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The bytes.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.raw[self.start..self.start + self.len]
    }

    /// The bytes, writable.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.raw[self.start..self.start + self.len]
    }
}

/// The segments of `0..len`: consecutive ranges of [`size`](Self::size) bytes, the last
/// one shorter. An iterator over the ranges, and random access with [`get`](Self::get)
/// for code that hands segment indices to workers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segments {
    len: u64,
    size: u64,
    next: u64,
}

impl Segments {
    /// Segments of `0..len` of `size` bytes, rounded up to a multiple of
    /// [`SEGMENT_ALIGN`] (a `size` of 0 means one alignment unit).
    #[must_use]
    pub const fn new(len: u64, size: u64) -> Self {
        let size = if size == 0 {
            ALIGN
        } else {
            match size.checked_next_multiple_of(ALIGN) {
                Some(rounded) => rounded,
                None => u64::MAX / ALIGN * ALIGN,
            }
        };
        Self { len, size, next: 0 }
    }

    /// The segments of `source`, of `size` bytes as in [`new`](Self::new).
    #[must_use]
    pub fn of<S: ByteSource + ?Sized>(source: &S, size: u64) -> Self {
        Self::new(source.len(), size)
    }

    /// Size of every segment but the last, a multiple of [`SEGMENT_ALIGN`].
    #[must_use]
    pub const fn size(&self) -> u64 {
        self.size
    }

    /// Number of segments in all; 0 for an empty source.
    #[must_use]
    pub const fn count_all(&self) -> u64 {
        self.len.div_ceil(self.size)
    }

    /// Range of segment `index`, or `None` past the last one.
    #[must_use]
    pub const fn get(&self, index: u64) -> Option<Range<u64>> {
        if index >= self.count_all() {
            return None;
        }
        // `index < ceil(len / size)` gives `index * size < len`: no overflow.
        let start = index * self.size;
        let rest = self.len - start;
        let end = start + if rest < self.size { rest } else { self.size };
        Some(start..end)
    }
}

impl Iterator for Segments {
    type Item = Range<u64>;

    fn next(&mut self) -> Option<Range<u64>> {
        let range = self.get(self.next)?;
        self.next += 1;
        Some(range)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let left = self.count_all().saturating_sub(self.next);
        usize::try_from(left).map_or((usize::MAX, None), |n| (n, Some(n)))
    }
}

/// Reads ranges of a [`ByteSource`] one at a time, reusing one aligned buffer.
///
/// For a source with [`as_slice`](ByteSource::as_slice) the bytes are returned in
/// place and nothing is allocated. Otherwise the buffer is allocated on first use and
/// grows to the longest range read; one reader per worker keeps that to one buffer each.
#[derive(Debug, Default)]
pub struct SegmentReader {
    buf: AlignedBuf,
}

impl SegmentReader {
    /// A reader with no buffer yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The bytes of `range` in `source`.
    ///
    /// # Errors
    ///
    /// [`ReadError::OutOfBounds`] if `range` is not inside the source (or ends before
    /// it starts), and [`ReadError::Backend`] if the source failed, or if the buffer
    /// for a range of this length cannot be allocated (the allocation error is the
    /// cause).
    pub fn read<'a, S: ByteSource + ?Sized>(
        &'a mut self,
        source: &'a S,
        range: Range<u64>,
    ) -> Result<&'a [u8], ReadError> {
        let source_len = source.len();
        let out_of_bounds = || ReadError::OutOfBounds {
            offset: range.start,
            len: range.end.saturating_sub(range.start),
            source_len,
        };
        if range.start > range.end || range.end > source_len {
            return Err(out_of_bounds());
        }
        if let Some(all) = source.as_slice() {
            // In bounds of a slice that exists, so both ends fit in `usize`.
            let start = usize::try_from(range.start).map_err(|_| out_of_bounds())?;
            let end = usize::try_from(range.end).map_err(|_| out_of_bounds())?;
            return all.get(start..end).ok_or_else(out_of_bounds);
        }
        let len = usize::try_from(range.end - range.start)
            .map_err(|_| ReadError::Backend("segment is larger than the address space".into()))?;
        // TODO(#31 integration): this buffer (8 MiB per worker with the default segment
        // size) is allocated outside the memory accountant.
        if self.buf.len() < len {
            let rounded = len.checked_next_multiple_of(SEGMENT_ALIGN).unwrap_or(len);
            // Drop the old buffer first, so that growing never holds both.
            self.buf = AlignedBuf::default();
            self.buf = AlignedBuf::try_zeroed(rounded).map_err(|e| ReadError::Backend(e.into()))?;
        }
        let buf = &mut self.buf.as_mut_slice()[..len];
        source.read_at(range.start, buf)?;
        Ok(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MemorySource, widen};

    /// A source without `as_slice`, so that the reader's buffer is used.
    struct Copying(Vec<u8>);

    impl ByteSource for Copying {
        fn len(&self) -> u64 {
            widen(self.0.len())
        }

        fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), ReadError> {
            MemorySource::new(self.0.as_slice()).read_at(offset, buf)
        }
    }

    fn sample(len: usize) -> Vec<u8> {
        (0..len)
            .map(|i| u8::try_from(i % 251).unwrap().wrapping_mul(7))
            .collect()
    }

    #[test]
    fn sizes_are_rounded_up_to_the_alignment() {
        assert_eq!(Segments::new(10, 0).size(), ALIGN);
        assert_eq!(Segments::new(10, 1).size(), ALIGN);
        assert_eq!(Segments::new(10, ALIGN).size(), ALIGN);
        assert_eq!(Segments::new(10, ALIGN + 1).size(), 2 * ALIGN);
        assert_eq!(Segments::new(10, u64::MAX).size(), u64::MAX / ALIGN * ALIGN);
        assert_eq!(Segments::new(10, u64::MAX).size() % ALIGN, 0);
    }

    /// The segments tile `0..len` exactly, in order, each `size` long but the last,
    /// and `get` agrees with the iterator, for lengths around every boundary.
    #[test]
    fn segments_tile_the_source() {
        for len in [0, 1, ALIGN - 1, ALIGN, ALIGN + 1, 3 * ALIGN, 5 * ALIGN + 17] {
            for size in [1, ALIGN, 2 * ALIGN, 3 * ALIGN + 5, u64::MAX] {
                let segments = Segments::new(len, size);
                let all: Vec<_> = segments.clone().collect();
                assert_eq!(
                    widen(all.len()),
                    segments.count_all(),
                    "len {len}, size {size}"
                );
                assert_eq!(segments.size_hint(), (all.len(), Some(all.len())));
                let mut expected_start = 0;
                for (i, range) in all.iter().enumerate() {
                    assert_eq!(range.start, expected_start);
                    assert!(range.end > range.start);
                    assert_eq!(range.start % ALIGN, 0);
                    if i + 1 < all.len() {
                        assert_eq!(range.end - range.start, segments.size());
                    }
                    assert_eq!(segments.get(widen(i)).as_ref(), Some(range));
                    expected_start = range.end;
                }
                assert_eq!(expected_start, len);
                assert_eq!(segments.get(widen(all.len())), None);
                assert_eq!(segments.get(u64::MAX), None);
            }
        }
    }

    #[test]
    fn segments_of_a_huge_source_do_not_overflow() {
        let segments = Segments::new(u64::MAX, DEFAULT_SEGMENT_SIZE);
        let last = segments.count_all() - 1;
        assert_eq!(segments.get(last).unwrap().end, u64::MAX);
        assert_eq!(segments.get(last + 1), None);
    }

    /// Reading every segment reassembles the source, through the buffer and in place,
    /// and the in-place path really hands out the source's own bytes.
    #[test]
    fn a_reader_reassembles_the_source_with_and_without_a_slice() {
        let bytes = sample(5 * SEGMENT_ALIGN + 123);
        let copying = Copying(bytes.clone());
        let in_memory = MemorySource::new(bytes.as_slice());
        for size in [1, ALIGN, 2 * ALIGN] {
            for source in [&copying as &dyn ByteSource, &in_memory] {
                let mut reader = SegmentReader::new();
                let mut out = Vec::new();
                for range in Segments::of(source, size) {
                    out.extend_from_slice(reader.read(source, range).unwrap());
                }
                assert_eq!(out, bytes, "size {size}");
            }
        }
        let mut reader = SegmentReader::new();
        let inside = reader.read(&in_memory, 10..20).unwrap();
        assert_eq!(inside.as_ptr(), bytes[10..].as_ptr());
        assert!(
            reader.buf.is_empty(),
            "nothing is allocated for an in-memory source"
        );
    }

    #[test]
    fn a_reader_grows_for_a_longer_range_and_reads_any_range() {
        let bytes = sample(3 * SEGMENT_ALIGN);
        let source = Copying(bytes.clone());
        let mut reader = SegmentReader::new();
        assert_eq!(reader.read(&source, 5..9).unwrap(), &bytes[5..9]);
        assert_eq!(reader.buf.len(), SEGMENT_ALIGN);
        assert_eq!(reader.read(&source, 1..9000).unwrap(), &bytes[1..9000]);
        assert_eq!(reader.buf.len(), 3 * SEGMENT_ALIGN);
        assert_eq!(reader.read(&source, 7..7).unwrap(), &[] as &[u8]);
    }

    #[test]
    fn a_reader_refuses_ranges_outside_the_source() {
        let bytes = sample(100);
        let copying = Copying(bytes.clone());
        let in_memory = MemorySource::new(bytes);
        for source in [&copying as &dyn ByteSource, &in_memory] {
            let mut reader = SegmentReader::new();
            for (range, len) in [
                (90..101, 11),
                (101..101, 0),
                (Range { start: 50, end: 40 }, 0),
                (u64::MAX - 1..u64::MAX, 1),
            ] {
                match reader.read(source, range.clone()) {
                    Err(ReadError::OutOfBounds {
                        offset,
                        len: l,
                        source_len,
                    }) => assert_eq!((offset, l, source_len), (range.start, len, 100)),
                    other => panic!("{range:?}: expected OutOfBounds, got {other:?}"),
                }
            }
        }
    }

    #[test]
    fn an_aligned_buffer_has_the_length_asked_for_and_is_zeroed() {
        for len in [0, 1, 4095, 4096, 10_000] {
            let mut buf = AlignedBuf::try_zeroed(len).unwrap();
            assert_eq!(buf.len(), len);
            assert_eq!(buf.is_empty(), len == 0);
            assert!(buf.as_slice().iter().all(|&b| b == 0));
            buf.as_mut_slice().fill(3);
            assert!(buf.as_slice().iter().all(|&b| b == 3));
        }
        assert!(AlignedBuf::try_zeroed(usize::MAX).is_err());
        assert!(AlignedBuf::try_zeroed(usize::MAX - SEGMENT_ALIGN).is_err());
    }
}
