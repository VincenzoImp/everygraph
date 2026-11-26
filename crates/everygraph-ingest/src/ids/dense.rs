// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Dense integer keys: a bitmap over `0..2^32` set while the input is read, and the rank
//! directory that turns it into a dictionary (the dense id of a key is the number of
//! keys below it).

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use everygraph_core::MemoryAccountant;

use super::DictionaryError;
use super::hash::index;
use super::scratch::{Reservation, widen};

/// Bits per segment of the bitmap: 2^16 (8 KiB), allocated when first touched, so that
/// sparse keys cost about 8 KiB each at worst while they are read (a few hundred
/// keys spread over `0..2^32` took 400 MB with 512 KiB segments).
const SEGMENT_BITS: u32 = 16;
const SEGMENT_WORDS: usize = 1 << (SEGMENT_BITS - 6);
const SEGMENTS: usize = 1 << (32 - SEGMENT_BITS);

const PURPOSE: &str = "id dictionary dense bitmap";

/// The bitmap of keys seen, shared by every reader thread.
pub(crate) struct DenseBits<'a> {
    segments: Vec<OnceLock<Box<[AtomicU64]>>>,
    reservation: std::sync::Mutex<Reservation<'a>>,
    /// The most the bitmap can take: its table and every segment below the key limit.
    /// A refusal names it, so that a larger budget is known to be enough.
    bound: u64,
}

impl std::fmt::Debug for DenseBits<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DenseBits").finish_non_exhaustive()
    }
}

/// A refusal for want of budget, of which `available` was left.
fn budget_error(available: u64) -> DictionaryError {
    DictionaryError::Budget(everygraph_core::InsufficientBudget::new(
        PURPOSE, 0, available,
    ))
}

impl<'a> DenseBits<'a> {
    /// A bitmap for keys below `limit` (at most 2^32); the caller stops at a key at or
    /// above it.
    pub(crate) fn new(
        accountant: &'a dyn MemoryAccountant,
        limit: u64,
    ) -> Result<Self, DictionaryError> {
        // The table of segments itself (65,536 entries).
        let table = widen(SEGMENTS * std::mem::size_of::<OnceLock<Box<[AtomicU64]>>>());
        let segments = limit.min(1 << 32).div_ceil(1 << SEGMENT_BITS);
        let bound = table + segments * widen(SEGMENT_WORDS * 8);
        let reservation =
            Reservation::new(accountant, PURPOSE, table).map_err(|e| Self::refused(bound, e))?;
        Ok(Self {
            segments: (0..SEGMENTS).map(|_| OnceLock::new()).collect(),
            reservation: std::sync::Mutex::new(reservation),
            bound,
        })
    }

    /// A refusal of the budget, naming the bitmap's bound as what it needs.
    fn refused(bound: u64, e: DictionaryError) -> DictionaryError {
        match e {
            DictionaryError::Budget(b) => DictionaryError::Budget(
                everygraph_core::InsufficientBudget::new(PURPOSE, bound, b.available),
            ),
            other => other,
        }
    }

    /// Marks `key` as present.
    pub(crate) fn set(&self, key: u32) -> Result<(), DictionaryError> {
        let bit = index(u64::from(key));
        let segment = &self.segments[bit >> SEGMENT_BITS];
        let words = match segment.get() {
            Some(w) if !w.is_empty() => w,
            Some(_) => {
                let available = self.reservation_available();
                return Err(Self::refused(self.bound, budget_error(available)));
            }
            None => {
                let mut failed = None;
                let w = segment.get_or_init(|| {
                    let bytes = widen(SEGMENT_WORDS * 8);
                    let mut r = self
                        .reservation
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    if let Err(e) = r.grow(bytes) {
                        failed = Some(e);
                        return Box::new([]);
                    }
                    (0..SEGMENT_WORDS).map(|_| AtomicU64::new(0)).collect()
                });
                if let Some(e) = failed {
                    return Err(Self::refused(self.bound, e));
                }
                if w.is_empty() {
                    let available = self.reservation_available();
                    return Err(Self::refused(self.bound, budget_error(available)));
                }
                w
            }
        };
        let word = (bit >> 6) & (SEGMENT_WORDS - 1);
        words[word].fetch_or(1 << (bit & 63), Ordering::Relaxed);
        Ok(())
    }

