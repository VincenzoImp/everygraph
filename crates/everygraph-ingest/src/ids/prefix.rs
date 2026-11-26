// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The optional sorted index for prefix and range search: keys in sorted order,
//! front-coded in blocks of 16 (each key after a block's first is stored as the length
//! of the prefix it shares with the one before and the rest), with the id of each key.
//! Built from the key store on first use; held in memory.

use everygraph_core::{ByteSource, MemoryAccountant, NodeId, widen};

use super::DictionaryError;
use super::dict::Dictionary;
use super::hash::index;
use super::scratch::Reservation;

const BLOCK: usize = 16;

/// A front-coded sorted index of the keys of a dictionary.
pub(crate) struct PrefixIndex<'a> {
    data: Vec<u8>,
    /// Offset in `data` of each block.
    blocks: Vec<u64>,
    /// Id of each key, in sorted order.
    ids: Vec<u32>,
    _memory: Reservation<'a>,
}

impl std::fmt::Debug for PrefixIndex<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PrefixIndex")
            .field("keys", &self.ids.len())
            .finish_non_exhaustive()
    }
}

fn put_var(out: &mut Vec<u8>, mut x: u64) {
    while x >= 0x80 {
        out.push(u8::try_from(x & 0x7F).expect("masked") | 0x80);
        x >>= 7;
    }
    out.push(u8::try_from(x).expect("below 0x80"));
}

fn get_var(data: &[u8], at: &mut usize) -> usize {
    let mut x = 0_u64;
    let mut shift = 0;
    loop {
        let b = data[*at];
        *at += 1;
        x |= u64::from(b & 0x7F) << shift;
        if b < 0x80 {
            return index(x);
        }
        shift += 7;
    }
}

impl<'a> PrefixIndex<'a> {
    pub(crate) fn build<S: ByteSource>(
        dict: &Dictionary<'_, S>,
        accountant: &'a dyn MemoryAccountant,
    ) -> Result<Self, DictionaryError> {
        let n = dict.len();
        // The index is built in memory, in single arrays: its ids and spans (16 bytes per
        // key) and the keys must each fit one allocation of at most 1 GiB (about 67 M
        // keys, 1 GiB of key bytes). Larger dictionaries are refused.
        let limit = everygraph_core::MAX_ALLOCATION_BYTES;
        let too_large = || {
            DictionaryError::Unsupported(
                "the prefix index holds at most 1 GiB of keys and 67 million keys",
            )
        };
        if u64::from(n) * 16 > limit {
            return Err(too_large());
        }
        // The keys and their ids, then sorted; reserved as the spans, then twice the
        // arena as it grows (the sorted copy is front-coded and smaller).
        let mut memory =
            Reservation::new(accountant, "id dictionary prefix index", u64::from(n) * 16)?;
        let mut arena: Vec<u8> = Vec::new();
        let mut spans: Vec<(u64, u32, u32)> = Vec::with_capacity(index(u64::from(n)));
        let mut key = Vec::new();
        for id in 0..n {
            dict.normalized_key(NodeId::new(id), &mut key)?;
            if arena.len() + key.len() > arena.capacity() {
                let new_cap = (2 * arena.capacity())
                    .max(arena.len() + key.len())
                    .max(1 << 16);
                if widen(new_cap) > limit {
                    return Err(too_large());
                }
                // The old and new arenas coexist while it grows.
                memory.grow(2 * widen(new_cap))?;
                arena.reserve_exact(new_cap - arena.len());
            }
            spans.push((
                widen(arena.len()),
                u32::try_from(key.len()).expect("short key"),
                id,
            ));
            arena.extend_from_slice(&key);
        }
        let k = |s: &(u64, u32, u32)| &arena[index(s.0)..index(s.0) + index(u64::from(s.1))];
        spans.sort_unstable_by(|a, b| k(a).cmp(k(b)));
        let mut data = Vec::with_capacity(arena.len() / 2);
        let mut blocks = Vec::with_capacity(spans.len().div_ceil(BLOCK));
        let mut prev: &[u8] = &[];
        for (r, s) in spans.iter().enumerate() {
            let key = k(s);
            if r % BLOCK == 0 {
                blocks.push(widen(data.len()));
                put_var(&mut data, widen(key.len()));
                data.extend_from_slice(key);
            } else {
                let shared = key.iter().zip(prev).take_while(|(a, b)| a == b).count();
                put_var(&mut data, widen(shared));
                put_var(&mut data, widen(key.len() - shared));
                data.extend_from_slice(&key[shared..]);
            }
            prev = key;
        }
        let ids = spans.iter().map(|s| s.2).collect();
        drop(arena);
        // What stays: the front-coded data, the block offsets, the ids.
        let kept = widen(data.capacity() + blocks.capacity() * 8 + spans.len() * 4);
        drop(memory);
        let memory = Reservation::new(accountant, "id dictionary prefix index", kept)?;
        Ok(Self {
            data,
            blocks,
            ids,
            _memory: memory,
        })
    }

    fn head(&self, b: usize) -> &[u8] {
        let mut at = index(self.blocks[b]);
        let len = get_var(&self.data, &mut at);
        &self.data[at..at + len]
    }

    /// Calls `f(id, key)` for keys in sorted order from the first that is `>= start`,
    /// until it returns false.
    fn scan(&self, start: &[u8], mut f: impl FnMut(u32, &[u8]) -> bool) {
        if self.blocks.is_empty() {
            return;
        }
        // The last block whose head is <= start (or the first block).
        let (mut lo, mut hi) = (0, self.blocks.len());
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.head(mid) <= start {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        let mut b = lo.saturating_sub(1);
        let mut key = Vec::new();
        while b < self.blocks.len() {
            let mut at = index(self.blocks[b]);
            for r in b * BLOCK..((b + 1) * BLOCK).min(self.ids.len()) {
                if r % BLOCK == 0 {
                    let len = get_var(&self.data, &mut at);
                    key.clear();
                    key.extend_from_slice(&self.data[at..at + len]);
                    at += len;
                } else {
                    let shared = get_var(&self.data, &mut at);
                    let rest = get_var(&self.data, &mut at);
                    key.truncate(shared);
                    key.extend_from_slice(&self.data[at..at + rest]);
                    at += rest;
                }
                if key.as_slice() >= start && !f(self.ids[r], &key) {
                    return;
                }
            }
            b += 1;
        }
    }

    pub(crate) fn prefix(&self, prefix: &[u8], limit: usize) -> Vec<NodeId> {
        let mut out = Vec::new();
        if limit == 0 {
            return out;
        }
        self.scan(prefix, |id, key| {
            if !key.starts_with(prefix) {
                return false;
            }
            out.push(NodeId::new(id));
            out.len() < limit
        });
        out
    }

    pub(crate) fn range(&self, start: &[u8], end: &[u8], limit: usize) -> Vec<NodeId> {
        let mut out = Vec::new();
        if limit == 0 {
            return out;
        }
        self.scan(start, |id, key| {
            if key >= end {
                return false;
            }
            out.push(NodeId::new(id));
            out.len() < limit
        });
        out
    }
}
