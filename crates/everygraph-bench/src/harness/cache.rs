// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The cold-cache protocol.
//!
//! There is no `sudo` here, so no `drop_caches`. Instead:
//!
//! 1. **Private copies.** `posix_fadvise(DONTNEED)` cannot drop pages that another
//!    process keeps mapped, and the shared datasets are mapped by whoever uses them. So
//!    a cold case reads a private copy of its dataset, made once under the work
//!    directory with a plain read/write loop (not `copy_file_range`, which may share
//!    blocks on a reflinking file system) and reused while its source is unchanged.
//! 2. **Evict, then check.** Before each repetition the copy is synced (dirty pages are
//!    not dropped) and evicted with `posix_fadvise(DONTNEED)`; then `mincore(2)`, through
//!    util-linux `fincore`, must report at most 1% of it resident, or the repetition is
//!    not measured and its row says `not_cold`.
//! 3. **Rotate.** A solid-state drive keeps a cache of its own: reading the same blocks twice
//!    in a row measures that cache, not the flash (FINDINGS § F1). Each dataset has at
//!    least two copies, and consecutive cold repetitions of it — across cases and across
//!    runs (the last copy read is kept in the copies' directory) — take them in turn.

use std::fs::File;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::Command;

use super::sys;

/// A repetition is not cold when more than this share of its copy is resident.
pub const MAX_RESIDENT_FRACTION: f64 = 0.01;

const MARKER: &str = ".everygraph-bench-copy";

/// The regular files under `path` (itself, if it is a file), sorted.
///
/// # Errors
///
/// `path` or a directory under it cannot be read.
pub fn files_under(path: &Path) -> Result<Vec<PathBuf>, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }
    let mut out = Vec::new();
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))? {
            let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
            let p = entry.path();
            let ty = entry
                .file_type()
                .map_err(|e| format!("{}: {e}", p.display()))?;
            if ty.is_dir() {
                stack.push(p);
            } else if ty.is_file() && entry.file_name() != MARKER {
                out.push(p);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Total size of the files under `path`.
///
/// # Errors
///
/// As [`files_under`], or a file vanishes.
pub fn size_of(path: &Path) -> Result<u64, String> {
    files_under(path)?
        .iter()
        .map(|f| {
            std::fs::metadata(f)
                .map(|m| m.len())
                .map_err(|e| format!("{}: {e}", f.display()))
        })
        .sum()
}

/// What a copy was made from: relative path, size and modification time of each file.
fn listing(source: &Path) -> Result<String, String> {
    // Paths relative to the parent keep the dataset's own name: `graph/targets.u32`.
    let base = source.parent().unwrap_or(source);
    let mut out = String::new();
    for f in files_under(source)? {
        let m = std::fs::metadata(&f).map_err(|e| format!("{}: {e}", f.display()))?;
        let mtime = m
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos());
        let rel = f.strip_prefix(base).unwrap_or(&f);
        out.push_str(&format!("{}\t{}\t{mtime}\n", rel.display(), m.len()));
    }
    Ok(out)
}

fn copy_file(from: &Path, to: &Path) -> Result<(), String> {
    let err = |what: &str, p: &Path, e: std::io::Error| format!("{what} {}: {e}", p.display());
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir).map_err(|e| err("creating", dir, e))?;
    }
    let mut src = File::open(from).map_err(|e| err("opening", from, e))?;
    let mut dst = File::create(to).map_err(|e| err("creating", to, e))?;
    let mut buf = vec![0u8; 8 << 20];
    loop {
        let n = src.read(&mut buf).map_err(|e| err("reading", from, e))?;
        if n == 0 {
            break;
        }
        dst.write_all(&buf[..n])
            .map_err(|e| err("writing", to, e))?;
    }
    dst.sync_all().map_err(|e| err("syncing", to, e))?;
    Ok(())
}

/// Private copies of one dataset.
#[derive(Debug, Clone)]
pub struct Copies {
    /// The dataset path inside each copy, in rotation order.
    pub paths: Vec<PathBuf>,
    /// Their directory, which keeps the index of the copy read last.
    pub dir: PathBuf,
}

