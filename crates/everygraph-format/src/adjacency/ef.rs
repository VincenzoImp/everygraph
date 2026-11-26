// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Elias–Fano coding of a non-decreasing sequence of `u64`, as `docs/spec/eg-adjacency.md`
//! § 2 defines it. Own implementation, safe code only, so that it builds for wasm32 and
//! runs under Miri. Select samples (the position of every 64th one) are rebuilt in
//! memory, never stored.

use everygraph_core::{usize_of, widen};

/// Ones between two select samples.
const SAMPLE: usize = 64;

/// The number of low bits for `len` values up to `universe` (`len ≥ 1`).
pub(crate) fn low_bits(len: u64, universe: u64) -> u32 {
    let quotient = universe / len.max(1);
    if quotient < 2 {
        0
    } else {
        63 - quotient.leading_zeros()
    }
}

/// Words of the low and of the high part of a sequence, or `None` if they would not
/// fit in memory on this target.
pub(crate) fn stored_words(len: u64, universe: u64) -> Option<(usize, usize)> {
    let l = low_bits(len, universe);
    let low = (len.checked_mul(u64::from(l))?).div_ceil(64);
    let high = len.checked_add(universe >> l)?.div_ceil(64);
    Some((usize::try_from(low).ok()?, usize::try_from(high).ok()?))
}

/// An Elias–Fano sequence in memory.
#[derive(Clone, Debug)]
pub(crate) struct EliasFano {
    len: usize,
    l: u32,
    /// The stored low words and one zero word, so that a value may straddle two words.
    low: Vec<u64>,
    /// The stored high words and one zero word.
    high: Vec<u64>,
    /// `samples[k]`: bit position in `high` of the one of value `k · SAMPLE`.
    samples: Vec<usize>,
}

impl EliasFano {
    /// Encodes `values`, which must be non-empty and non-decreasing; the universe is the
    /// last value.
    pub(crate) fn new(values: &[u64]) -> Self {
        debug_assert!(
            values.windows(2).all(|w| w[0] <= w[1]),
            "values must be non-decreasing"
        );
        Self::with_universe(values, values.last().copied().unwrap_or(0))
    }

    /// The bits of `values` with the given universe, without checking that the values
    /// are non-decreasing or within it (tests build damaged sequences with it). Values
    /// whose high positions `(x >> l) + i` collide lose a one.
    pub(crate) fn with_universe(values: &[u64], universe: u64) -> Self {
        Self::from_values(values.len(), universe, values.iter().copied())
            .expect("the values themselves are in memory")
    }

    /// The sequence of the `len` values that `values` yields, for a universe known in
    /// advance, so that the values need not be held in memory; `None` if the parts
    /// cannot be allocated. `values` must yield at least `len` values, non-decreasing
    /// and at most `universe`; they are not checked, and those beyond the first `len`
    /// are ignored.
    pub(crate) fn from_values(
        len: usize,
        universe: u64,
        values: impl IntoIterator<Item = u64>,
    ) -> Option<Self> {
        let len64 = widen(len);
        let l = low_bits(len64, universe);
        let (low_words, high_words) = stored_words(len64, universe)?;
        let zeros = |words: usize| {
            let mut v = Vec::new();
            v.try_reserve_exact(words.checked_add(1)?).ok()?;
            v.resize(words + 1, 0u64);
            Some(v)
        };
        let (mut low, mut high) = (zeros(low_words)?, zeros(high_words)?);
        let mask = low_mask(l);
        let mut bit = 0usize;
        for (i, x) in values.into_iter().take(len).enumerate() {
            if l > 0 {
                let (word, shift) = (bit / 64, bit % 64);
                let v = x & mask;
                low[word] |= v << shift;
                if shift + usize_of(l) > 64 {
                    low[word + 1] |= v >> (64 - shift);
                }
                bit += usize_of(l);
            }
            let h = usize::try_from(x >> l).expect("fits: bounded by the high words") + i;
            high[h / 64] |= 1 << (h % 64);
        }
        let samples = build_samples(&high, len);
        Some(Self {
            len,
            l,
            low,
            high,
            samples,
        })
    }

    /// Bytes [`write`](Self::write) produces for `len` values up to `universe`, or
    /// `None` if they would not fit in memory here.
    pub(crate) fn stored_bytes(len: u64, universe: u64) -> Option<u64> {
        let (low, high) = stored_words(len, universe)?;
        widen(low.checked_add(high)?).checked_mul(8)
    }

