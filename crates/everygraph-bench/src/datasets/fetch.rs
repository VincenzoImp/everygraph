// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `fetch` and `verify`: bring a dataset's files into the data root, check them, and
//! place them atomically.

use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use ureq::Agent;

use super::download::{self, DownloadOptions};
use super::error::{Context, Error, Result};
use super::registry::{Dataset, FileEntry};
use super::store::{self, DataRoot, FileStatus};

/// Settings of a fetch.
#[derive(Debug, Clone, Default)]
pub struct FetchOptions {
    /// Network behaviour.
    pub download: DownloadOptions,
    /// Look here first for a file of the same name (a copy downloaded earlier, for
    /// example). It is copied (never hard-linked, so the data root shares no inode with
    /// it) and the copy is checked like a download. The directory is only read.
    pub from_dir: Option<PathBuf>,
    /// Download files whose checksum the registry does not pin (tier 3), and report the
    /// computed one.
    pub allow_unpinned: bool,
    /// Delete a placed file that no longer matches the registry and fetch it again,
    /// instead of stopping with an error.
    pub replace: bool,
}

/// What happened to one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// It was already there and verified; nothing was read or downloaded.
    AlreadyVerified,
    /// It was there and has been verified now.
    VerifiedInPlace,
    /// It was downloaded (or generated, or taken from `--from-dir`) and verified.
    Placed {
        /// Where the bytes came from: a URL, "generated", or a local path.
        from: String,
    },
    /// Downloaded without a pinned checksum; this is the one computed.
    PlacedUnpinned {
        /// The URL.
        from: String,
        /// SHA-256 of the file, to be recorded in the registry.
        sha256: String,
    },
}

/// Fetches every file of `dataset` into `root`.
///
/// # Errors
/// The first file that cannot be obtained or does not match its checksum. Files already
/// placed stay placed; a partial download stays for the next attempt, unless its checksum
/// was wrong, in which case it is deleted.
pub fn fetch(
    root: &DataRoot,
    agent: &Agent,
    dataset: &Dataset,
    options: &FetchOptions,
    log: &mut dyn Write,
) -> Result<Vec<Outcome>> {
    let mut outcomes = Vec::new();
    for file in &dataset.files {
        let outcome = fetch_file(root, agent, dataset, file, options, log)
            .context(|| format!("{}/{}", dataset.name, file.name))?;
        outcomes.push(outcome);
    }
    Ok(outcomes)
}

fn fetch_file(
    root: &DataRoot,
    agent: &Agent,
    dataset: &Dataset,
    file: &FileEntry,
    options: &FetchOptions,
    log: &mut dyn Write,
) -> Result<Outcome> {
    let final_path = root.file_path(dataset, file);
    if let Some(outcome) = present(root, dataset, file, options, log)? {
        return Ok(outcome);
    }
    if file.sha256.is_none() && !options.allow_unpinned {
        return Err(Error::new(
            "the registry pins no checksum for this file; pass --allow-unpinned to download \
             it anyway and record the checksum it reports",
        ));
    }
    let partial_dir = root.partial_dir(dataset);
    fs::create_dir_all(&partial_dir).context(|| format!("creating {}", partial_dir.display()))?;
    let lock = Lock::take(&root.lock_path(dataset, file))?;
    // Another fetch may have placed the file while this one waited for the lock.
    if let Some(outcome) = present(root, dataset, file, options, log)? {
        return Ok(outcome);
    }
    let part = root.part_path(dataset, file);
    let state = root.state_path(dataset, file);
    if !state.exists() {
        // Without a state file a `.part` cannot be resumed: it is what a plain stream
        // or a crash just before placing left behind.
        let _ignored = fs::remove_file(&part);
    }

    let from = if let Some(generator) = dataset.generator {
        let _ignored = writeln!(log, "  generating {}", file.name);
        let out = File::create(&part).context(|| format!("creating {}", part.display()))?;
        let mut out = BufWriter::new(out);
        generator
            .write(&mut out)
            .context(|| format!("writing {}", part.display()))?;
        out.into_inner()
            .map_err(|e| Error::new(e.to_string()))?
            .sync_all()
            .context(|| format!("syncing {}", part.display()))?;
        "generated".to_string()
    } else if let Some(local) = options
        .from_dir
        .as_ref()
        .map(|d| d.join(&file.name))
        .filter(|p| p.is_file())
    {
        let _ignored = writeln!(log, "  copying {} from {}", file.name, local.display());
        copy_into(&local, &part)?;
        local.display().to_string()
    } else {
        let mut last_error = None;
        let mut got = None;
        for url in file.urls() {
            let _ignored = writeln!(log, "  downloading {url}");
            match download::download(agent, url, file.size, &part, &state, &options.download, log) {
                Ok(_) => {
                    got = Some(url.to_string());
                    break;
                }
                Err(e) => {
                    let _ignored = writeln!(log, "  failed: {e}");
                    last_error = Some(e);
                }
            }
        }
        match got {
            Some(url) => url,
            None => {
                return Err(last_error.unwrap_or_else(|| Error::new("no URL to download from")));
            }
        }
    };

    let _ignored = writeln!(log, "  checking {}", file.name);
    // Flushed while still writable: Windows flushes only through a handle with write
    // access, which a read-only file no longer grants ("Access is denied").
    OpenOptions::new()
        .write(true)
        .open(&part)
        .and_then(|f| f.sync_all())
        .context(|| format!("syncing {}", part.display()))?;
    // Read-only before the snapshot: a verified file is not meant to be edited in place.
    set_read_only(&part)?;
    let checked = store::hash_unchanged(&part, file.md5_published.is_some()).and_then(
        |(hashes, snapshot)| {
            store::check_hashes(file, &hashes)?;
            Ok((hashes, snapshot))
        },
    );
    let (hashes, snapshot) = match checked {
        Ok(v) => v,
        Err(e) => {
            // Wrong bytes are not worth resuming: start over next time.
            let _ignored = fs::remove_file(&part);
            let _ignored = fs::remove_file(&state);
            return Err(e);
        }
    };
    place(&part, &final_path)?;
    // Left over when an earlier download was interrupted and the file then came from
    // elsewhere (`--from-dir`, a server without ranges).
    let _ignored = fs::remove_file(&state);
    // The rename keeps size and modification time: the snapshot still describes the
    // hashed bytes, and `write_stamp` checks that it does.
    root.write_stamp(dataset, file, &hashes.sha256, &snapshot)?;
    drop(lock);
    // Succeeds only when nothing else is partial in this dataset.
    let _ignored = fs::remove_dir(&partial_dir);
    if file.sha256.is_none() {
        Ok(Outcome::PlacedUnpinned {
            from,
            sha256: hashes.sha256,
        })
    } else {
        Ok(Outcome::Placed { from })
    }
}

