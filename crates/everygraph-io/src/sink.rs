// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Sinks: where graphs and spill files are written.
//!
//! Three shapes, one per way the lab wrote files, and one for formats updated in place:
//!
//! - [`AppendSink`]: one writer, front to back, through a large buffer.
//! - [`BlockWriter`]: a file of known length filled in place by many threads, in aligned
//!   blocks, after preallocating it. In the lab, 32 threads `pwrite`-ing into one output
//!   file did not scale (FINDINGS § B1); this writer is the proposed fix and has not
//!   been measured yet.
//! - [`SpillLogs`]: sharded temporary logs for an out-of-core build, read back as
//!   [`FileSource`]s. Through the page cache, never direct: for small appending temp
//!   writes `O_DIRECT` cost 74.6–81.4 s against 33.2–45.3 s (Friendster, 8 GB budget;
//!   FINDINGS § B1).
//! - [`FileSink`]: a file read and written at any offset, with durable syncs: the
//!   [`ByteSink`] of the `.eg` container's update protocol
//!   (`docs/spec/eg-container.md` § 8).

use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::sync::Mutex;

use everygraph_core::{AlignedBuf, ByteSink, ByteSource, ReadError, SEGMENT_ALIGN, WriteError};

use crate::claim::{Claim, Use};
use crate::{FileSource, backend_error, check_bounds, sys};

/// Buffer of an [`AppendSink`]: 1 MiB.
const APPEND_BUFFER: usize = 1 << 20;

/// Bounce buffer of a direct [`BlockWriter`] for blocks that are not aligned in memory.
const BOUNCE: usize = 1 << 20;

/// A file written front to back through a 1 MiB buffer. It counts what was written, so
/// that a format writer can record offsets as it goes.
///
/// It refuses a file that a live [`MmapSource`](crate::MmapSource) of this process maps
/// (see there), and the file cannot be mapped until the sink is finished or dropped.
#[derive(Debug)]
pub struct AppendSink {
    // Declared before the claim: the file is flushed and closed first.
    out: BufWriter<File>,
    position: u64,
    claim: Claim,
}

impl AppendSink {
    /// Creates `path`, or truncates it.
    ///
    /// # Errors
    ///
    /// The file cannot be created, or a live `MmapSource` of this process maps it
    /// ([`io::ErrorKind::ResourceBusy`]; nothing is truncated then).
    pub fn create(path: impl AsRef<Path>) -> io::Result<Self> {
        let (file, claim) =
            open_claimed(OpenOptions::new().write(true), path.as_ref(), Use::Write)?;
        file.set_len(0)?;
        Ok(Self::with_claim(file, claim))
    }

    /// Appends to `file` from its current position; [`position`](Self::position) counts
    /// from 0 regardless.
    ///
    /// # Errors
    ///
    /// The file's identity cannot be read, or a live `MmapSource` of this process maps
    /// it ([`io::ErrorKind::ResourceBusy`]).
    pub fn from_file(file: File) -> io::Result<Self> {
        let claim = Claim::take(&file, Use::Write)?;
        Ok(Self::with_claim(file, claim))
    }

    fn with_claim(file: File, claim: Claim) -> Self {
        Self {
            out: BufWriter::with_capacity(APPEND_BUFFER, file),
            position: 0,
            claim,
        }
    }

    /// Bytes written so far.
    #[must_use]
    pub const fn position(&self) -> u64 {
        self.position
    }

    /// Flushes the buffer and returns the file, for the caller to `sync_all` if the
    /// output must be durable. From here on the file may be mapped; the caller must not
    /// write through the returned handle while it is.
    ///
    /// # Errors
    ///
    /// The final write failed.
    pub fn finish(self) -> io::Result<File> {
        let Self { out, claim, .. } = self;
        let file = out.into_inner().map_err(io::IntoInnerError::into_error);
        drop(claim);
        file
    }
}

impl Write for AppendSink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.out.write(buf)?;
        self.position += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}

