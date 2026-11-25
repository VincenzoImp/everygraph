// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Key typing: what the identifiers of a column look like, decided from a sample, and
//! the normalized bytes each key is stored, hashed and compared as.

use std::fmt;

/// Longest normalized key of a fixed-width kind, in bytes.
pub const MAX_FIXED_WIDTH: usize = 64;

/// The Arrow type of the input column the identifiers come from. Results carry
/// identifiers back in this type (`docs/api-sketch.md` § 2.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ColumnType {
    /// A signed integer column of `bits` bits (8, 16, 32 or 64).
    Int {
        /// Width of the column's integers.
        bits: u8,
    },
    /// An unsigned integer column of `bits` bits (8, 16, 32 or 64).
    UInt {
        /// Width of the column's integers.
        bits: u8,
    },
    /// A text column (`utf8`). Identifiers come back as text.
    Utf8,
    /// A binary column. Identifiers come back as bytes.
    Binary,
}

impl ColumnType {
    fn code(self) -> (u8, u8) {
        match self {
            Self::Int { bits } => (1, bits),
            Self::UInt { bits } => (2, bits),
            Self::Utf8 => (3, 0),
            Self::Binary => (4, 0),
        }
    }

    pub(crate) fn encode(self) -> [u8; 2] {
        let (a, b) = self.code();
        [a, b]
    }

    pub(crate) fn decode(bytes: [u8; 2]) -> Option<Self> {
        let valid_bits = matches!(bytes[1], 8 | 16 | 32 | 64);
        match bytes {
            [1, bits] if valid_bits => Some(Self::Int { bits }),
            [2, bits] if valid_bits => Some(Self::UInt { bits }),
            [3, 0] => Some(Self::Utf8),
            [4, 0] => Some(Self::Binary),
            _ => None,
        }
    }

    fn is_text(self) -> bool {
        matches!(self, Self::Utf8 | Self::Binary)
    }
}

/// One identifier as read from the input, or as given to a look-up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key<'a> {
    /// A signed integer.
    Int(i64),
    /// An unsigned integer.
    UInt(u64),
    /// Text (UTF-8) or binary bytes.
    Bytes(&'a [u8]),
}

impl<'a> From<&'a str> for Key<'a> {
    fn from(text: &'a str) -> Self {
        Self::Bytes(text.as_bytes())
    }
}

impl<'a> From<&'a [u8]> for Key<'a> {
    fn from(bytes: &'a [u8]) -> Self {
        Self::Bytes(bytes)
    }
}

/// An identifier given back by the dictionary, in the type of the input column.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum OwnedKey {
    /// From a signed integer column.
    Int(i64),
    /// From an unsigned integer column.
    UInt(u64),
    /// From a text column: hexadecimal and UUIDs in lowercase, with the input's prefix.
    Text(String),
    /// From a binary column.
    Bytes(Vec<u8>),
}

impl fmt::Display for OwnedKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Int(v) => v.fmt(f),
            Self::UInt(v) => v.fmt(f),
            Self::Text(t) => t.fmt(f),
            Self::Bytes(b) => {
                for byte in b {
                    write!(f, "{byte:02x}")?;
                }
                Ok(())
            }
        }
    }
}

/// The prefix of hexadecimal identifiers: the one the input used, kept for output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HexPrefix {
    /// No prefix (`deadbeef…`).
    None,
    /// `0x`.
    Lower,
    /// `0X`.
    Upper,
}

impl HexPrefix {
    fn text(self) -> &'static [u8] {
        match self {
            Self::None => b"",
            Self::Lower => b"0x",
            Self::Upper => b"0X",
        }
    }

    fn of(text: &[u8]) -> Self {
        match text {
            [b'0', b'x', ..] => Self::Lower,
            [b'0', b'X', ..] => Self::Upper,
            _ => Self::None,
        }
    }
}

