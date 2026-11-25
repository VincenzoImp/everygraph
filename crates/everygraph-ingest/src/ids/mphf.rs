// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A partitioned minimal perfect hash function: the `PtrHash` construction (Groot
//! Koerkamp, 2025: skewed buckets, one 8-bit pilot per bucket, slots past `n` remapped
//! into the holes below `n`), with the parts solved independently.
//!
//! Keys are split into `2^part_bits` parts by the top bits of their hash. Part `q` holds
//! `n_q` keys in `slots_q ≥ n_q` slots and `buckets_q` buckets; its global slots are
//! `slot_base_q .. slot_base_q + n_q`, where `slot_base_q` is the number of keys in the
//! parts before it. A part is a pure function of its set of hashes, so the whole
//! function is the same however the keys were found, in whatever order, at any thread
//! count or memory budget.

use std::collections::BinaryHeap;

use super::hash::{KeyHash, bucket, index, pilot_hash, slot, top_bits};

/// Parts aim at about this many keys (2^15): the slot table of one part (130 KiB) stays
/// in the L2 cache while it is solved, and the part table stays small (512 KiB at
/// 500 M keys).
const PART_TARGET_BITS: u32 = 15;
/// At most 2^16 parts (2^32 keys make parts of 2^16 keys).
const MAX_PART_BITS: u32 = 16;
/// Part seeds tried before a part is declared unsolvable. One is almost always enough.
const MAX_SEEDS: u32 = 64;
/// At most this many of the buckets placed most recently may not be evicted (against
/// cycles); a part of `b` buckets protects `min(b / 4, RECENT)`, as `PtrHash` does, so
/// that in small parts some bucket can always be evicted.
const RECENT: usize = 16;

/// Number of part bits for `n` keys.
pub(crate) fn part_bits_for(n: u64) -> u32 {
    let parts = n.div_ceil(1 << PART_TARGET_BITS).max(1).next_power_of_two();
    parts.trailing_zeros().min(MAX_PART_BITS)
}

/// Slots and buckets of a part of `n` keys: load factor about 0.99, 3.5 keys per bucket.
pub(crate) fn part_shape(n: u32) -> (u32, u32) {
    if n == 0 {
        return (0, 0);
    }
    let slots = n.saturating_add(n.div_ceil(100));
    let buckets = u32::try_from((u64::from(n) * 2).div_ceil(7))
        .expect("2n/7 of a u32 fits in a u32")
        .max(1);
    (slots, buckets)
}

/// One row of the part table, as stored (eight little-endian `u32`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PartRow {
    pub(crate) slot_base: u32,
    pub(crate) n: u32,
    pub(crate) slots: u32,
    pub(crate) bucket_base: u32,
    pub(crate) buckets: u32,
    pub(crate) remap_base: u32,
    pub(crate) seed: u32,
}

impl PartRow {
    pub(crate) const BYTES: usize = 32;

    pub(crate) fn encode(&self, out: &mut Vec<u8>) {
        for v in [
            self.slot_base,
            self.n,
            self.slots,
            self.bucket_base,
            self.buckets,
            self.remap_base,
            self.seed,
            0,
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }

    pub(crate) fn decode(bytes: &[u8]) -> Self {
        let w =
            |i: usize| u32::from_le_bytes(bytes[4 * i..4 * i + 4].try_into().expect("four bytes"));
        Self {
            slot_base: w(0),
            n: w(1),
            slots: w(2),
            bucket_base: w(3),
            buckets: w(4),
            remap_base: w(5),
            seed: w(6),
        }
    }
}

/// A solved part: its seed, one pilot per bucket and the remap of slots `n..slots`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Part {
    pub(crate) n: u32,
    pub(crate) slots: u32,
    pub(crate) buckets: u32,
    pub(crate) seed: u32,
    pub(crate) pilots: Vec<u8>,
    pub(crate) remap: Vec<u32>,
}

/// Why a part could not be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PartError {
    /// Two distinct keys have the same 128-bit hash: no function of the hash can
    /// separate them. The dictionary is rebuilt with another hash seed.
    Collision,
    /// No seed led to a solution within the eviction limit. Not expected to happen.
    Unsolvable,
}

impl Part {
    /// Local slot (`0..n`) of a key of this part.
    pub(crate) fn eval(&self, h: KeyHash) -> u32 {
        local_slot(
            h,
            self.seed,
            self.buckets,
            self.slots,
            self.n,
            |b| self.pilots[b],
            |i| self.remap[i],
        )
    }
}

