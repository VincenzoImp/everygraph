// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The `.egmap` file (`docs/spec/egmap.md`, #68).
//!
//! - Round trip: a random tile that passes § 4.3 decodes to itself, and its bytes are
//!   the only bytes of that tile: any damaged payload is refused or decodes to a tile
//!   that encodes back to exactly those bytes.
//! - Oracle: a pyramid placed by the rules of § 3 from a test generator
//!   (`tests/support`) reads back tile by tile and chunk by chunk, the strictest reader
//!   (`MapReader::verify`) accepts it, and its census finds no fault. The census finds
//!   each fault planted in it.
//! - Hostile input: damaged files are refused without a panic; directories that share
//!   or repeat tile bytes, and sizes the file cannot hold, are refused before anything
//!   is allocated for them.
//! - Faults: a write that fails at any operation ends in an error.

#![forbid(unsafe_code)]
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::cast_lossless
)]

mod support;

use std::io::Write;

use everygraph_core::{Memory, MemorySource};
use everygraph_io::faults::{Faulty, Plan};
use everygraph_map::egmap::{
    Census, EgmapError, Element, Label, MapInfo, MapReader, MapWriter, RegionKind, Tile, Values,
    min_zoom, tile_id, tile_of_id,
};
use proptest::prelude::*;
use support::{Pyramid, RULE, Rng, metadata, pyramid, random_tile, write_into};

type Mem = MemorySource<Vec<u8>>;

fn write(p: &Pyramid, chunk: u32) -> Vec<u8> {
    write_into(p, Mem::new(Vec::new()), chunk)
        .unwrap()
        .into_inner()
}

fn array(reader: &MapReader<Mem>, name: &str) -> Vec<u8> {
    let s = reader.section(name).unwrap();
    (0..s.chunks())
        .flat_map(|i| reader.chunk(s, i).unwrap())
        .collect()
}

fn le<T: Copy>(values: &[T], f: impl Fn(T) -> Vec<u8>) -> Vec<u8> {
    values.iter().flat_map(|&v| f(v)).collect()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 200, ..ProptestConfig::default() })]

    #[test]
    fn a_tile_round_trips_and_its_bytes_are_canonical(
        seed in any::<u64>(), z in 0_u8..=8, regions in 0_usize..40, nodes in 0_usize..60,
    ) {
        let tile = random_tile(&mut Rng(seed), z, regions, nodes);
        let bytes = tile.encode().unwrap();
        prop_assert_eq!(bytes.len() % 8, 0);
        let back = Tile::decode(&bytes).unwrap();
        prop_assert_eq!(&back, &tile);
        prop_assert_eq!(back.encode().unwrap(), bytes);
    }

    #[test]
    fn a_damaged_tile_is_refused_or_reads_as_exactly_its_bytes(
        seed in any::<u64>(), hits in proptest::collection::vec((any::<u16>(), any::<u8>()), 1..4),
        cut in any::<u16>(),
    ) {
        let tile = random_tile(&mut Rng(seed), 5, 6, 8);
        let mut bytes = tile.encode().unwrap();
        for (at, x) in hits {
            let i = usize::from(at) % bytes.len();
            bytes[i] ^= x | 1;
        }
        if cut % 4 == 0 {
            bytes.truncate(usize::from(cut) % bytes.len());
        }
        if let Ok(t) = Tile::decode(&bytes) {
            // Minor versions read alike (§ 8); the writer writes its own.
            bytes[6..8].fill(0);
            prop_assert_eq!(t.encode().unwrap(), bytes);
        }
    }

    #[test]
    fn a_damaged_file_is_refused_or_read_without_a_panic(
        hits in proptest::collection::vec((any::<u32>(), any::<u8>()), 1..6),
    ) {
        let mut bytes = SMALL.with(Clone::clone);
        for (at, x) in hits {
            let i = at as usize % bytes.len();
            bytes[i] ^= x | 1;
        }
        if let Ok(reader) = MapReader::open(Mem::new(bytes)) {
            let _ = reader.verify(&Memory::unlimited());
            let _ = reader.tile(2, 1, 1);
        }
    }
}

thread_local! {
    static SMALL: Vec<u8> = write(&pyramid(3, 3), 5);
}

