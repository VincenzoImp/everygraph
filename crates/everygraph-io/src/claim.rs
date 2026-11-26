// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The in-process guard between memory maps and sinks (`design/DECISIONS.md`, "Memory-mapped
//! files are read through a safe constructor", and "Range maps of a graph file").
//!
//! The bytes that a live [`MmapSource`](crate::MmapSource) maps must not be written,
//! nor the file truncated into them: the map is handed out as a `&[u8]`, and a write
//! would change it under that borrow (undefined behaviour) or remove its pages
//! (`SIGBUS`). Writers in other processes cannot be stopped; writers in this one go
//! through the sinks of this crate, and they register here, by device and inode.
//!
//! - [`AppendSink`](crate::AppendSink) and [`BlockWriter`](crate::BlockWriter) write
//!   anywhere: a file they hold cannot be mapped, and they refuse a mapped file.
//! - A [`FileSink`](crate::FileSink) holds a file alone (one writer at a time updates a
//!   container, `docs/spec/eg-container.md` § 11): no other sink of this process may
//!   open it while it does, nor it a file another sink holds. It may open a mapped file,
//!   and the file may be mapped while it is open: each of its writes and length changes
//!   is checked against the mapped ranges ([`Claim::guard`]) and refused if it would
//!   reach one.
//!
//! Refusals are [`io::ErrorKind::ResourceBusy`]. Writes through `std::fs` directly are
//! not seen; the `MmapSource` docs say so. On platforms without a device and inode
//! (Windows), the guard does nothing; Windows itself refuses to truncate a mapped file,
//! but not to write it.

use std::fs::File;
use std::io;
use std::ops::Range;
use std::sync::{Mutex, PoisonError};

/// What a claim holds the file for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Use {
    /// Mapping these bytes.
    Map(Range<u64>),
    /// Writing anywhere.
    Write,
    /// Writing, as the only sink of this process on the file, each write checked
    /// against the maps (`FileSink`).
    WriteAlone,
}

/// Device and inode, the ranges mapped, how many sinks hold the file, and whether one
/// sink holds it alone.
#[derive(Debug)]
struct Entry {
    id: (u64, u64),
    maps: Vec<Range<u64>>,
    writers: usize,
    alone: bool,
}

static CLAIMS: Mutex<Vec<Entry>> = Mutex::new(Vec::new());

/// A file held for one [`Use`]; dropping it lets the other uses in again.
#[derive(Debug)]
pub(crate) struct Claim {
    id: Option<(u64, u64)>,
    used_for: Use,
}

fn file_id(file: &File) -> io::Result<Option<(u64, u64)>> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = file.metadata()?;
        Ok(Some((meta.dev(), meta.ino())))
    }
    #[cfg(not(unix))]
    {
        let _ = file;
        Ok(None)
    }
}

fn busy(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::ResourceBusy, message)
}

const fn overlaps(a: &Range<u64>, b: &Range<u64>) -> bool {
    a.start < b.end && b.start < a.end
}

impl Claim {
    /// Holds `file` for `used_for`, or refuses if that conflicts with what holds it.
    pub(crate) fn take(file: &File, used_for: Use) -> io::Result<Self> {
        let id = file_id(file)?;
        let Some(key) = id else {
            return Ok(Self { id, used_for });
        };
        // The list is updated in whole statements; a panic elsewhere cannot leave it torn.
        let mut claims = CLAIMS.lock().unwrap_or_else(PoisonError::into_inner);
        let index = match claims.iter().position(|e| e.id == key) {
            Some(i) => i,
            None => {
                claims.push(Entry {
                    id: key,
                    maps: Vec::new(),
                    writers: 0,
                    alone: false,
                });
                claims.len() - 1
            }
        };
        let entry = &mut claims[index];
        // A new entry has no use, so a refusal always leaves an entry that is held.
        let refusal = match &used_for {
            Use::Map(_) if entry.writers > 0 && !entry.alone => {
                Some("refused to map the file: a sink of this process is writing it")
            }
            Use::Write if !entry.maps.is_empty() => {
                Some("refused to write the file: a live MmapSource of this process maps it")
            }
            Use::Write if entry.alone => {
                Some("refused to write the file: a FileSink of this process is updating it")
            }
            Use::WriteAlone if entry.writers > 0 => {
                Some("refused to update the file: another sink of this process is writing it")
            }
            _ => None,
        };
        if let Some(message) = refusal {
            return Err(busy(message));
        }
        match &used_for {
            Use::Map(range) => entry.maps.push(range.clone()),
            Use::Write => entry.writers += 1,
            Use::WriteAlone => {
                entry.writers += 1;
                entry.alone = true;
            }
        }
        Ok(Self { id, used_for })
    }

    /// Runs `write`, which changes the bytes `touched` of the file, unless a live map of
    /// this process covers one of them. The claim list stays locked while it runs, so
    /// that no map of those bytes can be taken in the middle of the write.
    pub(crate) fn guard<T>(
        &self,
        touched: Range<u64>,
        write: impl FnOnce() -> io::Result<T>,
    ) -> io::Result<T> {
        let Some(key) = self.id else {
            return write();
        };
        let claims = CLAIMS.lock().unwrap_or_else(PoisonError::into_inner);
        let mapped = claims
            .iter()
            .find(|e| e.id == key)
            .is_some_and(|e| e.maps.iter().any(|m| overlaps(m, &touched)));
        if mapped {
            return Err(busy(
                "refused to write the file: a live MmapSource of this process maps the bytes \
                 this write or length change would reach",
            ));
        }
        write()
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        let Some(key) = self.id else { return };
        let mut claims = CLAIMS.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(i) = claims.iter().position(|e| e.id == key) {
            let entry = &mut claims[i];
            match &self.used_for {
                Use::Map(range) => {
                    if let Some(m) = entry.maps.iter().position(|m| m == range) {
                        entry.maps.swap_remove(m);
                    }
                }
                Use::Write => entry.writers -= 1,
                Use::WriteAlone => {
                    entry.writers -= 1;
                    entry.alone = false;
                }
            }
            if entry.maps.is_empty() && entry.writers == 0 {
                claims.swap_remove(i);
            }
        }
    }
}
