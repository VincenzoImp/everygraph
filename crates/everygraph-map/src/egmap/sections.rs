// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The section table and its chunked arrays (§ 5): what the map holds besides tiles.

use std::ops::Range;

use everygraph_core::{le, usize_of};

use super::write::MapInfo;
use super::{EgmapError, MAX_DECODED_BYTES, invalid, malformed};

const MAGIC: &[u8; 8] = b"EGMAPTBL";
const MAJOR: u16 = 1;
const MINOR: u16 = 0;
pub(super) const TABLE_HEADER: usize = 48;
pub(super) const ENTRY: usize = 64;
const NAME: usize = 32;

/// The type of the values of an array.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Element {
    /// `u8`.
    U8 = 1,
    /// `u16`, little-endian.
    U16 = 2,
    /// `u32`, little-endian.
    U32 = 3,
    /// `u64`, little-endian.
    U64 = 4,
    /// IEEE 754 binary32, little-endian; finite, and never −0.0.
    F32 = 5,
}

impl Element {
    /// Bytes per value.
    #[must_use]
    pub const fn width(self) -> usize {
        match self {
            Self::U8 => 1,
            Self::U16 => 2,
            Self::U32 | Self::F32 => 4,
            Self::U64 => 8,
        }
    }

    fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            1 => Self::U8,
            2 => Self::U16,
            3 => Self::U32,
            4 => Self::U64,
            5 => Self::F32,
            _ => return None,
        })
    }
}

