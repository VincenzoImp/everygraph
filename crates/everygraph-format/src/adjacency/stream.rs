// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The encoder for builds that do not hold the graph: the bytes [`encode`](super::encode)
//! writes, produced list by list in node order. The records go wherever the caller
//! writes them ([`append_record`]); the index keeps 12 bytes per node until the end
//! ([`IndexBuilder`]); the `.events` section is made from each entry's number of events
//! ([`encode_events`]).

use std::cell::Cell;

use everygraph_core::{Memory, MemoryError, MemoryKind, SegVec};

use super::ef::EliasFano;
use super::encode::block_starts;
use super::index::AdjacencyIndex;
use super::{EncodeError, svb};

const PURPOSE: &str = "adjacency index";

/// Magic of the `.events` section (`docs/spec/eg-adjacency.md` § 3.1).
pub(crate) const EVENTS_MAGIC: [u8; 8] = *b"EGADJEVT";
/// Version of the `.events` section.
pub(crate) const EVENTS_VERSION: u32 = 1;
/// Bytes of the header of the `.events` section.
pub(crate) const EVENTS_HEADER: usize = 32;

/// The most bytes the record of a list of `degree` entries takes: `⌈d/4⌉ + 4d`.
#[must_use]
pub fn record_bound(degree: u64) -> u64 {
    degree.div_ceil(4).saturating_add(degree.saturating_mul(4))
}

/// Appends the record of the list of node `v` to `out` (`docs/spec/eg-adjacency.md`
/// § 1): the bytes [`encode`](super::encode) writes for it. Returns the list's length;
/// an empty list appends nothing. `out` grows as a `Vec` does, so a caller that keeps
/// within a budget gives it the capacity of [`record_bound`] first.
///
/// # Errors
///
/// [`EncodeError::NotIncreasing`] if the list is not strictly increasing (`out` is then
/// unchanged), and [`EncodeError::Offsets`] for a list of 2^32 entries or more.
pub fn append_record<I>(v: u32, list: I, out: &mut Vec<u8>) -> Result<u32, EncodeError>
where
    I: IntoIterator<Item = u32>,
    I::IntoIter: ExactSizeIterator,
{
    let list = list.into_iter();
    let degree = u32::try_from(list.len())
        .map_err(|_| EncodeError::Offsets("a list has 2^32 entries or more"))?;
    svb::append_record(v, list, out).map_err(|_| EncodeError::NotIncreasing { node: v })?;
    Ok(degree)
}

/// The index of an adjacency whose records are written elsewhere, list by list, in node
/// order: it keeps each node's degree and the end of its record (12 bytes per node,
/// reserved when it is made) and writes the index bytes [`encode`](super::encode)
/// writes for the same lists.
#[derive(Debug)]
pub struct IndexBuilder {
    memory: Memory,
    nodes: u32,
    degrees: SegVec<u32>,
    /// The byte offset, in the data part, of the end of each node's record.
    record_ends: SegVec<u64>,
    entries: u64,
}

impl IndexBuilder {
    /// A builder for `nodes` lists.
    ///
    /// # Errors
    ///
    /// The degrees and record ends do not fit the budget or cannot be allocated.
    pub fn new(memory: &Memory, nodes: u32) -> Result<Self, MemoryError> {
        let n = usize::try_from(nodes).map_err(|_| MemoryError::AllocationFailed {
            purpose: PURPOSE,
            bytes: u64::MAX,
        })?;
        let mut degrees = SegVec::new(memory, PURPOSE);
        degrees.try_reserve(n)?;
        let mut record_ends = SegVec::new(memory, PURPOSE);
        record_ends.try_reserve(n)?;
        Ok(Self {
            memory: memory.clone(),
            nodes,
            degrees,
            record_ends,
            entries: 0,
        })
    }

    /// Records the next node's list: its length and the length of its record.
    ///
    /// # Errors
    ///
    /// [`EncodeError::Offsets`] for a node beyond the count given to
    /// [`new`](Self::new), or a record whose length cannot be that of its list
    /// (`docs/spec/eg-adjacency.md` § 5).
    pub fn push(&mut self, degree: u32, record_bytes: u64) -> Result<(), EncodeError> {
        let d = u64::from(degree);
        let possible = if d == 0 {
            record_bytes == 0
        } else {
            (d.div_ceil(4) + d..=record_bound(d)).contains(&record_bytes)
        };
        if !possible {
            return Err(EncodeError::Offsets(
                "a record length does not fit its degree",
            ));
        }
        if self.degrees.len() >= usize_of(self.nodes) {
            return Err(EncodeError::Offsets("more lists than nodes"));
        }
        let end = self
            .data_bytes()
            .checked_add(record_bytes)
            .ok_or(EncodeError::Offsets("the data part exceeds 2^64 bytes"))?;
        // Within the capacity reserved by `new`: these pushes do not allocate.
        self.degrees.try_push(degree).map_err(out_of_memory)?;
        self.record_ends.try_push(end).map_err(out_of_memory)?;
        self.entries += d;
        Ok(())
    }

    /// Bytes of the data part so far.
    #[must_use]
    pub fn data_bytes(&self) -> u64 {
        self.record_ends
            .len()
            .checked_sub(1)
            .map_or(0, |last| self.record_ends[last])
    }

    /// Entries so far.
    #[must_use]
    pub fn entries(&self) -> u64 {
        self.entries
    }

