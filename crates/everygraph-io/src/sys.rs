// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Everything that differs between operating systems, and every `unsafe` block of the
//! crate.
//!
//! Why safe code is not enough: std has no memory maps, no way to preallocate a file,
//! no `posix_fadvise` and no `F_NOCACHE`. Each `unsafe` block below is one call into
//! the C library (or memmap2's `MmapOptions::map`, which is one `mmap`). None of them
//! passes a pointer: their arguments are a file descriptor this module borrows from a live
//! `File`, and integers. Miri cannot run foreign calls, so these blocks are reached by
//! the crate's tests natively (`tests/conformance.rs`), not under Miri.
//!
//! Platforms: Linux and Android bypass the page cache with `O_DIRECT`, macOS and iOS
//! with `F_NOCACHE` (no `O_DIRECT` there), Windows with `FILE_FLAG_NO_BUFFERING`.
//! Elsewhere, and wherever the file system refuses, files are read through the cache
//! and the caller is told so.

#![allow(unsafe_code)]

use std::fs::{File, OpenOptions};
use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use everygraph_core::AccessHint;
use memmap2::Mmap;

/// Fills `buf` from `offset`, looping over short reads.
pub(crate) fn read_exact_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::FileExt::read_exact_at(file, buf, offset)
    }
    #[cfg(windows)]
    {
        let (mut buf, mut offset) = (buf, offset);
        while !buf.is_empty() {
            match std::os::windows::fs::FileExt::seek_read(file, buf, offset) {
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(n) => {
                    buf = &mut buf[n..];
                    offset += n as u64;
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (file, buf, offset);
        Err(io::ErrorKind::Unsupported.into())
    }
}

/// Reads into `buf` from an aligned `offset` of a file opened for direct I/O, until
/// `buf` is full or the file ends; returns the number of bytes read.
///
/// A direct read comes back short of a whole block only at the end of the file, and the
/// next read would start at an unaligned offset, which Windows refuses (error 87) where
/// Linux returns 0. So a short read that is not a whole number of blocks ends the loop.
pub(crate) fn read_up_to(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    read_up_to_with(buf, offset, |piece, at| {
        #[cfg(unix)]
        let result = std::os::unix::fs::FileExt::read_at(file, piece, at);
        #[cfg(windows)]
        let result = std::os::windows::fs::FileExt::seek_read(file, piece, at);
        #[cfg(not(any(unix, windows)))]
        let result: io::Result<usize> = {
            let _ = (file, piece, at);
            Err(io::ErrorKind::Unsupported.into())
        };
        result
    })
}

/// The loop of [`read_up_to`], over any positional read (a test passes a fake one).
fn read_up_to_with(
    buf: &mut [u8],
    offset: u64,
    mut read: impl FnMut(&mut [u8], u64) -> io::Result<usize>,
) -> io::Result<usize> {
    let mut done = 0;
    while done < buf.len() {
        match read(&mut buf[done..], offset + done as u64) {
            Ok(0) => break,
            Ok(n) => {
                done += n;
                if n % everygraph_core::SEGMENT_ALIGN != 0 {
                    break;
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(done)
}

/// Writes all of `buf` at `offset`.
pub(crate) fn write_all_at(file: &File, buf: &[u8], offset: u64) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::FileExt::write_all_at(file, buf, offset)
    }
    #[cfg(windows)]
    {
        let (mut buf, mut offset) = (buf, offset);
        while !buf.is_empty() {
            match std::os::windows::fs::FileExt::seek_write(file, buf, offset) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(n) => {
                    buf = &buf[n..];
                    offset += n as u64;
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (file, buf, offset);
        Err(io::ErrorKind::Unsupported.into())
    }
}

/// Opens `path` with `options`, bypassing the page cache where the platform and the
/// file system allow it. The flag says whether they did.
pub(crate) fn open_uncached(options: &OpenOptions, path: &Path) -> io::Result<(File, bool)> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut direct = options.clone();
        direct.custom_flags(libc::O_DIRECT);
        match direct.open(path) {
            Ok(file) => Ok((file, true)),
            // The file system does not support O_DIRECT.
            Err(e) if e.raw_os_error() == Some(libc::EINVAL) => Ok((options.open(path)?, false)),
            Err(e) => Err(e),
        }
    }
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        use std::os::fd::AsRawFd;
        let file = options.open(path)?;
        // SAFETY: `fcntl(F_NOCACHE)` takes the descriptor of `file`, which is open for
        // the whole call, and an integer; it touches no memory of ours.
        let status = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_NOCACHE, 1) };
        Ok((file, status != -1))
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_FLAG_NO_BUFFERING (winbase.h). Offsets, lengths and buffer addresses must
        // then be multiples of the sector size; 4 KiB is.
        const FILE_FLAG_NO_BUFFERING: u32 = 0x2000_0000;
        let mut direct = options.clone();
        direct.custom_flags(FILE_FLAG_NO_BUFFERING);
        match direct.open(path) {
            Ok(file) => Ok((file, true)),
            Err(e) if e.kind() == io::ErrorKind::InvalidInput => Ok((options.open(path)?, false)),
            Err(e) => Err(e),
        }
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        windows
    )))]
    {
        Ok((options.open(path)?, false))
    }
}

