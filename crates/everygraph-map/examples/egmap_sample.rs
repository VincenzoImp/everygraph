// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The Rust side of `tools/check_egmap_readers.py`, which compares it with the
//! independent Python reader (`tools/egmap.py`) and the official PMTiles readers:
//!
//! - `write PATH` writes a sample map (the test pyramid of `tests/support`, zooms 0 to
//!   5, arrays in chunks of 7 values) to `PATH`;
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
use everygraph_map::egmap::{EgmapError, Element, MapReader, Tile};
use serde_json::{Value, json};

fn tile_json(t: &Tile) -> Value {
    let regions: Vec<Value> = t
        .regions
        .iter()
        .map(|g| {
            json!([
                g.id,
                g.parent,
                g.x.to_bits(),
                g.y.to_bits(),
                g.r.to_bits(),
                g.nodes,
                g.internal,
                g.volume,
                g.communities,
                g.rank,
                g.partner,
                g.kind as u8,
                g.depth,
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
        "z": t.z, "x": t.x, "y": t.y,
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
                    Element::U16 => json!(le::u16_at(c, 0)),
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
        [cmd, path] if cmd == "dump" => {
            let bytes = std::fs::read(path).expect("the file can be read");
            let out = dump(bytes).unwrap_or_else(|e| json!({"error": e.to_string()}));
            println!("{out}");
        }
        _ => {
            eprintln!("usage: egmap_sample write PATH | dump PATH");
            std::process::exit(2);
        }
    }
}