    fn reservation_available(&self) -> u64 {
        self.reservation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .available()
    }

    /// Number of words [`DenseBits::into_words`] allocates (at most).
    pub(crate) fn word_len(&self) -> usize {
        self.segments
            .iter()
            .rposition(|s| s.get().is_some_and(|w| !w.is_empty()))
            .map_or(0, |last| (last + 1) * SEGMENT_WORDS)
    }

    /// Number of keys set, and the largest, read from the segments in place.
    pub(crate) fn count_and_max(&self) -> (u64, Option<u32>) {
        let mut n = 0_u64;
        let mut max = None;
        for (i, s) in self.segments.iter().enumerate() {
            let Some(words) = s.get().filter(|w| !w.is_empty()) else {
                continue;
            };
            for (j, w) in words.iter().enumerate() {
                let w = w.load(Ordering::Relaxed);
                if w != 0 {
                    n += u64::from(w.count_ones());
                    let bit =
                        (i * SEGMENT_WORDS + j) * 64 + 63 - index(u64::from(w.leading_zeros()));
                    max = Some(u32::try_from(bit).expect("below 2^32"));
                }
            }
        }
        (n, max)
    }

    /// Calls `f` with every key set, in increasing order, for the segments of
    /// `segments` (a range of segment numbers).
    pub(crate) fn for_each_key(
        &self,
        segments: std::ops::Range<usize>,
        mut f: impl FnMut(u32) -> Result<(), DictionaryError>,
    ) -> Result<(), DictionaryError> {
        for i in segments {
            let Some(words) = self.segments[i].get().filter(|w| !w.is_empty()) else {
                continue;
            };
            for (j, w) in words.iter().enumerate() {
                let mut w = w.load(Ordering::Relaxed);
                while w != 0 {
                    let bit = (i * SEGMENT_WORDS + j) * 64 + index(u64::from(w.trailing_zeros()));
                    w &= w - 1;
                    f(u32::try_from(bit).expect("below 2^32"))?;
                }
            }
        }
        Ok(())
    }

    /// Number of segments.
    pub(crate) const SEGMENTS: usize = SEGMENTS;

    /// The bitmap as words, up to the last one with a bit set.
    pub(crate) fn into_words(self) -> Vec<u64> {
        let last = self
            .segments
            .iter()
            .rposition(|s| s.get().is_some_and(|w| !w.is_empty()));
        let Some(last) = last else { return Vec::new() };
        let mut words = Vec::with_capacity((last + 1) * SEGMENT_WORDS);
        for s in &self.segments[..=last] {
            match s.get() {
                Some(w) if !w.is_empty() => {
                    words.extend(w.iter().map(|a| a.load(Ordering::Relaxed)));
                }
                _ => words.extend(std::iter::repeat_n(0, SEGMENT_WORDS)),
            }
        }
        while words.last() == Some(&0) {
            words.pop();
        }
        words
    }
}

/// Words per rank block.
const BLOCK_WORDS: usize = 8;

/// A bitmap with a rank directory: the dictionary of dense integer keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RankBitmap {
    words: Vec<u64>,
    /// Number of bits set before each block of `BLOCK_WORDS` words, and the total last.
    blocks: Vec<u32>,
}

impl RankBitmap {
    pub(crate) fn new(words: Vec<u64>) -> Result<Self, DictionaryError> {
        let mut blocks = Vec::with_capacity(words.len().div_ceil(BLOCK_WORDS) + 1);
        let mut total = 0_u64;
        for chunk in words.chunks(BLOCK_WORDS) {
            blocks.push(u32::try_from(total).map_err(|_| super::too_many_nodes(total))?);
            total += chunk.iter().map(|w| u64::from(w.count_ones())).sum::<u64>();
        }
        blocks.push(u32::try_from(total).map_err(|_| super::too_many_nodes(total))?);
        super::check_node_count(total)?;
        Ok(Self { words, blocks })
    }