/// How the identifiers of a column are represented: chosen by [`KeyKind::detect`] from
/// a sample and checked on every key while the dictionary is built.
///
/// Every kind but [`KeyKind::Bytes`] stores its keys as fixed-width bytes, so the key
/// store needs no offsets. The normalized bytes compare in the order "by sorted key"
/// uses: numeric for integers, the order of the lowercase text for hexadecimal and
/// UUIDs, the order of the decoded bytes for base58, bytewise otherwise.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum KeyKind {
    /// Integers, from an integer column or as canonical decimal text (`0`, `17`, never
    /// `017` or `+17`; text never negative). Stored as 8 bytes. `dense` when every key is
    /// in `0..2^32` (counted from the smallest value of the column's type for signed
    /// columns' non-negative values): then, in sorted order, the dictionary is a rank
    /// bitmap.
    Integers {
        /// Every key is in `0..2^32`.
        dense: bool,
    },
    /// Hexadecimal text with a fixed number of digits and one prefix for all keys,
    /// matched without regard to case and given back in lowercase. Recognized only with
    /// a `0x`/`0X` prefix or at least 16 digits, so that ordinary words (`cafe`) are not
    /// taken for numbers.
    Hex {
        /// The prefix the input used.
        prefix: HexPrefix,
        /// Number of hexadecimal digits (1 to 128).
        digits: u8,
    },
    /// UUIDs in the canonical 8-4-4-4-12 form, any case; given back in lowercase.
    Uuid,
    /// Base58 text (the Bitcoin alphabet) of at least 20 characters whose decoded bytes
    /// all have the same length. Case-sensitive.
    Base58 {
        /// Length of the decoded bytes.
        width: u8,
    },
    /// Keys that all have the same length in bytes (1 to 64), stored as they are.
    Fixed {
        /// Length of every key.
        width: u8,
    },
    /// Anything else: arbitrary text or bytes, stored as they are with offsets.
    Bytes,
}

/// Number of keys [`KeyKind::detect`] needs to see; more are ignored.
pub const SAMPLE_SIZE: usize = 4096;

const BASE58: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