#[test]
fn a_tile_that_breaks_a_rule_is_refused_by_the_writer() {
    let base = random_tile(&mut Rng(7), 6, 8, 10);
    assert!(base.encode().is_ok());
    let a_child = base.regions.iter().position(|g| g.parent != 0).unwrap();
    type Edit = Box<dyn Fn(&mut Tile)>;
    let breaks: Vec<(&str, Edit)> = vec![
        ("zoom 21", Box::new(|t| t.z = 21)),
        ("x outside the zoom", Box::new(|t| t.x = 64)),
        ("ids out of order", Box::new(|t| t.regions.swap(0, 1))),
        (
            "parent missing",
            Box::new(move |t| {
                let p = t.regions[a_child].id - 1;
                assert!(t.regions.iter().all(|g| g.id != p));
                t.regions[a_child].parent = p;
            }),
        ),
        ("depth", Box::new(move |t| t.regions[a_child].depth += 1)),
        (
            "colour of the parent",
            Box::new(move |t| t.regions[a_child].colour = (t.regions[a_child].colour + 1) % 5),
        ),
        (
            "colour 5",
            Box::new(|t| t.regions.iter_mut().for_each(|g| g.colour = 5)),
        ),
        ("zmin above the zoom", Box::new(|t| t.nodes[0].zmin = 7)),
        ("no nodes", Box::new(|t| t.regions[0].nodes = 0)),
        (
            "volume below twice internal",
            Box::new(|t| t.regions[0].volume = 2 * t.regions[0].internal - 1),
        ),
        ("rank 0", Box::new(|t| t.regions[0].rank = 0)),
        (
            "own partner",
            Box::new(|t| t.regions[0].partner = t.regions[0].id),
        ),
        (
            "aggregate of nothing",
            Box::new(|t| {
                t.regions[0].kind = RegionKind::Aggregate;
                t.regions[0].communities = 0;
            }),
        ),
        ("NaN", Box::new(|t| t.nodes[0].x = f32::NAN)),
        ("negative zero", Box::new(|t| t.nodes[0].y = -0.0)),
        ("radius 0", Box::new(|t| t.nodes[0].r = 0.0)),
        (
            "disc away from the tile",
            Box::new(|t| {
                t.nodes[0].x = if t.x == 0 { 0.9 } else { 0.0 };
                t.nodes[0].r = 1e-6;
            }),
        ),
        ("node region missing", Box::new(|t| t.nodes[0].region = 0)),
        (
            "name out of range",
            Box::new(|t| t.nodes[0].name = t.strings.len() as u32),
        ),
        (
            "label of nothing",
            Box::new(|t| {
                t.labels.push(Label {
                    entity: 10_000,
                    ..t.labels[0]
                });
            }),
        ),
        (
            "label off at this zoom",
            Box::new(|t| t.labels[0].z_off = t.z),
        ),
        ("label past zoom 20", Box::new(|t| t.labels[0].z_off = 22)),
        (
            "label before its entity",
            Box::new(|t| {
                let i = t
                    .labels
                    .iter()
                    .position(|a| a.entity as usize >= t.regions.len())
                    .unwrap();
                let v = t.labels[i].entity as usize - t.regions.len();
                t.nodes[v].zmin = t.z;
                t.labels[i].z_on = t.z - 1;
            }),
        ),
    ];
    for (what, edit) in breaks {
        let mut t = base.clone();
        edit(&mut t);
        assert!(
            matches!(t.encode(), Err(EgmapError::Invalid(_))),
            "the writer took a tile with {what}"
        );
    }
}

#[test]
fn blocks_a_reader_does_not_know_are_skipped_only_when_optional() {
    let tile = random_tile(&mut Rng(11), 4, 3, 3);
    let bytes = tile.encode().unwrap();
    let with = |tag: u32| {
        let mut b = bytes.clone();
        b.extend_from_slice(&tag.to_le_bytes());
        b.extend_from_slice(&1_u32.to_le_bytes());
        b.extend_from_slice(&8_u64.to_le_bytes());
        b.extend_from_slice(&[9; 8]);
        b
    };
    assert_eq!(Tile::decode(&with(0x0000_0010)).unwrap(), tile);
    assert!(matches!(
        Tile::decode(&with(0x8000_0010)),
        Err(EgmapError::Unsupported(_))
    ));
    let mut later = bytes.clone();
    later[4] = 2;
    assert!(matches!(
        Tile::decode(&later),
        Err(EgmapError::Unsupported(_))
    ));
    let mut minor = bytes;
    minor[6] = 9;
    assert_eq!(Tile::decode(&minor).unwrap(), tile);
}