/// The local slot of `h` in a part described by its parameters and accessors.
pub(crate) fn local_slot(
    h: KeyHash,
    seed: u32,
    buckets: u32,
    slots: u32,
    n: u32,
    pilot: impl Fn(usize) -> u8,
    remap: impl Fn(usize) -> u32,
) -> u32 {
    let b = index(bucket(h.hi, seed, buckets));
    let s = u32::try_from(slot(h.lo, pilot_hash(pilot(b), seed), slots))
        .expect("a slot is below the part's slot count");
    if s < n {
        s
    } else {
        remap(index(u64::from(s - n)))
    }
}

/// Builds the part for `hashes`, which must be the hashes of distinct keys. The result
/// depends only on the set of hashes, not on their order.
pub(crate) fn build_part(hashes: &[KeyHash]) -> Result<Part, PartError> {
    let n = u32::try_from(hashes.len()).expect("a part holds fewer than 2^32 keys");
    let (slots, buckets) = part_shape(n);
    if n == 0 {
        return Ok(Part::default());
    }
    let mut sorted = hashes.to_vec();
    sorted.sort_unstable();
    if sorted.windows(2).any(|w| w[0] == w[1]) {
        return Err(PartError::Collision);
    }
    for seed in 0..MAX_SEEDS {
        if let Some((pilots, taken)) = Solver::new(&sorted, seed, slots, buckets).solve() {
            let remap = remap_table(&taken, n);
            return Ok(Part {
                n,
                slots,
                buckets,
                seed,
                pilots,
                remap,
            });
        }
    }
    Err(PartError::Unsolvable)
}

/// Maps every taken slot at or past `n` to a free slot below `n`, both in increasing
/// order. Untaken slots past `n` map to 0 (never read).
fn remap_table(taken: &[u32], n: u32) -> Vec<u32> {
    let n_index = index(u64::from(n));
    let mut free = taken[..n_index]
        .iter()
        .enumerate()
        .filter(|(_, owner)| **owner == FREE)
        .map(|(s, _)| u32::try_from(s).expect("slot below 2^32"));
    taken[n_index..]
        .iter()
        .map(|owner| {
            if *owner == FREE {
                0
            } else {
                free.next().expect("as many holes below n as keys past n")
            }
        })
        .collect()
}

const FREE: u32 = u32::MAX;

struct Solver {
    slots: u32,
    /// Keys' low hash halves grouped by bucket.
    los: Vec<u64>,
    starts: Vec<u32>,
    pilot_hashes: [u64; 256],
    /// Owner bucket of each slot, or `FREE`.
    taken: Vec<u32>,
    /// One bit per slot, set when taken: what the search tests (a part's bitset is
    /// 4 KiB, its owners 130 KiB).
    used: Vec<u64>,
    pilots: Vec<u8>,
    recent: [u32; RECENT],
    recent_len: usize,
    recent_at: usize,
    evictions: u64,
    max_evictions: u64,
}

impl Solver {
    fn new(hashes: &[KeyHash], seed: u32, slots: u32, buckets: u32) -> Self {
        let nb = index(u64::from(buckets));
        let bucket_of: Vec<u32> = hashes
            .iter()
            .map(|h| u32::try_from(bucket(h.hi, seed, buckets)).expect("bucket below 2^32"))
            .collect();
        let mut starts = vec![0_u32; nb + 1];
        for &b in &bucket_of {
            starts[index(u64::from(b)) + 1] += 1;
        }
        for i in 0..nb {
            starts[i + 1] += starts[i];
        }
        let mut fill = starts.clone();
        let mut los = vec![0_u64; hashes.len()];
        for (h, &b) in hashes.iter().zip(&bucket_of) {
            let at = &mut fill[index(u64::from(b))];
            los[index(u64::from(*at))] = h.lo;
            *at += 1;
        }
        let mut pilot_hashes = [0_u64; 256];
        for (p, ph) in (0_u8..=255).zip(pilot_hashes.iter_mut()) {
            *ph = pilot_hash(p, seed);
        }
        let n = u64::try_from(hashes.len()).expect("length fits in u64");
        Self {
            slots,
            los,
            starts,
            pilot_hashes,
            taken: vec![FREE; index(u64::from(slots))],
            used: vec![0; index(u64::from(slots)).div_ceil(64)],
            pilots: vec![0; nb],
            recent: [FREE; RECENT],
            recent_len: (nb / 4).min(RECENT),
            recent_at: 0,
            evictions: 0,
            max_evictions: 20 * n + 10_000,
        }
    }

    fn keys(&self, b: u32) -> &[u64] {
        let b = index(u64::from(b));
        &self.los[index(u64::from(self.starts[b]))..index(u64::from(self.starts[b + 1]))]
    }