/// Makes `file` `len` bytes long with its blocks allocated, so that writers filling it
/// in parallel do not contend on block allocation. Where the file system cannot
/// allocate ahead (or `off_t` is too narrow), the file is extended sparsely instead.
pub(crate) fn preallocate(file: &File, len: u64) -> io::Result<()> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        use std::os::fd::AsRawFd;
        if let Ok(off_len) = libc::off_t::try_from(len) {
            if off_len > 0 {
                // SAFETY: `fallocate` takes the descriptor of `file`, open for the whole
                // call, and integers; it touches no memory of ours.
                let status = unsafe { libc::fallocate(file.as_raw_fd(), 0, 0, off_len) };
                if status == 0 {
                    return Ok(());
                }
                let error = io::Error::last_os_error();
                if !matches!(error.raw_os_error(), Some(libc::EOPNOTSUPP | libc::ENOSYS)) {
                    return Err(error);
                }
            }
        }
    }
    file.set_len(len)
}

/// Passes an access hint for `range` of `file` on to the kernel's page cache. Advice
/// only: failures are ignored.
pub(crate) fn fadvise(file: &File, range: Range<u64>, hint: AccessHint) {
    #[cfg(any(target_os = "linux", target_os = "android", target_os = "freebsd"))]
    {
        use std::os::fd::AsRawFd;
        let advice = match hint {
            AccessHint::Sequential => libc::POSIX_FADV_SEQUENTIAL,
            AccessHint::Random => libc::POSIX_FADV_RANDOM,
            AccessHint::WillNeed => libc::POSIX_FADV_WILLNEED,
            _ => libc::POSIX_FADV_NORMAL,
        };
        let (Ok(offset), Ok(len)) = (
            libc::off_t::try_from(range.start),
            libc::off_t::try_from(range.end - range.start),
        ) else {
            return;
        };
        // SAFETY: `posix_fadvise` takes the descriptor of `file`, open for the whole
        // call, and integers; it touches no memory of ours.
        let _ = unsafe { libc::posix_fadvise(file.as_raw_fd(), offset, len, advice) };
    }
    #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "freebsd")))]
    {
        let _ = (file, range, hint);
    }
}

/// Maps `len` bytes of `file` from `offset`, read-only. memmap2 maps from the page
/// boundary at or below `offset` and hands out exactly the bytes asked for.
///
/// Sound only under the invariant of `design/DECISIONS.md`, "Memory-mapped files are read
/// through a safe constructor", which `MmapSource` documents: no writer, in this process
/// or any other, modifies the mapped bytes or truncates the file into them while they
/// are mapped.
pub(crate) fn map(file: &File, offset: u64, len: usize) -> io::Result<Mmap> {
    let mut options = memmap2::MmapOptions::new();
    options.offset(offset).len(len);
    // SAFETY: `map` is sound while nothing modifies the mapped bytes or truncates the
    // file into them for the life of the map: the bytes are handed out as `&[u8]`, and a
    // write by anyone (this process included) would change them under that borrow, a
    // truncation would make reading them raise SIGBUS. That is not enforced against
    // other processes or against `std::fs` writes in this one; it is the invariant
    // accepted by the decision above. Inside the process the sinks of this crate refuse
    // to write a mapped range (`claim`), and the caller (`MmapSource`) holds that claim
    // before calling this.
    unsafe { options.map(file) }
}

/// Takes the exclusive, advisory lock of `file` without waiting (`File::try_lock`, which
/// is `flock(LOCK_EX | LOCK_NB)` on Unix). The lock belongs to the open file and ends
/// when it is closed, or when the process dies. `Ok(false)` when another open file holds
/// it. Not on Windows: there `File::try_lock` is `LockFileEx`, a mandatory lock under
/// which other processes cannot even read the file, so Windows locks through the share
/// mode instead ([`lock_options`]).
pub(crate) fn try_lock(file: &File) -> io::Result<bool> {
    #[cfg(unix)]
    {
        match file.try_lock() {
            Ok(()) => Ok(true),
            Err(std::fs::TryLockError::WouldBlock) => Ok(false),
            Err(std::fs::TryLockError::Error(e)) => Err(e),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = file;
        Ok(true)
    }
}

/// On Windows, opens for writing so that no other handle may write the file while this
/// one is open: the share mode leaves out `FILE_SHARE_WRITE`. A second writer's open
/// then fails with a sharing violation ([`is_sharing_violation`]).
pub(crate) fn lock_options(options: &mut OpenOptions) {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_SHARE_READ | FILE_SHARE_DELETE (winnt.h).
        options.share_mode(0x1 | 0x4);
    }
    #[cfg(not(windows))]
    {
        let _ = options;
    }
}