/// Re-reads a placed file and checks it against the registry; records the result.
///
/// # Errors
/// If the file is missing, unreadable, or does not match. A mismatching file loses its
/// stamp but is left in place for inspection.
pub fn verify_file(root: &DataRoot, dataset: &Dataset, file: &FileEntry) -> Result<FileStatus> {
    let path = root.file_path(dataset, file);
    if !path.is_file() {
        return Err(Error::new(format!("{} is missing", path.display())));
    }
    let checked = store::hash_unchanged(&path, file.md5_published.is_some()).and_then(
        |(hashes, snapshot)| {
            store::check_hashes(file, &hashes)?;
            Ok((hashes, snapshot))
        },
    );
    let (hashes, snapshot) = match checked {
        Ok(v) => v,
        Err(e) => {
            root.remove_stamp(dataset, file);
            return Err(e);
        }
    };
    root.write_stamp(dataset, file, &hashes.sha256, &snapshot)?;
    Ok(root.status(dataset, file))
}

/// Handles a file that is already in place: `Some(outcome)` if nothing more is to be
/// done, `None` if it must be obtained.
fn present(
    root: &DataRoot,
    dataset: &Dataset,
    file: &FileEntry,
    options: &FetchOptions,
    log: &mut dyn Write,
) -> Result<Option<Outcome>> {
    match root.status(dataset, file) {
        FileStatus::Verified => Ok(Some(Outcome::AlreadyVerified)),
        FileStatus::Unpinned { .. } if options.allow_unpinned => Ok(Some(Outcome::AlreadyVerified)),
        FileStatus::Unverified | FileStatus::Unpinned { .. } => {
            let _ignored = writeln!(log, "  {} is present; verifying it", file.name);
            match verify_file(root, dataset, file) {
                Ok(_) => Ok(Some(Outcome::VerifiedInPlace)),
                Err(e) if options.replace => {
                    let _ignored = writeln!(log, "  {e}; replacing it");
                    let path = root.file_path(dataset, file);
                    fs::remove_file(&path).context(|| format!("removing {}", path.display()))?;
                    Ok(None)
                }
                Err(e) => Err(Error::new(format!(
                    "{e}. The file in place is not the one the registry describes: delete {} \
                     and run fetch again, or pass --replace",
                    root.file_path(dataset, file).display()
                ))),
            }
        }
        FileStatus::Missing | FileStatus::Partial { .. } => Ok(None),
    }
}

/// Moves the verified `.part`, flushed before it was made read-only, into place: rename,
/// flush the directory.
fn place(part: &Path, final_path: &Path) -> Result<()> {
    fs::rename(part, final_path)
        .context(|| format!("moving {} to {}", part.display(), final_path.display()))?;
    if let Some(dir) = final_path.parent() {
        // Directory fsync makes the rename durable on Linux; elsewhere opening a
        // directory may fail, and the rename is still atomic.
        if let Ok(d) = File::open(dir) {
            let _ignored = d.sync_all();
        }
    }
    Ok(())
}

