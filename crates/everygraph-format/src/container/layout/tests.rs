// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;

fn entry(name: &str, offset: u64, len: u64) -> Entry {
    Entry {
        name: name.into(),
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
    // Anything else in bytes 8..16 is a torn block, not a foreign file: a first
    // commit torn after 9 to 15 bytes leaves a partial byte order or version.
    for (at, value) in [(8, 0u8), (9, 0), (10, 0), (11, 0), (12, 0)] {
        let mut c = b;
        c[at] = value;
        assert!(matches!(decode_header(&c, 1), Block::Torn), "byte {at}");
    }
    // Generation 0 and a block that is not where it says are not valid, even
    // with a matching hash.
    let zero = encode_header(&Header { generation: 0, ..h });
    assert!(matches!(decode_header(&zero, 1), Block::Torn));
    assert!(matches!(decode_header(&b, 0), Block::Torn));
}

#[test]
fn the_name_area_must_be_the_names_in_entry_order() {
    let mut p = entry("p", 8192, 10);
    p.kind = Kind::PROVENANCE;
    let mut entries = [entry("ab", 12288, 1), entry("ac", 16384, 1), p];
    entries[0].provenance = Some("p".into());
    entries[1].provenance = Some("p".into());
    let toc = encode_toc(entries.iter()).unwrap();
    let decoded = decode_toc(&toc, 1 << 20).unwrap();
    // What is held of the names is the name area once: references share it.
    let record = &decoded[2].name;
    assert!(
        decoded[..2]
            .iter()
            .all(|e| Arc::ptr_eq(e.provenance.as_ref().unwrap(), record))
    );
    let held: usize = decoded.iter().map(|e| e.name.len()).sum();
    assert!(held <= toc.len());

    // Entry 1's name taken from entry 0's bytes (names that overlap).
    let mut c = toc.clone();
    c[TOC_HEADER + ENTRY_V1 + 56..TOC_HEADER + ENTRY_V1 + 60].fill(0);
    assert!(decode_toc(&c, 1 << 20).is_err());
    // Bytes in the name area that belong to no name.
    let mut c = toc.clone();
    c[16..24].copy_from_slice(&6u64.to_le_bytes());
    c.push(b'z');
    assert!(decode_toc(&c, 1 << 20).is_err());
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
    // A provenance record with a provenance of its own (which `compact` could not
    // copy in an order that satisfies every reference, cycles included).
    let mut records = [entry("p_a", 8192, 1), entry("p_b", 12288, 1)];
    records[0].kind = Kind::PROVENANCE;
    records[1].kind = Kind::PROVENANCE;
    records[0].provenance = Some("p_b".into());
    assert!(decode_toc(&encode_toc(records.iter()).unwrap(), 1 << 20).is_err());
}
