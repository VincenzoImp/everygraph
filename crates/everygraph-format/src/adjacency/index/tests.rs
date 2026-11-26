// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Hostile indexes: each breaks one rule of `parse`, and each
//! must be refused. The good index is built field by field, so that the header
//! may disagree with the parts.

use super::*;

/// An index as fields; `bytes()` lays them out as the specification says.
#[derive(Clone)]
struct Raw {
    magic: [u8; 8],
    version: u32,
    flags: u32,
    nodes: u64,
    entries: u64,
    data_bytes: u64,
    blocks: u64,
    starts: Vec<u32>,
    codecs: Vec<u8>,
    byte_offsets: Vec<u64>,
    entry_offsets: Vec<u64>,
}

/// Three nodes with degrees 4, 0, 4: records of 5 bytes each; `l` is 1 in both
/// sequences, so a low bit can make one decrease.
fn good() -> Raw {
    Raw {
        magic: MAGIC,
        version: VERSION,
        flags: 0,
        nodes: 3,
        entries: 8,
        data_bytes: 10,
        blocks: 1,
        starts: vec![0, 3],
        codecs: vec![0],
        byte_offsets: vec![0, 5, 5, 10],
        entry_offsets: vec![0, 4, 4, 8],
    }
}

impl Raw {
    fn bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.magic);
        out.extend_from_slice(&self.version.to_le_bytes());
        out.extend_from_slice(&self.flags.to_le_bytes());
        for x in [self.nodes, self.entries, self.data_bytes, self.blocks] {
            out.extend_from_slice(&x.to_le_bytes());
        }
        for s in &self.starts {
            out.extend_from_slice(&s.to_le_bytes());
        }
        out.resize(pad8(out.len()), 0);
        out.extend_from_slice(&self.codecs);
        out.resize(pad8(out.len()), 0);
        EliasFano::with_universe(&self.byte_offsets, self.data_bytes).write(&mut out);
        EliasFano::with_universe(&self.entry_offsets, self.entries).write(&mut out);
        out
    }

    fn with(&self, change: impl FnOnce(&mut Self)) -> Vec<u8> {
        let mut raw = self.clone();
        change(&mut raw);
        raw.bytes()
    }
}

fn refused(bytes: &[u8]) -> FormatError {
    AdjacencyIndex::parse(bytes).expect_err("a hostile index must be refused")
}

#[test]
fn the_good_index_is_what_the_encoder_writes() {
    let encoded = super::super::encode(&[0, 4, 4, 8], &[1, 2, 3, 4, 0, 1, 3, 5]).unwrap();
    assert_eq!(encoded.data.len(), 10);
    assert_eq!(good().bytes(), encoded.index);
    assert!(AdjacencyIndex::parse(&good().bytes()).is_ok());
}

#[test]
fn hostile_headers_are_refused() {
    let g = good();
    let b = g.bytes();
    assert_eq!(refused(&b[..HEADER_BYTES - 1]), FormatError::Truncated);
    assert_eq!(refused(&g.with(|r| r.magic[0] ^= 1)), FormatError::BadMagic);
    assert_eq!(
        refused(&g.with(|r| r.version = 2)),
        FormatError::UnsupportedVersion(2)
    );
    for flags in [1, 3, 4, 1 << 31] {
        assert_eq!(
            refused(&g.with(|r| r.flags = flags)),
            FormatError::UnsupportedFlags(flags)
        );
    }
    let events = AdjacencyIndex::parse(&g.with(|r| r.flags = FLAG_EVENTS)).unwrap();
    assert!(events.has_events());
    assert_eq!(events.write(), g.with(|r| r.flags = FLAG_EVENTS));
    let mut longer = b.clone();
    longer.extend_from_slice(&[0; 8]);
    assert!(matches!(refused(&longer), FormatError::Length { .. }));
    assert!(matches!(
        refused(&b[..b.len() - 8]),
        FormatError::Length { .. }
    ));
    // 2^32 nodes; 2^32 - 1 nodes with the body of three.
    assert!(matches!(
        refused(&g.with(|r| r.nodes = 1 << 32)),
        FormatError::Invalid(_)
    ));
    assert!(matches!(
        refused(&g.with(|r| r.nodes = u64::from(u32::MAX))),
        FormatError::Length { .. }
    ));
    // Totals at u64::MAX: the parts no longer have the lengths of the body.
    let either = |e: FormatError| matches!(e, FormatError::Length { .. } | FormatError::Invalid(_));
    assert!(either(refused(&g.with(|r| r.entries = u64::MAX))));
    assert!(either(refused(&g.with(|r| r.data_bytes = u64::MAX))));
}