#[test]
fn a_written_map_reads_back_whole_and_its_census_is_clean() {
    let p = pyramid(1, 6);
    assert!(
        p.tiles.len() > 500 && p.info.nodes > 40,
        "a pyramid too small to test"
    );
    let file = write(&p, 7);
    assert_eq!(write(&p, 7), file, "the same input gave other bytes");
    let reader = MapReader::open(Mem::new(file)).unwrap();
    assert_eq!(reader.info(), p.info);
    assert_eq!(reader.zooms(), (0, 6));
    assert_eq!(reader.tile_count(), p.tiles.len() as u64);
    let report = reader.verify(&Memory::unlimited()).unwrap();
    assert_eq!(report.tiles, p.tiles.len() as u64);
    assert_eq!(report.entities, p.info.nodes + p.info.regions - 1);
    assert!(report.labels > 0);
    let faults = [
        report.popping,
        report.vanished,
        report.late,
        report.inconsistent,
        report.label_popping,
        report.label_cut,
    ];
    assert_eq!(faults, [0; 6], "{report:?}");
    for t in &p.tiles {
        assert_eq!(reader.tile(t.z, t.x, t.y).unwrap().as_ref(), Some(t));
    }
    let absent = (0..=6)
        .flat_map(|z| (0..1_u32 << z).map(move |x| (z, x, 0)))
        .find(|&(z, x, y)| !p.tiles.iter().any(|t| (t.z, t.x, t.y) == (z, x, y)))
        .unwrap();
    assert_eq!(reader.tile(absent.0, absent.1, absent.2).unwrap(), None);
    assert_eq!(reader.tile(7, 0, 0).unwrap(), None);
    assert_eq!(
        array(&reader, "positions"),
        le(&p.positions, |v| v.to_le_bytes().to_vec())
    );
    assert_eq!(
        array(&reader, "adjacency.offsets"),
        le(&p.adjacency.0, |v| v.to_le_bytes().to_vec())
    );
    assert_eq!(
        array(&reader, "adjacency.targets"),
        le(&p.adjacency.1, |v| v.to_le_bytes().to_vec())
    );
    assert_eq!(
        array(&reader, "matrix.pairs"),
        le(&p.matrix.1, |v| v.to_le_bytes().to_vec())
    );
    assert_eq!(
        array(&reader, "matrix.weights"),
        le(&p.matrix.2, |v| v.to_le_bytes().to_vec())
    );
    let s = reader.section("positions").unwrap();
    assert_eq!(
        (s.element, s.chunk, s.count),
        (Element::F32, 7, 2 * p.info.nodes)
    );
    let meta: serde_json::Value = serde_json::from_str(&reader.metadata().unwrap()).unwrap();
    assert_eq!(meta["format"], "everygraph-egmap");
    assert_eq!(meta["version"], "1.0");
    assert_eq!(meta["name"], "test \"graph\"\n");
    assert_eq!(meta["zoom"]["node_px"], 1.5);
}

#[test]
fn the_census_finds_each_planted_fault() {
    let p = pyramid(2, 5);
    let census = |tiles: &[Tile]| {
        let mut c = Census::new(&Memory::unlimited());
        tiles.iter().for_each(|t| c.add(t).unwrap());
        c.finish(5)
    };
    assert_eq!(census(&p.tiles).popping, 0);
    // A node shown at its zmin, gone one zoom deeper, back after.
    let v = p
        .tiles
        .iter()
        .flat_map(|t| &t.nodes)
        .find(|v| v.zmin <= 3)
        .unwrap()
        .id;
    let zmin = p
        .tiles
        .iter()
        .flat_map(|t| &t.nodes)
        .find(|n| n.id == v)
        .unwrap()
        .zmin;
    let drop = |tiles: &mut Vec<Tile>, z: u8| {
        for t in tiles.iter_mut().filter(|t| t.z == z) {
            if let Some(j) = t.nodes.iter().position(|n| n.id == v) {
                let e = (t.regions.len() + j) as u32;
                t.labels.retain(|a| a.entity != e);
                for a in &mut t.labels {
                    if a.entity > e {
                        a.entity -= 1;
                    }
                }
                t.nodes.remove(j);
            }
        }
    };
    let mut gap = p.tiles.clone();
    drop(&mut gap, zmin + 1);
    let r = census(&gap);
    assert_eq!((r.popping, r.vanished, r.late), (1, 0, 0), "{r:?}");
    let mut late = p.tiles.clone();
    drop(&mut late, zmin);
    assert_eq!(census(&late).late, 1);
    let mut gone = p.tiles.clone();
    drop(&mut gone, 5);
    assert_eq!(census(&gone).vanished, 1);
    let mut moved = p.tiles.clone();
    let t = moved
        .iter_mut()
        .rev()
        .find(|t| t.regions.len() > 1)
        .unwrap();
    t.regions[1].internal += 1;
    t.regions[1].volume += 2;
    assert_eq!(census(&moved).inconsistent, 1);
    let mut unlabelled = p.tiles.clone();
    let (z, e) = unlabelled
        .iter()
        .find_map(|t| {
            t.labels
                .iter()
                .find(|a| a.z_off > t.z + 2)
                .map(|a| (t.z + 1, t.regions.get(a.entity as usize).map(|g| g.id)))
        })
        .unwrap();
    let id = e.unwrap();
    for t in unlabelled.iter_mut().filter(|t| t.z == z) {
        if let Some(i) = t.regions.iter().position(|g| g.id == id) {
            t.labels.retain(|a| a.entity as usize != i);
        }
    }
    let r = census(&unlabelled);
    assert_eq!((r.label_popping, r.label_cut), (1, 1), "{r:?}");
}

