// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The `.egmap` reader on untrusted bytes (`docs/spec/egmap.md` § 9, issue #68): decoding
//! a tile payload, opening a map, finding tiles, reading chunks and verifying all of it
//! must never panic, read out of bounds or allocate past the caps. A payload that decodes
//! encodes back to the same bytes, apart from its minor version and the optional blocks
//! it skipped (§ 4.3).
//!
//! The first byte picks the mode:
//! - 0 mod 3: the rest is a tile payload;
//! - 1 mod 3: the rest is a whole file;
//! - 2 mod 3: the rest damages a small valid map (each 3 bytes: a position, a byte to
//!   xor), which reaches framing that is valid up to a late byte.

#![no_main]

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
    let _ = reader.tile(0, 0, 0);
    let _ = reader.tile(1, 1, 0);
    for s in reader.sections() {
        let _ = reader.chunk(s, 0);
    }
    let _ = reader.verify(&Memory::unlimited());
}

/// A map of two tiles and three arrays.
fn sample() -> &'static [u8] {
    static SAMPLE: OnceLock<Vec<u8>> = OnceLock::new();
    SAMPLE.get_or_init(|| {
        let region = Region {
            id: 1,
            parent: 0,
            x: 0.25,
            y: 0.25,
            r: 0.2,
            nodes: 2,
            internal: 1,
            volume: 3,
            communities: 0,
            rank: 1,
            partner: 0,
            kind: RegionKind::Community,
            depth: 1,
            colour: 2,
            zmin: 0,
        };
        let node = Node {
            id: 0,
            region: 1,
            x: 0.2,
            y: 0.3,
            r: 0.01,
            name: 0,
            degree: 1,
            zmin: 1,
        };
        let mut top = Tile::new(0, 0, 0);
        top.regions.push(region);
        top.labels.push(Label {
            entity: 0,
            text: 0,
            z_on: 0,
            z_off: 2,
            anchor: Anchor::Centre,
        });
        top.strings.push("#1".into());
        let mut deep = Tile::new(1, 0, 0);
        deep.regions.push(region);
        deep.nodes.push(node);
        deep.labels = top.labels.clone();
        deep.strings = vec!["#1".into(), "n0".into()];
        deep.nodes[0].name = 1;
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

fuzz_target!(|bytes: &[u8]| {
    let Some((&mode, rest)) = bytes.split_first() else {
        return;
    };
    match mode % 3 {
        0 => tile(rest),
        1 => file(rest.to_vec()),
        _ => {
            let mut map = sample().to_vec();
            for hit in rest.chunks_exact(3) {
                let at = usize::from(u16::from_le_bytes([hit[0], hit[1]])) * 7 % map.len();
                map[at] ^= hit[2] | 1;
            }
            file(map);
        }
    }
});