#[test]
fn hostile_block_directories_are_refused() {
    let g = good();
    let invalid = |b: Vec<u8>| assert!(matches!(refused(&b), FormatError::Invalid(_)));
    invalid(g.with(|r| r.blocks = 0));
    invalid(g.with(|r| r.blocks = 4));
    invalid(g.with(|r| r.blocks = u64::MAX));
    invalid(g.with(|r| r.starts = vec![1, 3]));
    invalid(g.with(|r| r.starts = vec![0, 2]));
    invalid(g.with(|r| {
        r.blocks = 3;
        r.starts = vec![0, 1, 1, 3];
        r.codecs = vec![0; 3];
    }));
    invalid(g.with(|r| {
        r.blocks = 3;
        r.starts = vec![0, 2, 1, 3];
        r.codecs = vec![0; 3];
    }));
    assert_eq!(
        refused(&g.with(|r| r.codecs = vec![1])),
        FormatError::UnsupportedBlockCodec { block: 0, codec: 1 }
    );
    // Padding of the codecs (byte 1 of their 8), and of the starts with two blocks
    // (bytes 12..16 of their 16).
    let codecs_at = HEADER_BYTES + 8;
    let mut b = g.bytes();
    b[codecs_at + 1] = 1;
    invalid(b);
    let two = g.with(|r| {
        r.blocks = 2;
        r.starts = vec![0, 1, 3];
        r.codecs = vec![0; 2];
    });
    assert!(AdjacencyIndex::parse(&two).is_ok());
    let mut b = two;
    b[HEADER_BYTES + 12] = 1;
    invalid(b);
}

#[test]
fn hostile_offsets_are_refused() {
    let g = good();
    let invalid = |b: Vec<u8>| assert!(matches!(refused(&b), FormatError::Invalid(_)));
    // Elias–Fano structure: the entry offsets are the last 16 bytes (one low word,
    // one high word). Their high bits are at 0, 3, 4 and 7; bit 1 is a fifth one,
    // bit 63 is padding.
    let b = g.bytes();
    let high = b.len() - 8;
    let mut extra_one = b.clone();
    extra_one[high] |= 1 << 1;
    invalid(extra_one);
    let mut padding = b.clone();
    padding[high + 7] |= 1 << 7;
    invalid(padding);
    // Values: not starting at 0, not ending at the total, decreasing (a low bit:
    // 5 then 4 keeps every high bit, so the ones count is right).
    invalid(g.with(|r| r.entry_offsets = vec![1, 4, 4, 8]));
    invalid(g.with(|r| {
        r.data_bytes = 11;
        r.byte_offsets = vec![0, 5, 5, 10];
    }));
    invalid(g.with(|r| r.byte_offsets = vec![0, 5, 4, 10]));
    invalid(g.with(|r| r.entry_offsets = vec![0, 5, 4, 8]));
    // Record lengths against degrees: each bound on its own.
    // Too short: 4 bytes for degree 4 (at least 1 + 4); node 2 stays possible (6).
    invalid(g.with(|r| r.byte_offsets = vec![0, 4, 4, 10]));
    // Too long: degrees 1, 0, 2; 6 bytes for degree 1 (at most 1 + 4); node 2 has 3.
    invalid(g.with(|r| {
        r.entries = 3;
        r.entry_offsets = vec![0, 1, 1, 3];
        r.data_bytes = 9;
        r.byte_offsets = vec![0, 6, 6, 9];
    }));
    // A record for an empty list: node 1 has degree 0 and 1 byte.
    invalid(g.with(|r| {
        r.data_bytes = 11;
        r.byte_offsets = vec![0, 5, 6, 11];
    }));
    // A degree of 2^33, with a record length that would fit it.
    invalid(
        Raw {
            nodes: 1,
            entries: 1 << 33,
            data_bytes: (1 << 31) + (1 << 33),
            starts: vec![0, 1],
            byte_offsets: vec![0, (1 << 31) + (1 << 33)],
            entry_offsets: vec![0, 1 << 33],
            ..good()
        }
        .bytes(),
    );
}

#[test]
fn the_length_bounds_of_a_record_are_exact() {
    // Degree 4: 5 to 17 bytes are possible, 4 and 18 are not.
    let at = |len: u64| {
        Raw {
            nodes: 1,
            entries: 4,
            data_bytes: len,
            starts: vec![0, 1],
            byte_offsets: vec![0, len],
            entry_offsets: vec![0, 4],
            ..good()
        }
        .bytes()
    };
    for len in [5, 17] {
        assert!(AdjacencyIndex::parse(&at(len)).is_ok(), "{len}");
    }
    for len in [4, 18] {
        assert!(
            matches!(refused(&at(len)), FormatError::Invalid(_)),
            "{len}"
        );
    }
}