    fn size(&self, b: u32) -> u32 {
        let b = index(u64::from(b));
        self.starts[b + 1] - self.starts[b]
    }

    fn slot_of(&self, lo: u64, pilot: u8) -> usize {
        index(slot(lo, self.pilot_hashes[usize::from(pilot)], self.slots))
    }

    fn solve(mut self) -> Option<(Vec<u8>, Vec<u32>)> {
        // Buckets by decreasing size, then by index: a counting sort on the size.
        let nb = self.pilots.len();
        let max_size = (0..nb)
            .map(|b| self.size(u32::try_from(b).expect("bucket below 2^32")))
            .max()
            .unwrap_or(0);
        let mut by_size: Vec<Vec<u32>> = vec![Vec::new(); index(u64::from(max_size)) + 1];
        for b in 0..nb {
            let b = u32::try_from(b).expect("bucket below 2^32");
            by_size[index(u64::from(self.size(b)))].push(b);
        }
        let mut heap = BinaryHeap::new();
        let mut slots_buf = Vec::with_capacity(64);
        for group in by_size.iter().skip(1).rev() {
            for &b in group {
                heap.push((self.size(b), std::cmp::Reverse(b)));
                while let Some((_, std::cmp::Reverse(b))) = heap.pop() {
                    if !self.place(b, &mut heap, &mut slots_buf) {
                        return None;
                    }
                }
            }
        }
        Some((self.pilots, self.taken))
    }

    /// Places bucket `b`, evicting others if needed. False when the part must be retried
    /// with another seed.
    fn place(
        &mut self,
        b: u32,
        heap: &mut BinaryHeap<(u32, std::cmp::Reverse<u32>)>,
        buf: &mut Vec<usize>,
    ) -> bool {
        // A pilot whose slots are all free and distinct.
        for p in 0_u8..=255 {
            if self.fill_slots(b, p, buf) && buf.iter().all(|&s| !self.is_used(s)) {
                self.assign(b, p, buf);
                return true;
            }
        }
        // Otherwise the pilot that displaces the least, counting each displaced bucket's
        // size squared, and never one of the most recently placed buckets.
        let mut best: Option<(u64, u8)> = None;
        let mut owners: Vec<u32> = Vec::with_capacity(16);
        for p in 0_u8..=255 {
            if !self.fill_slots(b, p, buf) {
                continue;
            }
            owners.clear();
            let mut cost = 0_u64;
            let mut forbidden = false;
            for &s in buf.iter() {
                let owner = self.taken[s];
                if owner == FREE || owners.contains(&owner) {
                    continue;
                }
                if self.recent.contains(&owner) {
                    forbidden = true;
                    break;
                }
                owners.push(owner);
                cost += u64::from(self.size(owner)).pow(2);
            }
            if !forbidden && best.is_none_or(|(c, _)| cost < c) {
                best = Some((cost, p));
            }
        }
        let Some((_, p)) = best else {
            return false;
        };
        self.fill_slots(b, p, buf);
        owners.clear();
        for &s in buf.iter() {
            let owner = self.taken[s];
            if owner != FREE && !owners.contains(&owner) {
                owners.push(owner);
            }
        }
        for &owner in &owners {
            let pilot = self.pilots[index(u64::from(owner))];
            for i in 0..self.keys(owner).len() {
                let s = self.slot_of(self.keys(owner)[i], pilot);
                self.taken[s] = FREE;
                self.used[s / 64] &= !(1 << (s % 64));
            }
            heap.push((self.size(owner), std::cmp::Reverse(owner)));
        }
        self.assign(b, p, buf);
        self.evictions += u64::try_from(owners.len()).expect("small");
        self.evictions <= self.max_evictions
    }

    fn is_used(&self, s: usize) -> bool {
        self.used[s / 64] & (1 << (s % 64)) != 0
    }

    /// Computes the slots of bucket `b` under pilot `p` into `buf`; false if two of its
    /// keys share a slot.
    fn fill_slots(&self, b: u32, p: u8, buf: &mut Vec<usize>) -> bool {
        buf.clear();
        for &lo in self.keys(b) {
            let s = self.slot_of(lo, p);
            if buf.contains(&s) {
                return false;
            }
            buf.push(s);
        }
        true
    }

    fn assign(&mut self, b: u32, p: u8, buf: &[usize]) {
        for &s in buf {
            self.taken[s] = b;
            self.used[s / 64] |= 1 << (s % 64);
        }
        self.pilots[index(u64::from(b))] = p;
        if self.recent_len > 0 {
            self.recent[self.recent_at] = b;
            self.recent_at = (self.recent_at + 1) % self.recent_len;
        }
    }
}