const LAST_COPY: &str = "last-copy-read";

impl Copies {
    /// The next copy to read, among the first `n`, and its index: never the one read
    /// last, in this run or the previous one.
    ///
    /// # Errors
    ///
    /// The record of the last copy cannot be written.
    pub fn next(&self, n: u32) -> Result<(u32, PathBuf), String> {
        let n = n.clamp(1, u32::try_from(self.paths.len()).unwrap_or(u32::MAX));
        let record = self.dir.join(LAST_COPY);
        let last: Option<u32> = std::fs::read_to_string(&record)
            .ok()
            .and_then(|t| t.trim().parse().ok());
        let k = last.map_or(0, |l| (l + 1) % n);
        std::fs::write(&record, format!("{k}\n"))
            .map_err(|e| format!("writing {}: {e}", record.display()))?;
        Ok((k, self.paths[k as usize].clone()))
    }
}

/// SHA-256 of what [`prepare_copies`] compares: the relative path, size and modification
/// time of every file under `source`. Rows record it, so that two rows can be seen to
/// have read the same files without hashing gigabytes.
///
/// # Errors
///
/// As [`files_under`].
pub fn listing_sha256(source: &Path) -> Result<String, String> {
    Ok(crate::datasets::hex(
        &<sha2::Sha256 as sha2::Digest>::digest(listing(source)?.as_bytes()),
    ))
}

/// Makes (or reuses) `n` copies of `source` under `dir`: `dir/0/<name>`, `dir/1/<name>`…
/// A copy is reused when its marker lists the same files, sizes and times as the source.
///
/// # Errors
///
/// Reading the source or writing a copy fails.
pub fn prepare_copies(
    source: &Path,
    dir: &Path,
    n: u32,
    log: &mut dyn FnMut(&str),
) -> Result<Copies, String> {
    let name = source
        .file_name()
        .ok_or_else(|| format!("{} has no file name", source.display()))?;
    let want = listing(source)?;
    // Paths relative to the parent keep the dataset's own name: `graph/targets.u32`.
    let base = source.parent().unwrap_or(source);
    let mut paths = Vec::new();
    for k in 0..n {
        let root = dir.join(k.to_string());
        let marker = root.join(MARKER);
        let target = root.join(name);
        if std::fs::read_to_string(&marker).ok().as_deref() != Some(want.as_str()) {
            log(&format!(
                "copying {} to {} (cold-cache copy {k})",
                source.display(),
                target.display()
            ));
            if root.exists() {
                std::fs::remove_dir_all(&root)
                    .map_err(|e| format!("removing {}: {e}", root.display()))?;
            }
            for f in files_under(source)? {
                let rel = f.strip_prefix(base).unwrap_or(&f);
                copy_file(&f, &root.join(rel))?;
            }
            std::fs::write(&marker, &want)
                .map_err(|e| format!("writing {}: {e}", marker.display()))?;
        }
        evict(&target)?;
        paths.push(target);
    }
    Ok(Copies {
        paths,
        dir: dir.to_path_buf(),
    })
}

/// Syncs and drops every file under `path` from the page cache.
///
/// # Errors
///
/// A file cannot be opened, synced or advised.
pub fn evict(path: &Path) -> Result<(), String> {
    for f in files_under(path)? {
        let file = File::open(&f).map_err(|e| format!("opening {}: {e}", f.display()))?;
        file.sync_all()
            .map_err(|e| format!("syncing {}: {e}", f.display()))?;
        sys::drop_cache(&file).map_err(|e| format!("evicting {}: {e}", f.display()))?;
    }
    Ok(())
}