impl KeyKind {
    /// Chooses the kind of a column from a sample of its keys: the first
    /// [`SAMPLE_SIZE`] keys of the input in input order, so that the choice is the same
    /// at any thread count. A key that the full pass finds not to fit is reported by the
    /// builder, which then retries with [`KeyKind::widen`].
    pub fn detect<'a>(column: ColumnType, sample: impl IntoIterator<Item = Key<'a>>) -> Self {
        let sample: Vec<Key<'a>> = sample.into_iter().take(SAMPLE_SIZE).collect();
        let fits =
            |kind: Self| !sample.is_empty() && sample.iter().all(|k| kind.accepts(column, *k));
        if !column.is_text() {
            let dense = Self::Integers { dense: true };
            return if sample.iter().all(|k| dense.accepts(column, *k)) {
                dense
            } else {
                Self::Integers { dense: false }
            };
        }
        let Some(Key::Bytes(first)) = sample.first().copied() else {
            return Self::Bytes;
        };
        if column == ColumnType::Utf8 {
            for kind in [
                Self::Integers { dense: true },
                Self::Integers { dense: false },
                Self::Uuid,
            ] {
                if fits(kind) {
                    return kind;
                }
            }
            let prefix = HexPrefix::of(first);
            if let Ok(digits) = u8::try_from(first.len() - prefix.text().len()) {
                let hex = Self::Hex { prefix, digits };
                if (prefix != HexPrefix::None || digits >= 16) && digits <= 128 && fits(hex) {
                    return hex;
                }
            }
            if first.len() >= 20 {
                if let Some(width) = base58_decode(first).and_then(|d| u8::try_from(d.len()).ok()) {
                    let b58 = Self::Base58 { width };
                    if usize::from(width) <= MAX_FIXED_WIDTH && fits(b58) {
                        return b58;
                    }
                }
            }
        }
        match u8::try_from(first.len()) {
            Ok(width)
                if (1..=MAX_FIXED_WIDTH).contains(&first.len()) && fits(Self::Fixed { width }) =>
            {
                Self::Fixed { width }
            }
            _ => Self::Bytes,
        }
    }

    /// The next kind to try when a key does not fit this one. [`KeyKind::Bytes`] accepts
    /// every key of a text or binary column, sparse integers every key of an integer
    /// column.
    #[must_use]
    pub fn widen(self, column: ColumnType) -> Self {
        match self {
            Self::Integers { dense: true } => Self::Integers { dense: false },
            Self::Integers { dense: false } if !column.is_text() => self,
            _ => Self::Bytes,
        }
    }

    /// Length of the normalized keys, or `None` for [`KeyKind::Bytes`].
    #[must_use]
    pub const fn width(self) -> Option<usize> {
        match self {
            Self::Integers { .. } => Some(8),
            Self::Hex { digits, .. } => Some((digits as usize).div_ceil(2)),
            Self::Uuid => Some(16),
            Self::Base58 { width } | Self::Fixed { width } => Some(width as usize),
            Self::Bytes => None,
        }
    }

    /// Whether `key` belongs to this kind (for a dense integer kind: also lies in
    /// `0..2^32` from the column's base).
    pub(crate) fn accepts(self, column: ColumnType, key: Key<'_>) -> bool {
        let mut buf = [0_u8; MAX_FIXED_WIDTH];
        match self.normalize(column, key, &mut buf) {
            None => false,
            Some(bytes) => match self {
                Self::Integers { dense: true } => dense_index(column, bytes).is_some(),
                _ => true,
            },
        }
    }

    /// The normalized bytes of `key`, or `None` if it does not belong to this kind.
    /// Fixed-width kinds write into `buf`; [`KeyKind::Bytes`] returns the key itself.
    pub(crate) fn normalize<'k>(
        self,
        column: ColumnType,
        key: Key<'k>,
        buf: &'k mut [u8; MAX_FIXED_WIDTH],
    ) -> Option<&'k [u8]> {
        match self {
            Self::Integers { .. } => {
                let code = int_code(column, key)?;
                buf[..8].copy_from_slice(&code.to_be_bytes());
                Some(&buf[..8])
            }
            Self::Bytes => match key {
                Key::Bytes(b) => Some(b),
                _ => None,
            },
            _ => {
                let Key::Bytes(text) = key else { return None };
                let width = self.width()?;
                let out = &mut buf[..width];
                let ok = match self {
                    Self::Hex { prefix, digits } => parse_hex(text, prefix, digits, out),
                    Self::Uuid => parse_uuid(text, out),
                    Self::Base58 { .. } => base58_decode_into(text, out),
                    Self::Fixed { .. } => {
                        let fits = text.len() == width;
                        if fits {
                            out.copy_from_slice(text);
                        }
                        fits
                    }
                    Self::Integers { .. } | Self::Bytes => unreachable!("handled above"),
                };
                ok.then_some(&buf[..width])
            }
        }
    }

    /// Writes the identifier whose normalized bytes are `bytes` as the column gives it.
    pub(crate) fn render(self, column: ColumnType, bytes: &[u8]) -> OwnedKey {
        if let Self::Integers { .. } = self {
            let code = u64::from_be_bytes(bytes.try_into().expect("integer keys are 8 bytes"));
            return match column {
                ColumnType::Int { .. } => {
                    OwnedKey::Int(i64::from_ne_bytes((code ^ SIGN).to_ne_bytes()))
                }
                ColumnType::UInt { .. } => OwnedKey::UInt(code),
                ColumnType::Utf8 | ColumnType::Binary => OwnedKey::Text(code.to_string()),
            };
        }
        let mut out = Vec::with_capacity(bytes.len() * 2 + 2);
        match self {
            Self::Hex { prefix, digits } => {
                out.extend_from_slice(prefix.text());
                let skip = usize::from(digits % 2);
                for (i, nibble) in bytes.iter().flat_map(|b| [b >> 4, b & 15]).enumerate() {
                    if i >= skip {
                        out.push(HEX_DIGITS[usize::from(nibble)]);
                    }
                }
            }
            Self::Uuid => {
                for (i, b) in bytes.iter().enumerate() {
                    if matches!(i, 4 | 6 | 8 | 10) {
                        out.push(b'-');
                    }
                    out.push(HEX_DIGITS[usize::from(b >> 4)]);
                    out.push(HEX_DIGITS[usize::from(b & 15)]);
                }
            }
            Self::Base58 { .. } => out = base58_encode(bytes),
            _ => out.extend_from_slice(bytes),
        }
        match column {
            ColumnType::Binary => OwnedKey::Bytes(out),
            _ => OwnedKey::Text(
                String::from_utf8(out)
                    .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()),
            ),
        }
    }

    pub(crate) fn encode(self) -> [u8; 4] {
        match self {
            Self::Integers { dense } => [1, u8::from(dense), 0, 0],
            Self::Hex { prefix, digits } => [
                2,
                match prefix {
                    HexPrefix::None => 0,
                    HexPrefix::Lower => 1,
                    HexPrefix::Upper => 2,
                },
                digits,
                0,
            ],
            Self::Uuid => [3, 0, 0, 0],
            Self::Base58 { width } => [4, width, 0, 0],
            Self::Fixed { width } => [5, width, 0, 0],
            Self::Bytes => [6, 0, 0, 0],
        }
    }

    pub(crate) fn decode(bytes: [u8; 4]) -> Option<Self> {
        let kind = match bytes {
            [1, d @ (0 | 1), 0, 0] => Self::Integers { dense: d == 1 },
            [2, p @ 0..=2, digits @ 1..=128, 0] => Self::Hex {
                prefix: [HexPrefix::None, HexPrefix::Lower, HexPrefix::Upper][usize::from(p)],
                digits,
            },
            [3, 0, 0, 0] => Self::Uuid,
            [4, width @ 1..=64, 0, 0] => Self::Base58 { width },
            [5, width @ 1..=64, 0, 0] => Self::Fixed { width },
            [6, 0, 0, 0] => Self::Bytes,
            _ => return None,
        };
        Some(kind)
    }
}

