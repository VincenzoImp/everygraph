// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Where datasets live on disk, and what is known about the files there.
//!
//! Layout under the data root:
//!
//! ```text
//! <root>/<dataset>/<file>                  verified files, placed by rename
//! <root>/<dataset>/.verified/<file>        stamp: sha256, size and mtime at verification
//! <root>/<dataset>/.partial/<file>.part    download in progress
//! <root>/<dataset>/.partial/<file>.state   which byte ranges of the .part are written
//! <root>/<dataset>/.partial/<file>.lock    held by the fetch writing that file
//! ```
//!
//! A file counts as verified when its stamp matches its current size and modification
//! time and the registry's checksum: `list` stays cheap, and `verify` re-reads.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use md5::Md5;
use sha2::{Digest, Sha256};

use super::error::{Context, Error, Result};
use super::registry::{Dataset, FileEntry};

/// The environment variable that names the data root.
pub const DATA_ENV: &str = "EVERYGRAPH_DATA";

/// The data root used when [`DATA_ENV`] is not set, relative to the working directory.
/// It is listed in the repository's `.gitignore`.
pub const DEFAULT_ROOT: &str = "data";

/// The directory that holds the datasets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataRoot {
    path: PathBuf,
}

/// What is on disk for one file of a dataset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileStatus {
    /// Nothing, not even a partial download.
    Missing,
    /// A download was interrupted after this many bytes.
    Partial {
        /// Bytes already written.
        done: u64,
    },
    /// The file is there but has not been verified since it last changed.
    Unverified,
    /// The file matches the checksum pinned in the registry.
    Verified,
    /// The registry pins no checksum; the stamp records the one computed at download.
    Unpinned {
        /// SHA-256 computed when the file was placed.
        sha256: String,
    },
}

impl FileStatus {
    /// A short word for tables.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            FileStatus::Missing => "missing".into(),
            FileStatus::Partial { done } => format!("partial ({})", human_bytes(*done)),
            FileStatus::Unverified => "present, not verified".into(),
            FileStatus::Verified => "verified".into(),
            FileStatus::Unpinned { .. } => "present, unpinned".into(),
        }
    }
}

impl DataRoot {
    /// A data root at `path`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// `$EVERYGRAPH_DATA` if set and not empty, otherwise `./data`.
    #[must_use]
    pub fn from_env() -> Self {
        match std::env::var_os(DATA_ENV) {
            Some(p) if !p.is_empty() => Self::new(p),
            _ => Self::new(DEFAULT_ROOT),
        }
    }

    /// The root directory.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `<root>/<dataset>`.
    #[must_use]
    pub fn dataset_dir(&self, dataset: &Dataset) -> PathBuf {
        self.path.join(&dataset.name)
    }

    /// Where a verified file of the dataset lives.
    #[must_use]
    pub fn file_path(&self, dataset: &Dataset, file: &FileEntry) -> PathBuf {
        self.dataset_dir(dataset).join(&file.name)
    }

    pub(crate) fn partial_dir(&self, dataset: &Dataset) -> PathBuf {
        self.dataset_dir(dataset).join(".partial")
    }

    pub(crate) fn part_path(&self, dataset: &Dataset, file: &FileEntry) -> PathBuf {
        self.partial_dir(dataset)
            .join(format!("{}.part", file.name))
    }

    pub(crate) fn state_path(&self, dataset: &Dataset, file: &FileEntry) -> PathBuf {
        self.partial_dir(dataset)
            .join(format!("{}.state", file.name))
    }

    pub(crate) fn lock_path(&self, dataset: &Dataset, file: &FileEntry) -> PathBuf {
        self.partial_dir(dataset)
            .join(format!("{}.lock", file.name))
    }

    fn stamp_path(&self, dataset: &Dataset, file: &FileEntry) -> PathBuf {
        self.dataset_dir(dataset).join(".verified").join(&file.name)
    }

    /// What is on disk for `file`, without reading its contents.
    #[must_use]
    pub fn status(&self, dataset: &Dataset, file: &FileEntry) -> FileStatus {
        let path = self.file_path(dataset, file);
        let Ok(meta) = fs::metadata(&path) else {
            return match fs::metadata(self.state_path(dataset, file)) {
                Ok(_) => FileStatus::Partial {
                    done: super::download::State::load(&self.state_path(dataset, file))
                        .map(|s| s.done())
                        .unwrap_or(0),
                },
                Err(_) => FileStatus::Missing,
            };
        };
        let Some(stamp) = self.read_stamp(dataset, file) else {
            return FileStatus::Unverified;
        };
        if stamp.size != meta.len() || Some(stamp.mtime) != mtime_of(&meta) {
            return FileStatus::Unverified;
        }
        match &file.sha256 {
            Some(pinned) if *pinned == stamp.sha256 => FileStatus::Verified,
            Some(_) => FileStatus::Unverified,
            None => FileStatus::Unpinned {
                sha256: stamp.sha256,
            },
        }
    }