    /// Reads a sequence of `len` values up to `universe` from exactly the bytes that
    /// [`write`](Self::write) produces. Checks the structure (bit counts, zero padding),
    /// not the values.
    pub(crate) fn read(bytes: &[u8], len: u64, universe: u64) -> Result<Self, &'static str> {
        let (low_words, high_words) = stored_words(len, universe).ok_or("too large")?;
        if bytes.len() != (low_words + high_words) * 8 {
            return Err("Elias–Fano part has the wrong length");
        }
        let l = low_bits(len, universe);
        // With the room for the last word made first: a push past `collect`'s exact
        // capacity would double it, and the index would hold twice its `heap_bytes`.
        let words = |b: &[u8]| -> Vec<u64> {
            let mut v = Vec::with_capacity(b.len() / 8 + 1);
            v.extend(
                b.chunks_exact(8)
                    .map(|c| u64::from_le_bytes(c.try_into().expect("chunks of 8"))),
            );
            v.push(0);
            v
        };
        let (low_bytes, high_bytes) = bytes.split_at(low_words * 8);
        let (low, high) = (words(low_bytes), words(high_bytes));
        let len_usize = usize::try_from(len).map_err(|_| "too large")?;
        let low_used = len * u64::from(l);
        let high_used = len + (universe >> l);
        if !tail_is_zero(&low, low_used) || !tail_is_zero(&high, high_used) {
            return Err("Elias–Fano padding bits are not zero");
        }
        let ones: u64 = high.iter().map(|w| u64::from(w.count_ones())).sum();
        if ones != len {
            return Err("Elias–Fano high part has the wrong number of ones");
        }
        let samples = build_samples(&high, len_usize);
        Ok(Self {
            len: len_usize,
            l,
            low,
            high,
            samples,
        })
    }

    /// Appends the stored form: the low words, then the high words, little-endian.
    pub(crate) fn write(&self, out: &mut Vec<u8>) {
        for w in &self.low[..self.low.len() - 1] {
            out.extend_from_slice(&w.to_le_bytes());
        }
        for w in &self.high[..self.high.len() - 1] {
            out.extend_from_slice(&w.to_le_bytes());
        }
    }

    /// Number of values.
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    /// Bytes held in memory, select samples included.
    pub(crate) fn heap_bytes(&self) -> usize {
        (self.low.len() + self.high.len()) * 8 + self.samples.len() * size_of::<usize>()
    }

    /// Value `i`.
    ///
    /// # Panics
    ///
    /// If `i >= len`.
    pub(crate) fn get(&self, i: usize) -> u64 {
        assert!(
            i < self.len,
            "Elias–Fano index {i} out of range {}",
            self.len
        );
        self.value(i, self.select(i))
    }

    /// Values `i` and `i + 1`.
    ///
    /// # Panics
    ///
    /// If `i + 1 >= len`.
    pub(crate) fn get_pair(&self, i: usize) -> (u64, u64) {
        let mut it = self.iter_from(i);
        let a = it.next();
        let b = it.next();
        a.zip(b).expect("Elias–Fano pair out of range")
    }

    /// Calls `f` with values `i`, `i + 1`, … in order, until it returns `false` or the
    /// values run out. One tight loop, for the batch decoder's first pass (cheaper than
    /// an iterator per value when two sequences are walked).
    #[inline(always)]
    pub(crate) fn scan_from(&self, i: usize, mut f: impl FnMut(u64) -> bool) {
        if i >= self.len {
            return;
        }
        let p = self.select(i);
        let mut word = p / 64;
        let mut bits = self.high[word] & (u64::MAX << (p % 64));
        let l = self.l;
        let mask = low_mask(l);
        // The low bits as a stream: `cur` holds the next `avail` of them, and a word is
        // read only when it runs out (one bounds check per 64 bits, not per value).
        let bit = i * usize_of(l);
        let mut next = bit / 64 + 1;
        let shift = u32::try_from(bit % 64).expect("below 64");
        let mut cur = self.low[next - 1] >> shift;
        let mut avail = 64 - shift;
        for j in i..self.len {
            while bits == 0 {
                word += 1;
                bits = self.high[word];
            }
            let one_at = word * 64 + usize_of(bits.trailing_zeros());
            bits &= bits - 1;
            let mut x = widen(one_at - j) << l;
            if l > 0 {
                if avail >= l {
                    x |= cur & mask;
                    cur = cur.checked_shr(l).unwrap_or(0);
                    avail -= l;
                } else {
                    // `avail < l < 64`: the value straddles two words.
                    let fresh = self.low[next];
                    next += 1;
                    x |= (cur | (fresh << avail)) & mask;
                    cur = fresh >> (l - avail);
                    avail = 64 - (l - avail);
                }
            }
            if !f(x) {
                return;
            }
        }
    }

    /// The values from `i` on.
    pub(crate) fn iter_from(&self, i: usize) -> Iter<'_> {
        if i >= self.len {
            return Iter {
                ef: self,
                i: self.len,
                word: 0,
                bits: 0,
            };
        }
        let p = self.select(i);
        Iter {
            ef: self,
            i,
            word: p / 64,
            bits: self.high[p / 64] & (u64::MAX << (p % 64)),
        }
    }

    #[inline(always)]
    fn value(&self, i: usize, one_at: usize) -> u64 {
        let high = widen(one_at - i) << self.l;
        if self.l == 0 {
            return high;
        }
        let bit = i * usize_of(self.l);
        let (word, shift) = (bit / 64, bit % 64);
        let pair = u128::from(self.low[word]) | (u128::from(self.low[word + 1]) << 64);
        // Truncation intended: the low `l < 64` bits of the shifted pair.
        #[expect(clippy::cast_possible_truncation, reason = "masked to l < 64 bits")]
        let low = (pair >> shift) as u64 & low_mask(self.l);
        high | low
    }

    /// Bit position of the one of value `i < len`.
    fn select(&self, i: usize) -> usize {
        let start = self.samples[i / SAMPLE];
        let mut rank = i % SAMPLE;
        let mut word = start / 64;
        let mut bits = self.high[word] & (u64::MAX << (start % 64));
        loop {
            let ones = usize_of(bits.count_ones());
            if rank < ones {
                return word * 64 + select_in_word(bits, rank);
            }
            rank -= ones;
            word += 1;
            bits = self.high[word];
        }
    }
}