const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";
const SIGN: u64 = 1 << 63;

/// The base of dense integers: the code of the value 0 in the column's encoding.
pub(crate) const fn dense_base(column: ColumnType) -> u64 {
    match column {
        ColumnType::Int { .. } => SIGN,
        _ => 0,
    }
}

/// The bit of a dense integer key in the rank bitmap, if it lies in `0..2^32`.
pub(crate) fn dense_index(column: ColumnType, normalized: &[u8]) -> Option<u32> {
    let code = u64::from_be_bytes(normalized.try_into().ok()?);
    u32::try_from(code.checked_sub(dense_base(column))?).ok()
}

/// The order-preserving 64-bit code of an integer key in `column`'s domain.
fn int_code(column: ColumnType, key: Key<'_>) -> Option<u64> {
    let signed = matches!(column, ColumnType::Int { .. });
    match key {
        Key::Int(v) if signed => Some(u64::from_ne_bytes(v.to_ne_bytes()) ^ SIGN),
        Key::Int(v) => u64::try_from(v).ok(),
        Key::UInt(v) if signed => i64::try_from(v)
            .ok()
            .map(|v| u64::from_ne_bytes(v.to_ne_bytes()) ^ SIGN),
        Key::UInt(v) => Some(v),
        Key::Bytes(text) => {
            let (negative, digits) = match text {
                [b'-', rest @ ..] if signed => (true, rest),
                _ => (false, text),
            };
            let canonical = match digits {
                [] => false,
                [b'0'] => !negative,
                [first, ..] => *first != b'0' && digits.iter().all(u8::is_ascii_digit),
            };
            if !canonical {
                return None;
            }
            let mut value: u64 = 0;
            for d in digits {
                value = value.checked_mul(10)?.checked_add(u64::from(d - b'0'))?;
            }
            if signed {
                let v = if negative {
                    0_i64.checked_sub_unsigned(value)?
                } else {
                    i64::try_from(value).ok()?
                };
                Some(u64::from_ne_bytes(v.to_ne_bytes()) ^ SIGN)
            } else {
                Some(value)
            }
        }
    }
}