/// Values given to [`MapWriter::write_values`](super::MapWriter::write_values).
#[derive(Clone, Copy, Debug)]
pub enum Values<'a> {
    /// `u8` values.
    U8(&'a [u8]),
    /// `u16` values.
    U16(&'a [u16]),
    /// `u32` values.
    U32(&'a [u32]),
    /// `u64` values.
    U64(&'a [u64]),
    /// `f32` values.
    F32(&'a [f32]),
}

impl Values<'_> {
    /// Their element type.
    #[must_use]
    pub const fn element(&self) -> Element {
        match self {
            Self::U8(_) => Element::U8,
            Self::U16(_) => Element::U16,
            Self::U32(_) => Element::U32,
            Self::U64(_) => Element::U64,
            Self::F32(_) => Element::F32,
        }
    }

    /// How many there are.
    #[must_use]
    pub const fn len(&self) -> usize {
        match self {
            Self::U8(v) => v.len(),
            Self::U16(v) => v.len(),
            Self::U32(v) => v.len(),
            Self::U64(v) => v.len(),
            Self::F32(v) => v.len(),
        }
    }

    /// Whether there are none.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Appends the little-endian bytes of values `range` to `out`.
    pub(super) fn append(&self, range: Range<usize>, out: &mut Vec<u8>) -> Result<(), EgmapError> {
        match self {
            Self::U8(v) => out.extend_from_slice(&v[range]),
            Self::U16(v) => v[range]
                .iter()
                .for_each(|x| out.extend_from_slice(&x.to_le_bytes())),
            Self::U32(v) => v[range]
                .iter()
                .for_each(|x| out.extend_from_slice(&x.to_le_bytes())),
            Self::U64(v) => v[range]
                .iter()
                .for_each(|x| out.extend_from_slice(&x.to_le_bytes())),
            Self::F32(v) => {
                for x in &v[range] {
                    if !x.is_finite() || x.to_bits() == (-0.0_f32).to_bits() {
                        return Err(invalid("an f32 array holds a NaN, an infinity or −0.0"));
                    }
                    out.extend_from_slice(&x.to_le_bytes());
                }
            }
        }
        Ok(())
    }
}

/// One array of the section table (§ 5.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    /// Its name.
    pub name: String,
    /// The type of its values.
    pub element: Element,
    /// Values per chunk; the last chunk may hold fewer.
    pub chunk: u32,
    /// Values in all.
    pub count: u64,
    /// Where it starts in the file.
    pub offset: u64,
    /// Its length in bytes: the chunks, then the chunk index.
    pub len: u64,
}

impl Section {
    /// The number of chunks.
    #[must_use]
    pub fn chunks(&self) -> u64 {
        self.count.div_ceil(u64::from(self.chunk))
    }

    /// The values held by chunk `i`.
    #[must_use]
    pub fn values_of(&self, i: u64) -> Range<u64> {
        let start = i.saturating_mul(u64::from(self.chunk)).min(self.count);
        start..start.saturating_add(u64::from(self.chunk)).min(self.count)
    }

    /// Where the chunk index starts, relative to the section.
    pub(super) fn index_start(&self) -> Option<u64> {
        self.chunks()
            .checked_add(1)?
            .checked_mul(8)
            .and_then(|i| self.len.checked_sub(i))
    }
}

/// Whether `name` may name a section: 1 to 32 bytes of `a`–`z`, `0`–`9`, `.`, `_`, `-`.
pub(super) fn valid_name(name: &str) -> bool {
    (1..=NAME).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
}

pub(super) fn encode_table(info: &MapInfo, sections: &[Section]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&MAJOR.to_le_bytes());
    out.extend_from_slice(&MINOR.to_le_bytes());
    out.extend_from_slice(
        &u32::try_from(sections.len())
            .unwrap_or(u32::MAX)
            .to_le_bytes(),
    );
    for v in [info.nodes, info.regions, info.edges] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&info.summary.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    for s in sections {
        let mut name = [0_u8; NAME];
        name[..s.name.len()].copy_from_slice(s.name.as_bytes());
        out.extend_from_slice(&name);
        out.extend_from_slice(&[s.element as u8, 0, 0, 0]);
        out.extend_from_slice(&s.chunk.to_le_bytes());
        for v in [s.count, s.offset, s.len] {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    out
}

/// The byte length of the table that starts `head`, or why it cannot be one.
pub(super) fn table_len(head: &[u8]) -> Result<usize, EgmapError> {
    if head.len() < TABLE_HEADER || &head[..8] != MAGIC {
        return Err(EgmapError::Foreign(
            "a PMTiles archive without a map section table",
        ));
    }
    let major = le::u16_at(head, 8);
    if major != MAJOR {
        return Err(EgmapError::Unsupported(format!(
            "section table version {major}"
        )));
    }
    usize_of(le::u32_at(head, 12))
        .checked_mul(ENTRY)
        .and_then(|e| e.checked_add(TABLE_HEADER))
        .ok_or_else(|| malformed("a section table that does not fit"))
}

/// Parses the table, given exactly its bytes, and checks it against a file of
/// `file_len` bytes (§ 5.1, § 5.3).
pub(super) fn parse_table(
    bytes: &[u8],
    file_len: u64,
) -> Result<(MapInfo, Vec<Section>), EgmapError> {
    if bytes[44..48] != [0; 4] {
        return Err(malformed("reserved section table bytes are not zero"));
    }
    let info = MapInfo {
        nodes: le::u64_at(bytes, 16),
        regions: le::u64_at(bytes, 24),
        edges: le::u64_at(bytes, 32),
        summary: le::u32_at(bytes, 40),
    };
    let mut sections: Vec<Section> = Vec::new();
    for e in bytes[TABLE_HEADER..].chunks_exact(ENTRY) {
        let raw = &e[..NAME];
        let used = raw.iter().position(|&b| b == 0).unwrap_or(NAME);
        let name = std::str::from_utf8(&raw[..used])
            .ok()
            .filter(|n| valid_name(n) && raw[used..].iter().all(|&b| b == 0))
            .ok_or_else(|| malformed("a section name is not valid"))?;
        if sections.last().is_some_and(|s| s.name.as_str() >= name) {
            return Err(malformed("section names are not in increasing order"));
        }
        let element = Element::from_code(e[32])
            .ok_or_else(|| malformed(format!("section {name}: an unknown element type")))?;
        let s = Section {
            name: name.to_owned(),
            element,
            chunk: le::u32_at(e, 36),
            count: le::u64_at(e, 40),
            offset: le::u64_at(e, 48),
            len: le::u64_at(e, 56),
        };
        let chunk_bytes = usize_of(s.chunk).checked_mul(element.width());
        if e[33..36] != [0; 3]
            || s.chunk == 0
            || chunk_bytes.is_none_or(|b| b > MAX_DECODED_BYTES)
            || s.index_start().is_none()
            || s.offset.checked_add(s.len).is_none_or(|end| end > file_len)
        {
            return Err(malformed(format!("section {name} breaks a rule of § 5.1")));
        }
        sections.push(s);
    }
    check_known(&info, &sections)?;
    Ok((info, sections))
}

/// The arrays this version defines: their type and length, and which come together
/// (§ 5.3).
pub(super) fn check_known(info: &MapInfo, sections: &[Section]) -> Result<(), EgmapError> {
    let get = |name: &str| sections.iter().find(|s| s.name == name);
    let plus_one = |n: u64| n.checked_add(1);
    let expect = [
        ("positions", Element::F32, info.nodes.checked_mul(2)),
        ("adjacency.offsets", Element::U64, plus_one(info.nodes)),
        ("adjacency.targets", Element::U32, None),
        ("matrix.offsets", Element::U64, plus_one(info.regions)),
        ("matrix.pairs", Element::U32, None),
        (
            "matrix.weights",
            Element::U64,
            get("matrix.pairs").map(|s| s.count),
        ),
    ];
    for (name, element, count) in expect {
        if let Some(s) = get(name)
            && (s.element != element || count.is_some_and(|c| c != s.count))
        {
            return Err(malformed(format!(
                "section {name} has the wrong type or length"
            )));
        }
    }
    let together = |names: &[&str]| {
        names.iter().all(|n| get(n).is_some()) || names.iter().all(|n| get(n).is_none())
    };
    if !together(&["adjacency.offsets", "adjacency.targets"])
        || !together(&["matrix.offsets", "matrix.pairs", "matrix.weights"])
    {
        return Err(malformed("an array of a group is missing"));
    }
    if info.summary > 0
        && sections.iter().any(|s| {
            s.name == "positions"
                || s.name.starts_with("adjacency.")
                || s.name.starts_with("search.")
        })
    {
        return Err(malformed("a summary map holds node-level sections"));
    }
    Ok(())
}

/// Checks the chunk index entries `start` and `end` of `section` (relative offsets)
/// and gives the chunk's span relative to the section.
pub(super) fn chunk_span(
    section: &Section,
    start: u64,
    end: u64,
) -> Result<Range<u64>, EgmapError> {
    let index = section.index_start().unwrap_or(0);
    if start >= end || end > index {
        return Err(malformed(format!(
            "section {}: the chunk index does not increase within the section",
            section.name
        )));
    }
    Ok(start..end)
}

/// The decoded length chunk `i` must have.
pub(super) fn chunk_bytes(section: &Section, i: u64) -> usize {
    let r = section.values_of(i);
    usize::try_from(r.end - r.start).unwrap_or(usize::MAX) * section.element.width()
}

pub(super) fn index_bytes(offsets: &[u64]) -> Vec<u8> {
    let mut out = Vec::new();
    for o in offsets {
        out.extend_from_slice(&o.to_le_bytes());
    }
    out
}
