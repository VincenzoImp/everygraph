// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Adjacency records: Stream VByte (Lemire, Kurz and Rupp, 2018) of zigzag-coded first
//! neighbours and gaps (`docs/spec/eg-adjacency.md`, "Records").
//!
//! A record of `d ≥ 1` neighbours `x_0 < … < x_{d-1}` of node `v` codes the words
//! `w_0 = zigzag32(x_0 − v)` and `w_j = x_j − x_{j−1} − 1`, all in 32-bit wrapping
//! arithmetic, as ⌈d/4⌉ control bytes followed by the data bytes: word `j` takes
//! `1 + ((control[j / 4] >> 2·(j mod 4)) & 3)` little-endian bytes, the fewest that hold
//! it. Unused fields of the last control byte are zero.
//!
//! This module holds the encoder and the scalar decoder. The scalar decoder is the
//! reference: every SIMD decoder in `simd` must return exactly what it returns, for any
//! input, valid or not.

/// Bytes the shortest code of `w` takes (1 to 4).
#[inline]
fn word_len(w: u32) -> usize {
    match w {
        0..=0xff => 1,
        0x100..=0xffff => 2,
        0x1_0000..=0xff_ffff => 3,
        _ => 4,
    }
}

/// `zigzag32(x − v)`: the wrapping difference read as an `i32`, so any two ids give a
/// word, and ids close to `v` a small one.
#[inline]
fn first_word(v: u32, x: u32) -> u32 {
    let d = x.wrapping_sub(v);
    (d << 1) ^ (0u32.wrapping_sub(d >> 31))
}

/// Inverse of [`first_word`].
#[inline]
pub(crate) fn first_neighbour(v: u32, w: u32) -> u32 {
    v.wrapping_add((w >> 1) ^ 0u32.wrapping_sub(w & 1))
}

/// The words of a record, in order, or the index of the first entry that is not above
/// its predecessor.
fn words(v: u32, list: &[u32]) -> impl Iterator<Item = Result<u32, usize>> + '_ {
    list.iter().enumerate().map(move |(j, &x)| match j {
        0 => Ok(first_word(v, x)),
        _ if x > list[j - 1] => Ok(x - list[j - 1] - 1),
        _ => Err(j),
    })
}

/// Byte length of the record of `list` (0 for an empty list), or the index of the first
/// entry that is not strictly above its predecessor.
pub(crate) fn record_len(v: u32, list: &[u32]) -> Result<usize, usize> {
    let mut len = list.len().div_ceil(4);
    for w in words(v, list) {
        len += word_len(w?);
    }
    Ok(len)
}

/// Writes the record of `list`, which must be strictly increasing, into `out`, whose
/// length must be [`record_len`].
pub(crate) fn write_record(v: u32, list: &[u32], out: &mut [u8]) {
    let (control, mut data) = out.split_at_mut(list.len().div_ceil(4));
    control.fill(0);
    for (j, w) in words(v, list).enumerate() {
        let w = w.expect("checked by record_len");
        let len = word_len(w);
        let code = u8::try_from(len - 1).expect("1 to 4 bytes");
        control[j / 4] |= code << (2 * (j % 4));
        let (head, rest) = data.split_at_mut(len);
        head.copy_from_slice(&w.to_le_bytes()[..len]);
        data = rest;
    }
    debug_assert!(data.is_empty(), "out must be exactly record_len bytes");
}

/// Number of data bytes behind each control byte.
pub(crate) static GROUP_LEN: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut c = 0;
    while c < 256 {
        // At most 16 (four 4-byte words): no truncation.
        #[expect(clippy::cast_possible_truncation, reason = "values are 4 to 16")]
        {
            t[c] = (4 + (c & 3) + ((c >> 2) & 3) + ((c >> 4) & 3) + ((c >> 6) & 3)) as u8;
        }
        c += 1;
    }
    t
};

/// The record is not what the index says (too short, or the wrong length).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Corrupt;

/// What every decoder does before its own loop.
///
/// `bytes` starts at the record and may run on past it (decoders read ahead within it,
/// never beyond); the record has `total ≥ 1` words, of which the first `k`, with
/// `1 ≤ k ≤ total`, are wanted. Returns the number of control bytes and the running
/// value before the first word, chosen so that adding `w_0 + 1` to it gives `x_0`.
/// Unused fields of the last control byte are written as zero and never read.
#[inline]
pub(crate) fn prologue(
    bytes: &[u8],
    v: u32,
    total: usize,
    k: usize,
) -> Result<(usize, u32), Corrupt> {
    debug_assert!(1 <= k && k <= total);
    let groups = total.div_ceil(4);
    if bytes.len() < groups {
        return Err(Corrupt);
    }
    let w0 = read_word(bytes, groups, bytes[0] & 3)?;
    let x0 = first_neighbour(v, w0);
    Ok((groups, x0.wrapping_sub(w0).wrapping_sub(1)))
}

