// SPDX-License-Identifier: Apache-2.0 OR MIT

//! [`ByteSource`]: how every layer above reads bytes without knowing where they are.

use std::error::Error;
use std::fmt;
use std::ops::Range;

use crate::widen;

/// Positional, re-readable access to a fixed sequence of bytes.
///
/// A file, a memory map, an HTTP resource read by range requests and a `File` dropped on
/// a browser page are all byte sources. Parsers and readers are written against this
/// trait and never against any of those, which is what lets them run everywhere
/// (DESIGN D2). The contract:
///
/// - **Positional.** There is no cursor. Every read names its offset and takes `&self`,
///   so several workers can read different parts of one source.
/// - **Chunked.** Callers read bounded pieces into buffers they own; nothing in the
///   trait requires the whole source to be in memory or even to fit in the address
///   space. That is why lengths and offsets are `u64` on every target.
/// - **Re-readable.** [`len`](Self::len) is constant, and reading the same range twice
///   yields the same bytes. Two-pass builders (count, then fill) depend on it. Something
///   that can be consumed only once, such as a pipe, must be spooled by its backend
///   before it is offered as a `ByteSource`. One backend states an exception: a memory
///   map of one range of a file (`everygraph_io::MmapSource::from_file_range`) is
///   re-readable inside that range only.
///
/// The trait does not require `Send` or `Sync`, because a browser `File` handle is
/// neither. Code that reads from several threads asks for `ByteSource + Sync`.
///
/// Two optional hooks have defaults that every backend may keep: [`as_slice`] for
/// backends whose bytes are already addressable (memory, a memory map), and [`advise`]
/// for backends that can act on a read-ahead or access-pattern hint. The native
/// backends live in `everygraph-io`; a later backend (HTTP ranges, `io_uring`, a browser
/// `File`) implements the same three methods and needs nothing else from this trait.
///
/// [`as_slice`]: Self::as_slice
/// [`advise`]: Self::advise
pub trait ByteSource {
    /// Total length in bytes.
    fn len(&self) -> u64;

    /// Whether the source has no bytes.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Fills `buf` with the bytes at `offset..offset + buf.len()`.
    ///
    /// The read is exact: on `Ok` the whole buffer has been filled. A backend whose
    /// underlying call may return fewer bytes loops until it is done. An empty `buf`
    /// succeeds for any `offset <= len()`.
    ///
    /// # Errors
    ///
    /// [`ReadError::OutOfBounds`] if the range is not entirely inside the source, and
    /// [`ReadError::Backend`] if the backend failed. In both cases the contents of
    /// `buf` are unspecified.
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), ReadError>;

    /// The whole source as one slice, when the backend already holds it in addressable
    /// memory (an in-memory buffer, a memory map); `None` otherwise, which is the
    /// default. When it is `Some`, its length is [`len`](Self::len) and its bytes are
    /// the ones [`read_at`](Self::read_at) returns, so a reader may use either.
    fn as_slice(&self) -> Option<&[u8]> {
        None
    }

    /// The bytes of `range` as one slice, when the backend holds them in addressable
    /// memory; `None` otherwise, and for a range that is not inside the source. The
    /// default takes them from [`as_slice`](Self::as_slice). A backend that holds only
    /// part of its bytes in memory (a memory map of one range of a file) overrides it.
    fn slice(&self, range: Range<u64>) -> Option<&[u8]> {
        let start = usize::try_from(range.start).ok()?;
        let end = usize::try_from(range.end).ok()?;
        self.as_slice()?.get(start..end)
    }

    /// Tells the backend how `range` is about to be read. Advice only: it never fails,
    /// never changes what is read, and a backend may ignore it, which is the default.
    /// The part of `range` outside the source is ignored.
    ///
    /// The lab's case for it: on a cold memory-mapped graph, asking for a whole BFS
    /// frontier with [`AccessHint::WillNeed`] before touching it took a 2-hop query
    /// from 8.7 to 0.47 ms (FINDINGS § F1, orkut, lab prototype).
    fn advise(&self, range: Range<u64>, hint: AccessHint) {
        let _ = (range, hint);
    }
}

impl<S: ByteSource + ?Sized> ByteSource for &S {
    fn len(&self) -> u64 {
        (**self).len()
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), ReadError> {
        (**self).read_at(offset, buf)
    }

    fn as_slice(&self) -> Option<&[u8]> {
        (**self).as_slice()
    }

    fn slice(&self, range: Range<u64>) -> Option<&[u8]> {
        (**self).slice(range)
    }

    fn advise(&self, range: Range<u64>, hint: AccessHint) {
        (**self).advise(range, hint);
    }
}

