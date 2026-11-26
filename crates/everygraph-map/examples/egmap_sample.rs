// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The Rust side of `tools/check_egmap_readers.py`, which compares it with the
//! independent Python reader (`tools/egmap.py`) and the official PMTiles readers:
//!
//! - `write PATH` writes a sample map (the test pyramid of `tests/support`, zooms 0 to
//!   5, arrays in chunks of 7 values) to `PATH`;
//! - `leaves PATH` writes a map of 70,000 empty tiles, whose directory has leaves;
//! - `chain DEPTH PATH` writes a map without tiles whose region tree is a chain of
//!   `DEPTH` super regions (a reader refuses more than 255);
//! - `dump PATH` prints what the map at `PATH` holds, as the Rust reader reads all of it,
//!   or `{"error": "…"}`.
//!
//! The dump is JSON on standard output: the section table's figures, the zooms, the
//! metadata, the arrays (f32 values as their bits) and every tile (its records as lists
//! of numbers in the order of the spec's columns, f32 as bits).
//!
//! ```sh
//! cargo run -p everygraph-map --example egmap_sample -- write sample.egmap
//! cargo run -p everygraph-map --example egmap_sample -- dump sample.egmap
//! ```

#![forbid(unsafe_code)]
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::cast_lossless
)]

#[path = "../tests/support/mod.rs"]
mod support;

use everygraph_core::{Memory, MemorySource, le};
use everygraph_map::egmap::{EgmapError, Element, MapInfo, MapReader, MapWriter, RegionKind, Tile};
use serde_json::{Value, json};

fn tile_json(t: &Tile) -> Value {
    let regions: Vec<Value> = t
        .regions
        .iter()
        .map(|g| {
            json!([
                g.id,
                g.x.to_bits(),
                g.y.to_bits(),
                g.r.to_bits(),
                g.nodes,
                g.kind as u8,
                g.colour,
                g.zmin
            ])
        })
        .collect();
    let nodes: Vec<Value> = t
        .nodes
        .iter()
        .map(|v| {
            json!([
                v.id,
                v.region,
                v.x.to_bits(),
                v.y.to_bits(),
                v.r.to_bits(),
                v.name,
                v.degree,
                v.colour,
                v.zmin
            ])
        })
        .collect();
    let labels: Vec<Value> = t
        .labels
        .iter()
        .map(|a| json!([a.entity, a.text, a.z_on, a.z_off, a.anchor as u8]))
        .collect();
    json!({
        "z": t.z, "x": t.x, "y": t.y, "cover": t.cover,
        "omitted": [t.omitted.regions, t.omitted.region_nodes, t.omitted.nodes],
        "regions": regions, "nodes": nodes, "labels": labels, "strings": t.strings,
    })
}

fn dump(bytes: Vec<u8>) -> Result<Value, EgmapError> {
    let reader = MapReader::open(MemorySource::new(bytes))?;
    let report = reader.verify(&Memory::unlimited())?;
    let mut tiles = Vec::new();
    reader.for_each_tile(|t| {
        tiles.push(tile_json(&t));
        Ok(())
    })?;
    let mut arrays = serde_json::Map::new();
    let mut sections = Vec::new();
    for s in reader.sections() {
        let mut values = Vec::new();
        for i in 0..s.chunks() {
            let bytes = reader.chunk(s, i)?;
            let width = s.element.width();
            for c in bytes.chunks_exact(width) {
                values.push(match s.element {
                    Element::U8 => json!(c[0]),
                    Element::U32 | Element::F32 => json!(le::u32_at(c, 0)),
                    Element::U64 => json!(le::u64_at(c, 0)),
                });
            }
        }
        arrays.insert(s.name.clone(), Value::Array(values));
        sections.push(
            json!({"name": s.name, "element": s.element as u8, "chunk": s.chunk, "count": s.count}),
        );
    }
    let info = reader.info();
    let metadata: Value = serde_json::from_str(&reader.metadata()?)
        .map_err(|e| EgmapError::Malformed(e.to_string()))?;
    Ok(json!({
        "info": {"nodes": info.nodes, "regions": info.regions, "edges": info.edges, "summary": info.summary},
        "zooms": [reader.zooms().0, reader.zooms().1],
        "tile_count": reader.tile_count(),
        "metadata": metadata,
        "sections": sections,
        "arrays": arrays,
        "tiles": tiles,
        "popping": report.popping,
    }))
}

fn chain(depth: u32) -> Vec<u8> {
    let info = MapInfo {
        nodes: 1,
        regions: u64::from(depth) + 1,
        edges: 0,
        summary: 0,
    };
    let mut table = support::RegionTable::root(1);
    for i in 1..=depth {
        table.parent.push(i - 1);
        table.kind.push(RegionKind::Super as u8);
        table.nodes.push(1);
        table.internal.push(0);
        table.volume.push(0);
        table.communities.push(0);
        table.partner.push(0);
    }
    let mut w = MapWriter::create(MemorySource::new(Vec::new()), &Memory::unlimited(), info)
        .expect("an empty sink");
    support::write_table(&mut w, &table, 100).expect("valid arrays");
    w.finish(&support::metadata()).expect("a map").into_inner()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [cmd, path] if cmd == "write" => {
            let p = support::pyramid(1, 5);
            let file = support::write_into(&p, MemorySource::new(Vec::new()), 7)
                .expect("the sample is a valid map")
                .into_inner();
            std::fs::write(path, file).expect("the sample can be written");
        }
        [cmd, path] if cmd == "leaves" => {
            let file = support::empty_tiles(MemorySource::new(Vec::new()), 70_000)
                .expect("the sample is a valid map")
                .into_inner();
            std::fs::write(path, file).expect("the sample can be written");
        }
        [cmd, depth, path] if cmd == "chain" => {
            let depth: u32 = depth.parse().expect("a depth");
            std::fs::write(path, chain(depth)).expect("the sample can be written");
        }
        [cmd, path] if cmd == "dump" => {
            let bytes = std::fs::read(path).expect("the file can be read");
            let out = dump(bytes).unwrap_or_else(|e| json!({"error": e.to_string()}));
            println!("{out}");
        }
        _ => {
            eprintln!(
                "usage: egmap_sample write PATH | leaves PATH | chain DEPTH PATH | dump PATH"
            );
            std::process::exit(2);
        }
    }
}