/// A file of a length known in advance, written in place by any number of threads at
/// once, in blocks of [`SEGMENT_ALIGN`] bytes.
///
/// The file is preallocated when it is created (`fallocate` on Linux; elsewhere it is
/// extended to its length). Every write starts at a multiple of the block size and
/// covers whole blocks, except the write that ends the file. In direct mode
/// ([`create`](Self::create) with `direct`) blocks bypass the page cache; a block
/// whose bytes are aligned in memory ([`AlignedBuf`]) goes to the device without a copy.
/// The rules are the same in both modes, so a caller can switch between them freely.
///
/// Like [`AppendSink`], it refuses a file that a live [`MmapSource`](crate::MmapSource)
/// of this process maps, and holds the file against mapping until it is finished.
#[derive(Debug)]
pub struct BlockWriter {
    // Declared before the claim: the file is closed first.
    file: File,
    len: u64,
    direct: bool,
    claim: Claim,
}

impl BlockWriter {
    /// Creates (or truncates) `path` and preallocates `len` bytes. With `direct`, the
    /// writes bypass the page cache where the platform and file system allow it
    /// ([`is_direct`](Self::is_direct)).
    ///
    /// # Errors
    ///
    /// The file cannot be created or preallocated (for example, the disk is full), or a
    /// live `MmapSource` of this process maps it ([`io::ErrorKind::ResourceBusy`];
    /// nothing is truncated then).
    pub fn create(path: impl AsRef<Path>, len: u64, direct: bool) -> io::Result<Self> {
        let mut options = OpenOptions::new();
        options.read(true).write(true);
        let (file, claim, direct) = if direct {
            options.create(true).truncate(false);
            let (file, direct) = sys::open_uncached(&options, path.as_ref())?;
            let claim = Claim::take(&file, Use::Write)?;
            (file, claim, direct)
        } else {
            let (file, claim) = open_claimed(&mut options, path.as_ref(), Use::Write)?;
            (file, claim, false)
        };
        // Truncate only once the claim is held: no map can exist from here on.
        file.set_len(0)?;
        sys::preallocate(&file, len)?;
        Ok(Self {
            file,
            len,
            direct,
            claim,
        })
    }

    /// Length of the file being written.
    #[must_use]
    pub const fn len(&self) -> u64 {
        self.len
    }

    /// Whether the file has length 0.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Whether writes bypass the page cache.
    #[must_use]
    pub const fn is_direct(&self) -> bool {
        self.direct
    }

    /// Writes `data` at `offset`. Several threads may write disjoint ranges at once.
    ///
    /// # Errors
    ///
    /// [`io::ErrorKind::InvalidInput`] if `offset` is not a multiple of
    /// [`SEGMENT_ALIGN`], if `data` is not a whole number of blocks and does not end the
    /// file, or if it would end past the file's length; otherwise the write failed.
    pub fn write_at(&self, offset: u64, data: &[u8]) -> io::Result<()> {
        let align = SEGMENT_ALIGN as u64;
        let end = offset.checked_add(data.len() as u64);
        let ends_file = end == Some(self.len);
        if !offset.is_multiple_of(align)
            || end.is_none_or(|end| end > self.len)
            || (!data.len().is_multiple_of(SEGMENT_ALIGN) && !ends_file)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "block write of {} bytes at offset {offset} into a file of {} bytes is not \
                     aligned to {SEGMENT_ALIGN}-byte blocks or does not fit",
                    data.len(),
                    self.len
                ),
            ));
        }
        let aligned_in_memory = data.as_ptr().align_offset(SEGMENT_ALIGN) == 0;
        if !self.direct || (aligned_in_memory && data.len().is_multiple_of(SEGMENT_ALIGN)) {
            return sys::write_all_at(&self.file, data, offset);
        }
        // TODO(#31 integration): up to 1 MiB allocated per call, outside the memory
        // accountant.
        // Direct and not aligned in memory, or the last partial block: copy into an
        // aligned buffer, padding the last block with zeros (`finish` cuts it off).
        let mut bounce =
            AlignedBuf::try_zeroed(BOUNCE.min(data.len().next_multiple_of(SEGMENT_ALIGN)))
                .map_err(io::Error::other)?;
        let bounce = bounce.as_mut_slice();
        let mut at = offset;
        for piece in data.chunks(bounce.len()) {
            let padded = piece.len().next_multiple_of(SEGMENT_ALIGN);
            bounce[..piece.len()].copy_from_slice(piece);
            bounce[piece.len()..padded].fill(0);
            sys::write_all_at(&self.file, &bounce[..padded], at)?;
            at += piece.len() as u64;
        }
        Ok(())
    }

    /// Sets the file to its exact length (a padded last block is cut off) and returns
    /// it, for the caller to `sync_all` if the output must be durable.
    ///
    /// # Errors
    ///
    /// The length cannot be set.
    pub fn finish(self) -> io::Result<File> {
        let Self {
            file, len, claim, ..
        } = self;
        file.set_len(len)?;
        drop(claim);
        Ok(file)
    }
}

