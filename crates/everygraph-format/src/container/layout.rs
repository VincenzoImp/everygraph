// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The bytes of the header blocks and of the table of contents
//! (`docs/spec/eg-container.md` § 2 and § 4). Pure functions: no I/O.

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
/// Required feature bits this crate understands (none are defined in 1.0).
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

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("4 bytes"))
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().expect("8 bytes"))
}

fn hash32(b: &[u8], at: usize) -> [u8; 32] {
    b[at..at + 32].try_into().expect("32 bytes")
}

/// Decodes the first [`HEADER_USED`] bytes of header block `index`.
pub(crate) fn decode_header(b: &[u8], index: u32) -> Block {
    if b.len() < HEADER_USED || b[..8] != MAGIC {
        return Block::Absent;
    }
    // Bytes 0..16 mean the same in every version: magic, byte order, major, minor.
    if u32_at(b, 8) != BYTE_ORDER {
        return Block::Foreign(ContainerError::UnsupportedByteOrder);
    }
    let (major, minor) = (u16_at(b, 12), u16_at(b, 14));
    if major != MAJOR {
        return Block::Foreign(ContainerError::UnsupportedVersion { major, minor });
    }
    if *blake3::hash(&b[..HEADER_HASHED]).as_bytes() != hash32(b, HEADER_HASHED) {
        return Block::Torn;
    }
    Block::Valid(Header {
        block: index,
        minor,
        generation: u64_at(b, 24),
        required: u64_at(b, 32),
        optional: u64_at(b, 40),
        toc_offset: u64_at(b, 48),
        toc_len: u64_at(b, 56),
        toc_hash: hash32(b, 64),
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
    let mut raw = Vec::with_capacity(n);
    for i in 0..n {
        let e = &b[TOC_HEADER + i * entry_size..][..ENTRY_V1];
        let kind = Kind::new(u32_at(e, 0)).ok_or(corrupt("an entry has kind 0"))?;
        let codec = Codec::new(u16_at(e, 4));
        let offset = u64_at(e, 8);
        let len = u64_at(e, 16);
        let hash = hash32(e, 24);
        let name_at = usize::try_from(u32_at(e, 56)).map_err(|_| corrupt("name offset"))?;
        let name_len = usize::try_from(u32_at(e, 60)).map_err(|_| corrupt("name length"))?;
        let provenance = u32_at(e, 64);
        if name_len == 0 || name_len > MAX_NAME_BYTES {
            return Err(corrupt("a section name is empty or longer than 1024 bytes"));
        }
        let name = name_at
            .checked_add(name_len)
            .and_then(|end| names.get(name_at..end))
            .ok_or(corrupt("a section name lies outside the name area"))?;
        let name = std::str::from_utf8(name)
            .map_err(|_| corrupt("a section name is not UTF-8"))?
            .to_owned();
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
    if raw.windows(2).any(|w| w[0].0.name >= w[1].0.name) {
        return Err(corrupt(
            "section names are not unique and in increasing byte order",
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
    let mut entries = Vec::with_capacity(n);
    for (i, (entry, provenance)) in raw.iter().enumerate() {
        let mut entry = entry.clone();
        if provenance != &NO_PROVENANCE {
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
            entry.provenance = Some(record.0.name.clone());
        }
        entries.push(entry);
    }
    Ok(entries)
}

/// Encodes `entries` (sorted by name, provenance names among them) as a table of
/// contents.
pub(crate) fn encode_toc<'a>(
    entries: impl ExactSizeIterator<Item = &'a Entry> + Clone,
) -> Result<Vec<u8>, ContainerError> {
    let too_large = || ContainerError::TooLarge("the table of contents would exceed 16 MiB");
    let n = entries.len();
    // Entries are sorted by name, so a reference is found by binary search.
    let names: Vec<&str> = entries.clone().map(|e| e.name.as_str()).collect();
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
            Some(p) => index_of(p).ok_or_else(|| ContainerError::NoSuchSection(p.clone()))?,
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
mod tests {
    use super::*;

    fn entry(name: &str, offset: u64, len: u64) -> Entry {
        Entry {
            name: name.to_owned(),
            kind: Kind::new(0x8000_0000).unwrap(),
            codec: Codec::NONE,
            offset,
            len,
            hash: [7; 32],
            provenance: None,
        }
    }

    #[test]
    fn padding_is_at_least_16_bytes_to_the_next_block() {
        assert_eq!(pad_end(8192, 0), Some(8192 + 4096));
        assert_eq!(pad_end(8192, 4080), Some(8192 + 4096));
        assert_eq!(pad_end(8192, 4081), Some(8192 + 8192));
        assert_eq!(pad_end(u64::MAX - 10, 0), None);
    }

    #[test]
    fn a_header_round_trips_and_any_flipped_bit_tears_it() {
        let h = Header {
            block: 1,
            minor: 0,
            generation: 9,
            required: 0,
            optional: 4,
            toc_offset: 12288,
            toc_len: 104,
            toc_hash: [3; 32],
        };
        let b = encode_header(&h);
        assert!(matches!(decode_header(&b, 1), Block::Valid(d) if d == h));
        // Every bit natively; a sample under Miri, where each hash takes ~50 ms.
        for bit in (16 * 8..HEADER_USED * 8).step_by(if cfg!(miri) { 61 } else { 1 }) {
            let mut c = b;
            c[bit / 8] ^= 1 << (bit % 8);
            assert!(matches!(decode_header(&c, 1), Block::Torn), "bit {bit}");
        }
        let mut c = b;
        c[0] ^= 1;
        assert!(matches!(decode_header(&c, 1), Block::Absent));
        let mut c = b;
        c[12] = 2;
        assert!(matches!(
            decode_header(&c, 1),
            Block::Foreign(ContainerError::UnsupportedVersion { major: 2, .. })
        ));
        let mut c = b;
        c[8..12].copy_from_slice(&BYTE_ORDER.to_be_bytes());
        assert!(matches!(
            decode_header(&c, 1),
            Block::Foreign(ContainerError::UnsupportedByteOrder)
        ));
    }

    #[test]
    fn a_toc_round_trips_and_larger_entries_of_a_later_minor_are_read() {
        let mut p = entry("p", 8192, 10);
        p.kind = Kind::PROVENANCE;
        let mut a = entry("a/b", 12288, 5000);
        a.provenance = Some("p".into());
        let entries = vec![a, p];
        let toc = encode_toc(entries.iter()).unwrap();
        assert_eq!(toc.len(), TOC_HEADER + 2 * ENTRY_V1 + 4);
        assert_eq!(decode_toc(&toc, 1 << 20).unwrap(), entries);

        // The same table with 80-byte entries (8 bytes a later minor added).
        let mut wide = toc[..TOC_HEADER].to_vec();
        wide[12..16].copy_from_slice(&80u32.to_le_bytes());
        for i in 0..2 {
            wide.extend_from_slice(&toc[TOC_HEADER + i * ENTRY_V1..][..ENTRY_V1]);
            wide.extend_from_slice(&[0xEE; 8]);
        }
        wide.extend_from_slice(&toc[TOC_HEADER + 2 * ENTRY_V1..]);
        assert_eq!(decode_toc(&wide, 1 << 20).unwrap(), entries);
    }

    #[test]
    fn a_toc_with_broken_structure_is_refused() {
        let entries = [entry("a", 8192, 100), entry("b", 12288, 100)];
        let toc = encode_toc(entries.iter()).unwrap();
        assert!(decode_toc(&toc, 16384).is_ok());
        // A section that runs into the table of contents.
        assert!(decode_toc(&toc, 12288).is_err());
        // Overlap, names out of order, kind 0, a bad name, a bad length.
        let overlapping = [entry("a", 8192, 5000), entry("b", 12288, 100)];
        assert!(decode_toc(&encode_toc(overlapping.iter()).unwrap(), 1 << 20).is_err());
        let unordered = [entry("b", 8192, 1), entry("a", 12288, 1)];
        assert!(decode_toc(&encode_toc(unordered.iter()).unwrap(), 1 << 20).is_err());
        let mut c = toc.clone();
        c[TOC_HEADER..TOC_HEADER + 4].fill(0);
        assert!(decode_toc(&c, 1 << 20).is_err());
        let mut c = toc.clone();
        let last = c.len() - 1;
        c[last] = 0xFF;
        assert!(decode_toc(&c, 1 << 20).is_err());
        assert!(decode_toc(&toc[..toc.len() - 1], 1 << 20).is_err());
        let misaligned = [entry("a", 8193, 1)];
        assert!(decode_toc(&encode_toc(misaligned.iter()).unwrap(), 1 << 20).is_err());
        let mut self_ref = entry("a", 8192, 1);
        self_ref.kind = Kind::PROVENANCE;
        self_ref.provenance = Some("a".into());
        assert!(decode_toc(&encode_toc([self_ref].iter()).unwrap(), 1 << 20).is_err());
        let mut not_a_record = [entry("a", 8192, 1), entry("b", 12288, 1)];
        not_a_record[1].provenance = Some("a".into());
        assert!(decode_toc(&encode_toc(not_a_record.iter()).unwrap(), 1 << 20).is_err());
    }
}
