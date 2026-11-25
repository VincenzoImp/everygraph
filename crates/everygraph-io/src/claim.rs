// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The in-process guard between memory maps and sinks (`design/DECISIONS.md`, "Memory-mapped
//! files are read through a safe constructor").
//!
//! A file that a live [`MmapSource`](crate::MmapSource) maps must not be written or
//! truncated: the map is handed out as a `&[u8]`, and a write would change it under
//! that borrow (undefined behaviour) or remove its pages (`SIGBUS`). Writers in other
//! processes cannot be stopped; writers in this one go through the sinks of this crate,
//! and they register here. A file, identified by device and inode, is either mapped
//! (any number of times) or open in a sink (any number of times), never both: whichever
//! comes second is refused with [`io::ErrorKind::ResourceBusy`].
//!
//! Writes through `std::fs` directly are not seen; the `MmapSource` docs say so. On
//! platforms without a device and inode (Windows), the guard does nothing; Windows
//! itself refuses to truncate a mapped file, but not to write it.

use std::fs::File;
use std::io;
use std::sync::{Mutex, PoisonError};

/// What a claim holds the file for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Use {
    Map,
    Write,
}

/// Device and inode, and how many maps and sinks hold the file.
#[derive(Debug)]
struct Entry {
    id: (u64, u64),
    maps: usize,
    writers: usize,
}

static CLAIMS: Mutex<Vec<Entry>> = Mutex::new(Vec::new());

/// A file held for one [`Use`]; dropping it lets the other use in again.
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

impl Claim {
    /// Holds `file` for `used_for`, or refuses if it is held for the other use.
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
                    maps: 0,
                    writers: 0,
                });
                claims.len() - 1
            }
        };
        let entry = &mut claims[index];
        let (mine, other) = match used_for {
            Use::Map => (&mut entry.maps, entry.writers),
            Use::Write => (&mut entry.writers, entry.maps),
        };
        // A new entry has neither use, so a refusal always leaves an entry that is held.
        if other > 0 {
            let message = match used_for {
                Use::Map => "refused to map the file: a sink of this process is writing it",
                Use::Write => {
                    "refused to write the file: a live MmapSource of this process maps it"
                }
            };
            return Err(io::Error::new(io::ErrorKind::ResourceBusy, message));
        }
        *mine += 1;
        Ok(Self { id, used_for })
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        let Some(key) = self.id else { return };
        let mut claims = CLAIMS.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(i) = claims.iter().position(|e| e.id == key) {
            let entry = &mut claims[i];
            match self.used_for {
                Use::Map => entry.maps -= 1,
                Use::Write => entry.writers -= 1,
            }
            if entry.maps == 0 && entry.writers == 0 {
                claims.swap_remove(i);
            }
        }
    }
}
