// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The bytes of the header blocks and of the table of contents
//! (`docs/spec/eg-container.md` § 2 and § 4). Pure functions: no I/O.

use std::sync::Arc;

use everygraph_core::le::{array, u16_at, u32_at, u64_at};

use super::{
    BLOCK, Codec, ContainerError, Entry, HEADER_BYTES, Kind, MAX_NAME_BYTES, MAX_TOC_BYTES,
    MIN_PADDING,
};

/// The file's magic: a non-ASCII byte (catches 7-bit transfers), "EGRAPH", and a line
/// feed (catches line-ending conversion).
pub(crate) const MAGIC: [u8; 8] = *b"\x89EGRAPH\n";
/// `0x0A0B0C0D` as written by a little-endian writer.
pub(crate) const BYTE_ORDER: u32 = 0x0A0B_0C0D;
/// The major version this crate reads and writes.
pub const MAJOR: u16 = 1;
/// The minor version this crate writes; it reads every minor of [`MAJOR`].
pub const MINOR: u16 = 0;
/// Required feature bits this crate understands: none. Bit 0,
/// [`REQUIRED_SEGMENTS`](super::REQUIRED_SEGMENTS), is reserved and refused like any
/// other until segments are built.
pub(crate) const KNOWN_REQUIRED: u64 = 0;

/// Bytes of a header block that carry fields; the rest of the block is zero.
pub(crate) const HEADER_USED: usize = 128;
/// Bytes covered by a header block's own hash.
const HEADER_HASHED: usize = 96;

pub(crate) const TOC_MAGIC: [u8; 8] = *b"EGTOC\0\0\0";
pub(crate) const TOC_HEADER: usize = 32;
/// Size of a table-of-contents entry in 1.0. Later minors may write larger entries;
/// readers read these fields and skip the rest.
pub(crate) const ENTRY_V1: usize = 72;
pub(crate) const NO_PROVENANCE: u32 = u32::MAX;

/// The end of the zero padding that follows an item of `len` bytes at `offset`: the
/// first multiple of [`BLOCK`] at least [`MIN_PADDING`] bytes past its last byte.
pub(crate) fn pad_end(offset: u64, len: u64) -> Option<u64> {
    offset
        .checked_add(len)?
        .checked_add(MIN_PADDING)?
        .checked_next_multiple_of(BLOCK)
}

/// [`pad_end`] of an item already checked to have one (saturates instead of panicking).
pub(crate) fn padded(offset: u64, len: u64) -> u64 {
    pad_end(offset, len).unwrap_or(u64::MAX)
}

/// A length known to be small (below 16 MiB) as a `usize`.
pub(crate) fn small(len: u64) -> usize {
    usize::try_from(len).unwrap_or(usize::MAX)
}

/// [`BLOCK`](super::BLOCK) as a `usize`.
pub(crate) const BLOCK_USIZE: usize = 4096;
const _: () = assert!(BLOCK_USIZE as u64 == BLOCK);

/// A decoded header block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Header {
    pub(crate) block: u32,
    pub(crate) minor: u16,
    pub(crate) generation: u64,
    pub(crate) required: u64,
    pub(crate) optional: u64,
    pub(crate) toc_offset: u64,
    pub(crate) toc_len: u64,
    pub(crate) toc_hash: [u8; 32],
}

/// What one header block holds.
#[derive(Debug)]
pub(crate) enum Block {
    /// No magic: never written, or not a container.
    Absent,
    /// The magic, but the block's hash does not match: torn or damaged.
    Torn,
    /// A container of a byte order or major version this crate does not read.
    Foreign(ContainerError),
    /// A usable block.
    Valid(Header),
}

/// `0x0A0B0C0D` as a big-endian writer would have written it.
const BYTE_ORDER_SWAPPED: u32 = 0x0D0C_0B0A;

/// Decodes the first [`HEADER_USED`] bytes of header block `index`.
///
/// A block is foreign only when it says so exactly — the byte-swapped byte order, or a
/// major version above ours — so that a first commit torn inside bytes 8 to 15 reads as
/// torn, not as a file of another version.
pub(crate) fn decode_header(b: &[u8], index: u32) -> Block {
    if b.len() < HEADER_USED || b[..8] != MAGIC {
        return Block::Absent;
    }
    // Bytes 0..16 mean the same in every version: magic, byte order, major, minor.
    let (order, major, minor) = (u32_at(b, 8), u16_at(b, 12), u16_at(b, 14));
    if order == BYTE_ORDER_SWAPPED {
        return Block::Foreign(ContainerError::UnsupportedByteOrder);
    }
    if order == BYTE_ORDER && major > MAJOR {
        return Block::Foreign(ContainerError::UnsupportedVersion { major, minor });
    }
    if order != BYTE_ORDER
        || major != MAJOR
        || *blake3::hash(&b[..HEADER_HASHED]).as_bytes() != array(b, HEADER_HASHED)
    {
        return Block::Torn;
    }
    let generation = u64_at(b, 24);
    // Generation 0 is never written, and a block must say where it is.
    if generation == 0 || u32_at(b, 16) != index {
        return Block::Torn;
    }
    Block::Valid(Header {
        block: index,
        minor,
        generation,
        required: u64_at(b, 32),
        optional: u64_at(b, 40),
        toc_offset: u64_at(b, 48),
        toc_len: u64_at(b, 56),
        toc_hash: array(b, 64),
    })
}

