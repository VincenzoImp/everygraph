// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The `.events` section of an adjacency (`docs/spec/eg-adjacency.md` § 3.1): where the
//! events of each entry start, for a graph that keeps repeated pairs as events.

use std::ops::Range;
use std::sync::OnceLock;

use everygraph_core::le::{u32_at, u64_at};

use super::FormatError;
use super::ef::EliasFano;
use super::index::AdjacencyIndex;
use super::stream::{EVENTS_HEADER, EVENTS_MAGIC, EVENTS_VERSION, events_section_bytes};

/// The event offsets of an adjacency, read from its `.events` section.
///
/// [`parse`](Self::parse) checks the header and the length; the rest of the section is
/// checked at its first use, before any value read from it is returned
/// (`docs/spec/eg-adjacency.md` § 5), so that a kernel that never asks pays nothing.
#[derive(Debug)]
pub struct AdjacencyEvents<'a> {
    bytes: &'a [u8],
    entries: u64,
    events: u64,
    sequence: OnceLock<Result<EliasFano, FormatError>>,
}

impl<'a> AdjacencyEvents<'a> {
    /// Reads the header of `bytes`, the `.events` section of the adjacency of `index`.
    ///
    /// # Errors
    ///
    /// [`FormatError::Invalid`] when the index does not announce the section (flag bit
    /// 1), when the entries differ from the index's, or when the section has no more
    /// events than entries; [`FormatError::Truncated`], [`FormatError::BadMagic`],
    /// [`FormatError::UnsupportedVersion`], [`FormatError::UnsupportedFlags`] and
    /// [`FormatError::Length`] as for an index.
    pub fn parse(bytes: &'a [u8], index: &AdjacencyIndex) -> Result<Self, FormatError> {
        let header = bytes.get(..EVENTS_HEADER).ok_or(FormatError::Truncated)?;
        if header[..8] != EVENTS_MAGIC {
            return Err(FormatError::BadMagic);
        }
        let word = |at| u32_at(header, at);
        let long = |at| u64_at(header, at);
        if word(8) != EVENTS_VERSION {
            return Err(FormatError::UnsupportedVersion(word(8)));
        }
        if word(12) != 0 {
            return Err(FormatError::UnsupportedFlags(word(12)));
        }
        let (entries, events) = (long(16), long(24));
        if !index.has_events() {
            return Err(FormatError::Invalid(
                "a .events section for an index without flag bit 1",
            ));
        }
        if entries != index.entries() {
            return Err(FormatError::Invalid(
                ".events entries differ from the index",
            ));
        }
        if events <= entries {
            return Err(FormatError::Invalid(
                "a .events section with no repeated pair",
            ));
        }
        let expected = events_section_bytes(entries, events)
            .ok_or(FormatError::Invalid(".events section too large"))?;
        let actual = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if expected != actual {
            return Err(FormatError::Length { expected, actual });
        }
        Ok(Self {
            bytes,
            entries,
            events,
            sequence: OnceLock::new(),
        })
    }

    /// Entries of the adjacency.
    #[must_use]
    pub fn entries(&self) -> u64 {
        self.entries
    }

    /// Stored events of the adjacency: the rows of a per-event column.
    #[must_use]
    pub fn events(&self) -> u64 {
        self.events
    }

    /// Checks the whole section: the structure of the sequence, and that it starts at
    /// 0, ends at [`events`](Self::events) and strictly increases (every entry has an
    /// event). Done once; every accessor calls it first.
    ///
    /// # Errors
    ///
    /// [`FormatError::Invalid`] naming the first problem.
    pub fn check(&self) -> Result<(), FormatError> {
        self.sequence().map(|_| ())
    }

    fn sequence(&self) -> Result<&EliasFano, FormatError> {
        self.sequence
            .get_or_init(|| {
                let len = self.entries + 1;
                let body = &self.bytes[EVENTS_HEADER..];
                let ef = EliasFano::read(body, len, self.events).map_err(FormatError::Invalid)?;
                let mut previous = None;
                for x in ef.iter_from(0) {
                    if previous.is_some_and(|p| x <= p) || (previous.is_none() && x != 0) {
                        return Err(FormatError::Invalid(
                            "event offsets do not start at 0 and strictly increase",
                        ));
                    }
                    previous = Some(x);
                }
                if previous != Some(self.events) {
                    return Err(FormatError::Invalid("event offsets do not end at events"));
                }
                Ok(ef)
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    /// The event offsets of entry `entry`: where its events are in a per-event column.
    ///
    /// # Errors
    ///
    /// As [`check`](Self::check).
    ///
    /// # Panics
    ///
    /// If `entry` is not below [`entries`](Self::entries).
    pub fn event_range(&self, entry: u64) -> Result<Range<u64>, FormatError> {
        assert!(entry < self.entries, "entry {entry} of {}", self.entries);
        let at = usize::try_from(entry).expect("checked against an in-memory sequence");
        let (start, end) = self.sequence()?.get_pair(at);
        Ok(start..end)
    }

    /// The number of stored events of every entry, in entry order.
    ///
    /// # Errors
    ///
    /// As [`check`](Self::check).
    pub fn multiplicities(&self) -> Result<impl Iterator<Item = u64> + '_, FormatError> {
        let mut offsets = self.sequence()?.iter_from(0);
        let mut previous = offsets.next().unwrap_or(0);
        Ok(offsets.map(move |x| {
            let m = x - previous;
            previous = x;
            m
        }))
    }
}