/// The assembled function: its parts in hash order, each with its own pilots and
/// remap (no single allocation holds them all: at 2^32 keys the pilots alone are
/// 1.2 GB), and where each part's slots start.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Mphf {
    pub(crate) part_bits: u32,
    pub(crate) parts: Vec<Part>,
    pub(crate) slot_base: Vec<u32>,
}

impl Mphf {
    /// Global slot of `h`, in `0..n` for every key the function was built on; some slot
    /// in `0..n` for any other hash (callers verify).
    pub(crate) fn eval(&self, h: KeyHash) -> u32 {
        let q = top_bits(h.hi, self.part_bits);
        let part = &self.parts[q];
        if part.n == 0 {
            return 0;
        }
        self.slot_base[q] + part.eval(h)
    }

    /// Appends `part` as the next part of the function.
    pub(crate) fn push(&mut self, part: Part) {
        let base = match (self.slot_base.last(), self.parts.last()) {
            (Some(b), Some(p)) => b + p.n,
            _ => 0,
        };
        self.slot_base.push(base);
        self.parts.push(part);
    }

    /// The rows of the `ids.parts` section.
    pub(crate) fn rows(&self) -> Vec<PartRow> {
        let mut bucket_base = 0_u32;
        let mut remap_base = 0_u32;
        let as_u32 = |len: usize| u32::try_from(len).expect("fewer than 2^32 entries");
        self.parts
            .iter()
            .zip(&self.slot_base)
            .map(|(p, &slot_base)| {
                let row = PartRow {
                    slot_base,
                    n: p.n,
                    slots: p.slots,
                    bucket_base,
                    buckets: p.buckets,
                    remap_base,
                    seed: p.seed,
                };
                bucket_base += as_u32(p.pilots.len());
                remap_base += as_u32(p.remap.len());
                row
            })
            .collect()
    }

    /// Bytes this function occupies in memory.
    pub(crate) fn bytes(&self) -> u64 {
        let len = |l: usize| u64::try_from(l).expect("length fits in u64");
        self.parts.iter().map(Part::bytes).sum::<u64>() + len(self.slot_base.len() * 4)
    }
}

impl Part {
    /// Bytes this part holds in memory: its pilots, its remap and itself.
    pub(crate) fn bytes(&self) -> u64 {
        let len = |l: usize| u64::try_from(l).expect("length fits in u64");
        len(self.pilots.len() + 4 * self.remap.len() + std::mem::size_of::<Self>())
    }

    /// Bytes a solved part of `n` keys holds, before it is solved.
    pub(crate) fn bytes_for(n: u32) -> u64 {
        let (slots, buckets) = part_shape(n);
        u64::from(buckets)
            + 4 * u64::from(slots - n)
            + u64::try_from(std::mem::size_of::<Self>()).expect("small")
    }

