// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Little-endian fields of a byte slice. Each reader panics when the field does not fit
//! in the slice: callers check the length of a record before they read its fields.

/// The `u16` at `at`.
#[must_use]
pub fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(array(b, at))
}

/// The `u32` at `at`.
#[must_use]
pub fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(array(b, at))
}

/// The `u64` at `at`.
#[must_use]
pub fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(array(b, at))
}

/// The `N` bytes at `at`.
#[must_use]
pub fn array<const N: usize>(b: &[u8], at: usize) -> [u8; N] {
    let mut out = [0; N];
    out.copy_from_slice(&b[at..at + N]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_are_little_endian_at_their_offset() {
        let b = [9, 1, 2, 3, 4, 5, 6, 7, 8];
        assert_eq!(u16_at(&b, 1), 0x0201);
        assert_eq!(u32_at(&b, 1), 0x0403_0201);
        assert_eq!(u64_at(&b, 1), 0x0807_0605_0403_0201);
        assert_eq!(array::<2>(&b, 7), [7, 8]);
    }

    #[test]
    #[should_panic(expected = "range end index")]
    fn a_field_past_the_end_panics() {
        let _ = u32_at(&[0; 4], 1);
    }
}
