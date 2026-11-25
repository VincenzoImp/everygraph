// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Everything that differs between operating systems, and every `unsafe` block of the
//! crate.
//!
//! Why safe code is not enough: std has no memory maps, no way to preallocate a file,
//! no `posix_fadvise` and no `F_NOCACHE`. Each `unsafe` block below is one call into the
//! C library (or memmap2's `Mmap::map`, which is one `mmap`). None of them passes a
//! pointer: their arguments are a file descriptor this module borrows from a live
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

/// Reads into `buf` from `offset` until it is full or the file ends; returns the number
/// of bytes read.
pub(crate) fn read_up_to(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    let mut done = 0;
    while done < buf.len() {
        let at = offset + done as u64;
        #[cfg(unix)]
        let result = std::os::unix::fs::FileExt::read_at(file, &mut buf[done..], at);
        #[cfg(windows)]
        let result = std::os::windows::fs::FileExt::seek_read(file, &mut buf[done..], at);
        #[cfg(not(any(unix, windows)))]
        let result: io::Result<usize> = Err(io::ErrorKind::Unsupported.into());
        match result {
            Ok(0) => break,
            Ok(n) => done += n,
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

/// Maps all of `file` read-only.
///
/// The map is only as stable as the file: if another process truncates the file while
/// it is mapped, reading the lost pages raises `SIGBUS` (on Windows the truncation is
/// refused instead). `ByteSource` requires a source that does not change; the backends
/// document that the file must not be modified while they are open.
pub(crate) fn map(file: &File) -> io::Result<Mmap> {
    // SAFETY: the map is read-only and never handed out mutably, so no Rust reference
    // into it can be written through by this process. Writes or truncation by other
    // processes are outside what this function can prevent; the contract above makes
    // them the caller's responsibility, as every memory-mapping API does.
    unsafe { Mmap::map(file) }
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
