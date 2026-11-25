// SPDX-License-Identifier: Apache-2.0 OR MIT

//! [`MmapSource`]: a file mapped into memory.

use std::fs::File;
use std::io;
use std::ops::Range;
use std::path::Path;

use everygraph_core::{AccessHint, ByteSource, MemorySource, ReadError};
use memmap2::Mmap;

use crate::sys;

/// A file mapped read-only into memory: the engine's in-RAM mode.
///
/// In the lab a kernel over a warm map ran as fast as over the same bytes on the heap
/// (orkut, `PageRank` f32, 32 threads: 0.380 s mapped, 0.375 s on the heap; FINDINGS § B1,
/// ADR 0011), so there is no separate "load into memory" backend: a graph in RAM is a
/// mapped graph whose pages are resident. [`as_slice`](ByteSource::as_slice) hands the
/// whole map to readers without copying, and [`advise`](ByteSource::advise) passes
/// hints on to `madvise` — [`AccessHint::WillNeed`] on a whole frontier is what made
/// cold 2-hop queries fast in the lab (FINDINGS § F1).
///
/// The file must not be truncated or written while it is mapped: on Unix, touching a
/// page that a truncation removed raises `SIGBUS` and ends the process. The engine's
/// own files are written once and then only read; this backend is for those.
#[derive(Debug)]
pub struct MmapSource {
    // `None` for an empty file, which cannot be mapped on every platform.
    map: Option<Mmap>,
}

impl MmapSource {
    /// Maps the file at `path`.
    ///
    /// # Errors
    ///
    /// The file cannot be opened or mapped (on a 32-bit target, a file larger than the
    /// address space cannot be).
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::from_file(&File::open(path)?)
    }

    /// Maps `file`, which must be open for reading. The map stays valid after `file` is
    /// closed.
    ///
    /// # Errors
    ///
    /// The file cannot be mapped.
    pub fn from_file(file: &File) -> io::Result<Self> {
        let map = if file.metadata()?.len() == 0 {
            None
        } else {
            Some(sys::map(file)?)
        };
        Ok(Self { map })
    }

    fn bytes(&self) -> &[u8] {
        self.map.as_deref().unwrap_or_default()
    }
}

impl ByteSource for MmapSource {
    fn len(&self) -> u64 {
        self.bytes().len() as u64
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), ReadError> {
        MemorySource::new(self.bytes()).read_at(offset, buf)
    }

    fn as_slice(&self) -> Option<&[u8]> {
        Some(self.bytes())
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
            let start = usize::try_from(range.start).unwrap_or(len).min(len);
            let end = usize::try_from(range.end).unwrap_or(len).min(len);
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