    /// The index bytes, with flag bit 1 set when `events` (the adjacency has a
    /// `.events` section, [`encode_events`]). The index is built in memory, reserved
    /// against the budget first.
    ///
    /// # Errors
    ///
    /// [`EncodeError::Offsets`] if fewer lists than nodes were pushed, and
    /// [`EncodeError::OutOfMemory`] if the index does not fit the budget.
    pub fn finish(self, events: bool) -> Result<Vec<u8>, EncodeError> {
        let n = self.degrees.len();
        if n != usize_of(self.nodes) {
            return Err(EncodeError::Offsets("fewer lists than nodes"));
        }
        let (entries, data_bytes) = (self.entries, self.data_bytes());
        let len = widen(n) + 1;
        // The two sequences in memory and in the output, and the block directory.
        let bytes = EliasFano::stored_bytes(len, data_bytes)
            .zip(EliasFano::stored_bytes(len, entries))
            .and_then(|(a, b)| a.checked_add(b)?.checked_mul(2)?.checked_add(9 * len + 64))
            .ok_or(EncodeError::OutOfMemory { bytes: u64::MAX })?;
        let _reservation = self
            .memory
            .reserve(PURPOSE, MemoryKind::Anonymous, bytes)
            .map_err(|e| out_of_memory(e.into()))?;
        let oom = EncodeError::OutOfMemory { bytes };
        let ends = std::iter::once(0).chain(self.record_ends.iter().copied());
        let byte_offsets = EliasFano::from_values(n + 1, data_bytes, ends).ok_or(oom.clone())?;
        let mut sum = 0;
        let starts = std::iter::once(0).chain(self.degrees.iter().map(|&d| {
            sum += u64::from(d);
            sum
        }));
        let entry_offsets = EliasFano::from_values(n + 1, entries, starts).ok_or(oom)?;
        let blocks = block_starts(n, self.degrees.iter().map(|&d| u64::from(d)));
        let index = AdjacencyIndex::from_parts(
            events,
            self.nodes,
            entries,
            data_bytes,
            blocks,
            byte_offsets,
            entry_offsets,
        );
        Ok(index.write())
    }
}

/// Bytes of the `.events` section of an adjacency with `entries` entries and `events`
/// stored events, or `None` if they cannot be counted in a `u64`.
#[must_use]
pub fn events_section_bytes(entries: u64, events: u64) -> Option<u64> {
    EliasFano::stored_bytes(entries.checked_add(1)?, events)?.checked_add(widen(EVENTS_HEADER))
}

/// The `.events` section (`docs/spec/eg-adjacency.md` § 3.1) of an adjacency with
/// `entries` entries, from the number of stored events of each entry, in entry order;
/// they add up to `events`. Write it only when some entry has more than one event: a
/// section in which every entry has one is refused here, as readers refuse it.
///
/// The section is built in memory; [`events_section_bytes`] gives its size, twice of
/// which is held at the end.
///
/// # Errors
///
/// [`EncodeError::Offsets`] if `events` is not above `entries`, or `multiplicities` does
/// not yield `entries` values of at least 1 that add up to `events`;
/// [`EncodeError::OutOfMemory`] if the section cannot be allocated.
pub fn encode_events(
    entries: u64,
    events: u64,
    multiplicities: impl IntoIterator<Item = u64>,
) -> Result<Vec<u8>, EncodeError> {
    if events <= entries {
        return Err(EncodeError::Offsets(
            "a .events section needs more events than entries",
        ));
    }
    let bytes = events_section_bytes(entries, events)
        .ok_or(EncodeError::OutOfMemory { bytes: u64::MAX })?;
    let oom = EncodeError::OutOfMemory { bytes };
    let len = usize::try_from(entries)
        .ok()
        .and_then(|e| e.checked_add(1))
        .ok_or(oom.clone())?;
    let (count, sum, valid) = (Cell::new(0_u64), Cell::new(0_u64), Cell::new(true));
    let mut multiplicities = multiplicities.into_iter();
    let offsets = multiplicities.by_ref().map(|m| {
        count.set(count.get() + 1);
        let next = sum.get().checked_add(m).filter(|&s| m >= 1 && s <= events);
        valid.set(valid.get() && next.is_some());
        sum.set(next.unwrap_or(events));
        sum.get()
    });
    // Padded with `events`, so that a short iterator still makes a whole sequence
    // (and is refused below).
    let values = std::iter::once(0)
        .chain(offsets)
        .chain(std::iter::repeat(events));
    let sequence = EliasFano::from_values(len, events, values).ok_or(oom.clone())?;
    let more = multiplicities.next().is_some();
    if more || !valid.get() || count.get() != entries || sum.get() != events {
        return Err(EncodeError::Offsets(
            "the multiplicities are not `entries` values of at least 1 adding up to `events`",
        ));
    }
    let mut out = Vec::new();
    let capacity = usize::try_from(bytes).map_err(|_| oom.clone())?;
    out.try_reserve_exact(capacity).map_err(|_| oom)?;
    out.extend_from_slice(&EVENTS_MAGIC);
    out.extend_from_slice(&EVENTS_VERSION.to_le_bytes());
    out.extend_from_slice(&0_u32.to_le_bytes());
    out.extend_from_slice(&entries.to_le_bytes());
    out.extend_from_slice(&events.to_le_bytes());
    sequence.write(&mut out);
    Ok(out)
}

fn out_of_memory(e: MemoryError) -> EncodeError {
    EncodeError::OutOfMemory {
        bytes: match e {
            MemoryError::Budget(refused) => refused.needed,
            MemoryError::AllocationFailed { bytes, .. } => bytes,
            _ => u64::MAX,
        },
    }
}

fn usize_of(n: u32) -> usize {
    usize::try_from(n).expect("usize has at least 32 bits")
}

fn widen(x: usize) -> u64 {
    u64::try_from(x).expect("usize has at most 64 bits")
}