/// The value of each byte as a hexadecimal digit, or `INVALID`.
const HEX_VALUE: [u8; 256] = {
    const fn digit(c: u8) -> u8 {
        match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => INVALID,
        }
    }
    let mut t = [INVALID; 256];
    let mut c: u8 = 0;
    loop {
        t[c as usize] = digit(c);
        if c == u8::MAX {
            break;
        }
        c += 1;
    }
    t
};
const INVALID: u8 = 0xFF;

fn hex_value(c: u8) -> Option<u8> {
    let v = HEX_VALUE[usize::from(c)];
    (v != INVALID).then_some(v)
}

/// Parses `digits` hexadecimal digits after `prefix` (either case of `x` accepted when
/// a prefix is expected) into `out`, right-aligned (an odd count leaves the first
/// nibble 0).
fn parse_hex(text: &[u8], prefix: HexPrefix, digits: u8, out: &mut [u8]) -> bool {
    let body = match (prefix, text) {
        (HexPrefix::None, t) => t,
        (_, [b'0', b'x' | b'X', rest @ ..]) => rest,
        _ => return false,
    };
    if body.len() != usize::from(digits) {
        return false;
    }
    // An odd count has a lone first digit, in the low nibble of the first byte.
    let (lone, pairs) = body.split_at(usize::from(digits % 2));
    let mut bad = 0_u8;
    let mut at = 0;
    if let [c] = lone {
        let v = HEX_VALUE[usize::from(*c)];
        bad |= v;
        out[0] = v;
        at = 1;
    }
    for (pair, byte) in pairs.chunks_exact(2).zip(&mut out[at..]) {
        let (h, l) = (
            HEX_VALUE[usize::from(pair[0])],
            HEX_VALUE[usize::from(pair[1])],
        );
        // A valid digit is below 16; INVALID sets the high bit.
        bad |= h | l;
        *byte = (h << 4) | (l & 0x0F);
    }
    bad < 0x10
}

fn parse_uuid(text: &[u8], out: &mut [u8]) -> bool {
    if text.len() != 36 {
        return false;
    }
    let mut nibbles = 0;
    for (i, &c) in text.iter().enumerate() {
        if matches!(i, 8 | 13 | 18 | 23) {
            if c != b'-' {
                return false;
            }
            continue;
        }
        let Some(v) = hex_value(c) else { return false };
        out[nibbles / 2] = if nibbles % 2 == 0 {
            v << 4
        } else {
            out[nibbles / 2] | v
        };
        nibbles += 1;
    }
    true
}

fn base58_digit(c: u8) -> Option<u32> {
    BASE58
        .iter()
        .position(|&a| a == c)
        .and_then(|p| u32::try_from(p).ok())
}

/// Decodes base58 text (Bitcoin alphabet; each leading `1` is a zero byte).
fn base58_decode(text: &[u8]) -> Option<Vec<u8>> {
    let zeros = text.iter().take_while(|&&c| c == b'1').count();
    // Little-endian base-256 digits of the number.
    let mut num: Vec<u8> = Vec::with_capacity(text.len());
    for &c in &text[zeros..] {
        let mut carry = base58_digit(c)?;
        for byte in &mut num {
            carry += u32::from(*byte) * 58;
            *byte = u8::try_from(carry & 0xFF).expect("masked");
            carry >>= 8;
        }
        while carry > 0 {
            num.push(u8::try_from(carry & 0xFF).expect("masked"));
            carry >>= 8;
        }
    }
    let mut out = vec![0_u8; zeros];
    out.extend(num.iter().rev());
    Some(out)
}