/// Parses `fincore --bytes --noheadings --raw --output RES,SIZE` output: resident and
/// total bytes summed over its lines.
///
/// # Errors
///
/// A line without two numbers.
pub fn parse_fincore(text: &str) -> Result<(u64, u64), String> {
    let mut resident = 0u64;
    let mut total = 0u64;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let mut f = line.split_whitespace().map(str::parse::<u64>);
        match (f.next(), f.next()) {
            (Some(Ok(r)), Some(Ok(s))) => {
                resident = resident.saturating_add(r);
                total = total.saturating_add(s);
            }
            _ => return Err(format!("unexpected fincore output: {line:?}")),
        }
    }
    Ok((resident, total))
}

/// Bytes of the files under `path` in the page cache, and their total size, as
/// `mincore(2)` reports it through `fincore`.
///
/// # Errors
///
/// `fincore` is missing or fails.
pub fn resident(path: &Path) -> Result<(u64, u64), String> {
    let files = files_under(path)?;
    let out = Command::new("fincore")
        .args(["--bytes", "--noheadings", "--raw", "--output", "RES,SIZE"])
        .args(&files)
        .output()
        .map_err(|e| format!("running fincore (util-linux) for the cold-cache check: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "fincore failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    parse_fincore(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datasets::test_support::TempDir;

    #[test]
    fn fincore_output() {
        let text = "31985664 31983704\n277450752 277449512\n";
        assert_eq!(
            parse_fincore(text),
            Ok((31_985_664 + 277_450_752, 31_983_704 + 277_449_512))
        );
        assert_eq!(parse_fincore(""), Ok((0, 0)));
        assert!(parse_fincore("12 x\n").is_err());
    }

    #[test]
    fn rotation_never_repeats_a_copy_in_a_row_across_runs() {
        let dir = TempDir::new("rotation");
        let copies = |n: usize| Copies {
            paths: (0..n).map(|k| dir.path().join(k.to_string())).collect(),
            dir: dir.path().to_path_buf(),
        };
        let run = copies(2);
        let picks: Vec<u32> = (0..3).map(|_| run.next(2).expect("next").0).collect();
        assert_eq!(picks, [0, 1, 0]);
        // A new run (a new `Copies`) goes on from the copy read last.
        let again = copies(2);
        assert_eq!(again.next(2).expect("next").0, 1);
        // Fewer copies asked for than exist: still never the same twice in a row.
        let three = copies(3);
        assert_eq!(three.next(3).expect("next").0, 2);
        assert_eq!(three.next(3).expect("next").0, 0);
    }

    #[cfg(target_os = "linux")] // copies are evicted after copying: posix_fadvise
    #[test]
    fn copies_are_made_once_and_remade_when_the_source_changes() {
        let dir = TempDir::new("cold-copies");
        let src = dir.path().join("graph");
        std::fs::create_dir_all(src.join("sub")).expect("mkdir");
        std::fs::write(src.join("targets.u32"), vec![7u8; 100_000]).expect("write");
        std::fs::write(src.join("sub").join("meta.json"), "{}").expect("write");
        let mut log = Vec::new();
        let copies_dir = dir.path().join("copies");
        let c =
            prepare_copies(&src, &copies_dir, 2, &mut |l| log.push(l.to_string())).expect("copies");
        assert_eq!(c.paths.len(), 2);
        assert_eq!(log.len(), 2);
        for p in &c.paths {
            assert_eq!(
                std::fs::read(p.join("targets.u32")).expect("read").len(),
                100_000
            );
            assert!(p.join("sub").join("meta.json").is_file());
            assert_eq!(size_of(p).expect("size"), 100_002);
        }
        prepare_copies(&src, &copies_dir, 2, &mut |l| log.push(l.to_string())).expect("again");
        assert_eq!(log.len(), 2, "unchanged source: copies reused");
        std::fs::write(src.join("targets.u32"), vec![8u8; 50]).expect("change");
        prepare_copies(&src, &copies_dir, 2, &mut |l| log.push(l.to_string())).expect("remade");
        assert_eq!(log.len(), 4);
        assert_eq!(
            std::fs::read(c.paths[1].join("targets.u32")).expect("read"),
            vec![8u8; 50]
        );
    }
}
