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
    /// example); it is checked like a download and hard-linked, or copied if a link is
    /// impossible. The directory is only read.
    pub from_dir: Option<PathBuf>,
    /// Download files whose checksum the registry does not pin (tier 3), and report the
    /// computed one.
    pub allow_unpinned: bool,
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
    match root.status(dataset, file) {
        FileStatus::Verified => return Ok(Outcome::AlreadyVerified),
        FileStatus::Unpinned { .. } if options.allow_unpinned => {
            return Ok(Outcome::AlreadyVerified);
        }
        FileStatus::Unverified | FileStatus::Unpinned { .. } => {
            let _ignored = writeln!(log, "  {} is present; verifying it", file.name);
            verify_file(root, dataset, file)?;
            return Ok(Outcome::VerifiedInPlace);
        }
        FileStatus::Missing | FileStatus::Partial { .. } => {}
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
    let part = root.part_path(dataset, file);
    let state = root.state_path(dataset, file);

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
        let _ignored = writeln!(log, "  taking {} from {}", file.name, local.display());
        link_or_copy(&local, &part)?;
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
    let hashes = store::hash_file(&part, file.md5_published.is_some())?;
    if let Err(e) = store::check_hashes(file, &hashes) {
        // Wrong bytes are not worth resuming: start over next time.
        let _ignored = fs::remove_file(&part);
        let _ignored = fs::remove_file(&state);
        return Err(e);
    }
    place(&part, &final_path)?;
    // Left over when an earlier download was interrupted and the file then came from
    // elsewhere (`--from-dir`, a server without ranges).
    let _ignored = fs::remove_file(&state);
    root.write_stamp(dataset, file, &hashes.sha256)?;
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
    let hashes = store::hash_file(&path, file.md5_published.is_some())?;
    if let Err(e) = store::check_hashes(file, &hashes) {
        root.remove_stamp(dataset, file);
        return Err(e);
    }
    root.write_stamp(dataset, file, &hashes.sha256)?;
    Ok(root.status(dataset, file))
}

/// Moves the verified `.part` into place: flush, rename, flush the directory.
fn place(part: &Path, final_path: &Path) -> Result<()> {
    File::open(part)
        .and_then(|f| f.sync_all())
        .context(|| format!("syncing {}", part.display()))?;
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

fn link_or_copy(from: &Path, to: &Path) -> Result<()> {
    let _ignored = fs::remove_file(to);
    if fs::hard_link(from, to).is_ok() {
        return Ok(());
    }
    fs::copy(from, to)
        .map(|_| ())
        .context(|| format!("copying {} to {}", from.display(), to.display()))
}

/// A lock file held while a file is fetched, so that two fetches do not write the same
/// `.part`. It holds the owner's process id; a lock whose process is gone (checked on
/// Linux through `/proc`) is taken over.
struct Lock {
    path: PathBuf,
}

impl Lock {
    fn take(path: &Path) -> Result<Self> {
        for _ in 0..2 {
            match OpenOptions::new().write(true).create_new(true).open(path) {
                Ok(mut f) => {
                    let _ignored = writeln!(f, "{}", std::process::id());
                    return Ok(Self {
                        path: path.to_path_buf(),
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let owner = fs::read_to_string(path).unwrap_or_default();
                    let owner = owner.trim();
                    let alive = Path::new("/proc").is_dir()
                        && !owner.is_empty()
                        && Path::new("/proc").join(owner).exists();
                    if alive || !Path::new("/proc").is_dir() {
                        return Err(Error::new(format!(
                            "another fetch (process {owner}) holds {}; if no fetch is \
                             running, delete that file",
                            path.display()
                        )));
                    }
                    let _ignored = fs::remove_file(path);
                }
                Err(e) => return Err(Error::new(format!("creating {}: {e}", path.display()))),
            }
        }
        Err(Error::new(format!("could not take {}", path.display())))
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ignored = fs::remove_file(&self.path);
    }
}
