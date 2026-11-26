// SPDX-License-Identifier: Apache-2.0 OR MIT

//! [`MmapSource`]: a file mapped into memory.

use std::fs::File;
use std::io;
use std::ops::Range;
use std::path::Path;

use everygraph_core::{AccessHint, ByteSource, ReadError};
use memmap2::Mmap;

use crate::claim::Claim;
use crate::{backend_error, check_bounds, sys};

/// A file mapped read-only into memory, whole or one range of it: the engine's in-RAM
/// mode.
///
/// In the lab a kernel over a warm map ran as fast as over the same bytes on the heap
/// (orkut, `PageRank` f32, 32 threads: 0.380 s mapped, 0.375 s on the heap; FINDINGS § B1,
/// ADR 0011), so there is no separate "load into memory" backend: a graph in RAM is a
/// mapped graph whose pages are resident. The mapped bytes are handed to readers without
/// copying ([`as_slice`](ByteSource::as_slice) for a whole-file map,
/// [`slice`](ByteSource::slice) for any part of the mapped range), and
/// [`advise`](ByteSource::advise) passes hints on to `madvise` — [`AccessHint::WillNeed`]
/// on a whole frontier is what made cold 2-hop queries fast in the lab (FINDINGS § F1).
///
/// A **range map** ([`open_range`](Self::open_range)) maps one range of the file. The
/// source is still the whole file, as long as it was when mapped: offsets are offsets in
/// the file, bytes in the range are read from the map, and bytes outside it with
/// positional reads. Those are read from the file as it is at the time of the read, so
/// only the mapped bytes are re-readable in the sense of [`ByteSource`] when another
/// writer commits: a container reads its header blocks once, when it is opened.
///
/// # The mapped bytes must not change
///
/// The constructors are safe under an invariant recorded in `design/DECISIONS.md`
/// ("Memory-mapped files are read through a safe constructor"): Everygraph maps only
/// files it wrote itself, and **no writer anywhere — another process or this one —
/// modifies the mapped bytes, or truncates the file into them, while they are mapped**.
/// Breaking it is undefined behaviour (bytes change under the `&[u8]` handed out) or
/// ends the process with `SIGBUS` (a truncation removed the pages being read). That
/// includes this crate's own sinks and `std::fs` writes on the same file, under any path
/// or hard link.
///
/// **A graph file is mapped by range** (`design/DECISIONS.md`, "Range maps of a graph
/// file"; `docs/spec/eg-container.md` § 11). A commit to an `.eg` file writes 128 bytes
/// of a header block and bytes from the committed end on, and shortens the file only to
/// a length at or past every earlier committed end. So a map of `8192 … committed end`
/// of the commit a reader opened (`Container::stable_range`) is safe against every
/// commit, by this process or any other that follows the container's protocol. The
/// header blocks and the table are read once, by the probe:
///
/// ```no_run
/// use everygraph_format::container::Container;
/// use everygraph_io::{FileSource, MmapSource};
///
/// let file = std::fs::File::open("graph.eg")?;
/// let probe = Container::open(FileSource::from_file(file.try_clone()?)?)?;
/// let map = MmapSource::from_file_range(&file, probe.stable_range())?;
/// // The commit the probe opened, read through the map: not a second header read,
/// // which could see a newer commit whose sections lie past the map.
/// let graph = probe.with_source(map)?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// A whole-file map of a graph file is not safe against a commit by another process,
/// and in this process it makes every commit to the file fail (the header write is
/// refused, below).
///
/// Within the process the sinks are guarded (`src/claim.rs`), by device and inode, with
/// [`io::ErrorKind::ResourceBusy`]: [`AppendSink`](crate::AppendSink) and
/// [`BlockWriter`](crate::BlockWriter) refuse a mapped file, and a file one of them
/// writes cannot be mapped; a [`FileSink`](crate::FileSink) refuses each write or
/// length change that would reach a mapped range. Writes through `std::fs` and writes
/// by other processes are not guarded. On Windows there is no guard; the system refuses
/// to truncate a mapped file but not to write it.
#[derive(Debug)]
pub struct MmapSource {
    // `None` for an empty range, which cannot be mapped on every platform. Declared
    // before the claim, so that the map is gone before the claim is released.
    map: Option<Mmap>,
    /// Where the map starts in the file.
    start: u64,
    /// The length of the file when it was mapped.
    len: u64,
    /// For reads outside the map; `None` for a whole-file map.
    file: Option<File>,
    _claim: Claim,
}

