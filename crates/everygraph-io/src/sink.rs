// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Sinks: where graphs and spill files are written.
//!
//! Three shapes, one per way the lab wrote files:
//!
//! - [`AppendSink`]: one writer, front to back, through a large buffer.
//! - [`BlockWriter`]: a file of known length filled in place by many threads, in aligned
//!   blocks, after preallocating it. In the lab, 32 threads `pwrite`-ing into one output
//!   file did not scale and a shared mapping was 10–20% better (FINDINGS § B1); this
//!   writer is the proposed fix and has not been measured against either yet.
//! - [`SpillLogs`]: sharded temporary logs for an out-of-core build, read back as
//!   [`FileSource`]s. Through the page cache, never direct: for small appending temp
//!   writes `O_DIRECT` cost 74.6–81.4 s against 33.2–45.3 s (Friendster, 8 GB budget;
//!   FINDINGS § B1).

use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::sync::Mutex;

use everygraph_core::{AlignedBuf, SEGMENT_ALIGN};

use crate::{FileSource, sys};

/// Buffer of an [`AppendSink`]: 1 MiB.
const APPEND_BUFFER: usize = 1 << 20;

/// Bounce buffer of a direct [`BlockWriter`] for blocks that are not aligned in memory.
const BOUNCE: usize = 1 << 20;

/// A file written front to back through a 1 MiB buffer. It counts what was written, so
/// that a format writer can record offsets as it goes.
#[derive(Debug)]
pub struct AppendSink {
    out: BufWriter<File>,
    position: u64,
}

impl AppendSink {
    /// Creates `path`, or truncates it.
    ///
    /// # Errors
    ///
    /// The file cannot be created.
    pub fn create(path: impl AsRef<Path>) -> io::Result<Self> {
        Ok(Self::from_file(File::create(path)?))
    }

    /// Appends to `file` from its current position; [`position`](Self::position) counts
    /// from 0 regardless.
    #[must_use]
    pub fn from_file(file: File) -> Self {
        Self {
            out: BufWriter::with_capacity(APPEND_BUFFER, file),
            position: 0,
        }
    }

    /// Bytes written so far.
    #[must_use]
    pub const fn position(&self) -> u64 {
        self.position
    }

    /// Flushes the buffer and returns the file, for the caller to `sync_all` if the
    /// output must be durable.
    ///
    /// # Errors
    ///
    /// The final write failed.
    pub fn finish(self) -> io::Result<File> {
        self.out
            .into_inner()
            .map_err(io::IntoInnerError::into_error)
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
#[derive(Debug)]
pub struct BlockWriter {
    file: File,
    len: u64,
    direct: bool,
}

impl BlockWriter {
    /// Creates (or truncates) `path` and preallocates `len` bytes. With `direct`, the
    /// writes bypass the page cache where the platform and file system allow it
    /// ([`is_direct`](Self::is_direct)).
    ///
    /// # Errors
    ///
    /// The file cannot be created or preallocated (for example, the disk is full).
    pub fn create(path: impl AsRef<Path>, len: u64, direct: bool) -> io::Result<Self> {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(true);
        let (file, direct) = if direct {
            sys::open_uncached(&options, path.as_ref())?
        } else {
            (options.open(path)?, false)
        };
        sys::preallocate(&file, len)?;
        Ok(Self { file, len, direct })
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
        if offset % align != 0
            || end.is_none_or(|end| end > self.len)
            || (data.len() % SEGMENT_ALIGN != 0 && !ends_file)
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
        if !self.direct || (aligned_in_memory && data.len() % SEGMENT_ALIGN == 0) {
            return sys::write_all_at(&self.file, data, offset);
        }
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
        self.file.set_len(self.len)?;
        Ok(self.file)
    }
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
                )?)))
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