/// Whether `error` is Windows' `ERROR_SHARING_VIOLATION` (32), what an open refused by
/// [`lock_options`] returns.
pub(crate) fn is_sharing_violation(error: &io::Error) -> bool {
    cfg!(windows) && error.raw_os_error() == Some(32)
}

/// Whether `error` is Windows' `ERROR_USER_MAPPED_FILE` (1224): a file that a process
/// maps cannot be shortened.
pub(crate) fn is_user_mapped(error: &io::Error) -> bool {
    cfg!(windows) && error.raw_os_error() == Some(1224)
}

/// The process that holds the `flock` of `file`, where the system tells: Linux lists
/// every lock in `/proc/locks` with its process and its device and inode. `None`
/// elsewhere, or when it is not found.
pub(crate) fn lock_holder(file: &File) -> Option<u32> {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = file.metadata().ok()?;
        // `/proc/locks` writes the device as major:minor in hexadecimal, then the inode.
        let id = format!(
            "{:02x}:{:02x}:{}",
            libc::major(meta.dev()),
            libc::minor(meta.dev()),
            meta.ino()
        );
        let locks = std::fs::read_to_string("/proc/locks").ok()?;
        locks.lines().find_map(|line| {
            // `N: FLOCK ADVISORY WRITE PID DEVICE:INODE START END`; a waiter has `->`
            // after `N:`.
            let fields: Vec<&str> = line.split_whitespace().collect();
            match fields[..] {
                [_, "FLOCK", _, _, pid, at, ..] if at == id => pid.parse().ok(),
                _ => None,
            }
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = file;
        None
    }
}

/// Creates a new, empty file in `dir`, open for reading and writing, that the operating
/// system deletes when it is closed: unlinked at once on Unix, `FILE_FLAG_DELETE_ON_CLOSE`
/// on Windows. Nothing is left behind, even if the process is killed.
pub(crate) fn create_temp(dir: &Path) -> io::Result<File> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_FLAG_DELETE_ON_CLOSE | FILE_ATTRIBUTE_TEMPORARY (winbase.h, winnt.h).
        options.custom_flags(0x0400_0000).attributes(0x100);
    }
    loop {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path: PathBuf = dir.join(format!(".everygraph-spill-{}-{n}", std::process::id()));
        match options.open(&path) {
            Ok(file) => {
                #[cfg(not(windows))]
                std::fs::remove_file(&path)?;
                return Ok(file);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A direct-I/O file the way Windows serves one: a read at an unaligned offset is an
    /// error (os error 87 there), and a read reaching the end comes back short.
    fn windows_like(len: u64) -> impl FnMut(&mut [u8], u64) -> io::Result<usize> {
        move |piece, at| {
            if at % everygraph_core::SEGMENT_ALIGN as u64 != 0 {
                return Err(io::ErrorKind::InvalidInput.into());
            }
            let n = usize::try_from(len.saturating_sub(at))
                .unwrap()
                .min(piece.len());
            piece[..n].fill(1);
            Ok(n)
        }
    }

    /// Pins the Windows fix on every platform: after a short read at the end of the
    /// file, no further (unaligned) read is issued.
    #[test]
    fn a_short_read_at_the_end_ends_the_loop() {
        let mut buf = vec![0_u8; 3 * 4096];
        for (len, expected) in [(1, 1), (4095, 4095), (4096 + 10, 4096 + 10), (8192, 8192)] {
            let got = read_up_to_with(&mut buf, 0, windows_like(len)).unwrap();
            assert_eq!(got, expected, "file of {len} bytes");
        }
        // Whole blocks keep going, from aligned offsets, until the buffer is full.
        let mut calls = Vec::new();
        let mut inner = windows_like(1 << 20);
        let got = read_up_to_with(&mut buf, 4096, |piece, at| {
            calls.push(at);
            let n = piece.len().min(4096);
            inner(&mut piece[..n], at)
        })
        .unwrap();
        assert_eq!(got, buf.len());
        assert_eq!(calls, [4096, 8192, 12288]);
    }
}
