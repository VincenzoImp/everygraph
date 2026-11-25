// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The system calls the harness needs beyond `std` — `flock`, `posix_fadvise`, `sync`
//! and `kill` — through `rustix`'s safe wrappers. They exist on Linux only; elsewhere each
//! returns an "unsupported" error, so that the rest of the crate (specs, rows, the
//! report) builds and is tested on every platform.

use std::fs::File;
use std::io;

/// Signals the harness sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// SIGINT.
    Int,
    /// SIGTERM.
    Term,
    /// SIGKILL.
    Kill,
}

#[cfg(target_os = "linux")]
mod imp {
    use super::{File, Signal, io};
    use rustix::fs::{Advice, FlockOperation, fadvise, flock};
    use rustix::process::{Pid, kill_process};

    pub(super) fn try_lock(file: &File) -> io::Result<bool> {
        match flock(file, FlockOperation::NonBlockingLockExclusive) {
            Ok(()) => Ok(true),
            Err(rustix::io::Errno::WOULDBLOCK) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    pub(super) fn drop_cache(file: &File) -> io::Result<()> {
        // offset 0 and no length: the whole file.
        Ok(fadvise(file, 0, None, Advice::DontNeed)?)
    }

    pub(super) fn sync_all() -> io::Result<()> {
        rustix::fs::sync();
        Ok(())
    }

    pub(super) fn signal(pid: u32, sig: Signal) -> io::Result<()> {
        let pid = i32::try_from(pid)
            .ok()
            .and_then(Pid::from_raw)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not a process id"))?;
        let sig = match sig {
            Signal::Int => rustix::process::Signal::INT,
            Signal::Term => rustix::process::Signal::TERM,
            Signal::Kill => rustix::process::Signal::KILL,
        };
        Ok(kill_process(pid, sig)?)
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    use super::{File, Signal, io};

    fn unsupported<T>() -> io::Result<T> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "the benchmark harness runs on Linux only",
        ))
    }

    pub(super) fn try_lock(_: &File) -> io::Result<bool> {
        unsupported()
    }

    pub(super) fn drop_cache(_: &File) -> io::Result<()> {
        unsupported()
    }

    pub(super) fn signal(_: u32, _: Signal) -> io::Result<()> {
        unsupported()
    }

    pub(super) fn sync_all() -> io::Result<()> {
        unsupported()
    }
}

/// Takes an exclusive `flock` on `file` without waiting: `Ok(false)` when another open
/// file description holds it. The kernel releases it when the file is closed, also when
/// the process dies, so a crashed run leaves no stale lock.
///
/// # Errors
///
/// Any failure other than "held by someone else".
pub fn try_lock(file: &File) -> io::Result<bool> {
    imp::try_lock(file)
}

/// `posix_fadvise(POSIX_FADV_DONTNEED)` over the whole file: the kernel drops its clean,
/// unmapped pages from the page cache. Dirty pages are not dropped: sync first.
///
/// # Errors
///
/// The call fails.
pub fn drop_cache(file: &File) -> io::Result<()> {
    imp::drop_cache(file)
}

/// `sync(2)`: writes every dirty page of the machine back to storage, so that the
/// write-back of one repetition's output does not run during the next one.
///
/// # Errors
///
/// Not on Linux.
pub fn sync_all() -> io::Result<()> {
    imp::sync_all()
}

/// Sends `sig` to the process `pid`.
///
/// # Errors
///
/// The process does not exist or is not ours.
pub fn signal(pid: u32, sig: Signal) -> io::Result<()> {
    imp::signal(pid, sig)
}