#[test]
fn the_zoom_rule_is_exact_and_nests() {
    assert_eq!(min_zoom(3.0 / 256.0, 3.0), 0);
    assert_eq!(
        min_zoom(f32::from_bits((3.0_f32 / 256.0).to_bits() - 1), 3.0),
        1
    );
    assert_eq!(min_zoom(3.0 / 256.0 / 1024.0, 3.0), 10);
    assert_eq!(min_zoom(1e-12, 3.0), 21);
    for r in [0.4_f32, 0.01, 1e-4] {
        let parent = RULE.region_zmin(r, None, 12);
        let child = RULE.region_zmin(r / 3.0, Some((r, parent)), 12);
        let node = RULE.node_zmin(r / 30.0, (r / 3.0, child), 12);
        assert!(parent <= child && child <= node && node <= 12);
    }
}

/// An encoder of PMTiles directories written from the PMTiles v3 specification, to
/// build directories the writer never makes.
fn directory(entries: &[(u64, u64, u64, u64)]) -> Vec<u8> {
    fn varint(out: &mut Vec<u8>, mut v: u64) {
        while v >= 0x80 {
            out.push((v as u8) | 0x80);
            v >>= 7;
        }
        out.push(v as u8);
    }
    let mut raw = Vec::new();
    varint(&mut raw, entries.len() as u64);
    let mut last = 0;
    for &(id, ..) in entries {
        varint(&mut raw, id - last);
        last = id;
    }
    for &(_, run, ..) in entries {
        varint(&mut raw, run);
    }
    for &(.., len, _) in entries {
        varint(&mut raw, len);
    }
    for &(.., off) in entries {
        varint(&mut raw, off + 1);
    }
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&raw).unwrap();
    gz.finish().unwrap()
}

/// `file` with its root directory replaced by `root` (which must fit where it was).
fn with_root(file: &[u8], root: &[u8]) -> Vec<u8> {
    let mut out = file.to_vec();
    let old = u64::from_le_bytes(out[16..24].try_into().unwrap()) as usize;
    let table_at = 127 + old;
    let table_len = 48
        + 64 * u32::from_le_bytes(out[table_at + 12..table_at + 16].try_into().unwrap()) as usize;
    let table = out[table_at..table_at + table_len].to_vec();
    assert!(127 + root.len() + table.len() <= 16_384);
    out[127..127 + root.len()].copy_from_slice(root);
    out[127 + root.len()..127 + root.len() + table.len()].copy_from_slice(&table);
    out[16..24].copy_from_slice(&(root.len() as u64).to_le_bytes());
    out
}

