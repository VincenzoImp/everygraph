// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The `.egmap` file (`docs/spec/egmap.md`, #68).
//!
//! - Round trip: a random tile that passes § 4.3 decodes to itself, and its bytes are
//!   the only bytes of that tile: any damaged payload is refused or decodes to a tile
//!   that encodes back to those bytes.
//! - Oracle: a pyramid placed by the rules of § 3 from a test generator
//!   (`tests/support`) reads back tile by tile and chunk by chunk, the strictest reader
//!   (`MapReader::verify`) accepts it, and its census finds no fault. The census and
//!   `verify` find each fault planted in it.
//! - Hostile input: damaged files are refused without a panic; directories that share,
//!   repeat or overflow, and sizes the file cannot hold, are refused before anything is
//!   allocated for them.
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

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use everygraph_core::{Budget, ByteSource, Memory, MemorySource, ReadError};
use everygraph_io::faults::{Faulty, Plan};
use everygraph_map::egmap::{
    Census, EgmapError, Element, Label, MAX_ZOOM, MapInfo, MapReader, MapWriter, RegionKind, Tile,
    Values, tile_id, tile_of_id,
};
use proptest::prelude::*;
use support::{
    Pyramid, RULE, RegionTable, Rng, array as write_array, empty_tiles, metadata, pyramid,
    random_tile, size, write_into, write_table,
};

type Mem = MemorySource<Vec<u8>>;

fn write(p: &Pyramid, chunk: u32) -> Vec<u8> {
    write_into(p, Mem::new(Vec::new()), chunk)
        .unwrap()
        .into_inner()
}

fn read_array(reader: &MapReader<Mem>, name: &str) -> Vec<u8> {
    let s = reader.section(name).unwrap();
    (0..s.chunks())
        .flat_map(|i| reader.chunk(s, i).unwrap())
        .collect()
}

fn le<T: Copy, const W: usize>(values: &[T], f: impl Fn(T) -> [u8; W]) -> Vec<u8> {
    values.iter().flat_map(|&v| f(v)).collect()
}

/// A budget of 256 MiB: every test's `verify` runs under a limit, so the memory it
/// keeps per declared region is counted (§ 9).
fn bounded() -> Memory {
    Memory::from(Arc::new(Budget::new(256 << 20)))
}

fn verify(file: Vec<u8>) -> Result<everygraph_map::egmap::CensusReport, EgmapError> {
    MapReader::open(Mem::new(file))?.verify(&bounded())
}