/// The [`HEADER_USED`] bytes of a header block.
pub(crate) fn encode_header(h: &Header) -> [u8; HEADER_USED] {
    let mut b = [0u8; HEADER_USED];
    b[..8].copy_from_slice(&MAGIC);
    b[8..12].copy_from_slice(&BYTE_ORDER.to_le_bytes());
    b[12..14].copy_from_slice(&MAJOR.to_le_bytes());
    b[14..16].copy_from_slice(&h.minor.to_le_bytes());
    b[16..20].copy_from_slice(&h.block.to_le_bytes());
    // 20..24 reserved.
    b[24..32].copy_from_slice(&h.generation.to_le_bytes());
    b[32..40].copy_from_slice(&h.required.to_le_bytes());
    b[40..48].copy_from_slice(&h.optional.to_le_bytes());
    b[48..56].copy_from_slice(&h.toc_offset.to_le_bytes());
    b[56..64].copy_from_slice(&h.toc_len.to_le_bytes());
    b[64..96].copy_from_slice(&h.toc_hash);
    let own = *blake3::hash(&b[..HEADER_HASHED]).as_bytes();
    b[HEADER_HASHED..HEADER_HASHED + 32].copy_from_slice(&own);
    b
}

/// The checks on a header's pointer that do not need the table itself.
pub(crate) fn check_toc_pointer(h: &Header) -> Result<(), ContainerError> {
    if h.toc_offset < HEADER_BYTES || !h.toc_offset.is_multiple_of(BLOCK) {
        return Err(ContainerError::Corrupt(
            "the table of contents is not at a block boundary after the header",
        ));
    }
    if h.toc_len < TOC_HEADER as u64 {
        return Err(ContainerError::Corrupt(
            "the table of contents is shorter than its header",
        ));
    }
    if h.toc_len > MAX_TOC_BYTES {
        return Err(ContainerError::Corrupt(
            "the table of contents is larger than the 16 MiB a reader accepts",
        ));
    }
    if pad_end(h.toc_offset, h.toc_len).is_none() {
        return Err(ContainerError::Corrupt(
            "the table of contents ends past 2^64",
        ));
    }
    Ok(())
}

/// Decodes a table of contents whose hash has been checked. `toc_offset` is where it
/// lies: every section must end, padding included, at or before it.
pub(crate) fn decode_toc(b: &[u8], toc_offset: u64) -> Result<Vec<Entry>, ContainerError> {
    let corrupt = ContainerError::Corrupt;
    if b.len() < TOC_HEADER || b[..8] != TOC_MAGIC {
        return Err(corrupt("the table of contents has no magic"));
    }
    let n = usize::try_from(u32_at(b, 8)).map_err(|_| corrupt("too many entries"))?;
    let entry_size = usize::try_from(u32_at(b, 12)).map_err(|_| corrupt("entry size"))?;
    if entry_size < ENTRY_V1 || !entry_size.is_multiple_of(8) {
        return Err(corrupt("table-of-contents entries have an invalid size"));
    }
    let names_len = usize::try_from(u64_at(b, 16)).map_err(|_| corrupt("names length"))?;
    let names_at = n
        .checked_mul(entry_size)
        .and_then(|e| e.checked_add(TOC_HEADER))
        .ok_or(corrupt("table-of-contents length"))?;
    if names_at.checked_add(names_len) != Some(b.len()) {
        return Err(corrupt(
            "the table of contents does not have the length its header implies",
        ));
    }
    let names = &b[names_at..];
    // The name area is the names, concatenated in entry order: what a reader holds of
    // the names is at most the table's own size.
    let mut names_seen = 0usize;
    let mut raw: Vec<(Entry, u32)> = Vec::with_capacity(n);
    for i in 0..n {
        let e = &b[TOC_HEADER + i * entry_size..][..ENTRY_V1];
        let kind = Kind::new(u32_at(e, 0)).ok_or(corrupt("an entry has kind 0"))?;
        let codec = Codec::new(u16_at(e, 4));
        let offset = u64_at(e, 8);
        let len = u64_at(e, 16);
        let hash = array(e, 24);
        let name_at = usize::try_from(u32_at(e, 56)).map_err(|_| corrupt("name offset"))?;
        let name_len = usize::try_from(u32_at(e, 60)).map_err(|_| corrupt("name length"))?;
        let provenance = u32_at(e, 64);
        if name_len == 0 || name_len > MAX_NAME_BYTES {
            return Err(corrupt("a section name is empty or longer than 1024 bytes"));
        }
        if name_at != names_seen {
            return Err(corrupt(
                "the name area is not the names concatenated in entry order",
            ));
        }
        let name = names
            .get(name_at..name_at + name_len)
            .ok_or(corrupt("a section name lies outside the name area"))?;
        names_seen += name_len;
        let name: Arc<str> = std::str::from_utf8(name)
            .map_err(|_| corrupt("a section name is not UTF-8"))?
            .into();
        if raw.last().is_some_and(|(prev, _)| *prev.name >= *name) {
            return Err(corrupt(
                "section names are not unique and in increasing byte order",
            ));
        }
        if kind == Kind::PROVENANCE && provenance != NO_PROVENANCE {
            return Err(corrupt("a provenance record has a provenance reference"));
        }
        if offset < HEADER_BYTES || !offset.is_multiple_of(BLOCK) {
            return Err(corrupt(
                "a section does not start at a block boundary after the header",
            ));
        }
        match pad_end(offset, len) {
            Some(end) if end <= toc_offset => {}
            _ => {
                return Err(corrupt(
                    "a section does not end before the table of contents",
                ));
            }
        }
        raw.push((
            Entry {
                name,
                kind,
                codec,
                offset,
                len,
                hash,
                provenance: None,
            },
            provenance,
        ));
    }
    if names_seen != names_len {
        return Err(corrupt(
            "the name area is not the names concatenated in entry order",
        ));
    }
    // Sections and their padding do not overlap.
    let mut by_offset: Vec<(u64, u64)> = raw.iter().map(|(e, _)| (e.offset, e.len)).collect();
    by_offset.sort_unstable();
    for w in by_offset.windows(2) {
        let end = padded(w[0].0, w[0].1);
        if end > w[1].0 {
            return Err(corrupt("two sections overlap"));
        }
    }
    // Resolve provenance references; a reference shares the record's name.
    let mut references = Vec::with_capacity(n);
    for (i, (_, provenance)) in raw.iter().enumerate() {
        if *provenance == NO_PROVENANCE {
            references.push(None);
            continue;
        }
        let p = usize::try_from(*provenance).map_err(|_| corrupt("provenance index"))?;
        let record = raw
            .get(p)
            .filter(|_| p != i)
            .ok_or(corrupt("a provenance reference names no other entry"))?;
        if record.0.kind != Kind::PROVENANCE {
            return Err(corrupt(
                "a provenance reference names an entry that is not a provenance record",
            ));
        }
        references.push(Some(Arc::clone(&record.0.name)));
    }
    Ok(raw
        .into_iter()
        .zip(references)
        .map(|((mut entry, _), provenance)| {
            entry.provenance = provenance;
            entry
        })
        .collect())
}

