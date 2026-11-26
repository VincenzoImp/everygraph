// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Fault injection for tests (issue #190; `docs/research/sota-testing.md`, challenge 4).
//! Built only with the feature `faults`, which no shipped crate enables.
//!
//! [`Faulty`] wraps any [`ByteSource`] or [`ByteSink`] and makes its `k`-th operation
//! fail: a write or a length change with `ENOSPC` (after writing half of its bytes), a
//! read with `EIO` or as a short read, a sync with `EIO`. A test runs the same work once
//! for every `k` below the operation count of a clean run, and checks that each failure
//! comes back as a typed error and leaves the files as they should be.
//!
//! After a failed sync the file holds what [`AfterFailedSync`] says: the new bytes
//! (ext4 and XFS keep them in the page cache) or the bytes of the last successful sync
//! (btrfs reverts to them); either way a retried sync cannot be trusted (Rebello et al.,
//! ATC'20). [`Faulty::durable`] is what a crash at that moment would leave.

use std::io;
use std::ops::Range;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use everygraph_core::{AccessHint, ByteSink, ByteSource, ReadError, WriteError};

/// The kinds of operation counted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Op {
    /// [`ByteSource::read_at`].
    Read,
    /// [`ByteSink::write_at`].
    Write,
    /// [`ByteSink::set_len`].
    SetLen,
    /// [`ByteSink::sync`].
    Sync,
}

/// What a failed sync leaves in the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AfterFailedSync {
    /// The bytes written since the last sync stay (ext4, XFS).
    KeepNew,
    /// The bytes of the last successful sync come back (btrfs).
    Revert,
}

/// Which operation fails, and how.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    /// The operation that fails, counted from 0 over every operation; `None` for none.
    pub at: Option<u64>,
    /// A failed read is a short read (half the buffer filled, then end of file) rather
    /// than `EIO`.
    pub short_read: bool,
    /// What a failed sync leaves.
    pub after_failed_sync: AfterFailedSync,
}

impl Plan {
    /// Fails nothing: counts the operations of a clean run.
    pub const NONE: Self = Self {
        at: None,
        short_read: false,
        after_failed_sync: AfterFailedSync::KeepNew,
    };

    /// Fails operation `k`.
    #[must_use]
    pub const fn at(k: u64) -> Self {
        Self {
            at: Some(k),
            ..Self::NONE
        }
    }
}

/// A source or sink whose operation [`Plan::at`] fails. Reads never come from
/// [`ByteSource::as_slice`], so that every read is counted.
#[derive(Debug)]
pub struct Faulty<S> {
    inner: S,
    plan: Plan,
    ops: AtomicU64,
    failed: Mutex<Option<Op>>,
    durable: Vec<u8>,
}

impl<S: ByteSource> Faulty<S> {
    /// Wraps `inner`, whose bytes now are taken as durable.
    ///
    /// # Panics
    ///
    /// If `inner` cannot be read (test sources are in memory).
    pub fn new(inner: S, plan: Plan) -> Self {
        let durable = read_all(&inner);
        Self {
            inner,
            plan,
            ops: AtomicU64::new(0),
            failed: Mutex::new(None),
            durable,
        }
    }

    /// Operations so far, the failed one included.
    pub fn ops(&self) -> u64 {
        self.ops.load(Ordering::Relaxed)
    }

    /// The kind of the operation that was made to fail, if it was reached.
    ///
    /// # Panics
    ///
    /// If another thread panicked while it held the record.
    pub fn failed(&self) -> Option<Op> {
        *self.failed.lock().expect("not poisoned")
    }

    /// The bytes a crash now would leave: those of the last successful sync.
    pub fn durable(&self) -> &[u8] {
        &self.durable
    }

    /// The wrapped source or sink.
    pub fn into_inner(self) -> S {
        self.inner
    }