/// How a range of a [`ByteSource`] is about to be read; see [`ByteSource::advise`].
/// The native backends pass it on to `madvise` or `posix_fadvise`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AccessHint {
    /// No particular pattern: undo an earlier hint.
    Normal,
    /// Front to back, once: read ahead aggressively.
    Sequential,
    /// Scattered small reads: do not read ahead.
    Random,
    /// The range will be read soon: start fetching it now.
    WillNeed,
}

/// Why a [`ByteSource::read_at`] failed.
#[derive(Debug)]
#[non_exhaustive]
pub enum ReadError {
    /// The requested range is not entirely inside the source.
    OutOfBounds {
        /// Offset of the first byte requested.
        offset: u64,
        /// Number of bytes requested.
        len: u64,
        /// Length of the source.
        source_len: u64,
    },
    /// The backend could not deliver the bytes (I/O error, network failure, …).
    Backend(Box<dyn Error + Send + Sync>),
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfBounds {
                offset,
                len,
                source_len,
            } => write!(
                f,
                "read of {len} bytes at offset {offset} is outside a source of {source_len} bytes"
            ),
            Self::Backend(cause) => write!(f, "the byte source failed: {cause}"),
        }
    }
}

impl Error for ReadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::OutOfBounds { .. } => None,
            Self::Backend(cause) => Some(cause.as_ref()),
        }
    }
}

/// A [`ByteSource`] over bytes already in memory: a `Vec<u8>`, a `&[u8]`, an
/// `Arc<[u8]>`, or anything else that derefs to a byte slice.
#[derive(Clone, Debug, Default)]
pub struct MemorySource<B = Vec<u8>> {
    pub(crate) bytes: B,
}

impl<B: AsRef<[u8]>> MemorySource<B> {
    /// Wraps `bytes`. The bytes are not copied.
    pub const fn new(bytes: B) -> Self {
        Self { bytes }
    }
}

impl<B: AsRef<[u8]>> ByteSource for MemorySource<B> {
    fn len(&self) -> u64 {
        widen(self.bytes.as_ref().len())
    }