fn base58_decode_into(text: &[u8], out: &mut [u8]) -> bool {
    match base58_decode(text) {
        Some(d) if d.len() == out.len() => {
            out.copy_from_slice(&d);
            true
        }
        _ => false,
    }
}

pub(crate) fn base58_encode(bytes: &[u8]) -> Vec<u8> {
    let zeros = bytes.iter().take_while(|&&b| b == 0).count();
    // Little-endian base-58 digits.
    let mut digits: Vec<u8> = Vec::with_capacity(bytes.len() * 2);
    for &b in &bytes[zeros..] {
        let mut carry = u32::from(b);
        for d in &mut digits {
            carry += u32::from(*d) << 8;
            *d = u8::try_from(carry % 58).expect("below 58");
            carry /= 58;
        }
        while carry > 0 {
            digits.push(u8::try_from(carry % 58).expect("below 58"));
            carry /= 58;
        }
    }
    let mut out = vec![b'1'; zeros];
    out.extend(digits.iter().rev().map(|&d| BASE58[usize::from(d)]));
    out
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn text_keys(keys: &[&'static str]) -> Vec<Key<'static>> {
        keys.iter().map(|k| Key::from(*k)).collect()
    }

    fn detect_text(keys: &[&'static str]) -> KeyKind {
        KeyKind::detect(ColumnType::Utf8, text_keys(keys))
    }

    fn round_trip(kind: KeyKind, column: ColumnType, key: Key<'_>) -> OwnedKey {
        let mut buf = [0; MAX_FIXED_WIDTH];
        let bytes = kind
            .normalize(column, key, &mut buf)
            .expect("accepted")
            .to_vec();
        kind.render(column, &bytes)
    }

    #[test]
    fn kinds_are_recognized_from_a_sample() {
        assert_eq!(
            detect_text(&["0", "17", "4294967295"]),
            KeyKind::Integers { dense: true }
        );
        assert_eq!(
            detect_text(&["17", "4294967296"]),
            KeyKind::Integers { dense: false }
        );
        assert_eq!(detect_text(&["17", "017"]), KeyKind::Bytes);
        assert_eq!(detect_text(&["17", "01"]), KeyKind::Fixed { width: 2 });
        assert_eq!(
            detect_text(&[
                "0x28C6c06298d514Db089934071355E5743bf21d60",
                "0x0000000000000000000000000000000000000000"
            ]),
            KeyKind::Hex {
                prefix: HexPrefix::Lower,
                digits: 40
            }
        );
        assert_eq!(
            detect_text(&["deadbeefdeadbeef"]),
            KeyKind::Hex {
                prefix: HexPrefix::None,
                digits: 16
            }
        );
        assert_eq!(detect_text(&["cafe", "face"]), KeyKind::Fixed { width: 4 });
        assert_eq!(
            detect_text(&["550e8400-e29b-41d4-A716-446655440000"]),
            KeyKind::Uuid
        );
        assert_eq!(
            detect_text(&[
                "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa",
                "3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy"
            ]),
            KeyKind::Base58 { width: 25 }
        );
        assert_eq!(detect_text(&["alice", "bob"]), KeyKind::Bytes);
        assert_eq!(detect_text(&[]), KeyKind::Bytes);
        assert_eq!(
            KeyKind::detect(
                ColumnType::Binary,
                [Key::Bytes(&[1, 2, 3]), Key::Bytes(&[4, 5, 6])]
            ),
            KeyKind::Fixed { width: 3 }
        );
        let int = ColumnType::Int { bits: 64 };
        assert_eq!(
            KeyKind::detect(int, [Key::Int(3), Key::Int(1 << 31)]),
            KeyKind::Integers { dense: true }
        );
        assert_eq!(
            KeyKind::detect(int, [Key::Int(-1)]),
            KeyKind::Integers { dense: false }
        );
    }

    #[test]
    fn widening_ends_at_a_kind_that_accepts_everything() {
        let int = ColumnType::UInt { bits: 64 };
        assert_eq!(
            KeyKind::Integers { dense: true }.widen(int),
            KeyKind::Integers { dense: false }
        );
        assert_eq!(
            KeyKind::Integers { dense: false }.widen(int),
            KeyKind::Integers { dense: false }
        );
        assert_eq!(
            KeyKind::Integers { dense: false }.widen(ColumnType::Utf8),
            KeyKind::Bytes
        );
        assert_eq!(KeyKind::Uuid.widen(ColumnType::Utf8), KeyKind::Bytes);
    }

    #[test]
    fn hexadecimal_comes_back_lowercase_with_the_input_prefix_and_matches_any_case() {
        let kind = KeyKind::Hex {
            prefix: HexPrefix::Lower,
            digits: 40,
        };
        let mixed = "0x28C6c06298d514Db089934071355E5743bf21d60";
        let lower = mixed.to_ascii_lowercase();
        assert_eq!(
            round_trip(kind, ColumnType::Utf8, Key::from(mixed)),
            OwnedKey::Text(lower.clone())
        );
        let upper_x = format!("0X{}", &mixed[2..]);
        let mut a = [0; MAX_FIXED_WIDTH];
        let mut b = [0; MAX_FIXED_WIDTH];
        assert_eq!(
            kind.normalize(ColumnType::Utf8, Key::from(upper_x.as_str()), &mut a),
            kind.normalize(ColumnType::Utf8, Key::from(lower.as_str()), &mut b)
        );
        let none = KeyKind::Hex {
            prefix: HexPrefix::None,
            digits: 3,
        };
        assert_eq!(
            round_trip(none, ColumnType::Utf8, Key::from("aBc")),
            OwnedKey::Text("abc".into())
        );
        assert!(!kind.accepts(ColumnType::Utf8, Key::from(&mixed[2..])));
        assert!(!kind.accepts(ColumnType::Utf8, Key::from("0x28c6…1d60")));
        // A non-digit anywhere, in the lone first digit of an odd count, or in either
        // half of a pair, is refused.
        for at in [2, 3, 20, 41] {
            let mut bad = lower.clone().into_bytes();
            for c in [b'g', b'G', b' ', b'/', b':', b'@', b'`', 0xC3] {
                bad[at] = c;
                assert!(
                    !kind.accepts(ColumnType::Utf8, Key::Bytes(&bad)),
                    "{at} {c}"
                );
            }
        }
        assert!(!none.accepts(ColumnType::Utf8, Key::from("gbc")));
        assert!(!none.accepts(ColumnType::Utf8, Key::from("abg")));
        let upper = KeyKind::Hex {
            prefix: HexPrefix::Upper,
            digits: 2,
        };
        assert_eq!(
            round_trip(upper, ColumnType::Utf8, Key::from("0xAB")),
            OwnedKey::Text("0Xab".into())
        );
    }

    #[test]
    fn integers_come_back_in_the_column_type() {
        let i64c = ColumnType::Int { bits: 64 };
        let kind = KeyKind::Integers { dense: false };
        for v in [i64::MIN, -1, 0, 1, i64::MAX] {
            assert_eq!(round_trip(kind, i64c, Key::Int(v)), OwnedKey::Int(v));
            assert_eq!(
                round_trip(kind, i64c, Key::from(v.to_string().as_str())),
                OwnedKey::Int(v)
            );
        }
        let u = ColumnType::UInt { bits: 32 };
        assert_eq!(
            round_trip(kind, u, Key::UInt(u64::MAX)),
            OwnedKey::UInt(u64::MAX)
        );
        assert!(!kind.accepts(u, Key::Int(-1)));
        assert_eq!(
            round_trip(kind, ColumnType::Utf8, Key::from("18446744073709551615")),
            OwnedKey::Text("18446744073709551615".into())
        );
        assert!(!kind.accepts(ColumnType::Utf8, Key::from("18446744073709551616")));
        assert!(!kind.accepts(ColumnType::Utf8, Key::from("-1")));
        assert!(!kind.accepts(i64c, Key::from("-0")));
        assert_eq!(dense_index(i64c, &(SIGN + 5).to_be_bytes()), Some(5));
        assert_eq!(dense_index(i64c, &(SIGN - 1).to_be_bytes()), None);
    }

    #[test]
    fn uuids_and_base58_round_trip() {
        assert_eq!(
            round_trip(
                KeyKind::Uuid,
                ColumnType::Utf8,
                Key::from("550E8400-e29b-41d4-a716-446655440000")
            ),
            OwnedKey::Text("550e8400-e29b-41d4-a716-446655440000".into())
        );
        let b58 = KeyKind::Base58 { width: 25 };
        let addr = "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa";
        assert_eq!(
            round_trip(b58, ColumnType::Utf8, Key::from(addr)),
            OwnedKey::Text(addr.into())
        );
        assert!(!b58.accepts(
            ColumnType::Utf8,
            Key::from("1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfN0")
        ));
    }

    #[test]
    fn kinds_encode_and_decode() {
        for kind in [
            KeyKind::Integers { dense: true },
            KeyKind::Integers { dense: false },
            KeyKind::Hex {
                prefix: HexPrefix::Upper,
                digits: 7,
            },
            KeyKind::Uuid,
            KeyKind::Base58 { width: 25 },
            KeyKind::Fixed { width: 64 },
            KeyKind::Bytes,
        ] {
            assert_eq!(KeyKind::decode(kind.encode()), Some(kind));
        }
        assert_eq!(KeyKind::decode([9, 0, 0, 0]), None);
        for column in [
            ColumnType::Int { bits: 8 },
            ColumnType::UInt { bits: 64 },
            ColumnType::Utf8,
            ColumnType::Binary,
        ] {
            assert_eq!(ColumnType::decode(column.encode()), Some(column));
        }
        assert_eq!(ColumnType::decode([1, 7]), None);
    }

    proptest! {
        /// Sorted order of normalized bytes is numeric order for integers.
        #[test]
        fn integer_codes_preserve_order(a in any::<i64>(), b in any::<i64>()) {
            let c = ColumnType::Int { bits: 64 };
            let (x, y) = (int_code(c, Key::Int(a)).unwrap(), int_code(c, Key::Int(b)).unwrap());
            prop_assert_eq!(a.cmp(&b), x.cmp(&y));
        }

        #[test]
        fn base58_is_a_bijection(bytes in proptest::collection::vec(any::<u8>(), 0..40)) {
            prop_assert_eq!(base58_decode(&base58_encode(&bytes)), Some(bytes));
        }

        #[test]
        fn hex_round_trips_and_ignores_case(bytes in proptest::collection::vec(any::<u8>(), 1..32), upper in any::<bool>()) {
            let mut text = String::from("0x");
            for b in &bytes { text.push_str(&format!("{b:02x}")); }
            let digits = u8::try_from(bytes.len() * 2).unwrap();
            let kind = KeyKind::Hex { prefix: HexPrefix::Lower, digits };
            let input = if upper { text.to_ascii_uppercase().replacen("0X", "0x", 1) } else { text.clone() };
            prop_assert_eq!(round_trip(kind, ColumnType::Utf8, Key::from(input.as_str())), OwnedKey::Text(text));
        }
    }
}
