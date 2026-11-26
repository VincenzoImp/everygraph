// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The set of nodes a query has met: a hash table that grows with what it holds, or a
//! bit per node of the graph when that takes less memory than the largest table. A reader
//! keeps one from query to query, so that a query allocates nothing for it and empties
//! only what the last one used.

use everygraph_core::{Memory, SegVec, usize_of, widen};
use everygraph_kernels::edge_map::one_segment_mut;

use crate::QueryError;

/// A slot that holds no node: ids are below the number of nodes, at most `u32::MAX`.
const EMPTY: u32 = u32::MAX;
/// The first table: 64 slots.
const FIRST_BITS: u32 = 6;

pub(crate) struct Seen {
    memory: Memory,
    mode: Mode,
}

enum Mode {
    /// Open addressing with linear probing, at most half full.
    Table {
        slots: SegVec<u32>,
        bits: u32,
        len: usize,
    },
    /// A bit per node of a graph of `n` nodes, and the nodes set, to empty it.
    Bits {
        words: SegVec<u64>,
        n: u32,
        set: SegVec<u32>,
    },
}

/// The table's slot bits for `most` nodes: at most half full.
fn table_bits(most: usize) -> u32 {
    (2 * most.max(1))
        .next_power_of_two()
        .trailing_zeros()
        .max(FIRST_BITS)
}

/// Bytes of the largest table for `most` nodes, beside the one it grew from.
fn table_bytes(most: usize) -> u64 {
    let table = 4 * (1_u64 << table_bits(most));
    table + table / 2
}

/// Bytes of a bit per node of `n`, with the list of up to `most` nodes set.
fn bits_bytes(n: u32, most: usize) -> u64 {
    u64::from(n).div_ceil(64) * 8 + 4 * widen(most)
}

/// Whether a set of at most `most` of `n` nodes is bits: only when they take less memory
/// than the table. Bits up to 8 times the table's memory made 3-hop queries with a
/// limit of 10,000 on orkut 1.12× slower (exploratory, interleaved; PR of #90).
fn as_bits(n: u32, most: usize) -> bool {
    bits_bytes(n, most) < table_bytes(most)
}

impl Seen {
    /// The most bytes a set of at most `most` of `n` nodes holds.
    pub fn bytes(n: u32, most: usize) -> u64 {
        if as_bits(n, most) {
            bits_bytes(n, most)
        } else {
            table_bytes(most)
        }
    }

    /// A set that holds nothing yet.
    pub fn new(memory: &Memory) -> Self {
        Self {
            memory: memory.clone(),
            mode: Mode::Table {
                slots: SegVec::new(memory, "point query seen"),
                bits: 0,
                len: 0,
            },
        }
    }

    /// Empties the set, for at most `most` of `n` nodes.
    pub fn begin(&mut self, n: u32, most: usize) -> Result<(), QueryError> {
        let bits = as_bits(n, most);
        let purpose = "point query seen";
        match &mut self.mode {
            Mode::Bits { words, n: m, set } if bits && *m == n => {
                for &v in set.iter() {
                    words[usize_of(v / 64)] = 0;
                }
                set.clear();
            }
            Mode::Table {
                slots,
                bits: b,
                len,
            } if !bits && *b > 0 => {
                slots.fill(EMPTY);
                *len = 0;
            }
            // The old buffers go first, so that the two are never held at once.
            _ if bits => {
                self.mode = Mode::Table {
                    slots: SegVec::new(&self.memory, purpose),
                    bits: 0,
                    len: 0,
                };
                let words = usize_of(n).div_ceil(64);
                self.mode = Mode::Bits {
                    words: SegVec::try_with_len(&self.memory, purpose, words, 0)?,
                    n,
                    set: SegVec::new(&self.memory, purpose),
                };
            }
            _ => {
                self.mode = Mode::Table {
                    slots: SegVec::new(&self.memory, purpose),
                    bits: 0,
                    len: 0,
                };
                self.mode = Mode::Table {
                    slots: SegVec::try_with_len(&self.memory, purpose, 1 << FIRST_BITS, EMPTY)?,
                    bits: FIRST_BITS,
                    len: 0,
                };
            }
        }
        if let Mode::Bits { set, .. } = &mut self.mode {
            // Room for every node at once: a list that doubles would hold two buffers.
            set.try_reserve(most)?;
        }
        Ok(())
    }

    /// Adds `v`; whether it was not in the set.
    pub fn insert(&mut self, v: u32) -> Result<bool, QueryError> {
        match &mut self.mode {
            Mode::Bits { words, set, .. } => {
                let word = &mut words[usize_of(v / 64)];
                let bit = 1 << (v % 64);
                if *word & bit != 0 {
                    return Ok(false);
                }
                *word |= bit;
                set.try_push(v)?;
                Ok(true)
            }
            Mode::Table { slots, bits, len } => {
                if !place(one_segment_mut(slots)?, *bits, v) {
                    return Ok(false);
                }
                *len += 1;
                if 2 * *len > slots.len() {
                    let more = *bits + 1;
                    let mut grown =
                        SegVec::try_with_len(&self.memory, "point query seen", 1 << more, EMPTY)?;
                    let table = one_segment_mut(&mut grown)?;
                    for &u in slots.iter().filter(|&&u| u != EMPTY) {
                        place(table, more, u);
                    }
                    *slots = grown;
                    *bits = more;
                }
                Ok(true)
            }
        }
    }
}

/// Puts `v` in the table of `2^bits` slots; whether it was not there.
fn place(table: &mut [u32], bits: u32, v: u32) -> bool {
    let mask = table.len() - 1;
    let hash = u64::from(v).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> (64 - bits);
    let mut at = usize::try_from(hash).unwrap_or(0) & mask;
    loop {
        match table[at] {
            EMPTY => {
                table[at] = v;
                return true;
            }
            u if u == v => return false,
            _ => at = (at + 1) & mask,
        }
    }
}