/// Sequential access; each value costs a few instructions.
#[derive(Clone, Debug)]
pub(crate) struct Iter<'a> {
    ef: &'a EliasFano,
    i: usize,
    word: usize,
    bits: u64,
}

impl Iterator for Iter<'_> {
    type Item = u64;

    #[inline(always)]
    fn next(&mut self) -> Option<u64> {
        if self.i >= self.ef.len {
            return None;
        }
        while self.bits == 0 {
            self.word += 1;
            self.bits = self.ef.high[self.word];
        }
        let p = self.word * 64 + usize_of(self.bits.trailing_zeros());
        self.bits &= self.bits - 1;
        let v = self.ef.value(self.i, p);
        self.i += 1;
        Some(v)
    }
}

fn build_samples(high: &[u64], len: usize) -> Vec<usize> {
    let mut samples = Vec::with_capacity(len.div_ceil(SAMPLE));
    let mut seen = 0usize;
    for (word, &bits) in high.iter().enumerate() {
        let ones = usize_of(bits.count_ones());
        let mut next = seen.div_ceil(SAMPLE) * SAMPLE;
        while next < seen + ones {
            samples.push(word * 64 + select_in_word(bits, next - seen));
            next += SAMPLE;
        }
        seen += ones;
    }
    samples
}

/// Position of the one of rank `r` (from 0) in `w`, which has more than `r` ones.
fn select_in_word(w: u64, r: usize) -> usize {
    // Ones per byte, then their running sums: byte k of `running` counts bytes 0..=k.
    let mut x = w - ((w >> 1) & 0x5555_5555_5555_5555);
    x = (x & 0x3333_3333_3333_3333) + ((x >> 2) & 0x3333_3333_3333_3333);
    x = (x + (x >> 4)) & 0x0f0f_0f0f_0f0f_0f0f;
    let running = x.wrapping_mul(0x0101_0101_0101_0101);
    let byte_sum = |k: usize| usize::from(running.to_le_bytes()[k]);
    let mut k = 0;
    while k < 7 && byte_sum(k) <= r {
        k += 1;
    }
    let before = if k == 0 { 0 } else { byte_sum(k - 1) };
    let mut byte = w.to_le_bytes()[k];
    for _ in before..r {
        byte &= byte - 1;
    }
    8 * k + usize_of(byte.trailing_zeros())
}