/// A file read and written in place, at any offset, by one writer: the
/// [`ByteSink`] that `everygraph_format::container::Writer` commits through.
///
/// [`sync`](ByteSink::sync) is `fsync` (`File::sync_all`: data and length). Writes go
/// straight to the file (`pwrite`), unbuffered: the container writes sections in
/// pieces the caller chose, and a commit in a few large writes.
///
/// Like the other sinks it holds the file against mapping: it refuses a file that a
/// live [`MmapSource`](crate::MmapSource) of this process maps
/// ([`io::ErrorKind::ResourceBusy`]), and the file cannot be mapped while the sink is
/// open. So a graph that this process holds mapped cannot be committed to until the
/// map is dropped. It also holds the file alone: a second `FileSink`, or any other
/// sink, of this process on the same file is refused while it is open (one writer at a
/// time updates a container). Other processes are not stopped (see `MmapSource`).
#[derive(Debug)]
pub struct FileSink {
    // Declared before the claim: the file is closed first.
    file: File,
    len: u64,
    claim: Claim,
}

impl FileSink {
    /// Creates `path`, or empties it — at once and without a sync. Never point it at
    /// a live `NAME.eg`: write a new graph or a compacted copy to a temporary name and
    /// [`persist`] it over the old one.
    ///
    /// # Errors
    ///
    /// The file cannot be created, or a live `MmapSource` or another sink of this
    /// process holds it ([`io::ErrorKind::ResourceBusy`]; nothing is truncated then).
    pub fn create(path: impl AsRef<Path>) -> io::Result<Self> {
        let (file, claim) = open_claimed(
            OpenOptions::new().read(true).write(true),
            path.as_ref(),
            Use::WriteAlone,
        )?;
        file.set_len(0)?;
        Ok(Self {
            file,
            len: 0,
            claim,
        })
    }

    /// Opens the existing file `path` for reading and writing; nothing is changed.
    ///
    /// # Errors
    ///
    /// The file cannot be opened, or a live `MmapSource` or another sink of this
    /// process holds it ([`io::ErrorKind::ResourceBusy`]).
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        let claim = Claim::take(&file, Use::WriteAlone)?;
        let len = file.metadata()?.len();
        Ok(Self { file, len, claim })
    }

    /// Gives the file back. From here on it may be mapped; the caller must not write
    /// through the returned handle while it is.
    #[must_use]
    pub fn into_file(self) -> File {
        let Self { file, claim, .. } = self;
        drop(claim);
        file
    }
}

impl ByteSource for FileSink {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), ReadError> {
        check_bounds(offset, buf.len(), self.len)?;
        sys::read_exact_at(&self.file, buf, offset).map_err(backend_error)
    }
}

impl ByteSink for FileSink {
    fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<(), WriteError> {
        let end = offset
            .checked_add(data.len() as u64)
            .ok_or(WriteError::TooLarge { len: u64::MAX })?;
        if data.is_empty() && end > self.len {
            // A write of nothing past the end still extends the sink (the contract).
            self.file.set_len(end).map_err(write_error)?;
        }
        sys::write_all_at(&self.file, data, offset).map_err(write_error)?;
        self.len = self.len.max(end);
        Ok(())
    }

    fn set_len(&mut self, len: u64) -> Result<(), WriteError> {
        self.file.set_len(len).map_err(write_error)?;
        self.len = len;
        Ok(())
    }

    fn sync(&mut self) -> Result<(), WriteError> {
        self.file.sync_all().map_err(write_error)
    }
}

fn write_error(error: io::Error) -> WriteError {
    WriteError::Backend(Box::new(error))
}

