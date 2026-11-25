// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The one lock of official runs.
//!
//! An official run takes this lock, without waiting, before anything else, and holds it
//! to the end; nothing else in the harness takes a lock. A second official run — from
//! another shell, or nested inside a benchmark command — fails at once with the holder's
//! description instead of queueing (a nested `flock` once deadlocked a lab run for half
//! an hour).
//!
//! It is an `flock` on a file under `/run/user/<uid>` (one per user and machine; the
//! idle gate catches other users' work). The kernel releases it when the holder exits,
//! however it exits. The holder writes who it is into the file for the message.

use std::fs::{File, OpenOptions};
use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::path::{Path, PathBuf};

use super::sys;

/// Environment variable naming another lock file, honoured only in builds with the
/// `test-hooks` feature (which only the crate's own tests enable): two official runs
/// that name different files would not exclude each other.
pub const LOCK_ENV: &str = "EVERYGRAPH_BENCH_LOCK";

/// A held official-run lock; released when dropped.
#[derive(Debug)]
pub struct OfficialLock {
    _file: File,
    path: PathBuf,
}

/// Why the lock was not taken.
#[derive(Debug)]
pub enum LockError {
    /// Another official run holds it.
    Held {
        /// The lock file.
        path: PathBuf,
        /// What the holder wrote about itself.
        holder: String,
    },
    /// The lock file cannot be opened or locked.
    Io(String),
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LockError::Held { path, holder } => write!(
                f,
                "another official run is in progress ({holder}; lock {}). Official runs \
                 never queue: wait for it to finish and start this one again, or run \
                 without --official for an exploratory measurement",
                path.display()
            ),
            LockError::Io(e) => f.write_str(e),
        }
    }
}

/// The lock file: `/run/user/<uid>/everygraph-bench-official.lock`, else the same name
/// in the temporary directory with the uid in it. (Test builds: `$EVERYGRAPH_BENCH_LOCK`
/// first.)
#[must_use]
pub fn default_path() -> PathBuf {
    #[cfg(any(test, feature = "test-hooks"))]
    if let Some(p) = std::env::var_os(LOCK_ENV).filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    let uid = uid();
    let run = PathBuf::from(format!("/run/user/{uid}"));
    if run.is_dir() {
        run.join("everygraph-bench-official.lock")
    } else {
        std::env::temp_dir().join(format!("everygraph-bench-official-{uid}.lock"))
    }
}

fn uid() -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        // The owner of our own /proc entry is our real uid, without a system call.
        std::fs::metadata("/proc/self").map_or(0, |m| m.uid())
    }
    #[cfg(not(unix))]
    {
        0
    }
}

impl OfficialLock {
    /// Takes the lock at `path` without waiting and records `holder` in it.
    ///
    /// # Errors
    ///
    /// [`LockError::Held`] when another run holds it; [`LockError::Io`] otherwise.
    pub fn take(path: &Path, holder: &str) -> Result<Self, LockError> {
        let io = |what: &str, e: std::io::Error| {
            LockError::Io(format!("{what} {}: {e}", path.display()))
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io("creating the directory of", e))?;
        }
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|e| io("opening", e))?;
        if !sys::try_lock(&file).map_err(|e| io("locking", e))? {
            let mut holder = String::new();
            let _ignored = file.read_to_string(&mut holder);
            let holder = holder.trim();
            return Err(LockError::Held {
                path: path.to_path_buf(),
                holder: if holder.is_empty() {
                    "holder unknown".into()
                } else {
                    holder.to_string()
                },
            });
        }
        // Ours now: say who holds it. Failing to write is not fatal; the lock holds.
        let _ignored = file
            .set_len(0)
            .and_then(|()| file.seek(SeekFrom::Start(0)).map(drop))
            .and_then(|()| file.write_all(holder.as_bytes()))
            .and_then(|()| file.flush());
        Ok(Self {
            _file: file,
            path: path.to_path_buf(),
        })
    }

    /// The lock file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use crate::datasets::test_support::TempDir;

    #[test]
    fn a_second_taker_fails_at_once_with_the_holder() {
        let dir = TempDir::new("official-lock");
        let path = dir.path().join("sub").join("official.lock");
        let held = OfficialLock::take(&path, "pid 1, spec demo").expect("free lock");
        let started = std::time::Instant::now();
        let err = OfficialLock::take(&path, "pid 2").expect_err("held");
        assert!(
            started.elapsed() < std::time::Duration::from_millis(500),
            "it must not wait"
        );
        let LockError::Held { holder, .. } = &err else {
            panic!("{err}");
        };
        assert_eq!(holder, "pid 1, spec demo");
        assert!(err.to_string().contains("never queue"), "{err}");
        drop(held);
        // Another test of this binary may fork a child while the lock is held here; the
        // child shares the descriptor until its `exec` closes it (close-on-exec), so the
        // release can lag by that window. Allow it a second.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        let again = loop {
            match OfficialLock::take(&path, "pid 3") {
                Ok(l) => break l,
                Err(e) => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "not released on drop: {e}"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
        };
        assert_eq!(
            std::fs::read_to_string(again.path()).expect("read"),
            "pid 3"
        );
    }

    #[test]
    fn the_lock_dies_with_its_holder() {
        // A child process takes the lock and is killed; the kernel releases the lock and
        // no stale file blocks the next run. The child is a shell that locks its own fd 9
        // and then becomes `sleep` (exec keeps the fd): plain `flock FILE sleep 30` would
        // leave the lock with the forked `sleep` after `flock` itself is killed.
        let dir = TempDir::new("official-lock-death");
        let path = dir.path().join("official.lock");
        let Ok(mut child) = std::process::Command::new("sh")
            .args(["-c", "exec 9>>\"$1\" && flock -n 9 && exec sleep 30", "sh"])
            .arg(&path)
            .spawn()
        else {
            return; // no shell: nothing to test with
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while OfficialLock::take(&path, "probe").is_ok() {
            if child.try_wait().expect("poll our child").is_some() {
                return; // util-linux flock(1) missing: nothing to test with
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the child never took the lock"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        child.kill().expect("kill our own child by its handle");
        child.wait().expect("reap");
        OfficialLock::take(&path, "after").expect("released by the kernel");
    }
}
