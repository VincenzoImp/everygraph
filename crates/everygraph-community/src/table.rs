// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Per-worker tables that sum weights by community, and the pool that lends them.
//!
//! A table is open addressing over `u32` keys, sized to twice the keys it may receive
//! and grown against the memory budget. The sums are integers, so the value of a key
//! does not depend on the order of the additions, and a decision that reads the table
//! does not depend on the order in which it lists its keys (callers break ties by key).

use std::fmt;
use std::sync::{Mutex, PoisonError};

use everygraph_core::{InsufficientBudget, Memory, MemoryKind, Reservation};
use everygraph_kernels::edge_map::Error;

const EMPTY: u32 = u32::MAX;
/// Bytes per slot: a key and its sum; and, for at most half the slots, a position in
/// `used` and an entry in `sorted` (16 bytes).
const SLOT_BYTES: u64 = 16 + 10;

/// Sums of `u64` weights by `u32` key.
pub(crate) struct Table {
    /// A key and its sum, side by side: one cache miss per probe.
    slots: Vec<(u32, u64)>,
    /// The slots in use, in the order their keys arrived.
    used: Vec<u32>,
    bits: u32,
    reservation: Reservation,
    /// Scratch for the caller: the entries in key order.
    pub(crate) sorted: Vec<(u32, u64)>,
}

impl Table {
    fn new(memory: &Memory) -> Self {
        Self {
            slots: Vec::new(),
            used: Vec::new(),
            bits: 0,
            reservation: Reservation::empty(memory, "community tables", MemoryKind::Anonymous),
            sorted: Vec::new(),
        }
    }

    /// Empties the table and makes room for `keys` distinct keys. Only the first slots
    /// that many keys need are used, so a table that once grew for a long list keeps a
    /// small working set for short ones.
    pub(crate) fn begin(&mut self, keys: usize) -> Result<(), InsufficientBudget> {
        for &s in &self.used {
            self.slots[s as usize].0 = EMPTY;
        }
        self.used.clear();
        // At most 2^32 slots: their positions are `u32`.
        let most = usize::try_from(1_u64 << 32).unwrap_or(1 << (usize::BITS - 1));
        let slots = (keys.max(4) * 2).next_power_of_two().min(most);
        if slots > self.slots.len() {
            let more = (slots - self.slots.len()) as u64 * SLOT_BYTES;
            self.reservation.try_grow(more)?;
            self.slots = (0..slots).map(|_| (EMPTY, 0)).collect();
        }
        self.bits = slots.trailing_zeros();
        Ok(())
    }

    /// Adds `w` to the sum of `key`. There is always an empty slot among the slots in
    /// use: at most half of them hold keys, or fewer than 2^32 of 2^32.
    #[inline(always)]
    pub(crate) fn add(&mut self, key: u32, w: u64) {
        let mask = usize::MAX >> (usize::BITS - self.bits);
        let mut s = (key.wrapping_mul(0x9E37_79B1) >> (32 - self.bits)) as usize;
        loop {
            let slot = &mut self.slots[s];
            if slot.0 == key {
                slot.1 += w;
                return;
            }
            if slot.0 == EMPTY {
                *slot = (key, w);
                self.used.push(u32::try_from(s).expect("a slot below 2^32"));
                return;
            }
            s = (s + 1) & mask;
        }
    }

    /// The number of distinct keys.
    pub(crate) fn len(&self) -> usize {
        self.used.len()
    }

    /// The keys and their sums, in arrival order: a function of the order of the
    /// additions only.
    pub(crate) fn entries(&self) -> impl Iterator<Item = (u32, u64)> + '_ {
        self.used.iter().map(|&s| self.slots[s as usize])
    }

    /// Fills [`sorted`](Self::sorted) with the entries in key order.
    pub(crate) fn sort(&mut self) {
        let mut sorted = std::mem::take(&mut self.sorted);
        sorted.clear();
        sorted.extend(self.entries());
        sorted.sort_unstable_by_key(|e| e.0);
        self.sorted = sorted;
    }
}

/// Tables lent to workers and given back when the worker's call ends, so that a level
/// grows each table once and not once per sub-round.
pub(crate) struct Tables {
    free: Mutex<Vec<Table>>,
    memory: Memory,
    /// The first error seen by a body, which cannot return it.
    failed: Mutex<Option<Error>>,
}

impl fmt::Debug for Tables {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tables").finish_non_exhaustive()
    }
}

impl Tables {
    pub(crate) fn new(memory: &Memory) -> Self {
        Self {
            free: Mutex::new(Vec::new()),
            memory: memory.clone(),
            failed: Mutex::new(None),
        }
    }

    /// A table for one worker, for the rest of one driver call.
    pub(crate) fn lend(&self) -> Lent<'_> {
        let table = self
            .free
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop()
            .unwrap_or_else(|| Table::new(&self.memory));
        Lent {
            table: Some(table),
            home: self,
        }
    }

    /// Records an error of a body; [`check`](Self::check) returns the first.
    pub(crate) fn fail(&self, e: impl Into<Error>) {
        let e = e.into();
        let mut failed = self.failed.lock().unwrap_or_else(PoisonError::into_inner);
        failed.get_or_insert(e);
    }

    /// The first refusal recorded since the last call.
    pub(crate) fn check(&self) -> Result<(), Error> {
        let failed = self
            .failed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        failed.map_or(Ok(()), Err)
    }
}

/// A lent table; it goes back to its pool when dropped.
pub(crate) struct Lent<'p> {
    table: Option<Table>,
    home: &'p Tables,
}

impl Lent<'_> {
    /// The table, emptied and with room for `keys` keys; `None` if the budget refused
    /// the room, which is then recorded in the pool.
    pub(crate) fn begin(&mut self, keys: usize) -> Option<&mut Table> {
        let table = self.table.as_mut().expect("lent until dropped");
        match table.begin(keys) {
            Ok(()) => Some(table),
            Err(e) => {
                self.home.fail(e);
                None
            }
        }
    }
}

impl Drop for Lent<'_> {
    fn drop(&mut self) {
        if let Some(table) = self.table.take() {
            self.home
                .free
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(table);
        }
    }
}
