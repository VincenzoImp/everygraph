// SPDX-License-Identifier: Apache-2.0 OR MIT

//! [`FileSource`]: positional reads of a file, through the page cache (`pread`) or
//! around it (direct I/O).

use std::cell::RefCell;
use std::fs::{File, OpenOptions};
use std::io;
use std::ops::Range;
use std::path::Path;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};

use everygraph_core::{AccessHint, AlignedBuf, ByteSource, ReadError, SEGMENT_ALIGN};

use crate::{backend_error, check_bounds, sys};

const ALIGN: u64 = SEGMENT_ALIGN as u64;

/// Largest single direct read: 8 MiB, the lab's per-worker chunk. A longer read is
/// issued as consecutive requests of this size.
const REQUEST: usize = 8 << 20;

/// Size of the per-thread bounce buffer for the parts of a direct read that are not
/// aligned: 1 MiB.
const BOUNCE: usize = 1 << 20;

/// Bytes of direct reads one source lets be in flight at once: 32 MiB. A thread whose
/// read would exceed it waits until enough reads have finished (one read always goes).
///
/// Without the bound, more threads made the device slower. Exploratory, the lab's
/// drive, orkut `targets.u32` read with `O_DIRECT`, one read per 8 MiB segment per thread:
/// 4 threads 7.2–7.4 GB/s, 8 threads 5.5–5.8, 32 threads 5.0–5.5; 32 threads reading
/// 1 MiB segments, 7.4 (not recorded). The official numbers, on Friendster, are in
/// `bench/results/io/20261125T205022Z-21498df39590.jsonl`.
const MAX_IN_FLIGHT: u64 = 32 << 20;

/// A file read with positional reads, from any number of threads at once.
///
/// Two modes, chosen when the file is opened:
///
/// - [`open`](Self::open): ordinary `pread` through the page cache. The default for
///   files that are read more than once or that fit in memory.
/// - [`open_direct`](Self::open_direct): direct I/O (`O_DIRECT` on Linux, `F_NOCACHE`
///   on macOS, `FILE_FLAG_NO_BUFFERING` on Windows). For a large input read once, front
///   to back: in the lab it made the input side of ingest faster (FINDINGS § B1).
///   Reads whose offset and buffer are aligned to [`SEGMENT_ALIGN`]
///   go straight to the device into the caller's buffer, which is what
///   [`SegmentReader`](everygraph_core::SegmentReader) produces; other reads are
///   correct but go through a bounce buffer. At most 32 MiB of reads are in flight per
///   source, whatever the number of threads: more made the lab's drive slower.
///
/// The length is taken when the file is opened. The file must not change while the
/// source is open (the `ByteSource` contract); if it shrinks, reads past the new end
/// fail with [`ReadError::Backend`].
#[derive(Debug)]
pub struct FileSource {
    file: File,
    len: u64,
    direct: bool,
    in_flight: InFlight,
}

impl FileSource {
    /// Opens `path` for reading through the page cache.
    ///
    /// # Errors
    ///
    /// The file cannot be opened or its length read.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::from_file(File::open(path)?)
    }

    /// Opens `path` for direct reads that bypass the page cache. Where the platform or
    /// the file system cannot do that, the file is opened for ordinary reads instead
    /// and [`is_direct`](Self::is_direct) says so.
    ///
    /// # Errors
    ///
    /// The file cannot be opened or its length read.
    pub fn open_direct(path: impl AsRef<Path>) -> io::Result<Self> {
        let (file, direct) = sys::open_uncached(OpenOptions::new().read(true), path.as_ref())?;
        let len = file.metadata()?.len();
        Ok(Self {
            file,
            len,
            direct,
            in_flight: InFlight::default(),
        })
    }

    /// Reads `file` through the page cache, from its current length.
    ///
    /// # Errors
    ///
    /// The file's length cannot be read.
    pub fn from_file(file: File) -> io::Result<Self> {
        let len = file.metadata()?.len();
        Ok(Self {
            file,
            len,
            direct: false,
            in_flight: InFlight::default(),
        })
    }

    /// Whether reads bypass the page cache.
    #[must_use]
    pub const fn is_direct(&self) -> bool {
        self.direct
    }

    fn read_direct(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        // The bytes before the first aligned file offset, and whether the buffer is
        // aligned at that same point: then everything between there and the last
        // aligned offset can go to the device without a copy.
        let head = usize::try_from(offset.next_multiple_of(ALIGN) - offset)
            .unwrap_or(usize::MAX)
            .min(buf.len());
        let body = (buf.len() - head) / SEGMENT_ALIGN * SEGMENT_ALIGN;
        if body > 0 && buf[head..].as_ptr().align_offset(SEGMENT_ALIGN) == 0 {
            let (first, rest) = buf.split_at_mut(head);
            let (middle, last) = rest.split_at_mut(body);
            self.bounced(offset, first)?;
            let mut at = offset + head as u64;
            for piece in middle.chunks_mut(REQUEST) {
                let _slot = self.in_flight.enter(piece.len() as u64);
                sys::read_exact_at(&self.file, piece, at)?;
                at += piece.len() as u64;
            }
            self.bounced(at, last)
        } else {
            self.bounced(offset, buf)
        }
    }

    /// Fills `buf` from `offset` through an aligned bounce buffer, in aligned reads of
    /// at most [`BOUNCE`] bytes.
    fn bounced(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        if buf.is_empty() {
            return Ok(());
        }
        BOUNCE_BUF.with_borrow_mut(|slot| {
            if slot.is_none() {
                *slot = Some(AlignedBuf::try_zeroed(BOUNCE).map_err(io::Error::other)?);
            }
            let bounce = slot
                .as_mut()
                .map(AlignedBuf::as_mut_slice)
                .unwrap_or_default();
            let (mut buf, mut offset) = (buf, offset);
            while !buf.is_empty() {
                let start = offset / ALIGN * ALIGN;
                let skip = usize::try_from(offset - start).unwrap_or(0);
                let wanted = (skip + buf.len())
                    .next_multiple_of(SEGMENT_ALIGN)
                    .min(BOUNCE);
                let got = {
                    let _slot = self.in_flight.enter(wanted as u64);
                    sys::read_up_to(&self.file, &mut bounce[..wanted], start)?
                };
                let n = got.saturating_sub(skip).min(buf.len());
                if n == 0 {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
                buf[..n].copy_from_slice(&bounce[skip..skip + n]);
                buf = &mut buf[n..];
                offset += n as u64;
            }
            Ok(())
        })
    }
}

