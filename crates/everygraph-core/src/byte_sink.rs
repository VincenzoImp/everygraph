// SPDX-License-Identifier: Apache-2.0 OR MIT

//! [`ByteSink`]: how format writers write bytes in place without knowing where they are.

use std::error::Error;
use std::fmt;

use crate::{ByteSource, MemorySource, ReadError};

/// Positional storage that can be written as well as read: a [`ByteSource`] with
/// [`write_at`](Self::write_at), [`set_len`](Self::set_len) and a durability barrier,
/// [`sync`](Self::sync).
///
/// Format writers that update a file in place (the `.eg` container's update protocol,
/// `docs/spec/eg-container.md`) are written against this trait, as readers are written
/// against [`ByteSource`]: a file on disk, a buffer in memory and a browser's private
/// file system can all be sinks, and a test can be one that fails or "loses power" at a
/// chosen step. The contract:
///
/// - **A source between writes.** Writing takes `&mut self`, so for as long as the sink
///   is borrowed shared, its bytes and length do not change and the [`ByteSource`]
///   contract holds: a reader handed `&sink` reads a fixed sequence of bytes.
/// - **Positional.** [`write_at`](Self::write_at) writes exactly `data` at `offset`;
///   writing past the end extends the sink, and bytes skipped over read as zero.
/// - **Durability is explicit.** Nothing written is promised to survive a crash of the
///   machine until [`sync`](Self::sync) returns; once it has, everything written and
///   every length set before the call survives. Between two syncs, writes may survive in
///   part, in any order, or not at all (a write that crosses a 512-byte boundary may be
///   torn). A crash of the process alone loses nothing that a write had returned. This
///   is the model the container's crash tests inject.
///
/// No file-system concept appears here: what a sink is stored in, how it is named and
/// how it is opened is the backend's business (`everygraph-io` for native files).
pub trait ByteSink: ByteSource {
    /// Writes all of `data` at `offset`, extending the sink if it ends past the end.
    ///
    /// # Errors
    ///
    /// [`WriteError::Backend`] if the backend failed. What was written of `data` is then
    /// unspecified.
    fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<(), WriteError>;

    /// Sets the length to `len`, cutting off what lies beyond it or extending with
    /// zeros.
    ///
    /// # Errors
    ///
    /// [`WriteError::Backend`] if the backend failed.
    fn set_len(&mut self, len: u64) -> Result<(), WriteError>;

    /// Makes everything written and every length set so far durable: when this returns
    /// `Ok`, a crash of the machine no longer loses it. A sink that has nothing to make
    /// durable (memory) returns at once.
    ///
    /// # Errors
    ///
    /// [`WriteError::Backend`] if the backend failed; nothing is then promised about
    /// what is durable.
    fn sync(&mut self) -> Result<(), WriteError>;
}

impl<S: ByteSink + ?Sized> ByteSink for &mut S {
    fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<(), WriteError> {
        (**self).write_at(offset, data)
    }

    fn set_len(&mut self, len: u64) -> Result<(), WriteError> {
        (**self).set_len(len)
    }

    fn sync(&mut self) -> Result<(), WriteError> {
        (**self).sync()
    }
}

impl<S: ByteSource + ?Sized> ByteSource for &mut S {
    fn len(&self) -> u64 {
        (**self).len()
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), ReadError> {
        (**self).read_at(offset, buf)
    }

    fn as_slice(&self) -> Option<&[u8]> {
        (**self).as_slice()
    }

    fn advise(&self, range: std::ops::Range<u64>, hint: crate::AccessHint) {
        (**self).advise(range, hint);
    }
}

/// Why a [`ByteSink`] write, length change or sync failed.
#[derive(Debug)]
#[non_exhaustive]
pub enum WriteError {
    /// The position or length does not fit in memory on this target (an in-memory
    /// sink asked to grow past `usize`).
    TooLarge {
        /// The length that was asked for.
        len: u64,
    },
    /// The backend failed (I/O error, disk full, an injected fault, …).
    Backend(Box<dyn Error + Send + Sync>),
}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { len } => {
                write!(
                    f,
                    "a sink of {len} bytes does not fit in memory on this target"
                )
            }
            Self::Backend(cause) => write!(f, "the byte sink failed: {cause}"),
        }
    }
}

impl Error for WriteError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::TooLarge { .. } => None,
            Self::Backend(cause) => Some(cause.as_ref()),
        }
    }
}

impl MemorySource<Vec<u8>> {
    /// The bytes, given back.
    #[must_use]
    pub fn into_inner(self) -> Vec<u8> {
        self.bytes
    }
}

/// An in-memory sink: the browser's and the tests' storage. [`sync`](ByteSink::sync)
/// has nothing to do.
impl ByteSink for MemorySource<Vec<u8>> {
    fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<(), WriteError> {
        let too_large = || WriteError::TooLarge {
            len: offset.saturating_add(crate::widen(data.len())),
        };
        let start = usize::try_from(offset).map_err(|_| too_large())?;
        let end = start.checked_add(data.len()).ok_or_else(too_large)?;
        grow(&mut self.bytes, end).map_err(|()| too_large())?;
        self.bytes[start..end].copy_from_slice(data);
        Ok(())
    }

    fn set_len(&mut self, len: u64) -> Result<(), WriteError> {
        let too_large = || WriteError::TooLarge { len };
        let new_len = usize::try_from(len).map_err(|_| too_large())?;
        grow(&mut self.bytes, new_len).map_err(|()| too_large())?;
        self.bytes.truncate(new_len);
        Ok(())
    }

    fn sync(&mut self) -> Result<(), WriteError> {
        Ok(())
    }
}

/// Extends `bytes` with zeros to at least `len`, refusing (instead of aborting) when
/// the memory cannot be had.
fn grow(bytes: &mut Vec<u8>, len: usize) -> Result<(), ()> {
    if let Some(more) = len.checked_sub(bytes.len()) {
        bytes.try_reserve(more).map_err(|_| ())?;
        bytes.resize(len, 0);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_memory_sink_writes_extends_with_zeros_and_truncates() {
        let mut sink = MemorySource::new(Vec::new());
        sink.write_at(4, b"abc").unwrap();
        assert_eq!(sink.len(), 7);
        assert_eq!(sink.as_slice().unwrap(), b"\0\0\0\0abc");
        sink.write_at(1, b"xy").unwrap();
        sink.set_len(5).unwrap();
        sink.sync().unwrap();
        assert_eq!(sink.into_inner(), b"\0xy\0a");
    }

    #[test]
    fn a_memory_sink_refuses_what_cannot_fit() {
        let mut sink = MemorySource::new(Vec::new());
        assert!(matches!(
            sink.write_at(u64::MAX, b"a"),
            Err(WriteError::TooLarge { .. })
        ));
        let error = sink.write_at(u64::MAX, b"").unwrap_err();
        assert!(error.to_string().contains("does not fit"));
    }
}
