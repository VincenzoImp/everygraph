// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The job's scratch directory, `.everygraph-ID` in the scratch root
//! (`docs/ux-contract.md` § 5.3), removed when the job ends.

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use everygraph_ingest::ids::Scratch;
use everygraph_io::MmapSource;

/// The job's identifier: the first 16 hexadecimal digits of the XXH3-128 of its
/// `fingerprint` (§ 5.3, R-4).
pub(crate) fn job_id(fingerprint: &str) -> String {
    let hash = xxhash_rust::xxh3::xxh3_128(fingerprint.as_bytes());
    format!("{:016x}", hash >> 64)
}

/// `path` with its directory made absolute, for a fingerprint.
pub(crate) fn absolute(path: &Path) -> PathBuf {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::canonicalize(dir).map_or_else(
        |_| path.to_path_buf(),
        |d| d.join(path.file_name().unwrap_or_default()),
    )
}

/// The scratch directory of one job, removed on drop. On Unix its files are unlinked as
/// soon as they are made, so a hard stop leaves at most the empty directory, which the
/// next run of the same job takes over.
#[derive(Debug)]
pub(crate) struct JobScratch {
    dir: PathBuf,
    next: AtomicU64,
}

impl JobScratch {
    /// Creates `.everygraph-JOB` in `root`.
    pub(crate) fn create(root: &Path, job: &str) -> io::Result<Self> {
        let dir = root.join(format!(".everygraph-{job}"));
        match std::fs::create_dir(&dir) {
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            other => other?,
        }
        Ok(Self {
            dir,
            next: AtomicU64::new(0),
        })
    }

    /// The directory.
    pub(crate) fn dir(&self) -> &Path {
        &self.dir
    }
}

impl Drop for JobScratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Scratch for JobScratch {
    type Source = MmapSource;
    type Writer = File;

    fn create(&self, purpose: &'static str) -> io::Result<File> {
        let i = self.next.fetch_add(1, Ordering::Relaxed);
        let path = self
            .dir
            .join(format!("{i:08}-{}", purpose.replace(' ', "-")));
        let file = File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)?;
        // Elsewhere an open file cannot be unlinked; the directory's removal takes it.
        #[cfg(unix)]
        std::fs::remove_file(&path)?;
        Ok(file)
    }

    /// Maps the file: it is never written again (the map's invariant, `MmapSource`).
    fn seal(&self, file: File) -> io::Result<MmapSource> {
        MmapSource::from_file(&file)
    }
}