#[inline]
fn read_word(bytes: &[u8], at: usize, code: u8) -> Result<u32, Corrupt> {
    // Four bytes and a mask where they are there (no copy of a variable length).
    if let Some(&four) = bytes.get(at..).and_then(|b| b.first_chunk::<4>()) {
        let unused = 8 * (3 - u32::from(code));
        return Ok((u32::from_le_bytes(four) << unused) >> unused);
    }
    let src = bytes.get(at..at + usize::from(code) + 1).ok_or(Corrupt)?;
    Ok(src.iter().rev().fold(0, |w, &b| (w << 8) | u32::from(b)))
}

/// Decodes words `from..k` one at a time, the data of word `from` starting at byte
/// `at`, after the running value `acc`. Returns the byte just past the last word read.
/// The SIMD decoders call this for what their loop leaves over.
#[inline]
pub(crate) fn tail(
    bytes: &[u8],
    from: usize,
    mut at: usize,
    mut acc: u32,
    k: usize,
    out: &mut [u32],
) -> Result<usize, Corrupt> {
    for j in from..k {
        let code = (bytes[j / 4] >> (2 * (j % 4))) & 3;
        let w = read_word(bytes, at, code)?;
        acc = acc.wrapping_add(w).wrapping_add(1);
        out[j] = acc;
        at += usize::from(code) + 1;
    }
    Ok(at)
}

/// The scalar decoder: the first `k` neighbours of a record of `total` words into
/// `out[..k]`. Returns the record's byte length when `k == total` (a smaller number,
/// of no use, otherwise).
pub(crate) fn decode(
    bytes: &[u8],
    v: u32,
    total: usize,
    k: usize,
    out: &mut [u32],
) -> Result<usize, Corrupt> {
    let (groups, base) = prologue(bytes, v, total, k)?;
    tail(bytes, 0, groups, base, k, out)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn zigzag_of_the_wrapping_difference_covers_every_pair() {
        for (v, x) in [
            (0, 0),
            (0, u32::MAX),
            (u32::MAX, 0),
            (5, 3),
            (3, 5),
            (1 << 31, 0),
        ] {
            assert_eq!(first_neighbour(v, first_word(v, x)), x, "v {v} x {x}");
        }
        assert_eq!(first_word(10, 10), 0);
        assert_eq!(first_word(10, 9), 1);
        assert_eq!(first_word(10, 11), 2);
        // Byte-identical to the lab's zigzag of the 64-bit difference wherever that fits
        // 32 bits (lab/m1-13-compressed/src/store.rs).
        let lab = |v: i64, x: i64| u32::try_from(((x - v) << 1) ^ ((x - v) >> 63)).unwrap();
        for (v, x) in [(0i64, 1i64), (100, 3), (1 << 30, 0), (0, (1 << 31) - 1)] {
            let (v32, x32) = (u32::try_from(v).unwrap(), u32::try_from(x).unwrap());
            assert_eq!(first_word(v32, x32), lab(v, x));
        }
    }

    /// The worked example of `docs/spec/eg-adjacency.md` § 1, byte for byte.
    #[test]
    fn the_example_of_the_specification() {
        let list = [3, 4, 300];
        assert_eq!(record_len(5, &list), Ok(5));
        let mut rec = [0xaa; 5];
        write_record(5, &list, &mut rec);
        assert_eq!(rec, [0x10, 0x03, 0x00, 0x27, 0x01]);
        let mut out = [0; 3];
        assert_eq!(decode(&rec, 5, 3, 3, &mut out), Ok(5));
        assert_eq!(out, list);
    }

    #[test]
    fn group_lengths() {
        assert_eq!(GROUP_LEN[0], 4);
        assert_eq!(GROUP_LEN[0xff], 16);
        assert_eq!(GROUP_LEN[0b11_10_01_00], 10);
    }

    proptest! {
        #![proptest_config(crate::adjacency::test_config(8, 512))]

        #[test]
        fn records_round_trip(v in any::<u32>(), mut list in proptest::collection::vec(any::<u32>(), 1..40)) {
            list.sort_unstable();
            list.dedup();
            let len = record_len(v, &list).unwrap();
            let mut rec = vec![0u8; len];
            write_record(v, &list, &mut rec);
            let d = list.len();
            let mut out = vec![0u32; d];
            prop_assert_eq!(decode(&rec, v, d, d, &mut out), Ok(len));
            prop_assert_eq!(&out, &list);
            for k in 1..d {
                let mut out = vec![0u32; k];
                decode(&rec, v, d, k, &mut out).unwrap();
                prop_assert_eq!(&out[..], &list[..k]);
            }
            // Any shorter slice is too short.
            prop_assert_eq!(decode(&rec[..len - 1], v, d, d, &mut vec![0; d]), Err(Corrupt));
        }

        #[test]
        fn unsorted_lists_are_refused(v in any::<u32>(), list in proptest::collection::vec(0u32..8, 2..10)) {
            let sorted = list.windows(2).all(|w| w[0] < w[1]);
            prop_assert_eq!(record_len(v, &list).is_ok(), sorted);
        }
    }
}