    pub(crate) fn words(&self) -> &[u64] {
        &self.words
    }

    /// Bytes held in memory.
    pub(crate) fn bytes(words: usize) -> u64 {
        widen(words * 8 + (words.div_ceil(BLOCK_WORDS) + 1) * 4)
    }

    pub(crate) fn len(&self) -> u32 {
        *self.blocks.last().expect("at least one block entry")
    }

    /// The rank of `key` if it is present.
    pub(crate) fn rank(&self, key: u32) -> Option<u32> {
        let bit = index(u64::from(key));
        let w = *self.words.get(bit >> 6)?;
        if w & (1 << (bit & 63)) == 0 {
            return None;
        }
        let word = bit >> 6;
        let block = word / BLOCK_WORDS;
        let mut r = self.blocks[block];
        for &x in &self.words[block * BLOCK_WORDS..word] {
            r += x.count_ones();
        }
        Some(r + (w & ((1 << (bit & 63)) - 1)).count_ones())
    }

    /// The key of rank `r`.
    pub(crate) fn select(&self, r: u32) -> Option<u32> {
        if r >= self.len() {
            return None;
        }
        // The last block whose count before it is <= r.
        let block = self.blocks.partition_point(|&c| c <= r) - 1;
        let mut left = r - self.blocks[block];
        for (i, &w) in self.words[block * BLOCK_WORDS..].iter().enumerate() {
            let ones = w.count_ones();
            if left < ones {
                let mut w = w;
                for _ in 0..left {
                    w &= w - 1;
                }
                let bit = (block * BLOCK_WORDS + i) * 64 + index(u64::from(w.trailing_zeros()));
                return u32::try_from(bit).ok();
            }
            left -= ones;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::test_support::Budget;

    #[test]
    fn rank_and_select_are_inverse_over_a_bitmap_filled_from_threads() {
        let budget = Budget::new(1 << 30);
        let bits = DenseBits::new(&budget, 1 << 32).unwrap();
        let keys: Vec<u32> = (0..20_000_u32)
            .map(|i| i.wrapping_mul(2_654_435_761) >> 9)
            .collect();
        everygraph_core::par::for_each_range(
            keys.len(),
            std::num::NonZeroUsize::new(100).unwrap(),
            |r| {
                for &k in &keys[r] {
                    bits.set(k).unwrap();
                }
            },
        );
        let table = widen(SEGMENTS * std::mem::size_of::<OnceLock<Box<[AtomicU64]>>>());
        let reserved = budget.total() - budget.available() - table;
        assert_eq!(reserved % widen(SEGMENT_WORDS * 8), 0);
        let rb = RankBitmap::new(bits.into_words()).unwrap();
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(rb.len(), u32::try_from(sorted.len()).unwrap());
        for (r, &k) in sorted.iter().enumerate() {
            let r = u32::try_from(r).unwrap();
            assert_eq!(rb.rank(k), Some(r));
            assert_eq!(rb.select(r), Some(k));
        }
        assert_eq!(rb.select(rb.len()), None);
        assert_eq!(rb.rank(u32::MAX), None);
    }

    #[test]
    fn a_bitmap_that_does_not_fit_the_budget_is_refused() {
        let table = widen(SEGMENTS * std::mem::size_of::<OnceLock<Box<[AtomicU64]>>>());
        assert!(DenseBits::new(&Budget::new(table - 1), 1 << 32).is_err());
        let budget = Budget::new(table + 1000);
        // The refusal names the whole bound: the table and the segments below the
        // limit, here 3 of them.
        let limit = 2 << SEGMENT_BITS | 1;
        let bound = table + 3 * widen(SEGMENT_WORDS * 8);
        let bits = DenseBits::new(&budget, limit).unwrap();
        for key in [5, 6] {
            match bits.set(key) {
                Err(DictionaryError::Budget(b)) => assert_eq!(b.needed, bound),
                other => panic!("{other:?}"),
            }
        }
        let empty = RankBitmap::new(Vec::new()).unwrap();
        assert_eq!(empty.len(), 0);
        assert_eq!(empty.rank(0), None);
    }
}
