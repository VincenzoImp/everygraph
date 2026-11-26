// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The in-memory set of distinct keys of the dedupe: open addressing over the 128-bit
//! hash already computed for each key, the keys themselves in one arena, and for each
//! key the smallest position it was seen at.

use std::mem::size_of;

use everygraph_core::Pages;

use super::DictionaryError;
use super::hash::{KeyHash, index};
use super::scratch::{Buf, NoMemory, Reservation, widen};

/// Memory a table may use: a reservation of which `used` bytes are taken, never
/// shrinking below `base`.
#[derive(Debug)]
pub(crate) struct Credit<'a> {
    reservation: Reservation<'a>,
    used: u64,
    base: u64,
}

impl<'a> Credit<'a> {
    pub(crate) fn new(
        accountant: &'a dyn everygraph_core::MemoryAccountant,
        purpose: &'static str,
        base: u64,
    ) -> Result<Self, DictionaryError> {
        Ok(Self::from_reservation(Reservation::new(
            accountant, purpose, base,
        )?))
    }

    /// A credit whose base is `reservation`.
    pub(crate) fn from_reservation(reservation: Reservation<'a>) -> Self {
        let base = reservation.bytes();
        Self {
            reservation,
            used: 0,
            base,
        }
    }

    /// Takes `more` bytes, reserving what the credit does not already hold. False if
    /// the accountant refuses.
    fn take(&mut self, more: u64) -> bool {
        let want = self.used + more;
        if want > self.reservation.bytes()
            && self
                .reservation
                .grow(want - self.reservation.bytes())
                .is_err()
        {
            return false;
        }
        self.used = want;
        true
    }

    /// Gives back `bytes` taken for a transient need, keeping at least the base.
    pub(crate) fn give(&mut self, bytes: u64) {
        self.used = self.used.saturating_sub(bytes);
        self.reservation.shrink_to(self.used.max(self.base));
    }

    /// Takes `bytes`, or fails with the accountant's refusal.
    pub(crate) fn require(&mut self, bytes: u64) -> Result<(), DictionaryError> {
        if self.take(bytes) {
            Ok(())
        } else {
            Err(DictionaryError::Budget(
                everygraph_core::InsufficientBudget::new(
                    self.reservation.purpose(),
                    bytes,
                    self.reservation.available(),
                ),
            ))
        }
    }

    /// Nothing is used any more; gives back everything above the base (or tries to
    /// get back to the base, if bytes were split off below it).
    pub(crate) fn reset(&mut self) {
        self.used = 0;
        let held = self.reservation.bytes();
        if held > self.base {
            self.reservation.shrink_to(self.base);
        } else {
            let _ = self.reservation.grow(self.base - held);
        }
    }

    /// The reservation, for the caller to hold.
    pub(crate) fn into_reservation(self) -> Reservation<'a> {
        self.reservation
    }

    /// Moves `bytes` of the reservation out, for memory that outlives the table.
    pub(crate) fn split_off(&mut self, bytes: u64) -> Reservation<'a> {
        self.reservation.split_off(bytes)
    }

    #[cfg(test)]
    pub(crate) fn reserved(&self) -> u64 {
        self.reservation.bytes()
    }
}

/// One distinct key.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Entry {
    pub(crate) hash: KeyHash,
    pub(crate) pos: u64,
    off: u64,
    len: u32,
}

const EMPTY: u32 = u32::MAX;

/// A set of distinct keys with their first positions.
#[derive(Debug)]
pub(crate) struct Table {
    entries: Buf<Entry>,
    arena: Buf<u8>,
    slots: Buf<u32>,
}

/// What [`Table::insert`] did.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Inserted {
    /// The key is in the table (new, or with its position lowered).
    Done,
    /// The table needs more memory than the reservation could get: flush it first.
    Full,
}