    /// Records that `path` (the placed file) hashed to `sha256`.
    pub(crate) fn write_stamp(
        &self,
        dataset: &Dataset,
        file: &FileEntry,
        sha256: &str,
    ) -> Result<()> {
        let path = self.file_path(dataset, file);
        let meta = fs::metadata(&path).context(|| format!("reading {}", path.display()))?;
        let mtime = mtime_of(&meta).unwrap_or_default();
        let stamp = self.stamp_path(dataset, file);
        if let Some(dir) = stamp.parent() {
            fs::create_dir_all(dir).context(|| format!("creating {}", dir.display()))?;
        }
        let text = format!("sha256 {sha256}\nsize {}\nmtime {mtime}\n", meta.len());
        write_atomically(&stamp, text.as_bytes())
    }

    /// Removes the stamp of `file`, if any.
    pub(crate) fn remove_stamp(&self, dataset: &Dataset, file: &FileEntry) {
        let _ignored = fs::remove_file(self.stamp_path(dataset, file));
    }

    fn read_stamp(&self, dataset: &Dataset, file: &FileEntry) -> Option<Stamp> {
        let text = fs::read_to_string(self.stamp_path(dataset, file)).ok()?;
        let mut sha256 = None;
        let mut size = None;
        let mut mtime = None;
        for line in text.lines() {
            match line.split_once(' ') {
                Some(("sha256", v)) => sha256 = Some(v.to_string()),
                Some(("size", v)) => size = v.parse().ok(),
                Some(("mtime", v)) => mtime = Some(v.to_string()),
                _ => {}
            }
        }
        Some(Stamp {
            sha256: sha256?,
            size: size?,
            mtime: mtime?,
        })
    }
}

struct Stamp {
    sha256: String,
    size: u64,
    mtime: String,
}

fn mtime_of(meta: &fs::Metadata) -> Option<String> {
    let t = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some(format!("{}.{:09}", t.as_secs(), t.subsec_nanos()))
}

/// The hashes of a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hashes {
    /// SHA-256, lowercase hex.
    pub sha256: String,
    /// MD5, lowercase hex, when asked for.
    pub md5: Option<String>,
    /// Bytes read.
    pub size: u64,
}

/// Reads `path` once and computes its SHA-256 and, if `with_md5`, its MD5.
///
/// # Errors
/// If the file cannot be read.
pub fn hash_file(path: &Path, with_md5: bool) -> Result<Hashes> {
    let mut f = File::open(path).context(|| format!("opening {}", path.display()))?;
    let mut sha = Sha256::new();
    let mut md5 = with_md5.then(Md5::new);
    let mut buf = vec![0_u8; 1 << 20];
    let mut size = 0_u64;
    loop {
        let n = f
            .read(&mut buf)
            .context(|| format!("reading {}", path.display()))?;
        if n == 0 {
            break;
        }
        let chunk = buf.get(..n).unwrap_or_default();
        sha.update(chunk);
        if let Some(m) = md5.as_mut() {
            m.update(chunk);
        }
        size += u64::try_from(n).unwrap_or(u64::MAX);
    }
    Ok(Hashes {
        sha256: hex(&sha.finalize()),
        md5: md5.map(|m| hex(&m.finalize())),
        size,
    })
}

/// Lowercase hexadecimal.
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _infallible = write!(s, "{b:02x}");
    }
    s
}

/// Checks `hashes` of `file` against the registry: size, pinned SHA-256 and the MD5
/// published by the source.
///
/// # Errors
/// A message naming each mismatch.
pub fn check_hashes(file: &FileEntry, hashes: &Hashes) -> Result<()> {
    let mut problems = Vec::new();
    if let Some(size) = file.size {
        if size != hashes.size {
            problems.push(format!(
                "size is {} bytes, registry says {size}",
                hashes.size
            ));
        }
    }
    if let Some(pinned) = &file.sha256 {
        if *pinned != hashes.sha256 {
            problems.push(format!(
                "SHA-256 is {}, registry says {pinned}",
                hashes.sha256
            ));
        }
    }
    if let (Some(published), Some(md5)) = (&file.md5_published, &hashes.md5) {
        if published != md5 {
            problems.push(format!("MD5 is {md5}, the source publishes {published}"));
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(Error::new(format!(
            "{} does not match: {}",
            file.name,
            problems.join("; ")
        )))
    }
}

/// Writes `bytes` to `path` through a temporary file and a rename.
pub(crate) fn write_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    fs::write(&tmp, bytes).context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, path).context(|| format!("renaming {} into place", tmp.display()))
}

/// `1234567` → `1.2 MB` (decimal units, as download sizes are usually quoted).
#[must_use]
pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    let mut unit = 0;
    let mut scaled = n;
    let mut rem = 0;
    while scaled >= 1000 && unit + 1 < UNITS.len() {
        rem = scaled % 1000;
        scaled /= 1000;
        unit += 1;
    }
    let name = UNITS.get(unit).copied().unwrap_or("B");
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{scaled}.{} {name}", rem / 100)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_match_known_vectors() {
        let dir = super::super::test_support::TempDir::new("hash");
        let p = dir.path().join("abc");
        fs::write(&p, b"abc").expect("write");
        let h = hash_file(&p, true).expect("hash");
        assert_eq!(
            h.sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(h.md5.as_deref(), Some("900150983cd24fb0d6963f7d28e17f72"));
        assert_eq!(h.size, 3);
    }

    #[test]
    fn human_bytes_uses_decimal_units() {
        assert_eq!(human_bytes(999), "999 B");
        assert_eq!(human_bytes(124_262_769), "124.2 MB");
        assert_eq!(human_bytes(9_371_322_431), "9.3 GB");
    }
}