impl MmapSource {
    /// Maps the whole file at `path`.
    ///
    /// # Errors
    ///
    /// The file cannot be opened or mapped (on a 32-bit target, a file larger than the
    /// address space cannot be), or a sink of this process writes it
    /// ([`io::ErrorKind::ResourceBusy`]).
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::from_file(&File::open(path)?)
    }

    /// Maps the whole of `file`, which must be open for reading. The map stays valid
    /// after `file` is closed.
    ///
    /// # Errors
    ///
    /// As [`open`](Self::open).
    pub fn from_file(file: &File) -> io::Result<Self> {
        Self::map(file, None, None)
    }

    /// Maps `range` of the file at `path`; see the type's docs.
    ///
    /// # Errors
    ///
    /// As [`from_file_range`](Self::from_file_range).
    pub fn open_range(path: impl AsRef<Path>, range: Range<u64>) -> io::Result<Self> {
        Self::from_file_range(&File::open(path)?, range)
    }

    /// Maps `range` of `file`, which must be open for reading; the source keeps its own
    /// handle to the file for the reads outside the range.
    ///
    /// # Errors
    ///
    /// [`io::ErrorKind::InvalidInput`] if `range` is not inside the file; the file
    /// cannot be mapped; a sink of this process other than a `FileSink` writes it
    /// ([`io::ErrorKind::ResourceBusy`]).
    pub fn from_file_range(file: &File, range: Range<u64>) -> io::Result<Self> {
        Self::map(file, Some(range), Some(file.try_clone()?))
    }

    /// Maps `range`, or the whole file. The length is read and the range checked under
    /// the claim, so no sink of this process can shorten the file in between.
    fn map(file: &File, range: Option<Range<u64>>, keep: Option<File>) -> io::Result<Self> {
        let (claim, range, len) = Claim::map(file, |len| match &range {
            None => Ok(0..len),
            Some(r) if r.start <= r.end && r.end <= len => Ok(r.clone()),
            Some(r) => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("cannot map bytes {r:?} of a file of {len} bytes"),
            )),
        })?;
        let map_len = usize::try_from(range.end - range.start).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "the range is larger than the address space",
            )
        })?;
        let map = if map_len == 0 {
            None
        } else {
            Some(sys::map(file, range.start, map_len)?)
        };
        Ok(Self {
            map,
            start: range.start,
            len,
            file: keep,
            _claim: claim,
        })
    }

    /// The mapped bytes: exactly the range asked for.
    #[must_use]
    pub fn mapped(&self) -> &[u8] {
        self.map.as_deref().unwrap_or_default()
    }
}

impl ByteSource for MmapSource {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), ReadError> {
        check_bounds(offset, buf.len(), self.len)?;
        let end = offset + buf.len() as u64;
        if let Some(bytes) = self.slice(offset..end) {
            buf.copy_from_slice(bytes);
            return Ok(());
        }
        // A whole-file map has no file, and serves every read in bounds from the map.
        let file = self.file.as_ref().ok_or(ReadError::OutOfBounds {
            offset,
            len: buf.len() as u64,
            source_len: self.len,
        })?;
        sys::read_exact_at(file, buf, offset).map_err(backend_error)
    }

    fn as_slice(&self) -> Option<&[u8]> {
        (self.start == 0 && self.mapped().len() as u64 == self.len).then(|| self.mapped())
    }

    fn slice(&self, range: Range<u64>) -> Option<&[u8]> {
        let start = usize::try_from(range.start.checked_sub(self.start)?).ok()?;
        let end = usize::try_from(range.end.checked_sub(self.start)?).ok()?;
        self.mapped().get(start..end)
    }

    fn advise(&self, range: Range<u64>, hint: AccessHint) {
        #[cfg(unix)]
        {
            use memmap2::Advice;
            let Some(map) = &self.map else { return };
            let advice = match hint {
                AccessHint::Sequential => Advice::Sequential,
                AccessHint::Random => Advice::Random,
                AccessHint::WillNeed => Advice::WillNeed,
                _ => Advice::Normal,
            };
            let len = map.len();
            let inside = |at: u64| {
                usize::try_from(at.saturating_sub(self.start))
                    .unwrap_or(len)
                    .min(len)
            };
            let (start, end) = (inside(range.start), inside(range.end));
            if start < end {
                // memmap2 rounds the start down to a page boundary. Advice: the result
                // does not matter.
                let _ = map.advise_range(advice, start, end - start);
            }
        }
        #[cfg(not(unix))]
        {
            let _ = (range, hint);
        }
    }
}