/// Copies a local file into the `.part`. A copy, never a hard link: the source (on the
/// lab machine, the read-only `raw/` directory) must not share an inode with anything
/// in the data root. `std::fs::copy` uses `copy_file_range` on Linux, which shares
/// extents (a reflink) on file systems that support it, and copies otherwise. A
/// symbolic link as source is followed.
fn copy_into(from: &Path, to: &Path) -> Result<()> {
    let _ignored = fs::remove_file(to);
    fs::copy(from, to)
        .map(|_| ())
        .context(|| format!("copying {} to {}", from.display(), to.display()))
}

fn set_read_only(path: &Path) -> Result<()> {
    let mut permissions = fs::metadata(path)
        .context(|| format!("reading {}", path.display()))?
        .permissions();
    permissions.set_readonly(true);
    fs::set_permissions(path, permissions)
        .context(|| format!("making {} read-only", path.display()))
}

/// A lock file held while a file is fetched, so that two fetches do not write the same
/// `.part`. It holds the owner's process id from the moment it exists: the id is written
/// to a temporary file which is then hard-linked to the lock name, an atomic step that
/// fails if the lock exists. A lock without a readable id counts as held. A lock whose
/// process is gone (checked through `/proc`, so on Linux only) is taken over.
///
/// Not covered: two fetches taking over the *same* stale lock at the same instant (both
/// may remove it before either links its own), and processes on other hosts or in other
/// PID namespaces sharing the data root.
struct Lock {
    path: PathBuf,
}

impl Lock {
    fn take(path: &Path) -> Result<Self> {
        let pid = std::process::id();
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(format!(".{pid}.tmp"));
        let tmp = PathBuf::from(tmp);
        fs::write(&tmp, format!("{pid}\n")).context(|| format!("writing {}", tmp.display()))?;
        let result = Self::link(&tmp, path);
        let _ignored = fs::remove_file(&tmp);
        result
    }

    fn link(tmp: &Path, path: &Path) -> Result<Self> {
        for _ in 0..2 {
            match fs::hard_link(tmp, path) {
                Ok(()) => {
                    return Ok(Self {
                        path: path.to_path_buf(),
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let owner = fs::read_to_string(path).unwrap_or_default();
                    let owner = owner.trim().to_string();
                    if !lock_is_stale(&owner) {
                        return Err(Error::new(format!(
                            "another fetch (process {owner:?}) holds {}; if no fetch is \
                             running, delete that file",
                            path.display()
                        )));
                    }
                    // Remove only if it still names the same dead process.
                    if fs::read_to_string(path).unwrap_or_default().trim() == owner {
                        let _ignored = fs::remove_file(path);
                    }
                }
                Err(e) => return Err(Error::new(format!("creating {}: {e}", path.display()))),
            }
        }
        Err(Error::new(format!("could not take {}", path.display())))
    }
}

/// Whether a lock naming `owner` may be taken over: only a well-formed process id with no
/// `/proc` entry. An empty or unreadable owner, or a system without `/proc`, means held.
fn lock_is_stale(owner: &str) -> bool {
    let proc = Path::new("/proc");
    proc.is_dir() && owner.parse::<u32>().is_ok() && !proc.join(owner).exists()
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ignored = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datasets::test_support::TempDir;

    #[test]
    fn a_lock_held_by_a_live_process_or_without_an_owner_is_refused() {
        let dir = TempDir::new("lock");
        let path = dir.path().join("f.lock");
        let held = Lock::take(&path).expect("free lock");
        assert_eq!(
            fs::read_to_string(&path).expect("read"),
            format!("{}\n", std::process::id()),
            "the id is in the lock from the moment it exists"
        );
        // This process is alive: a second taker must be refused.
        let err = Lock::take(&path).err().expect("held lock refused");
        assert!(err.to_string().contains("holds"), "{err}");
        drop(held);
        assert!(!path.exists(), "released on drop");
        // An empty lock (a writer caught between create and write) counts as held.
        fs::write(&path, "").expect("write");
        assert!(Lock::take(&path).is_err());
        fs::write(&path, "not a pid\n").expect("write");
        assert!(Lock::take(&path).is_err());
    }

    // Linux only: elsewhere there is no `/proc` to tell a dead owner from a live one, so
    // a lock is never taken over (`lock_is_stale`), by design.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_lock_of_a_dead_process_is_taken_over() {
        let dir = TempDir::new("stale-lock");
        let path = dir.path().join("f.lock");
        // Above Linux's largest pid_max (2^22), so no such process can exist.
        fs::write(&path, "4294967\n").expect("write");
        let lock = Lock::take(&path).expect("stale lock taken over");
        assert_eq!(
            fs::read_to_string(&path).expect("read"),
            format!("{}\n", std::process::id())
        );
        drop(lock);
        let leftovers: Vec<_> = fs::read_dir(dir.path()).expect("ls").collect();
        assert!(
            leftovers.is_empty(),
            "no temporary file left: {leftovers:?}"
        );
    }
}