    /// Counts an operation of kind `op`; whether it is the one to fail.
    fn fails(&self, op: Op) -> bool {
        let k = self.ops.fetch_add(1, Ordering::Relaxed);
        let fails = self.plan.at == Some(k);
        if fails {
            *self.failed.lock().expect("not poisoned") = Some(op);
        }
        fails
    }
}

fn read_all<S: ByteSource>(source: &S) -> Vec<u8> {
    let len = usize::try_from(source.len()).expect("a test source fits in memory");
    let mut bytes = vec![0; len];
    source
        .read_at(0, &mut bytes)
        .expect("a test source can be read");
    bytes
}

fn no_space() -> WriteError {
    WriteError::Backend(Box::new(io::Error::new(
        io::ErrorKind::StorageFull,
        "injected ENOSPC",
    )))
}

fn eio() -> io::Error {
    io::Error::other("injected EIO")
}

impl<S: ByteSource> ByteSource for Faulty<S> {
    fn len(&self) -> u64 {
        self.inner.len()
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), ReadError> {
        if !self.fails(Op::Read) {
            return self.inner.read_at(offset, buf);
        }
        if self.plan.short_read {
            let half = buf.len() / 2;
            self.inner.read_at(offset, &mut buf[..half])?;
            return Err(ReadError::Backend(Box::new(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "injected short read",
            ))));
        }
        Err(ReadError::Backend(Box::new(eio())))
    }

    fn advise(&self, range: Range<u64>, hint: AccessHint) {
        self.inner.advise(range, hint);
    }
}

impl<S: ByteSink> ByteSink for Faulty<S> {
    fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<(), WriteError> {
        if !self.fails(Op::Write) {
            return self.inner.write_at(offset, data);
        }
        self.inner.write_at(offset, &data[..data.len() / 2])?;
        Err(no_space())
    }

    fn set_len(&mut self, len: u64) -> Result<(), WriteError> {
        if self.fails(Op::SetLen) {
            return Err(no_space());
        }
        self.inner.set_len(len)
    }

    fn sync(&mut self) -> Result<(), WriteError> {
        if !self.fails(Op::Sync) {
            self.inner.sync()?;
            self.durable = read_all(&self.inner);
            return Ok(());
        }
        if self.plan.after_failed_sync == AfterFailedSync::Revert {
            self.inner.set_len(self.durable.len() as u64)?;
            self.inner.write_at(0, &self.durable)?;
        }
        Err(WriteError::Backend(Box::new(eio())))
    }
}

#[cfg(test)]
mod tests {
    use everygraph_core::MemorySource;

    use super::*;

    #[test]
    fn the_planned_operation_fails_and_a_failed_sync_keeps_or_reverts() {
        for (after, left) in [
            (AfterFailedSync::KeepNew, &b"abcd"[..]),
            (AfterFailedSync::Revert, b"ab"),
        ] {
            let plan = Plan {
                after_failed_sync: after,
                ..Plan::at(3)
            };
            let mut sink = Faulty::new(MemorySource::new(Vec::new()), plan);
            sink.write_at(0, b"ab").unwrap();
            sink.sync().unwrap();
            sink.write_at(2, b"cd").unwrap();
            assert!(sink.sync().is_err());
            assert_eq!((sink.failed(), sink.ops()), (Some(Op::Sync), 4));
            assert_eq!(sink.durable(), b"ab");
            assert_eq!(sink.into_inner().into_inner(), left);
        }
        let mut sink = Faulty::new(MemorySource::new(Vec::new()), Plan::at(0));
        let error = sink.write_at(0, b"abcd").unwrap_err();
        assert!(error.to_string().contains("ENOSPC"));
        assert_eq!(sink.into_inner().into_inner(), b"ab");
        let source = Faulty::new(
            MemorySource::new(b"abcd".to_vec()),
            Plan {
                short_read: true,
                ..Plan::at(1)
            },
        );
        let mut buf = [0; 4];
        source.read_at(0, &mut buf).unwrap();
        assert!(source.read_at(0, &mut buf).is_err());
        assert!(source.as_slice().is_none());
    }
}