#[test]
fn directories_that_repeat_or_share_tile_bytes_are_refused() {
    let p = pyramid(4, 2);
    let file = write(&p, 64);
    let ok = MapReader::open(Mem::new(file.clone())).unwrap();
    let mut entries = Vec::new();
    let mut off = 0;
    ok.for_each_tile(|t| {
        let len = {
            let mut b = Vec::new();
            let mut gz = flate2::write::GzEncoder::new(&mut b, flate2::Compression::new(6));
            gz.write_all(&t.encode().unwrap()).unwrap();
            gz.finish().unwrap();
            b.len() as u64
        };
        entries.push((tile_id(t.z, t.x, t.y), 1, len, off));
        off += len;
        Ok(())
    })
    .unwrap();
    // The rebuilt root is the writer's: the file still reads.
    let same = with_root(&file, &directory(&entries));
    MapReader::open(Mem::new(same))
        .unwrap()
        .verify(&Memory::unlimited())
        .unwrap();
    // A run of 2: one tile's bytes stand for two tiles.
    let mut run = entries.clone();
    run[0].1 = 2;
    let refused = MapReader::open(Mem::new(with_root(&file, &directory(&run))));
    assert!(
        matches!(refused, Err(EgmapError::Malformed(_))),
        "{refused:?}"
    );
    // Two entries on the same bytes.
    let mut shared = entries.clone();
    shared[1].3 = shared[0].3;
    shared[1].2 = shared[0].2;
    let reader = MapReader::open(Mem::new(with_root(&file, &directory(&shared)))).unwrap();
    assert!(matches!(
        reader.verify(&Memory::unlimited()),
        Err(EgmapError::Malformed(_))
    ));
    // A tile stored under another tile's id.
    let mut swapped = entries.clone();
    let (a, b) = (swapped[0].3, swapped[1].3);
    swapped[0].3 = b;
    swapped[1].3 = a;
    (swapped[0].2, swapped[1].2) = (swapped[1].2, swapped[0].2);
    let (z, x, y) = tile_of_id(entries[0].0).unwrap();
    let reader = MapReader::open(Mem::new(with_root(&file, &directory(&swapped)))).unwrap();
    assert!(matches!(
        reader.tile(z, x, y),
        Err(EgmapError::Malformed(_))
    ));
    // An entry past the tile data.
    let mut past = entries;
    past.last_mut().unwrap().2 += 1;
    assert!(MapReader::open(Mem::new(with_root(&file, &directory(&past)))).is_err());
}

#[test]
fn sizes_the_file_cannot_hold_are_refused_before_they_are_allocated() {
    let file = SMALL.with(Clone::clone);
    let root_len = u64::from_le_bytes(file[16..24].try_into().unwrap()) as usize;
    let table = 127 + root_len;
    let open = |edit: &dyn Fn(&mut Vec<u8>)| {
        let mut f = file.clone();
        edit(&mut f);
        MapReader::open(Mem::new(f))
    };
    // A section table of 4 billion entries.
    assert!(open(&|f| f[table + 12..table + 16].copy_from_slice(&u32::MAX.to_le_bytes())).is_err());
    // A section that would end past the file.
    assert!(
        open(&|f| {
            let e = table + 48 + 56;
            f[e..e + 8].copy_from_slice(&(u64::MAX / 2).to_le_bytes());
        })
        .is_err()
    );
    // Metadata of 2^60 bytes.
    assert!(open(&|f| f[32..40].copy_from_slice(&(1_u64 << 60).to_le_bytes())).is_err());
    // Counts that disagree.
    assert!(open(&|f| f[80] ^= 1).is_err());
    // Another tile type: an MVT archive is not a map.
    assert!(matches!(open(&|f| f[99] = 1), Err(EgmapError::Foreign(_))));
    // A cut file.
    assert!(MapReader::open(Mem::new(file[..file.len() - 1].to_vec())).is_err());
    assert!(MapReader::open(Mem::new(file[..200].to_vec())).is_err());
}

#[test]
fn many_tiles_go_through_leaf_directories() {
    let count = 70_000_u64;
    let first = tile_id(9, 0, 0);
    let info = MapInfo::default();
    let mut w = MapWriter::create(Mem::new(Vec::new()), &Memory::unlimited(), info).unwrap();
    for id in first..first + count {
        let (z, x, y) = tile_of_id(id).unwrap();
        w.add_tile(&Tile::new(z, x, y)).unwrap();
    }
    let file = w.finish(&metadata()).unwrap().into_inner();
    let leaves = u64::from_le_bytes(file[48..56].try_into().unwrap());
    assert!(leaves > 0, "70,000 tiles fit the root directory");
    let reader = MapReader::open(Mem::new(file)).unwrap();
    for id in [
        first,
        first + 1,
        first + 4095,
        first + 4096,
        first + count - 1,
    ] {
        let (z, x, y) = tile_of_id(id).unwrap();
        assert_eq!(reader.tile(z, x, y).unwrap(), Some(Tile::new(z, x, y)));
    }
    let (z, x, y) = tile_of_id(first + count).unwrap();
    assert_eq!(reader.tile(z, x, y).unwrap(), None);
    let mut seen = 0;
    reader
        .for_each_tile(|_| {
            seen += 1;
            Ok(())
        })
        .unwrap();
    assert_eq!(seen, count);
}