    /// Working memory of [`build_part`] for `n` keys, at most: the sorted copy (16 B
    /// per key), bucket numbers (4), low halves (8), slot owners (4 per slot), the
    /// used-slot bitset, and per bucket its start, fill, pilot, size list and heap
    /// entry (4 + 4 + 1 + 4 + 8).
    pub(crate) fn solver_bytes(n: u32) -> u64 {
        let (slots, buckets) = part_shape(n);
        let n = u64::from(n);
        let (slots, buckets) = (u64::from(slots), u64::from(buckets));
        n * (16 + 4 + 8)
            + slots * 4
            + slots.div_ceil(64) * 8
            + (buckets + 1) * (4 + 4 + 1 + 4 + 8)
            + (1 << 12)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn hashes(keys: impl Iterator<Item = u64>, seed: u64) -> Vec<KeyHash> {
        keys.map(|k| KeyHash::of(&k.to_le_bytes(), seed)).collect()
    }

    fn assert_bijection(part: &Part, hs: &[KeyHash]) {
        let mut seen = vec![false; hs.len()];
        for &h in hs {
            let s = index(u64::from(part.eval(h)));
            assert!(s < hs.len(), "slot {s} out of 0..{}", hs.len());
            assert!(!seen[s], "slot {s} given twice");
            seen[s] = true;
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        /// Every key gets its own slot in 0..n, whatever the set.
        #[test]
        fn a_part_is_a_bijection_onto_0_to_n(n in 0_u64..3000, offset in any::<u64>(), seed in any::<u64>()) {
            let hs = hashes((0..n).map(|i| i.wrapping_add(offset)), seed);
            let part = build_part(&hs).unwrap();
            assert_bijection(&part, &hs);
            prop_assert_eq!(part.pilots.len(), index(u64::from(part.buckets)));
            prop_assert_eq!(part.remap.len(), index(u64::from(part.slots - part.n)));
        }

        /// The part depends on the set of hashes only.
        #[test]
        fn input_order_does_not_matter(n in 1_u64..2000, rotate in 0_usize..2000) {
            let hs = hashes(0..n, 7);
            let mut rotated = hs.clone();
            rotated.rotate_left(rotate % hs.len());
            rotated.reverse();
            prop_assert_eq!(build_part(&hs).unwrap(), build_part(&rotated).unwrap());
        }
    }

    /// A full-size part (2^15 keys) solves at the documented density.
    #[test]
    fn a_part_of_target_size_is_solved_at_about_2_6_bits_per_key() {
        let n = 1_u64 << PART_TARGET_BITS;
        let hs = hashes(0..n, 1);
        let part = build_part(&hs).unwrap();
        assert_bijection(&part, &hs);
        let bits = (part.pilots.len() + 4 * part.remap.len()) as f64 * 8.0 / n as f64;
        assert!(bits < 2.7, "{bits} bits per key");
    }

    /// Equal hashes are found wherever they are in the input (the sort brings them
    /// together).
    #[test]
    fn equal_hashes_are_a_collision_even_when_apart() {
        let h = KeyHash::of(b"x", 0);
        assert_eq!(build_part(&[h, h]), Err(PartError::Collision));
        let mut hs: Vec<KeyHash> = (0..1000_u64)
            .map(|i| KeyHash::of(&i.to_le_bytes(), 0))
            .collect();
        hs.push(hs[3]);
        hs.rotate_left(500);
        assert_eq!(build_part(&hs), Err(PartError::Collision));
    }

    /// Hash sets no pilot can separate end in `Unsolvable`, which the builder turns into
    /// a retry with the next seed (the review of PR #158 built them directly as hashes:
    /// real keys would need a partial XXH3-128 collision).
    #[test]
    fn inseparable_hash_sets_are_unsolvable() {
        let same_lo = [KeyHash { hi: 1, lo: 7 }, KeyHash { hi: 2, lo: 7 }];
        assert_eq!(build_part(&same_lo), Err(PartError::Unsolvable));
    }

    /// The solver's memory estimate covers what it allocates for a full part.
    #[test]
    fn solver_memory_is_bounded_by_its_estimate() {
        let n = 1_u32 << 15;
        let (slots, buckets) = part_shape(n);
        let measured = 16 * u64::from(n)
            + 4 * u64::from(n)
            + 8 * u64::from(n)
            + 4 * u64::from(slots)
            + u64::from(slots).div_ceil(64) * 8
            + (u64::from(buckets) + 1) * 9;
        assert!(Part::solver_bytes(n) >= measured);
        let part = build_part(&hashes(0..u64::from(n), 5)).unwrap();
        assert_eq!(part.bytes(), Part::bytes_for(n));
    }

    #[test]
    fn the_assembled_function_is_minimal_across_parts() {
        let n = 50_000_u64;
        let hs = hashes(0..n, 3);
        let bits = 3;
        let mut by_part: Vec<Vec<KeyHash>> = vec![Vec::new(); 1 << bits];
        for &h in &hs {
            by_part[top_bits(h.hi, bits)].push(h);
        }
        let mut f = Mphf {
            part_bits: bits,
            ..Mphf::default()
        };
        for p in &by_part {
            f.push(build_part(p).unwrap());
        }
        let mut seen = vec![false; hs.len()];
        for &h in &hs {
            let s = index(u64::from(f.eval(h)));
            assert!(!seen[s]);
            seen[s] = true;
        }
        let rows = f.rows();
        let mut encoded = Vec::new();
        rows[5].encode(&mut encoded);
        assert_eq!(PartRow::decode(&encoded), rows[5]);
        assert_eq!(rows[5].slot_base, f.slot_base[5]);
    }

    #[test]
    fn part_counts_follow_the_number_of_keys() {
        assert_eq!(part_bits_for(0), 0);
        assert_eq!(part_bits_for(1 << 15), 0);
        assert_eq!(part_bits_for((1 << 15) + 1), 1);
        assert_eq!(part_bits_for(500_000_000), 14);
        assert_eq!(part_bits_for(u64::from(u32::MAX)), 16);
        assert_eq!(part_shape(1), (2, 1));
        assert_eq!(part_shape(700), (707, 200));
    }
}