/// Encodes `entries` (sorted by name, provenance names among them) as a table of
/// contents.
pub(crate) fn encode_toc<'a>(
    entries: impl ExactSizeIterator<Item = &'a Entry> + Clone,
) -> Result<Vec<u8>, ContainerError> {
    let too_large = || ContainerError::TooLarge("the table of contents would exceed 16 MiB");
    let n = entries.len();
    // Entries are sorted by name, so a reference is found by binary search.
    let names: Vec<&str> = entries.clone().map(|e| &*e.name).collect();
    let index_of = |name: &str| {
        names
            .binary_search(&name)
            .ok()
            .and_then(|p| u32::try_from(p).ok())
    };
    let names_len: usize = entries.clone().map(|e| e.name.len()).sum();
    let total = TOC_HEADER + n * ENTRY_V1 + names_len;
    if total as u64 > MAX_TOC_BYTES {
        return Err(too_large());
    }
    let mut b = Vec::with_capacity(total);
    b.extend_from_slice(&TOC_MAGIC);
    b.extend_from_slice(&u32::try_from(n).map_err(|_| too_large())?.to_le_bytes());
    b.extend_from_slice(&u32::try_from(ENTRY_V1).expect("72").to_le_bytes());
    b.extend_from_slice(&(names_len as u64).to_le_bytes());
    b.extend_from_slice(&[0; 8]);
    let mut name_at = 0u32;
    for e in entries.clone() {
        let name_len = u32::try_from(e.name.len()).map_err(|_| too_large())?;
        let provenance = match &e.provenance {
            None => NO_PROVENANCE,
            Some(p) => index_of(p).ok_or_else(|| ContainerError::NoSuchSection(p.to_string()))?,
        };
        b.extend_from_slice(&e.kind.code().to_le_bytes());
        b.extend_from_slice(&e.codec.code().to_le_bytes());
        b.extend_from_slice(&[0; 2]);
        b.extend_from_slice(&e.offset.to_le_bytes());
        b.extend_from_slice(&e.len.to_le_bytes());
        b.extend_from_slice(&e.hash);
        b.extend_from_slice(&name_at.to_le_bytes());
        b.extend_from_slice(&name_len.to_le_bytes());
        b.extend_from_slice(&provenance.to_le_bytes());
        b.extend_from_slice(&[0; 4]);
        name_at += name_len;
    }
    for e in entries {
        b.extend_from_slice(e.name.as_bytes());
    }
    debug_assert_eq!(b.len(), total);
    Ok(b)
}

#[cfg(test)]
mod tests;