impl Table {
    /// An empty table whose arrays will come from `pages`.
    pub(crate) fn new(pages: Pages) -> Self {
        Self {
            entries: Vec::new_in(pages.allocator()),
            arena: Vec::new_in(pages.allocator()),
            slots: Vec::new_in(pages.allocator()),
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub(crate) fn key(&self, e: &Entry) -> &[u8] {
        &self.arena[index(e.off)..index(e.off) + index(u64::from(e.len))]
    }

    /// Inserts `key` seen at `pos`, keeping the smaller position if it is already there.
    /// Growth is reserved through `credit` when one is given (otherwise the caller has
    /// reserved for the whole table).
    ///
    /// # Errors
    ///
    /// The system refused memory that the credit had room for. The table is then only
    /// to be dropped.
    pub(crate) fn insert(
        &mut self,
        hash: KeyHash,
        pos: u64,
        key: &[u8],
        credit: Option<&mut Credit<'_>>,
    ) -> Result<Inserted, NoMemory> {
        if self.slots.is_empty()
            || self.entries.len() == self.entries.capacity()
            || self.arena.len() + key.len() > self.arena.capacity()
        {
            if let Some(found) = self.find(hash, key) {
                let e = &mut self.entries[found];
                e.pos = e.pos.min(pos);
                return Ok(Inserted::Done);
            }
            if !self.grow(key.len(), credit)? {
                return Ok(Inserted::Full);
            }
        }
        let mask = self.slots.len() - 1;
        let mut at = index(hash.lo) & mask;
        loop {
            let slot = self.slots[at];
            if slot == EMPTY {
                break;
            }
            let e = &self.entries[index(u64::from(slot))];
            if e.hash == hash && self.key(e) == key {
                let e = &mut self.entries[index(u64::from(slot))];
                e.pos = e.pos.min(pos);
                return Ok(Inserted::Done);
            }
            at = (at + 1) & mask;
        }
        self.slots[at] =
            u32::try_from(self.entries.len()).expect("a table holds fewer than 2^32 keys");
        self.entries.push(Entry {
            hash,
            pos,
            off: widen(self.arena.len()),
            len: u32::try_from(key.len()).expect("a key shorter than 4 GiB"),
        });
        self.arena.extend_from_slice(key);
        Ok(Inserted::Done)
    }

    fn find(&self, hash: KeyHash, key: &[u8]) -> Option<usize> {
        if self.slots.is_empty() {
            return None;
        }
        let mask = self.slots.len() - 1;
        let mut at = index(hash.lo) & mask;
        loop {
            let slot = self.slots[at];
            if slot == EMPTY {
                return None;
            }
            let e = &self.entries[index(u64::from(slot))];
            if e.hash == hash && self.key(e) == key {
                return Some(index(u64::from(slot)));
            }
            at = (at + 1) & mask;
        }
    }

    /// Makes room for one more key of `key_len` bytes. False if the reservation was
    /// refused; an error if the system refused the memory, and then the credit keeps
    /// what it took.
    fn grow(&mut self, key_len: usize, credit: Option<&mut Credit<'_>>) -> Result<bool, NoMemory> {
        let entries_cap = if self.entries.len() == self.entries.capacity() {
            (self.entries.capacity() * 2).max(1 << 10)
        } else {
            self.entries.capacity()
        };
        let arena_cap = if self.arena.len() + key_len > self.arena.capacity() {
            (self.arena.capacity() * 2)
                .max(self.arena.len() + key_len)
                .max(1 << 14)
        } else {
            self.arena.capacity()
        };
        let slots_len = (entries_cap * 2).next_power_of_two();
        // No single allocation above the workspace's ceiling (1 GiB): a table that
        // would need one is full.
        let limit = everygraph_core::MAX_ALLOCATION_BYTES;
        if widen(entries_cap * size_of::<Entry>()) > limit
            || widen(arena_cap) > limit
            || widen(slots_len * size_of::<u32>()) > limit
        {
            return Ok(false);
        }
        let bytes = |entries: usize, arena: usize, slots: usize| {
            widen(entries * size_of::<Entry>() + arena + slots * size_of::<u32>())
        };
        let old = (
            self.entries.capacity(),
            self.arena.capacity(),
            self.slots.len(),
        );
        let more =
            bytes(entries_cap, arena_cap, slots_len).saturating_sub(bytes(old.0, old.1, old.2));
        // While the entries or the arena grow, their old and new blocks both exist. The
        // slots are an index of the entries: they are freed before they are made again.
        let transient = bytes(
            if entries_cap == old.0 { 0 } else { old.0 },
            if arena_cap == old.1 { 0 } else { old.1 },
            0,
        );
        let mut credit = credit;
        if let Some(c) = credit.as_deref_mut() {
            if more > 0 && !c.take(more + transient) {
                return Ok(false);
            }
        }
        let refused = |elements: usize, size: usize| NoMemory {
            bytes: widen(elements.saturating_mul(size)),
        };
        let more_entries = entries_cap - self.entries.len();
        self.entries
            .try_reserve_exact(more_entries)
            .map_err(|_| refused(more_entries, size_of::<Entry>()))?;
        let more_arena = arena_cap - self.arena.len();
        self.arena
            .try_reserve_exact(more_arena)
            .map_err(|_| refused(more_arena, 1))?;
        if slots_len != self.slots.len() {
            self.slots = Vec::new_in(*self.slots.allocator());
            self.slots
                .try_reserve_exact(slots_len)
                .map_err(|_| refused(slots_len, size_of::<u32>()))?;
            self.slots.resize(slots_len, EMPTY);
            let mask = slots_len - 1;
            for (i, e) in self.entries.iter().enumerate() {
                let mut at = index(e.hash.lo) & mask;
                while self.slots[at] != EMPTY {
                    at = (at + 1) & mask;
                }
                self.slots[at] = u32::try_from(i).expect("fewer than 2^32 keys");
            }
        }
        if let Some(c) = credit {
            if more > 0 {
                c.give(transient);
            }
        }
        Ok(true)
    }

    /// Empties the table, keeping its memory.
    #[cfg(test)]
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.arena.clear();
        self.slots.fill(EMPTY);
    }

    /// Groups the entries by `bucket` (below `buckets`) in place, in increasing order
    /// of bucket (an American flag sort), and returns where each bucket starts. The
    /// table can then only be read or dropped.
    pub(crate) fn group_entries_by(
        &mut self,
        buckets: usize,
        bucket: impl Fn(&Entry) -> usize,
    ) -> Vec<usize> {
        let mut starts = vec![0_usize; buckets + 1];
        for e in &self.entries {
            starts[bucket(e) + 1] += 1;
        }
        for b in 0..buckets {
            starts[b + 1] += starts[b];
        }
        let mut next = starts.clone();
        for b in 0..buckets {
            while next[b] < starts[b + 1] {
                let i = next[b];
                let to = bucket(&self.entries[i]);
                if to == b {
                    next[b] += 1;
                } else {
                    self.entries.swap(i, next[to]);
                    next[to] += 1;
                }
            }
        }
        self.slots.fill(EMPTY);
        starts
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::test_support::Budget;

    #[test]
    fn keys_are_kept_once_with_their_first_position() {
        let mut t = Table::new(Pages::HEAP);
        for round in 0..3_u64 {
            for i in 0..5000_u64 {
                let key = (i % 1234).to_string();
                let h = KeyHash::of(key.as_bytes(), 0);
                assert_eq!(
                    t.insert(h, 10_000 - i - round, key.as_bytes(), None)
                        .unwrap(),
                    Inserted::Done
                );
            }
        }
        assert_eq!(t.len(), 1234);
        for e in t.entries() {
            let k: u64 = std::str::from_utf8(t.key(e)).unwrap().parse().unwrap();
            let last_i = k + 1234 * ((4999 - k) / 1234);
            assert_eq!(e.pos, 10_000 - last_i - 2);
        }
    }

    /// Two different keys with the same hash stay two keys.
    #[test]
    fn equal_hashes_of_different_keys_are_kept_apart() {
        let mut t = Table::new(Pages::HEAP);
        let h = KeyHash { hi: 1, lo: 2 };
        t.insert(h, 5, b"a", None).unwrap();
        t.insert(h, 3, b"b", None).unwrap();
        t.insert(h, 1, b"a", None).unwrap();
        assert_eq!(t.len(), 2);
        assert_eq!(t.entries()[0].pos, 1);
        assert_eq!(t.entries()[1].pos, 3);
    }

    #[test]
    fn grouping_puts_every_entry_in_its_bucket_in_order() {
        let mut t = Table::new(Pages::HEAP);
        for i in 0..10_000_u64 {
            let key = i.to_le_bytes();
            t.insert(KeyHash::of(&key, 0), i, &key, None).unwrap();
        }
        let mut before: Vec<u64> = t.entries().iter().map(|e| e.pos).collect();
        let starts = t.group_entries_by(37, |e| usize::try_from(e.pos % 37).unwrap());
        for b in 0..37 {
            assert!(
                t.entries()[starts[b]..starts[b + 1]]
                    .iter()
                    .all(|e| usize::try_from(e.pos % 37).unwrap() == b)
            );
        }
        let mut after: Vec<u64> = t.entries().iter().map(|e| e.pos).collect();
        before.sort_unstable();
        after.sort_unstable();
        assert_eq!(before, after);
    }

    #[test]
    fn a_table_reports_full_when_its_reservation_cannot_grow() {
        let budget = Budget::new(200_000);
        let mut r = Credit::new(&budget, "table", 1000).unwrap();
        let mut t = Table::new(Pages::HEAP);
        let mut inserted = 0;
        for i in 0..100_000_u64 {
            let key = i.to_le_bytes();
            match t
                .insert(KeyHash::of(&key, 0), i, &key, Some(&mut r))
                .unwrap()
            {
                Inserted::Done => inserted += 1,
                Inserted::Full => break,
            }
        }
        assert!(inserted > 1000 && inserted < 100_000, "{inserted}");
        assert!(r.reserved() <= 200_000);
        r.reset();
        assert_eq!(r.reserved(), 1000);
        t.clear();
        assert!(t.is_empty());
        let key = 7_u64.to_le_bytes();
        assert_eq!(
            t.insert(KeyHash::of(&key, 0), 0, &key, Some(&mut r))
                .unwrap(),
            Inserted::Done
        );
    }
}