/// Opens `path` with `options`, creating it if needed but not truncating it, and claims
/// it for writing: a file that a live map holds is refused before anything changes.
fn open_claimed(
    options: &mut OpenOptions,
    path: &Path,
    used_for: Use,
) -> io::Result<(File, Claim)> {
    let file = options.create(true).truncate(false).open(path)?;
    let claim = Claim::take(&file, used_for)?;
    Ok((file, claim))
}

/// Gives the file `partial` the name `destination`, replacing what had that name, and
/// makes the new name durable: the last step of writing a graph (`NAME.eg.partial` →
/// `NAME.eg`, ux-contract C-1) or of a compaction. At every instant `destination` is
/// either the old file or the new one, whole.
///
/// The caller has made the file's content durable first (a container's commit does).
/// Both paths must be on the same file system. On Unix the rename is `rename(2)` and
/// the directory holding `destination` is then synced, which is what makes the rename
/// itself survive a power cut. On Windows the rename is `MoveFileExW` with
/// `MOVEFILE_REPLACE_EXISTING` (std's `rename`); std offers no write-through flag and
/// no way to sync a directory there, so the rename is durable only once NTFS commits
/// its journal (a power cut just after may leave the old file under the name; never a
/// mixture).
///
/// # Errors
///
/// The rename failed (nothing changed), or the directory could not be synced (the
/// rename happened; it may not survive a power cut).
pub fn persist(partial: impl AsRef<Path>, destination: impl AsRef<Path>) -> io::Result<()> {
    let destination = destination.as_ref();
    std::fs::rename(partial.as_ref(), destination)?;
    #[cfg(unix)]
    {
        let dir = match destination.parent() {
            Some(dir) if !dir.as_os_str().is_empty() => dir,
            _ => Path::new("."),
        };
        File::open(dir)?.sync_all()?;
    }
    Ok(())
}

/// Sharded temporary logs: append-only files that an out-of-core build spills into and
/// reads back.
///
/// Each shard is one temporary file behind its own lock, so workers appending to
/// different shards do not wait for each other; a worker is expected to batch its
/// records and append large pieces, as the lab's ingest did (8 MiB per spill chunk).
/// The files are deleted by the operating system when the last handle to them closes
/// (unlinked at creation on Unix), so nothing is left behind, even after a crash.
#[derive(Debug)]
pub struct SpillLogs {
    shards: Vec<Mutex<AppendSink>>,
}

impl SpillLogs {
    /// Creates `shards` empty logs in `dir`.
    ///
    /// # Errors
    ///
    /// A file cannot be created in `dir`.
    pub fn create(dir: impl AsRef<Path>, shards: usize) -> io::Result<Self> {
        let shards = (0..shards)
            .map(|_| {
                Ok(Mutex::new(AppendSink::from_file(sys::create_temp(
                    dir.as_ref(),
                )?)?))
            })
            .collect::<io::Result<_>>()?;
        Ok(Self { shards })
    }

    /// Number of shards.
    #[must_use]
    pub fn shards(&self) -> usize {
        self.shards.len()
    }

    /// Appends `bytes` to shard `shard`.
    ///
    /// # Errors
    ///
    /// The write failed, or a thread panicked while appending to this shard (its
    /// contents are then unknown).
    ///
    /// # Panics
    ///
    /// If `shard` is not below [`shards`](Self::shards).
    pub fn append(&self, shard: usize, bytes: &[u8]) -> io::Result<()> {
        self.shards[shard]
            .lock()
            .map_err(|_| io::Error::other("a writer panicked while appending to this spill shard"))?
            .write_all(bytes)
    }

    /// Flushes every shard and returns them, in shard order, as sources to read back.
    /// Each file is deleted when its source is dropped.
    ///
    /// # Errors
    ///
    /// A final write failed, or a writer had panicked (see [`append`](Self::append)).
    pub fn finish(self) -> io::Result<Vec<FileSource>> {
        self.shards
            .into_iter()
            .map(|shard| {
                let sink = shard.into_inner().map_err(|_| {
                    io::Error::other("a writer panicked while appending to this spill shard")
                })?;
                FileSource::from_file(sink.finish()?)
            })
            .collect()
    }
}