// TODO(#31 integration): 1 MiB per thread that ever bounced, kept until the thread
// exits, outside the memory accountant.
thread_local! {
    static BOUNCE_BUF: RefCell<Option<AlignedBuf>> = const { RefCell::new(None) };
}

/// The bytes of direct reads in flight on one source, bounded by [`MAX_IN_FLIGHT`].
#[derive(Debug, Default)]
struct InFlight {
    bytes: Mutex<u64>,
    finished: Condvar,
}

/// One read in flight; leaving its scope, also by unwinding, ends it.
struct Slot<'a> {
    in_flight: &'a InFlight,
    bytes: u64,
}

impl InFlight {
    fn lock(&self) -> MutexGuard<'_, u64> {
        // The count is updated in one statement, so a panicking holder cannot leave it
        // half-written.
        self.bytes.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn enter(&self, bytes: u64) -> Slot<'_> {
        let mut in_flight = self.lock();
        while *in_flight > 0 && *in_flight + bytes > MAX_IN_FLIGHT {
            in_flight = self
                .finished
                .wait(in_flight)
                .unwrap_or_else(PoisonError::into_inner);
        }
        *in_flight += bytes;
        Slot {
            in_flight: self,
            bytes,
        }
    }
}

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        *self.in_flight.lock() -= self.bytes;
        self.in_flight.finished.notify_all();
    }
}

impl ByteSource for FileSource {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), ReadError> {
        check_bounds(offset, buf.len(), self.len)?;
        if self.direct {
            self.read_direct(offset, buf)
        } else {
            sys::read_exact_at(&self.file, buf, offset)
        }
        .map_err(backend_error)
    }

    fn advise(&self, range: Range<u64>, hint: AccessHint) {
        // Direct reads do not go through the cache the advice is for.
        if !self.direct {
            let end = range.end.min(self.len);
            if range.start < end {
                sys::fadvise(&self.file, range.start..end, hint);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, mpsc};
    use std::time::Duration;

    use super::*;

    /// Many threads entering with reads of mixed sizes never have more than the bound
    /// in flight, and a read larger than the bound still goes, alone. Under a deadline,
    /// so that a cap that blocks for good fails the test instead of hanging it.
    #[test]
    fn reads_in_flight_stay_under_the_bound() {
        let in_flight = Arc::new(InFlight::default());
        let peak = Arc::new(AtomicU64::new(0));
        let (done, finished) = mpsc::channel();
        {
            let (in_flight, peak) = (Arc::clone(&in_flight), Arc::clone(&peak));
            std::thread::spawn(move || {
                let now = AtomicU64::new(0);
                let sizes = [8 << 20, 1 << 20, 4096, MAX_IN_FLIGHT, MAX_IN_FLIGHT + 1];
                std::thread::scope(|s| {
                    for t in 0..16 {
                        let (in_flight, now, peak) = (&*in_flight, &now, &*peak);
                        s.spawn(move || {
                            for i in 0..20 {
                                let bytes = sizes[(t + i) % sizes.len()];
                                let _slot = in_flight.enter(bytes);
                                let current = now.fetch_add(bytes, Ordering::SeqCst) + bytes;
                                peak.fetch_max(current, Ordering::SeqCst);
                                assert!(
                                    current <= MAX_IN_FLIGHT || current == bytes,
                                    "{current} bytes in flight"
                                );
                                std::thread::sleep(Duration::from_micros(200));
                                now.fetch_sub(bytes, Ordering::SeqCst);
                            }
                        });
                    }
                });
                let _ = done.send(());
            });
        }
        finished
            .recv_timeout(Duration::from_secs(20))
            .expect("the readers did not finish: the cap blocks, or a reader panicked");
        assert_eq!(*in_flight.lock(), 0);
        assert!(peak.load(Ordering::SeqCst) > 8 << 20, "reads did overlap");
    }
}