/// True if every bit of `words` from bit `used` on is zero.
fn tail_is_zero(words: &[u64], used: u64) -> bool {
    let Ok(used) = usize::try_from(used) else {
        return false;
    };
    let (word, shift) = (used / 64, used % 64);
    let Some(first) = words.get(word) else {
        return used == words.len() * 64;
    };
    first >> shift == 0 && words[word + 1..].iter().all(|&w| w == 0)
}

fn low_mask(l: u32) -> u64 {
    if l == 0 { 0 } else { u64::MAX >> (64 - l) }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn sequence() -> impl Strategy<Value = Vec<u64>> {
        // Steps from 0 to huge, so that l ranges from 0 to about 52.
        (
            proptest::collection::vec(0u64..4, 1..300),
            0u32..52,
            proptest::bool::ANY,
        )
            .prop_map(|(steps, scale, start_zero)| {
                let mut acc = if start_zero { 0 } else { 1u64 << scale };
                steps
                    .into_iter()
                    .map(|s| {
                        acc += s << scale;
                        acc
                    })
                    .collect()
            })
    }

    proptest! {
        #![proptest_config(crate::adjacency::test_config(4, 256))]

        #[test]
        fn every_access_path_returns_the_values(values in sequence()) {
            let ef = EliasFano::new(&values);
            let mut bytes = Vec::new();
            ef.write(&mut bytes);
            let universe = *values.last().unwrap();
            let len = values.len() as u64;
            let back = EliasFano::read(&bytes, len, universe).unwrap();
            for ef in [&ef, &back] {
                prop_assert_eq!(ef.iter_from(0).collect::<Vec<_>>(), values.clone());
                for (i, &x) in values.iter().enumerate() {
                    prop_assert_eq!(ef.get(i), x);
                    prop_assert_eq!(ef.iter_from(i).next(), Some(x));
                    if i + 1 < values.len() {
                        prop_assert_eq!(ef.get_pair(i), (x, values[i + 1]));
                    }
                }
                prop_assert_eq!(ef.iter_from(values.len()).next(), None);
                for start in [0, values.len() / 2, values.len()] {
                    let mut all = Vec::new();
                    ef.scan_from(start, |x| {
                        all.push(x);
                        true
                    });
                    prop_assert_eq!(&all[..], &values[start..]);
                    let mut two = Vec::new();
                    ef.scan_from(start, |x| {
                        two.push(x);
                        two.len() < 2
                    });
                    prop_assert_eq!(&two[..], &values[start..(start + 2).min(values.len())]);
                }
            }
        }

        #[test]
        fn read_rejects_every_single_bit_flip_or_accepts_a_valid_sequence(
            values in sequence(), flip in any::<prop::sample::Index>()
        ) {
            let ef = EliasFano::new(&values);
            let mut bytes = Vec::new();
            ef.write(&mut bytes);
            let universe = *values.last().unwrap();
            let len = values.len() as u64;
            if bytes.is_empty() {
                return Ok(());
            }
            let bit = flip.index(bytes.len() * 8);
            bytes[bit / 8] ^= 1 << (bit % 8);
            // A flipped low bit is a different, structurally valid sequence; a flipped
            // high or padding bit changes the count of ones or a padding bit.
            if let Ok(back) = EliasFano::read(&bytes, len, universe) {
                prop_assert_eq!(back.iter_from(0).count(), values.len());
            }
            prop_assert!(EliasFano::read(&bytes[..bytes.len() - 1], len, universe).is_err());
        }
    }

    #[test]
    fn select_in_word_matches_the_slow_way() {
        for w in [1u64, 0x8000_0000_0000_0000, u64::MAX, 0xF0F0_0000_0101_8001] {
            let ones: Vec<usize> = (0..64).filter(|b| w >> b & 1 == 1).collect();
            for (r, &pos) in ones.iter().enumerate() {
                assert_eq!(select_in_word(w, r), pos, "w {w:#x} r {r}");
            }
        }
    }

    #[test]
    fn low_bits_is_floor_log2_of_the_mean_gap() {
        assert_eq!(low_bits(1, 0), 0);
        assert_eq!(low_bits(10, 19), 0);
        assert_eq!(low_bits(10, 20), 1);
        assert_eq!(low_bits(10, 39), 1);
        assert_eq!(low_bits(10, 40), 2);
        assert_eq!(low_bits(1, u64::MAX), 63);
    }
}
