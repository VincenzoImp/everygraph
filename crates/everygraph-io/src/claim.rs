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
//! - A map reads the file's length and registers its range in one step, under the
//!   claim list's lock ([`Claim::map`]), and waits for a write in flight over that
//!   range to end. So no length change can fall between the two.
//!
//! The list's lock is held for bookkeeping only, never during a write: a write in
//! flight is recorded in its file's entry, so sinks on different files never wait for
//! each other.
//!
//! Refusals are [`io::ErrorKind::ResourceBusy`]. Writes through `std::fs` directly are
//! not seen; the `MmapSource` docs say so. On platforms without a device and inode
//! (Windows), the guard does nothing; Windows itself refuses to truncate a mapped file,
//! but not to write it.

use std::fs::File;
use std::io;
use std::ops::Range;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};

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

/// Device and inode, the ranges mapped, the ranges being written, how many sinks hold
/// the file, and whether one sink holds it alone.
#[derive(Debug)]
struct Entry {
    id: (u64, u64),
    maps: Vec<Range<u64>>,
    in_flight: Vec<Range<u64>>,
    writers: usize,
    alone: bool,
}

static CLAIMS: Mutex<Vec<Entry>> = Mutex::new(Vec::new());
/// Signalled when a write in flight ends.
static WRITE_ENDED: Condvar = Condvar::new();

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

/// Whether the bytes `touched` fall inside `mapped`; touching nothing never does.
const fn overlaps(mapped: &Range<u64>, touched: &Range<u64>) -> bool {
    touched.start < touched.end && mapped.start < touched.end && touched.start < mapped.end
}

// The list is updated in whole statements; a panic elsewhere cannot leave it torn.
fn claims() -> MutexGuard<'static, Vec<Entry>> {
    CLAIMS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The entry of `key`, created empty if there is none.
fn entry(claims: &mut Vec<Entry>, key: (u64, u64)) -> &mut Entry {
    let index = match claims.iter().position(|e| e.id == key) {
        Some(i) => i,
        None => {
            claims.push(Entry {
                id: key,
                maps: Vec::new(),
                in_flight: Vec::new(),
                writers: 0,
                alone: false,
            });
            claims.len() - 1
        }
    };
    &mut claims[index]
}

/// Removes the entry of `key` once nothing holds the file.
fn release_if_unused(claims: &mut Vec<Entry>, key: (u64, u64)) {
    if let Some(i) = claims.iter().position(|e| e.id == key) {
        let e = &claims[i];
        if e.maps.is_empty() && e.writers == 0 && e.in_flight.is_empty() {
            claims.swap_remove(i);
        }
    }
}

/// A write in flight over `range`; dropping it (after the write, or on a panic in it)
/// takes it off the list and wakes the maps that wait for it.
struct InFlight {
    key: (u64, u64),
    range: Range<u64>,
}

impl Drop for InFlight {
    fn drop(&mut self) {
        let mut claims = claims();
        if let Some(e) = claims.iter_mut().find(|e| e.id == self.key) {
            if let Some(i) = e.in_flight.iter().position(|r| *r == self.range) {
                e.in_flight.swap_remove(i);
            }
        }
        release_if_unused(&mut claims, self.key);
        WRITE_ENDED.notify_all();
    }
}

impl Claim {
    /// Holds `file` for writing ([`Use::Write`] or [`Use::WriteAlone`]), or refuses if
    /// that conflicts with what holds it.
    pub(crate) fn take(file: &File, used_for: Use) -> io::Result<Self> {
        debug_assert!(
            !matches!(used_for, Use::Map(_)),
            "maps go through Claim::map"
        );
        let id = file_id(file)?;
        let Some(key) = id else {
            return Ok(Self { id, used_for });
        };
        let mut claims = claims();
        let entry = entry(&mut claims, key);
        let refusal = match &used_for {
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
            release_if_unused(&mut claims, key);
            return Err(busy(message));
        }
        if used_for == Use::WriteAlone {
            entry.alone = true;
        }
        entry.writers += 1;
        Ok(Self { id, used_for })
    }

    /// Holds `file` for mapping the range `choose` picks from the file's length; returns
    /// the claim, the range and the length. The length is read, and the range
    /// registered, with no length change of this process in between.
    pub(crate) fn map(
        file: &File,
        choose: impl Fn(u64) -> io::Result<Range<u64>>,
    ) -> io::Result<(Self, Range<u64>, u64)> {
        let id = file_id(file)?;
        let Some(key) = id else {
            let len = file.metadata()?.len();
            let range = choose(len)?;
            let used_for = Use::Map(range.clone());
            return Ok((Self { id, used_for }, range, len));
        };
        let mut claims = claims();
        loop {
            let entry = entry(&mut claims, key);
            if entry.writers > 0 && !entry.alone {
                release_if_unused(&mut claims, key);
                return Err(busy(
                    "refused to map the file: a sink of this process is writing it",
                ));
            }
            let picked = file
                .metadata()
                .and_then(|meta| Ok((choose(meta.len())?, meta.len())));
            let (range, len) = match picked {
                Ok(picked) => picked,
                Err(e) => {
                    release_if_unused(&mut claims, key);
                    return Err(e);
                }
            };
            if entry.in_flight.iter().any(|w| overlaps(&range, w)) {
                claims = WRITE_ENDED
                    .wait(claims)
                    .unwrap_or_else(PoisonError::into_inner);
                continue;
            }
            entry.maps.push(range.clone());
            let used_for = Use::Map(range.clone());
            return Ok((Self { id, used_for }, range, len));
        }
    }

    /// Runs `write`, which changes the bytes `touched` of the file, unless a live map of
    /// this process covers one of them. While it runs, no map of those bytes can be
    /// taken.
    pub(crate) fn guard<T>(
        &self,
        touched: Range<u64>,
        write: impl FnOnce() -> io::Result<T>,
    ) -> io::Result<T> {
        let Some(key) = self.id else {
            return write();
        };
        let _in_flight = {
            let mut claims = claims();
            let entry = entry(&mut claims, key);
            if entry.maps.iter().any(|m| overlaps(m, &touched)) {
                return Err(busy(
                    "refused to write the file: a live MmapSource of this process maps the \
                     bytes this write or length change would reach",
                ));
            }
            entry.in_flight.push(touched.clone());
            InFlight {
                key,
                range: touched,
            }
        };
        write()
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        let Some(key) = self.id else { return };
        let mut claims = claims();
        if let Some(entry) = claims.iter_mut().find(|e| e.id == key) {
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
        }
        release_if_unused(&mut claims, key);
    }
}
