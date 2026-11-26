// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The `.egmap` reader on untrusted bytes (`docs/spec/egmap.md` § 9, issue #68): decoding
//! a tile payload, opening a map, finding tiles, reading chunks and verifying all of it
//! must never panic, read out of bounds or allocate past the caps. A payload that decodes
//! encodes back to the same bytes, apart from its minor version and the optional blocks
//! it skipped (§ 4.3).
//!
//! The first byte picks the mode:
//! - 0 mod 4: the rest is a tile payload;
//! - 1 mod 4: the rest is a whole file;
//! - 2 mod 4: the rest damages a small valid map (each 3 bytes: a position, a byte to
//!   xor), which reaches framing that is valid up to a late byte;
//! - 3 mod 4: the rest is a zoom pair, a tile count, then the uncompressed root
//!   directory, leaf directory and tile payload (each but the last after a `u16`
//!   length). The harness compresses them and lays them out in the small map, so that
//!   the directory and tile rules are reached behind gzip's check.

#![no_main]

use std::io::Write;
use std::sync::OnceLock;

use everygraph_core::{Memory, MemorySource};
use everygraph_map::egmap::{
    Anchor, Label, MapInfo, MapReader, MapWriter, Metadata, Node, Region, RegionKind, Tile,
    Values, ZoomRule,
};
use libfuzzer_sys::fuzz_target;

fn tile(bytes: &[u8]) {
    if let Ok(t) = Tile::decode(bytes) {
        let mut again = t.encode().expect("a decoded tile encodes");
        again[6..8].copy_from_slice(&bytes[6..8]);
        // Optional blocks a reader skips come after the known ones (their numbers are
        // higher), so the writer's bytes are the input up to them.
        assert!(bytes.starts_with(&again), "a tile has two encodings");
    }
}

fn file(bytes: Vec<u8>) {
    let Ok(reader) = MapReader::open(MemorySource::new(bytes)) else {
        return;
    };
    let _ = reader.metadata();
    for (z, x, y) in [(0, 0, 0), (1, 1, 0), (1, 0, 0), (2, 1, 1)] {
        let _ = reader.tile(z, x, y);
    }
    for s in reader.sections() {
        let _ = reader.chunk(s, 0);
    }
    let _ = reader.verify(&Memory::unlimited());
}

fn gzip(data: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    e.write_all(data).expect("into a Vec");
    e.finish().expect("into a Vec")
}

/// A map of two tiles, the region table and two node arrays.
fn sample() -> &'static [u8] {
    static SAMPLE: OnceLock<Vec<u8>> = OnceLock::new();
    SAMPLE.get_or_init(|| {
        let region = Region {
            id: 1,
            x: 0.25,
            y: 0.25,
            r: 0.2,
            nodes: 2,
            kind: RegionKind::Community,
            colour: 2,
            zmin: 0,
        };
        let node = Node {
            id: 0,
            region: 1,
            x: 0.2,
            y: 0.3,
            r: 0.01,
            name: 1,
            degree: 1,
            colour: 2,
            zmin: 1,
        };
        let label = Label {
            entity: 0,
            text: 0,
            z_on: 0,
            z_off: 2,
            anchor: Anchor::Centre,
        };
        let mut top = Tile::new(0, 0, 0);
        top.regions.push(region);
        top.labels.push(label);
        top.strings.push("#1".into());
        let mut deep = Tile::new(1, 0, 0);
        deep.regions.push(region);
        deep.nodes.push(node);
        deep.labels.push(label);
        deep.strings = vec!["#1".into(), "n0".into()];
        let info = MapInfo {
            nodes: 2,
            regions: 2,
            edges: 1,
            summary: 0,
        };
        let mut w = MapWriter::create(MemorySource::new(Vec::new()), &Memory::unlimited(), info)
            .expect("an empty sink");
        w.add_tile(&top).expect("a valid tile");
        w.add_tile(&deep).expect("a valid tile");
        for (name, values) in [
            ("adjacency.offsets", Values::U64(&[0, 1, 2])),
            ("adjacency.targets", Values::U32(&[1, 0])),
            ("positions", Values::F32(&[0.2, 0.3, 0.3, 0.2])),
            ("regions.parent", Values::U32(&[0, 0])),
            ("regions.kind", Values::U8(&[5, 0])),
            ("regions.nodes", Values::U64(&[2, 2])),
            ("regions.internal", Values::U64(&[1, 1])),
            ("regions.volume", Values::U64(&[2, 2])),
            ("regions.communities", Values::U32(&[0, 0])),
            ("regions.partner", Values::U32(&[0, 0])),
        ] {
            w.begin_array(name, values.element(), 2).expect("a free name");
            w.write_values(values).expect("valid values");
            w.end_array().expect("an open array");
        }
        let metadata = Metadata {
            name: "fuzz".into(),
            description: String::new(),
            attribution: String::new(),
            zoom: ZoomRule {
                region_px: 3.0,
                open_px: 12.0,
                node_px: 1.5,
                label_off_px: 600.0,
            },
        };
        w.finish(&metadata).expect("a valid map").into_inner()
    })
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().expect("8 bytes"))
}

/// The sample with a root directory, a leaf and a tile made from `rest` (mode 3).
fn laid_out(rest: &[u8]) -> Option<Vec<u8>> {
    let [lo, hi, count, rest @ ..] = rest else {
        return None;
    };
    let mut parts = rest;
    let mut take = || {
        let (len, tail) = parts.split_first_chunk::<2>()?;
        let n = usize::from(u16::from_le_bytes(*len)).min(tail.len());
        let (part, tail) = tail.split_at(n);
        parts = tail;
        Some(part)
    };
    let (root, leaf) = (take()?, take()?);
    let (root, leaf, payload) = (gzip(root), gzip(leaf), gzip(parts));
    let mut map = sample().to_vec();
    let table_at = 127 + usize::try_from(u64_at(&map, 16)).ok()?;
    let entries = u32::from_le_bytes(map[table_at + 12..table_at + 16].try_into().ok()?);
    let table = map[table_at..table_at + 48 + 64 * usize::try_from(entries).ok()?].to_vec();
    if 127 + root.len() + table.len() > 16_384 {
        return None;
    }
    map[127..16_384].fill(0);
    map[127..127 + root.len()].copy_from_slice(&root);
    map[127 + root.len()..127 + root.len() + table.len()].copy_from_slice(&table);
    let len = |b: &[u8]| u64::try_from(b.len()).expect("a length fits in 64 bits");
    let tiles_at = len(&map);
    map.extend_from_slice(&payload);
    let leaves_at = len(&map);
    map.extend_from_slice(&leaf);
    for (offset, value) in [
        (16, len(&root)),
        (40, leaves_at),
        (48, len(&leaf)),
        (56, tiles_at),
        (64, len(&payload)),
        (72, u64::from(*count)),
        (80, u64::from(*count)),
        (88, u64::from(*count)),
    ] {
        map[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }
    map[100] = *lo % 17;
    map[101] = *hi % 17;
    Some(map)
}

fuzz_target!(|bytes: &[u8]| {
    let Some((&mode, rest)) = bytes.split_first() else {
        return;
    };
    match mode % 4 {
        0 => tile(rest),
        1 => file(rest.to_vec()),
        2 => {
            let mut map = sample().to_vec();
            for hit in rest.chunks_exact(3) {
                let at = usize::from(u16::from_le_bytes([hit[0], hit[1]])) * 7 % map.len();
                map[at] ^= hit[2] | 1;
            }
            file(map);
        }
        _ => {
            if let Some(map) = laid_out(rest) {
                file(map);
            }
        }
    }
});
