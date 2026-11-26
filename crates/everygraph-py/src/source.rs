// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Where a graph's bytes are: a mapped `.eg` file, or memory for a graph that was
//! never written (`docs/api-sketch.md` § 2.8); and the file read by positional reads,
//! for the results committed after it was mapped.

use std::ops::Range;

use everygraph_core::{AccessHint, ByteSource, MemorySource, ReadError};
use everygraph_io::{FileSource, MmapSource};

/// The bytes of a graph.
#[derive(Debug)]
pub(crate) enum Bytes {
    File(MmapSource),
    Memory(MemorySource<Vec<u8>>),
    Read(FileSource),
}

impl ByteSource for Bytes {
    fn len(&self) -> u64 {
        match self {
            Self::File(s) => s.len(),
            Self::Memory(s) => s.len(),
            Self::Read(s) => s.len(),
        }
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), ReadError> {
        match self {
            Self::File(s) => s.read_at(offset, buf),
            Self::Memory(s) => s.read_at(offset, buf),
            Self::Read(s) => s.read_at(offset, buf),
        }
    }

    fn as_slice(&self) -> Option<&[u8]> {
        match self {
            Self::File(s) => s.as_slice(),
            Self::Memory(s) => s.as_slice(),
            Self::Read(s) => s.as_slice(),
        }
    }

    fn slice(&self, range: Range<u64>) -> Option<&[u8]> {
        match self {
            Self::File(s) => s.slice(range),
            Self::Memory(s) => s.slice(range),
            Self::Read(s) => s.slice(range),
        }
    }

    fn advise(&self, range: Range<u64>, hint: AccessHint) {
        match self {
            Self::File(s) => s.advise(range, hint),
            Self::Memory(s) => s.advise(range, hint),
            Self::Read(s) => s.advise(range, hint),
        }
    }
}