fn refused(result: Result<impl std::fmt::Debug, EgmapError>, what: &str) {
    assert!(
        matches!(result, Err(EgmapError::Malformed(_))),
        "{what}: {result:?}"
    );
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 200, ..ProptestConfig::default() })]

    #[test]
    fn a_tile_round_trips_and_its_bytes_are_canonical(
        seed in any::<u64>(), z in 0_u8..=MAX_ZOOM, regions in 0_usize..40, nodes in 0_usize..60,
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
            // Minor versions read alike (§ 8); the writer writes its own. Optional
            // blocks it skipped follow the known ones (§ 4.2).
            bytes[6..8].fill(0);
            prop_assert!(bytes.starts_with(&t.encode().unwrap()));
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
            let _ = reader.verify(&bounded());
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
    type Edit = Box<dyn Fn(&mut Tile)>;
    let breaks: Vec<(&str, Edit)> = vec![
        ("zoom 17", Box::new(|t| t.z = 17)),
        ("x outside the zoom", Box::new(|t| t.x = 64)),
        ("ids out of order", Box::new(|t| t.regions.swap(0, 1))),
        ("region 0", Box::new(|t| t.regions[0].id = 0)),
        (
            "kind root",
            Box::new(|t| t.regions[0].kind = RegionKind::Root),
        ),
        ("colour 5", Box::new(|t| t.regions[0].colour = 5)),
        ("node colour 5", Box::new(|t| t.nodes[0].colour = 5)),
        ("zmin above the zoom", Box::new(|t| t.nodes[0].zmin = 7)),
        ("no nodes", Box::new(|t| t.regions[0].nodes = 0)),
        ("node in region 0", Box::new(|t| t.nodes[0].region = 0)),
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
        ("disc covering the tile", Box::new(|t| t.regions[0].r = 1.0)),
        ("cover in the tile", Box::new(|t| t.cover = t.regions[0].id)),
        (
            "cover label without a cover",
            Box::new(|t| {
                t.labels.push(Label {
                    entity: (t.regions.len() + t.nodes.len()) as u32,
                    ..t.labels[0]
                });
            }),
        ),
        (
            "name out of range",
            Box::new(|t| t.nodes[0].name = t.strings.len() as u32),
        ),
        (
            "strings out of order",
            Box::new(|t| {
                t.strings.push(String::new());
            }),
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
        ("label past zoom 16", Box::new(|t| t.labels[0].z_off = 18)),
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
fn block_numbers_start_at_1_and_unknown_ones_are_skipped_only_when_optional() {
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
    // A block number 0, alone after the header.
    let mut zero = bytes[..48].to_vec();
    zero.extend_from_slice(&with(0)[bytes.len()..]);
    refused(Tile::decode(&zero), "block number 0");
    let mut later = bytes.clone();
    later[4] = 2;
    assert!(matches!(
        Tile::decode(&later),
        Err(EgmapError::Unsupported(_))
    ));
    let mut minor = bytes.clone();
    minor[6] = 9;
    assert_eq!(Tile::decode(&minor).unwrap(), tile);
    let mut bare = bytes;
    bare[48 + 3] &= 0x7F;
    refused(
        Tile::decode(&bare),
        "a known block without its required bit",
    );
}

#[test]
fn a_written_map_reads_back_whole_and_its_census_is_clean() {
    let p = pyramid(1, 6);
    assert!(
        p.tiles.len() > 300 && p.info.nodes > 40,
        "a pyramid too small to test"
    );
    let file = write(&p, 7);
    assert_eq!(write(&p, 7), file, "the same input gave other bytes");
    let reader = MapReader::open(Mem::new(file)).unwrap();
    assert_eq!(reader.info(), p.info);
    assert_eq!(reader.zooms(), (0, 6));
    assert_eq!(reader.tile_count(), p.tiles.len() as u64);
    let report = reader.verify(&bounded()).unwrap();
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
    let t = &p.table;
    let arrays: [(&str, Vec<u8>); 12] = [
        ("positions", le(&p.positions, f32::to_le_bytes)),
        ("adjacency.offsets", le(&p.adjacency.0, u64::to_le_bytes)),
        ("adjacency.targets", le(&p.adjacency.1, u32::to_le_bytes)),
        ("matrix.pairs", le(&p.matrix.1, u32::to_le_bytes)),
        ("matrix.weights", le(&p.matrix.2, u64::to_le_bytes)),
        ("regions.parent", le(&t.parent, u32::to_le_bytes)),
        ("regions.kind", t.kind.clone()),
        ("regions.nodes", le(&t.nodes, u64::to_le_bytes)),
        ("regions.internal", le(&t.internal, u64::to_le_bytes)),
        ("regions.volume", le(&t.volume, u64::to_le_bytes)),
        ("regions.communities", le(&t.communities, u32::to_le_bytes)),
        ("regions.partner", le(&t.partner, u32::to_le_bytes)),
    ];
    for (name, bytes) in arrays {
        assert_eq!(read_array(&reader, name), bytes, "{name}");
    }
    let covered = p.tiles.iter().filter(|t| t.cover != 0).count();
    let empty = p
        .tiles
        .iter()
        .filter(|t| t.regions.is_empty() && t.nodes.is_empty())
        .count();
    let cover_labels = p
        .tiles
        .iter()
        .filter(|t| {
            t.labels
                .iter()
                .any(|a| a.entity as usize == t.regions.len() + t.nodes.len())
        })
        .count();
    assert!(
        covered > 0 && empty > 0 && cover_labels > 0,
        "{covered} {empty} {cover_labels}"
    );
    let kinds: std::collections::BTreeSet<u8> = t.kind.iter().copied().collect();
    assert_eq!(
        kinds.len(),
        5,
        "root, super, community, subcommunity, aggregate"
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
    t.regions[1].nodes += 1;
    assert_eq!(census(&moved).inconsistent, 1);
    let mut unlabelled = p.tiles.clone();
    let (z, id) = unlabelled
        .iter()
        .find_map(|t| {
            t.labels
                .iter()
                .find(|a| a.z_off > t.z + 2 && (a.entity as usize) < t.regions.len())
                .map(|a| (t.z + 1, t.regions[a.entity as usize].id))
        })
        .unwrap();
    for t in unlabelled.iter_mut().filter(|t| t.z == z) {
        if let Some(i) = t.regions.iter().position(|g| g.id == id) {
            t.labels.retain(|a| a.entity as usize != i);
        }
    }
    let r = census(&unlabelled);
    assert_eq!((r.label_popping, r.label_cut), (1, 1), "{r:?}");
}

#[test]
fn zoom_16_separates_pixels_and_zoom_17_is_refused() {
    // Between 0.5 and 1 the f32 spacing is 2^-24 of the map: one pixel at zoom 16.
    let below_one = f32::from_bits(1.0_f32.to_bits() - 1);
    let gap = (1.0 - f64::from(below_one)) * 256.0 * f64::from(1_u32 << MAX_ZOOM);
    assert_eq!(gap, 1.0);
    assert_eq!(MAX_ZOOM, 16);
    assert_eq!(size(3.0 / 256.0, 3.0), 0);
    assert_eq!(
        size(f32::from_bits((3.0_f32 / 256.0).to_bits() - 1), 3.0),
        1
    );
    assert_eq!(size(1e-12, 3.0), 17);
    refused(
        Tile::new(17, 0, 0).encode().map_err(|e| match e {
            EgmapError::Invalid(w) => EgmapError::Malformed(w),
            e => e,
        }),
        "zoom 17",
    );
    let mut w = MapWriter::create(
        Mem::new(Vec::new()),
        &Memory::unlimited(),
        MapInfo::default(),
    )
    .unwrap();
    assert!(w.add_tile(&Tile::new(17, 0, 0)).is_err());
    let _ = RULE;
}

/// An encoder of PMTiles directories written from the PMTiles v3 specification, to
/// build directories the writer never makes; an offset of `u64::MAX` is "follows".
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
        varint(&mut raw, if off == u64::MAX { 0 } else { off + 1 });
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
    let entries = u32::from_le_bytes(out[table_at + 12..table_at + 16].try_into().unwrap());
    let table = out[table_at..table_at + 48 + 64 * entries as usize].to_vec();
    assert!(127 + root.len() + table.len() <= 16_384);
    out[127..16_384].fill(0);
    out[127..127 + root.len()].copy_from_slice(root);
    out[127 + root.len()..127 + root.len() + table.len()].copy_from_slice(&table);
    out[16..24].copy_from_slice(&(root.len() as u64).to_le_bytes());
    out
}

#[test]
fn directories_that_repeat_share_or_overflow_are_refused() {
    let p = pyramid(4, 2);
    let file = write(&p, 64);
    let ok = MapReader::open(Mem::new(file.clone())).unwrap();
    let mut entries = Vec::new();
    let mut off = 0;
    ok.for_each_tile(|t| {
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::new(6));
        gz.write_all(&t.encode().unwrap()).unwrap();
        let len = gz.finish().unwrap().len() as u64;
        entries.push((tile_id(t.z, t.x, t.y), 1, len, off));
        off += len;
        Ok(())
    })
    .unwrap();
    // The rebuilt root is the writer's: the file still reads.
    verify(with_root(&file, &directory(&entries))).unwrap();
    // A run of 2: one tile's bytes stand for two tiles.
    let mut run = entries.clone();
    run[0].1 = 2;
    refused(
        MapReader::open(Mem::new(with_root(&file, &directory(&run)))),
        "a run of 2",
    );
    // Two entries on the same bytes.
    let mut shared = entries.clone();
    shared[1].3 = shared[0].3;
    shared[1].2 = shared[0].2;
    refused(
        verify(with_root(&file, &directory(&shared))),
        "shared bytes",
    );
    // A tile stored under another tile's id.
    let mut swapped = entries.clone();
    let (a, b) = (swapped[0].3, swapped[1].3);
    swapped[0].3 = b;
    swapped[1].3 = a;
    (swapped[0].2, swapped[1].2) = (swapped[1].2, swapped[0].2);
    let (z, x, y) = tile_of_id(entries[0].0).unwrap();
    let reader = MapReader::open(Mem::new(with_root(&file, &directory(&swapped)))).unwrap();
    refused(reader.tile(z, x, y), "a tile under another id");
    // An entry past the tile data.
    let mut past = entries;
    past.last_mut().unwrap().2 += 1;
    assert!(MapReader::open(Mem::new(with_root(&file, &directory(&past)))).is_err());
    // Leaf lengths that overflow 64 bits, on a leaf region of 1 byte.
    let mut overflow = with_root(
        &file,
        &directory(&[(0, 0, u64::MAX, 0), (1, 0, 2, u64::MAX)]),
    );
    overflow[48..56].copy_from_slice(&1_u64.to_le_bytes());
    refused(
        MapReader::open(Mem::new(overflow)),
        "leaf lengths past 2^64",
    );
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
    assert!(open(&|f| f[table + 12..table + 16].copy_from_slice(&u32::MAX.to_le_bytes())).is_err());
    assert!(
        open(&|f| {
            let e = table + 48 + 56;
            f[e..e + 8].copy_from_slice(&(u64::MAX / 2).to_le_bytes());
        })
        .is_err()
    );
    assert!(open(&|f| f[32..40].copy_from_slice(&(1_u64 << 60).to_le_bytes())).is_err());
    assert!(open(&|f| f[80] ^= 1).is_err());
    assert!(matches!(open(&|f| f[99] = 1), Err(EgmapError::Foreign(_))));
    assert!(MapReader::open(Mem::new(file[..file.len() - 1].to_vec())).is_err());
    assert!(MapReader::open(Mem::new(file[..200].to_vec())).is_err());
}

#[test]
fn the_whole_file_check_refuses_what_one_tile_cannot_show() {
    // A header whose deepest zoom has no tile.
    let mut deep = SMALL.with(Clone::clone);
    deep[101] = 9;
    assert!(MapReader::open(Mem::new(deep.clone())).is_ok());
    refused(verify(deep), "max zoom 9 on a pyramid of zooms 0 to 3");
    // Metadata that is not JSON: the metadata is the last region of the file.
    let mut meta = SMALL.with(Clone::clone);
    let at = u64::from_le_bytes(meta[24..32].try_into().unwrap()) as usize;
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(b"not json at all").unwrap();
    let packed = gz.finish().unwrap();
    meta.truncate(at);
    meta.extend_from_slice(&packed);
    meta[32..40].copy_from_slice(&(packed.len() as u64).to_le_bytes());
    refused(verify(meta), "metadata that is not JSON");
    // A region colour other than its parent's, in every tile.
    let mut p = pyramid(9, 3);
    let child = (1..p.table.parent.len())
        .find(|&i| p.table.parent[i] != 0)
        .unwrap() as u32;
    for t in &mut p.tiles {
        for g in t.regions.iter_mut().filter(|g| g.id == child) {
            g.colour = (g.colour + 1) % 5;
        }
    }
    refused(verify(write(&p, 5)), "a colour other than the parent's");
    // A table count other than the tiles'.
    let mut p = pyramid(9, 3);
    let g = p.table.nodes.iter().rposition(|&n| n > 1).unwrap();
    p.table.nodes[g] -= 1;
    refused(verify(write(&p, 5)), "counts other than the tiles'");
    // One direction of a pair missing.
    let mut p = pyramid(8, 3);
    let u = p.adjacency.0.windows(2).position(|w| w[1] > w[0]).unwrap();
    p.adjacency.1.remove(p.adjacency.0[u] as usize);
    p.adjacency.0[u + 1..].iter_mut().for_each(|o| *o -= 1);
    let r = verify(write(&p, 5));
    assert!(
        matches!(r, Err(EgmapError::Malformed(ref m)) if m.contains("symmetric")),
        "{r:?}"
    );
    // A covering region taken away, and one that does not cover its tile.
    let p = pyramid(9, 3);
    let i = p
        .tiles
        .iter()
        .position(|t| t.cover != 0 && !t.regions.is_empty())
        .unwrap();
    let uncover = |t: &mut Tile, cover: u32| {
        let entities = (t.regions.len() + t.nodes.len()) as u32;
        t.labels.retain(|a| a.entity < entities);
        t.cover = cover;
    };
    let mut none = p.clone();
    uncover(&mut none.tiles[i], 0);
    refused(
        verify(write(&none, 5)),
        "a tile without the region that covers it",
    );
    let mut wrong = p.clone();
    let t = &wrong.tiles[i];
    let other = (1..p.info.regions as u32)
        .find(|&g| g != t.cover && t.regions.iter().all(|r| r.id != g))
        .unwrap();
    uncover(&mut wrong.tiles[i], other);
    assert!(
        verify(write(&wrong, 5)).is_err(),
        "a cover that is not above the tile's entities"
    );
    // A matrix pair of a rank past the parent's children.
    let mut p = pyramid(8, 3);
    p.matrix.1[0] = 1 << 16 | 0xFFFF;
    refused(verify(write(&p, 5)), "a rank past the children");
}

/// A map without tiles whose tree is a chain of `depth` super regions.
fn chain(depth: u32) -> Vec<u8> {
    let regions = depth + 1;
    let info = MapInfo {
        nodes: 1,
        regions: u64::from(regions),
        edges: 0,
        summary: 0,
    };
    let mut table = RegionTable::root(1);
    for i in 1..regions {
        table.parent.push(i - 1);
        table.kind.push(RegionKind::Super as u8);
        table.nodes.push(1);
        table.internal.push(0);
        table.volume.push(0);
        table.communities.push(0);
        table.partner.push(0);
    }
    let mut w = MapWriter::create(Mem::new(Vec::new()), &Memory::unlimited(), info).unwrap();
    write_table(&mut w, &table, 100).unwrap();
    w.finish(&metadata()).unwrap().into_inner()
}

#[test]
fn a_tree_deeper_than_255_is_refused() {
    verify(chain(255)).unwrap();
    refused(verify(chain(256)), "a chain of 256 regions");
}

#[test]
fn many_tiles_go_through_leaf_directories() {
    let count = 70_000_u64;
    let file = empty_tiles(Mem::new(Vec::new()), count)
        .unwrap()
        .into_inner();
    let leaves = u64::from_le_bytes(file[48..56].try_into().unwrap());
    assert!(leaves > 0, "70,000 tiles fit the root directory");
    let reader = MapReader::open(Mem::new(file)).unwrap();
    let first = tile_id(9, 0, 0);
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
    let report = reader.verify(&bounded()).unwrap();
    assert_eq!(report.tiles, count);
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
    write_table(&mut w, &p.table, 16).unwrap();
    assert!(
        w.finish(&metadata()).is_err(),
        "positions of the wrong length"
    );
    let w = MapWriter::create(Mem::new(Vec::new()), &memory, p.info).unwrap();
    assert!(w.finish(&metadata()).is_err(), "no region table");
    let summary = MapInfo {
        summary: 5,
        ..p.info
    };
    let mut w = MapWriter::create(Mem::new(Vec::new()), &memory, summary).unwrap();
    let with_nodes = p.tiles.iter().find(|t| !t.nodes.is_empty()).unwrap();
    assert!(w.add_tile(with_nodes).is_err(), "nodes in a summary map");
    write_array(&mut w, "positions", Values::F32(&p.positions), 4).unwrap();
    write_table(&mut w, &p.table, 16).unwrap();
    assert!(w.finish(&metadata()).is_err(), "positions in a summary map");
    assert!(MapWriter::create(Mem::new(vec![0]), &memory, p.info).is_err());
    // Entities the section table does not count, and kind other in a full map.
    let t = p.tiles.iter().find(|t| !t.nodes.is_empty()).unwrap();
    for info in [
        MapInfo {
            regions: 1,
            ..p.info
        },
        MapInfo { nodes: 0, ..p.info },
    ] {
        let mut w = MapWriter::create(Mem::new(Vec::new()), &memory, info).unwrap();
        assert!(w.add_tile(t).is_err(), "{info:?}");
    }
    let mut other = t.clone();
    other.regions[0].kind = RegionKind::Other;
    let mut w = MapWriter::create(Mem::new(Vec::new()), &memory, p.info).unwrap();
    assert!(w.add_tile(&other).is_err(), "kind other in a full map");
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
    verify(expected).unwrap();
}

/// A source that counts the bytes read from it.
struct Counting {
    inner: Mem,
    read: AtomicU64,
}

impl ByteSource for Counting {
    fn len(&self) -> u64 {
        self.inner.len()
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), ReadError> {
        self.read.fetch_add(buf.len() as u64, Ordering::Relaxed);
        self.inner.read_at(offset, buf)
    }
}

/// A map without tiles of one parent with `children` children of one node each, whose
/// partners are given by `partner(child)`, in chunks of 131,072 values (1 MiB of `u64`).
fn siblings(children: u32, partner: impl Fn(u32) -> u32) -> Vec<u8> {
    let n = u64::from(children);
    let info = MapInfo {
        nodes: n,
        regions: n + 1,
        edges: 0,
        summary: 0,
    };
    let mut table = RegionTable::root(n);
    for i in 1..=children {
        table.parent.push(0);
        table.kind.push(RegionKind::Community as u8);
        table.nodes.push(1);
        table.internal.push(0);
        table.volume.push(0);
        table.communities.push(0);
        table.partner.push(partner(i));
    }
    let mut w = MapWriter::create(Mem::new(Vec::new()), &Memory::unlimited(), info).unwrap();
    write_table(&mut w, &table, 1 << 17).unwrap();
    w.finish(&metadata()).unwrap().into_inner()
}

#[test]
fn verify_reads_each_chunk_a_bounded_number_of_times() {
    // Partners alternating between the two ends of 300,000 siblings: a check that read
    // each partner's parent would inflate a 1 MiB chunk for almost every region.
    let last = 299_999;
    let file = siblings(300_000, |i| if i % 2 == 1 && i != last { last } else { 1 });
    let len = file.len() as u64;
    let source = Counting {
        inner: Mem::new(file),
        read: AtomicU64::new(0),
    };
    let reader = MapReader::open(&source).unwrap();
    reader.verify(&bounded()).unwrap();
    let read = source.read.load(Ordering::Relaxed);
    // 79,689 bytes of a 44,028-byte file when this was written.
    assert!(
        read <= 4 * len,
        "verify read {read} bytes of a {len}-byte file"
    );
    // A partner outside the siblings is found by the same pass.
    let mut outside = RegionTable::root(2);
    for (parent, nodes) in [(0, 2), (1, 1), (1, 1)] {
        outside.parent.push(parent);
        outside.kind.push(RegionKind::Community as u8);
        outside.nodes.push(nodes);
        outside.internal.push(0);
        outside.volume.push(0);
        outside.communities.push(0);
        outside.partner.push(0);
    }
    outside.partner[2] = 1;
    let info = MapInfo {
        nodes: 2,
        regions: 4,
        edges: 0,
        summary: 0,
    };
    let mut w = MapWriter::create(Mem::new(Vec::new()), &Memory::unlimited(), info).unwrap();
    write_table(&mut w, &outside, 2).unwrap();
    refused(
        verify(w.finish(&metadata()).unwrap().into_inner()),
        "a partner that is a parent",
    );
}

#[test]
fn the_memory_verify_keeps_per_region_is_held_to_the_budget() {
    // 3,000,000 regions declared in a few kilobytes: verify asks the budget first.
    let file = siblings(3_000_000, |_| 0);
    assert!(file.len() < 1 << 20, "{} bytes", file.len());
    let reader = MapReader::open(Mem::new(file)).unwrap();
    let small = Memory::from(Arc::new(Budget::new(16 << 20)));
    assert!(matches!(reader.verify(&small), Err(EgmapError::Memory(_))));
}
