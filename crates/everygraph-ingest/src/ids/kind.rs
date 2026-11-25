// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Key typing: what the identifiers of a column look like, decided from a sample, and
//! the normalized bytes each key is stored, hashed and compared as.

use std::fmt;

/// Longest normalized key of a fixed-width kind, in bytes.
pub const MAX_FIXED_WIDTH: usize = 64;

/// Longest key, in bytes. A longer key is an invalid row: it is not added, and the
/// builder reports it (`Distinct::invalid`).
pub const MAX_KEY_BYTES: usize = 1 << 16;

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
    /// A fixed-size binary column (Arrow `fixed_size_binary`) of `width` bytes, 1 to 64.
    FixedBinary {
        /// Length of every value.
        width: u8,
    },
}

impl ColumnType {
    pub(crate) fn encode(self) -> [u8; 2] {
        match self {
            Self::Int { bits } => [1, bits],
            Self::UInt { bits } => [2, bits],
            Self::Utf8 => [3, 0],
            Self::Binary => [4, 0],
            Self::FixedBinary { width } => [5, width],
        }
    }

    pub(crate) fn decode(bytes: [u8; 2]) -> Option<Self> {
        let valid_bits = matches!(bytes[1], 8 | 16 | 32 | 64);
        match bytes {
            [1, bits] if valid_bits => Some(Self::Int { bits }),
            [2, bits] if valid_bits => Some(Self::UInt { bits }),
            [3, 0] => Some(Self::Utf8),
            [4, 0] => Some(Self::Binary),
            [5, width @ 1..=64] => Some(Self::FixedBinary { width }),
            _ => None,
        }
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
    /// From a text column: hexadecimal and UUIDs in lowercase, hexadecimal with `0x`
    /// when the input had a prefix.
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

/// How the identifiers of a column are represented: chosen by [`KeyKind::detect`] from
/// a sample and checked on every key while the dictionary is built. A key that does
/// not fit is an invalid row (`design/DECISIONS.md`, "Identifier columns"): it is left
/// out and reported, and the kind does not change.
///
/// Every kind but [`KeyKind::Bytes`] stores its keys as fixed-width bytes, so the key
/// store needs no offsets. The normalized bytes compare in the order "by sorted key"
/// uses: numeric for integers, the order of the lowercase text for hexadecimal and
/// UUIDs, the order of the decoded bytes for base58, bytewise otherwise.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum KeyKind {
    /// Integers, from an integer column or as canonical decimal text (`0`, `17`, never
    /// `017` or `+17`; text never negative). Normalized: 8 bytes, big-endian, of the
    /// value for unsigned columns and text, of the value with its sign bit flipped for
    /// signed columns.
    Integers,
    /// Hexadecimal text with a fixed number of digits, matched in any case and given
    /// back in lowercase, with `0x` if the input used a prefix (`0x` or `0X`).
    /// Recognized only with a prefix or at least 16 digits, so that ordinary words
    /// (`cafe`) are not taken for numbers. Normalized: the digits as bytes,
    /// right-aligned (an odd count leaves the first nibble zero).
    Hex {
        /// Whether keys have a `0x`/`0X` prefix.
        prefixed: bool,
        /// Number of hexadecimal digits (1 to 128).
        digits: u8,
    },
    /// UUIDs in the canonical 8-4-4-4-12 form, any case; given back in lowercase.
    /// Normalized: the 16 bytes.
    Uuid,
    /// Base58 text (the Bitcoin alphabet) of 20 to 88 characters whose decoded bytes
    /// have one length. Case-sensitive. Normalized: the decoded bytes, each leading `1`
    /// a zero byte.
    Base58 {
        /// Length of the decoded bytes.
        width: u8,
    },
    /// The values of a fixed-size binary column, stored as they are.
    Fixed {
        /// Length of every key.
        width: u8,
    },
    /// Anything else: arbitrary text or bytes (at most [`MAX_KEY_BYTES`]), stored as
    /// they are with offsets.
    Bytes,
}

/// Number of keys [`KeyKind::detect`] looks at; more are ignored.
pub const SAMPLE_SIZE: usize = 4096;

const BASE58: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/// Longest base58 text whose decoded bytes can be `width` long: `width` times
/// log(256)/log(58) = 1.3657, rounded up (a leading zero byte takes one character).
const fn base58_max_len(width: usize) -> usize {
    (width * 13_658).div_ceil(10_000)
}

/// Whether `count` keys out of `of` are enough to give a column a kind: all but at most
/// one in a hundred (stray values become invalid rows, not a different kind).
fn enough(count: usize, of: usize) -> bool {
    count > 0 && count * 100 >= of * 99
}

impl KeyKind {
    /// Chooses the kind of a column from a sample of its keys: the first
    /// [`SAMPLE_SIZE`] keys of the input in input order, so that the choice is the same
    /// at any thread count. A kind is chosen when at least 99 % of the sample fits it.
    pub fn detect<'a>(column: ColumnType, sample: impl IntoIterator<Item = Key<'a>>) -> Self {
        let sample: Vec<Key<'a>> = sample.into_iter().take(SAMPLE_SIZE).collect();
        let of = sample.len();
        let fitting = |kind: Self| sample.iter().filter(|k| kind.accepts(column, **k)).count();
        match column {
            ColumnType::Int { .. } | ColumnType::UInt { .. } => return Self::Integers,
            ColumnType::FixedBinary { width } => return Self::Fixed { width },
            ColumnType::Binary => return Self::Bytes,
            ColumnType::Utf8 => {}
        }
        for kind in [Self::Integers, Self::Uuid] {
            if enough(fitting(kind), of) {
                return kind;
            }
        }
        // The most frequent hexadecimal form, then the most frequent base58 width.
        let mut forms: Vec<(usize, Self)> = Vec::new();
        let mut count = |kind: Self| match forms.iter_mut().find(|(_, k)| *k == kind) {
            Some((c, _)) => *c += 1,
            None => forms.push((1, kind)),
        };
        for key in &sample {
            let Key::Bytes(text) = key else { continue };
            let text: &[u8] = text;
            let (prefixed, body) = match text {
                [b'0', b'x' | b'X', rest @ ..] => (true, rest),
                _ => (false, text),
            };
            if let Ok(digits) = u8::try_from(body.len()) {
                let hex_like = (1..=128).contains(&digits)
                    && (prefixed || digits >= 16)
                    && body.iter().all(|&c| HEX_VALUE[usize::from(c)] != INVALID);
                if hex_like {
                    count(Self::Hex { prefixed, digits });
                    continue;
                }
            }
            if (20..=base58_max_len(MAX_FIXED_WIDTH)).contains(&text.len()) {
                if let Some(width) = base58_decode(text).and_then(|d| u8::try_from(d.len()).ok()) {
                    if usize::from(width) <= MAX_FIXED_WIDTH
                        && text.len() <= base58_max_len(usize::from(width))
                    {
                        count(Self::Base58 { width });
                    }
                }
            }
        }
        // Most frequent first; ties in a fixed order of the forms' codes.
        forms.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.encode().cmp(&b.1.encode())));
        match forms.first() {
            Some(&(c, kind)) if enough(c, of) => kind,
            _ => Self::Bytes,
        }
    }

    /// Length of the normalized keys, or `None` for [`KeyKind::Bytes`].
    #[must_use]
    pub const fn width(self) -> Option<usize> {
        match self {
            Self::Integers => Some(8),
            Self::Hex { digits, .. } => Some((digits as usize).div_ceil(2)),
            Self::Uuid => Some(16),
            Self::Base58 { width } | Self::Fixed { width } => Some(width as usize),
            Self::Bytes => None,
        }
    }

    /// Whether `key` belongs to this kind.
    pub(crate) fn accepts(self, column: ColumnType, key: Key<'_>) -> bool {
        let mut buf = [0_u8; MAX_FIXED_WIDTH];
        self.normalize(column, key, &mut buf).is_some()
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
            Self::Integers => {
                let code = int_code(column, key)?;
                buf[..8].copy_from_slice(&code.to_be_bytes());
                Some(&buf[..8])
            }
            Self::Bytes => match key {
                Key::Bytes(b) if b.len() <= MAX_KEY_BYTES => Some(b),
                _ => None,
            },
            _ => {
                let Key::Bytes(text) = key else { return None };
                let width = self.width()?;
                let out = &mut buf[..width];
                let ok = match self {
                    Self::Hex { prefixed, digits } => parse_hex(text, prefixed, digits, out),
                    Self::Uuid => parse_uuid(text, out),
                    Self::Base58 { .. } => base58_decode_into(text, out),
                    Self::Fixed { .. } => {
                        let fits = text.len() == width;
                        if fits {
                            out.copy_from_slice(text);
                        }
                        fits
                    }
                    Self::Integers | Self::Bytes => unreachable!("handled above"),
                };
                ok.then_some(&buf[..width])
            }
        }
    }

    /// Writes the identifier whose normalized bytes are `bytes` as the column gives it.
    pub(crate) fn render(self, column: ColumnType, bytes: &[u8]) -> OwnedKey {
        if let Self::Integers = self {
            let code = u64::from_be_bytes(bytes.try_into().expect("integer keys are 8 bytes"));
            return match column {
                ColumnType::Int { .. } => {
                    OwnedKey::Int(i64::from_ne_bytes((code ^ SIGN).to_ne_bytes()))
                }
                ColumnType::UInt { .. } => OwnedKey::UInt(code),
                _ => OwnedKey::Text(code.to_string()),
            };
        }
        let mut out = Vec::with_capacity(bytes.len() * 2 + 2);
        match self {
            Self::Hex { prefixed, digits } => {
                if prefixed {
                    out.extend_from_slice(b"0x");
                }
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
            ColumnType::Utf8 => OwnedKey::Text(
                String::from_utf8(out)
                    .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()),
            ),
            _ => OwnedKey::Bytes(out),
        }
    }

    pub(crate) fn encode(self) -> [u8; 4] {
        match self {
            Self::Integers => [1, 0, 0, 0],
            Self::Hex { prefixed, digits } => [2, u8::from(prefixed), digits, 0],
            Self::Uuid => [3, 0, 0, 0],
            Self::Base58 { width } => [4, width, 0, 0],
            Self::Fixed { width } => [5, width, 0, 0],
            Self::Bytes => [6, 0, 0, 0],
        }
    }

    pub(crate) fn decode(bytes: [u8; 4]) -> Option<Self> {
        let kind = match bytes {
            [1, 0, 0, 0] => Self::Integers,
            [2, p @ (0 | 1), digits @ 1..=128, 0] => Self::Hex {
                prefixed: p == 1,
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

    /// Whether this kind of keys may be stored in `column`.
    pub(crate) fn fits_column(self, column: ColumnType) -> bool {
        match (self, column) {
            (Self::Integers, _) => true,
            (Self::Fixed { width }, ColumnType::FixedBinary { width: w }) => width == w,
            (Self::Bytes, ColumnType::Utf8 | ColumnType::Binary) => true,
            (Self::Hex { .. } | Self::Uuid | Self::Base58 { .. }, ColumnType::Utf8) => true,
            _ => false,
        }
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

/// The bit of an integer key in a rank bitmap, if it lies in `0..2^32`.
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
                [first, ..] => {
                    *first != b'0' && digits.len() <= 20 && digits.iter().all(u8::is_ascii_digit)
                }
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

/// Parses `digits` hexadecimal digits, after a `0x`/`0X` prefix if `prefixed`, into
/// `out`, right-aligned (an odd count leaves the first nibble 0).
fn parse_hex(text: &[u8], prefixed: bool, digits: u8, out: &mut [u8]) -> bool {
    let body = match (prefixed, text) {
        (false, t) => t,
        (true, [b'0', b'x' | b'X', rest @ ..]) => rest,
        (true, _) => return false,
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

/// Decodes base58 text (Bitcoin alphabet; each leading `1` is a zero byte). Quadratic
/// in the length: callers bound the length first.
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
    // Longer text cannot decode to `out.len()` bytes: refused before the quadratic
    // decoding.
    if text.len() > base58_max_len(out.len()) {
        return false;
    }
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

    fn detect_text(keys: &[&str]) -> KeyKind {
        KeyKind::detect(ColumnType::Utf8, keys.iter().map(|k| Key::from(*k)))
    }

    fn round_trip(kind: KeyKind, column: ColumnType, key: Key<'_>) -> OwnedKey {
        let mut buf = [0; MAX_FIXED_WIDTH];
        let bytes = kind
            .normalize(column, key, &mut buf)
            .expect("accepted")
            .to_vec();
        kind.render(column, &bytes)
    }

    const ADDRESS: &str = "0x28C6c06298d514Db089934071355E5743bf21d60";

    #[test]
    fn kinds_are_recognized_from_a_sample() {
        let hex40 = KeyKind::Hex {
            prefixed: true,
            digits: 40,
        };
        assert_eq!(detect_text(&["0", "17", "4294967296"]), KeyKind::Integers);
        assert_eq!(detect_text(&["17", "017"]), KeyKind::Bytes);
        assert_eq!(
            detect_text(&[ADDRESS, "0x0000000000000000000000000000000000000000"]),
            hex40
        );
        assert_eq!(
            detect_text(&["deadbeefdeadbeef"]),
            KeyKind::Hex {
                prefixed: false,
                digits: 16
            }
        );
        assert_eq!(detect_text(&["cafe", "face"]), KeyKind::Bytes);
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
        // Constant length is not a kind for text: a later longer name would be refused.
        assert_eq!(detect_text(&["alice", "carol"]), KeyKind::Bytes);
        // `0x` alone is not hexadecimal.
        assert_eq!(detect_text(&["0x", "0x"]), KeyKind::Bytes);
        assert_eq!(
            KeyKind::detect(
                ColumnType::FixedBinary { width: 3 },
                [Key::Bytes(&[1, 2, 3])]
            ),
            KeyKind::Fixed { width: 3 }
        );
        assert_eq!(
            KeyKind::detect(ColumnType::Binary, [Key::Bytes(&[1, 2, 3])]),
            KeyKind::Bytes
        );
        let int = ColumnType::Int { bits: 64 };
        assert_eq!(KeyKind::detect(int, [Key::Int(-1)]), KeyKind::Integers);
    }

    /// A stray value (`0x`, an empty field) does not change the kind of the column:
    /// up to 1 % of the sample may be invalid rows.
    #[test]
    fn stray_keys_in_the_sample_do_not_change_the_kind() {
        let mut keys: Vec<String> = (0..1000_u64).map(|i| format!("0x{i:040x}")).collect();
        keys[0] = "0x".into();
        keys[500] = String::new();
        let refs: Vec<&str> = keys.iter().map(String::as_str).collect();
        let kind = detect_text(&refs);
        assert_eq!(
            kind,
            KeyKind::Hex {
                prefixed: true,
                digits: 40
            }
        );
        assert!(!kind.accepts(ColumnType::Utf8, Key::from("0x")));
        // Mixed 0x and 0X prefixes are one form.
        let mixed: Vec<String> = (0..100_u64)
            .map(|i| format!("{}{i:040x}", if i % 2 == 0 { "0x" } else { "0X" }))
            .collect();
        let refs: Vec<&str> = mixed.iter().map(String::as_str).collect();
        assert_eq!(detect_text(&refs), kind);
        // Two in a hundred is too many.
        keys[1] = "zz".into();
        let refs: Vec<&str> = keys[..100].iter().map(String::as_str).collect();
        assert_eq!(detect_text(&refs), KeyKind::Bytes);
    }

    #[test]
    fn hexadecimal_comes_back_lowercase_with_0x_and_matches_any_case() {
        let kind = KeyKind::Hex {
            prefixed: true,
            digits: 40,
        };
        let lower = ADDRESS.to_ascii_lowercase();
        assert_eq!(
            round_trip(kind, ColumnType::Utf8, Key::from(ADDRESS)),
            OwnedKey::Text(lower.clone())
        );
        let upper_x = format!("0X{}", &ADDRESS[2..]);
        assert_eq!(
            round_trip(kind, ColumnType::Utf8, Key::from(upper_x.as_str())),
            OwnedKey::Text(lower.clone())
        );
        let none = KeyKind::Hex {
            prefixed: false,
            digits: 3,
        };
        assert_eq!(
            round_trip(none, ColumnType::Utf8, Key::from("aBc")),
            OwnedKey::Text("abc".into())
        );
        assert!(!kind.accepts(ColumnType::Utf8, Key::from(&ADDRESS[2..])));
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
    }

    #[test]
    fn integers_come_back_in_the_column_type() {
        let i64c = ColumnType::Int { bits: 64 };
        let kind = KeyKind::Integers;
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

    /// A long key is refused by its length, before the quadratic decoding (one
    /// 200,000-character key took 5.6 s in the review of PR #158).
    #[test]
    fn long_base58_text_is_refused_before_decoding() {
        let b58 = KeyKind::Base58 { width: 25 };
        let long = "2".repeat(200_000);
        assert!(!b58.accepts(ColumnType::Utf8, Key::from(long.as_str())));
        assert_eq!(base58_max_len(25), 35);
        assert_eq!(base58_max_len(64), 88);
        // The bound is exact enough to accept the longest real encodings.
        for width in 1..=64 {
            let max = base58_encode(&vec![0xFF; width]).len();
            assert!(max <= base58_max_len(width), "{width}");
            assert!(base58_encode(&vec![0; width]).len() <= base58_max_len(width));
        }
        let huge = vec![b'a'; MAX_KEY_BYTES + 1];
        assert!(!KeyKind::Bytes.accepts(ColumnType::Utf8, Key::Bytes(&huge)));
        assert!(KeyKind::Bytes.accepts(ColumnType::Utf8, Key::Bytes(&huge[1..])));
    }

    #[test]
    fn kinds_encode_and_decode() {
        for kind in [
            KeyKind::Integers,
            KeyKind::Hex {
                prefixed: true,
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
        assert_eq!(KeyKind::decode([2, 1, 0, 0]), None);
        for column in [
            ColumnType::Int { bits: 8 },
            ColumnType::UInt { bits: 64 },
            ColumnType::Utf8,
            ColumnType::Binary,
            ColumnType::FixedBinary { width: 20 },
        ] {
            assert_eq!(ColumnType::decode(column.encode()), Some(column));
        }
        assert_eq!(ColumnType::decode([1, 7]), None);
        assert_eq!(ColumnType::decode([5, 0]), None);
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
            let kind = KeyKind::Hex { prefixed: true, digits };
            let input = if upper { text.to_ascii_uppercase() } else { text.clone() };
            prop_assert_eq!(round_trip(kind, ColumnType::Utf8, Key::from(input.as_str())), OwnedKey::Text(text));
        }
    }
}