    fn as_slice(&self) -> Option<&[u8]> {
        Some(self.bytes.as_ref())
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), ReadError> {
        let bytes = self.bytes.as_ref();
        let wanted = buf.len();
        let out_of_bounds = || ReadError::OutOfBounds {
            offset,
            len: widen(wanted),
            source_len: widen(bytes.len()),
        };
        // An offset that does not fit in `usize` is past the end of anything in memory.
        let start = usize::try_from(offset).map_err(|_| out_of_bounds())?;
        let end = start.checked_add(wanted).ok_or_else(out_of_bounds)?;
        let range = bytes.get(start..end).ok_or_else(out_of_bounds)?;
        buf.copy_from_slice(range);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn sample(len: u8) -> Vec<u8> {
        (0..len)
            .map(|i| i.wrapping_mul(37).wrapping_add(11))
            .collect()
    }

    /// Reads the whole source in chunks of at most `chunk` bytes.
    fn read_chunked(source: &dyn ByteSource, chunk: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let mut buf = vec![0_u8; chunk];
        let mut offset = 0_u64;
        while offset < source.len() {
            let rest = usize::try_from(source.len() - offset).unwrap_or(usize::MAX);
            let n = chunk.min(rest);
            source.read_at(offset, &mut buf[..n]).unwrap();
            out.extend_from_slice(&buf[..n]);
            offset += widen(n);
        }
        out
    }

    fn assert_out_of_bounds(result: Result<(), ReadError>, offset: u64, len: u64, source_len: u64) {
        match result {
            Err(ReadError::OutOfBounds {
                offset: o,
                len: l,
                source_len: s,
            }) => assert_eq!((o, l, s), (offset, len, source_len)),
            other => panic!("expected OutOfBounds, got {other:?}"),
        }
    }

    #[test]
    fn length_is_the_length_of_the_bytes() {
        assert_eq!(MemorySource::new(sample(23)).len(), 23);
        assert!(!MemorySource::new(sample(23)).is_empty());
        assert_eq!(MemorySource::new(Vec::new()).len(), 0);
        assert!(MemorySource::new(Vec::new()).is_empty());
        assert!(MemorySource::<Vec<u8>>::default().is_empty());
    }

    /// Every (offset, length) pair around a small source behaves like slicing.
    #[test]
    fn read_at_agrees_with_slicing_for_every_range() {
        let bytes = sample(16);
        let source = MemorySource::new(bytes.as_slice());
        for offset in 0..=18_usize {
            for len in 0..=18_usize {
                let mut buf = vec![0xAA_u8; len];
                let result = source.read_at(widen(offset), &mut buf);
                let range = widen(offset)..widen(offset + len);
                match bytes.get(offset..offset + len) {
                    Some(expected) => {
                        result.unwrap();
                        assert_eq!(buf, expected, "offset {offset}, len {len}");
                        assert_eq!(source.slice(range), Some(expected));
                    }
                    None => {
                        assert_out_of_bounds(result, widen(offset), widen(len), 16);
                        assert_eq!(source.slice(range), None);
                    }
                }
            }
        }
    }

    #[test]
    fn an_empty_read_succeeds_up_to_the_end_and_not_beyond() {
        let source = MemorySource::new(sample(8));
        source.read_at(0, &mut []).unwrap();
        source.read_at(8, &mut []).unwrap();
        assert_out_of_bounds(source.read_at(9, &mut []), 9, 0, 8);
    }

    /// Offsets are `u64` on every target; the largest ones must fail cleanly instead of
    /// wrapping (64-bit) or being truncated to an in-bounds index (32-bit).
    #[test]
    fn huge_offsets_are_out_of_bounds_not_wrapped() {
        let source = MemorySource::new(sample(8));
        let mut buf = [0_u8; 4];
        for offset in [1 << 32, (1 << 32) + 2, u64::MAX - 3, u64::MAX] {
            assert_out_of_bounds(source.read_at(offset, &mut buf), offset, 4, 8);
        }
    }

    #[test]
    fn chunked_reads_reassemble_the_source_and_can_be_repeated() {
        let bytes = sample(200);
        let source = MemorySource::new(bytes.clone());
        for chunk in [1, 7, 64, 200, 4096] {
            let first_pass = read_chunked(&source, chunk);
            let second_pass = read_chunked(&source, chunk);
            assert_eq!(first_pass, bytes, "chunk {chunk}");
            assert_eq!(second_pass, bytes, "chunk {chunk}, second pass");
        }
    }

    #[test]
    fn any_byte_container_can_back_a_source() {
        let bytes = sample(50);
        let owned = MemorySource::new(bytes.clone());
        let borrowed = MemorySource::new(bytes.as_slice());
        let shared = MemorySource::new(Arc::<[u8]>::from(bytes.as_slice()));
        let fixed = MemorySource::new([1_u8, 2, 3]);
        assert_eq!(read_chunked(&owned, 16), bytes);
        assert_eq!(read_chunked(&borrowed, 16), bytes);
        assert_eq!(read_chunked(&shared, 16), bytes);
        assert_eq!(read_chunked(&fixed, 2), [1, 2, 3]);
    }

    /// Generic code can take a source by value and be handed a reference.
    #[test]
    fn a_reference_to_a_source_is_a_source() {
        fn first_byte(source: impl ByteSource) -> u8 {
            let mut buf = [0_u8; 1];
            source.read_at(0, &mut buf).unwrap();
            buf[0]
        }
        let source = MemorySource::new(vec![9_u8, 8, 7]);
        let dynamic: &dyn ByteSource = &source;
        assert_eq!(first_byte(&source), 9);
        assert_eq!(first_byte(dynamic), 9);
        assert_eq!(dynamic.len(), 3);
        assert!(!dynamic.is_empty());
        assert_eq!(dynamic.as_slice(), Some(&[9_u8, 8, 7][..]));
        dynamic.advise(0..u64::MAX, AccessHint::WillNeed);
        assert_eq!(first_byte(dynamic), 9);
    }

    /// A backend that implements only the two required methods gets the defaults: no
    /// slice, and advice that changes nothing.
    #[test]
    fn the_optional_hooks_default_to_nothing() {
        struct Minimal;
        impl ByteSource for Minimal {
            fn len(&self) -> u64 {
                1
            }
            fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), ReadError> {
                MemorySource::new([42_u8]).read_at(offset, buf)
            }
        }
        assert_eq!(Minimal.as_slice(), None);
        let by_reference: &dyn ByteSource = &Minimal;
        assert_eq!(by_reference.as_slice(), None);
        assert_eq!(by_reference.slice(0..1), None);
        for hint in [
            AccessHint::Normal,
            AccessHint::Sequential,
            AccessHint::Random,
            AccessHint::WillNeed,
        ] {
            Minimal.advise(0..1, hint);
        }
        let mut buf = [0_u8];
        Minimal.read_at(0, &mut buf).unwrap();
        assert_eq!(buf, [42]);
    }

    #[test]
    fn an_in_memory_source_can_be_shared_between_threads() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<MemorySource<Vec<u8>>>();
        assert_send_sync::<MemorySource<Arc<[u8]>>>();
        assert_send_sync::<ReadError>();
    }

    #[test]
    fn errors_describe_themselves_and_keep_their_cause() {
        let out_of_bounds = ReadError::OutOfBounds {
            offset: 10,
            len: 4,
            source_len: 12,
        };
        assert_eq!(
            out_of_bounds.to_string(),
            "read of 4 bytes at offset 10 is outside a source of 12 bytes"
        );
        assert!(out_of_bounds.source().is_none());

        let backend = ReadError::Backend("connection reset".into());
        assert_eq!(
            backend.to_string(),
            "the byte source failed: connection reset"
        );
        assert_eq!(backend.source().unwrap().to_string(), "connection reset");
    }
}