#[test]
fn the_writer_refuses_what_breaks_the_order_or_the_summary_rules() {
    let p = pyramid(5, 3);
    let memory = Memory::unlimited();
    let mut w = MapWriter::create(Mem::new(Vec::new()), &memory, p.info).unwrap();
    w.add_tile(&p.tiles[1]).unwrap();
    assert!(w.add_tile(&p.tiles[0]).is_err(), "a tile out of order");
    w.begin_array("positions", Element::F32, 4).unwrap();
    assert!(
        w.write_values(Values::U32(&[1])).is_err(),
        "values of another type"
    );
    assert!(w.write_values(Values::F32(&[f32::NAN])).is_err());
    assert!(
        w.begin_array("other", Element::U8, 4).is_err(),
        "two arrays open"
    );
    w.end_array().unwrap();
    assert!(w.add_tile(&p.tiles[2]).is_err(), "a tile after an array");
    assert!(
        w.begin_array("positions", Element::F32, 4).is_err(),
        "a name taken"
    );
    assert!(w.begin_array("Bad name", Element::U8, 4).is_err());
    assert!(
        w.begin_array("big", Element::U64, 1 << 18).is_err(),
        "a chunk over 1 MiB"
    );
    assert!(
        w.finish(&metadata()).is_err(),
        "positions of the wrong length"
    );
    let summary = MapInfo {
        summary: 5,
        ..p.info
    };
    let mut w = MapWriter::create(Mem::new(Vec::new()), &memory, summary).unwrap();
    let with_nodes = p.tiles.iter().find(|t| !t.nodes.is_empty()).unwrap();
    assert!(w.add_tile(with_nodes).is_err(), "nodes in a summary map");
    w.begin_array("positions", Element::F32, 4).unwrap();
    w.write_values(Values::F32(&p.positions)).unwrap();
    w.end_array().unwrap();
    assert!(w.finish(&metadata()).is_err(), "positions in a summary map");
    assert!(MapWriter::create(Mem::new(vec![0]), &memory, p.info).is_err());
}

#[test]
fn a_write_that_fails_at_any_operation_ends_in_an_error() {
    let p = pyramid(6, 2);
    let clean = Faulty::new(Mem::new(Vec::new()), Plan::NONE);
    let done = write_into(&p, clean, 3).unwrap();
    let ops = done.ops();
    let expected = done.into_inner().into_inner();
    assert!(ops > 10);
    for k in 0..ops {
        let sink = Faulty::new(Mem::new(Vec::new()), Plan::at(k));
        assert!(
            write_into(&p, sink, 3).is_err(),
            "operation {k} failed and the write succeeded"
        );
    }
    let reader = MapReader::open(Mem::new(expected)).unwrap();
    reader.verify(&Memory::unlimited()).unwrap();
}

#[test]
fn a_map_refuses_what_does_not_fit_it() {
    // One direction of a pair missing: written, then refused by verify.
    let mut p = pyramid(8, 3);
    let u = p.adjacency.0.windows(2).position(|w| w[1] > w[0]).unwrap();
    p.adjacency.1.remove(p.adjacency.0[u] as usize);
    p.adjacency.0[u + 1..].iter_mut().for_each(|o| *o -= 1);
    let reader = MapReader::open(Mem::new(write(&p, 5))).unwrap();
    let refused = reader.verify(&Memory::unlimited());
    assert!(
        matches!(refused, Err(EgmapError::Malformed(ref m)) if m.contains("symmetric")),
        "{refused:?}"
    );
    // Entities the section table does not count, and kind other in a full map.
    let p = pyramid(8, 3);
    let t = p.tiles.iter().find(|t| !t.nodes.is_empty()).unwrap();
    let memory = Memory::unlimited();
    for info in [
        MapInfo {
            regions: 1,
            ..p.info
        },
        MapInfo { nodes: 1, ..p.info },
    ] {
        let mut w = MapWriter::create(Mem::new(Vec::new()), &memory, info).unwrap();
        assert!(w.add_tile(t).is_err(), "{info:?}");
    }
    let mut other = t.clone();
    other.regions[0].kind = RegionKind::Other;
    other.regions[0].communities = 1;
    let mut w = MapWriter::create(Mem::new(Vec::new()), &memory, p.info).unwrap();
    assert!(w.add_tile(&other).is_err(), "kind other in a full map");
    // A known block number without its required bit.
    let mut bytes = t.encode().unwrap();
    bytes[48 + 3] &= 0x7F;
    assert!(matches!(
        Tile::decode(&bytes),
        Err(EgmapError::Malformed(_))
    ));
}
