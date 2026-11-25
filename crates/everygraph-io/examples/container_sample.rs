// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Writes a sample `.eg` container to the path given and prints what it holds, as
//! JSON, on standard output: the fixture of `tools/check_container_python.py`, which
//! reads the file with the independent Python reader (`tools/eg_container.py`) and
//! compares.
//!
//! The file has two commits: a small graph (a provenance record, the two sections of a
//! compressed adjacency, an empty section, one of a kind no specification defines, and
//! sections of 0 to 70,000 bytes, so that BLAKE3's tree has one and several chunks),
//! then a result appended with its own record and one section removed — so it also
//! holds a replaced table of contents and bytes no commit refers to.
//!
//! ```sh
//! cargo run -p everygraph-io --example container_sample -- sample.eg
//! ```

#![forbid(unsafe_code)]

use std::io::Write;

use everygraph_format::adjacency;
use everygraph_format::container::{Container, Input, Kind, Provenance, Writer};
use everygraph_io::{FileSink, FileSource};

fn bytes(seed: u64, len: usize) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x.to_le_bytes()[0]
        })
        .collect()
}

fn record(operation: &str) -> Provenance {
    Provenance {
        engine: format!("everygraph {}", env!("CARGO_PKG_VERSION")),
        operation: operation.into(),
        started: Some(1_764_100_000_000_000_000),
        finished: Some(1_764_100_001_500_000_000),
        inputs: vec![Input {
            name: "edges.csv".into(),
            bytes: 1234,
            modified: Some(1_764_000_000_000_000_000),
            algorithm: "blake3".into(),
            digest: vec![0xAB; 32],
        }],
        parameters: [("directed", "false"), ("seed", "42")]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect(),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or("usage: container_sample PATH")?;

    // A ring of 1,000 nodes with chords, as a compressed adjacency.
    let n = 1000u32;
    let mut offsets = vec![0u64];
    let mut targets = Vec::new();
    for v in 0..n {
        let mut list = vec![(v + 1) % n, (v + n - 1) % n, (v * 7 + 3) % n];
        list.sort_unstable();
        list.dedup();
        list.retain(|&t| t != v);
        targets.extend_from_slice(&list);
        offsets.push(targets.len() as u64);
    }
    let encoded = adjacency::encode(&offsets, &targets)?;
    let private = Kind::new(0x8000_0001).ok_or("kind")?;

    let mut w = Writer::create(FileSink::create(&path)?)?;
    w.add_provenance("provenance/ingest", &record("ingest"))?;
    let ingest = Some("provenance/ingest");
    w.add_section(
        "adjacency.out.index",
        Kind::ADJACENCY_INDEX,
        &encoded.index,
        ingest,
    )?;
    w.add_section(
        "adjacency.out.data",
        Kind::ADJACENCY_DATA,
        &encoded.data,
        ingest,
    )?;
    w.add_section("ids.header", Kind::ID_DICTIONARY, &bytes(1, 64), ingest)?;
    w.add_section("empty", private, &[], None)?;
    w.add_section("x.chunk-boundary", private, &bytes(2, 1024), None)?;
    w.add_section("x.many-chunks", private, &bytes(3, 70_000), None)?;
    w.add_section(
        "x.unknown-kind",
        Kind::new(0xC0DE).ok_or("kind")?,
        &bytes(4, 17),
        None,
    )?;
    w.commit()?;
    w.add_provenance("provenance/pagerank", &record("pagerank"))?;
    w.add_section(
        "results/pagerank",
        private,
        &bytes(5, 4000),
        Some("provenance/pagerank"),
    )?;
    w.remove("x.chunk-boundary")?;
    w.commit()?;
    drop(w);

    // What the file holds, read back through the Rust reader.
    let file = Container::open(FileSource::open(&path)?)?;
    file.verify_all()?;
    let mut out = String::new();
    out.push_str(&format!(
        "{{\"generation\": {}, \"version\": [{}, {}], \"sections\": [",
        file.generation(),
        file.version().0,
        file.version().1
    ));
    for (i, e) in file.entries().iter().enumerate() {
        let hash: String = e.hash.iter().map(|b| format!("{b:02x}")).collect();
        let provenance = e
            .provenance
            .as_ref()
            .map_or("null".to_owned(), |p| format!("\"{p}\""));
        out.push_str(&format!(
            "{}{{\"name\": \"{}\", \"kind\": {}, \"codec\": {}, \"offset\": {}, \"length\": {}, \"hash\": \"{hash}\", \"provenance\": {provenance}}}",
            if i == 0 { "" } else { ", " },
            e.name,
            e.kind.code(),
            e.codec.code(),
            e.offset,
            e.len,
        ));
    }
    out.push_str("]}\n");
    std::io::stdout().lock().write_all(out.as_bytes())?;
    Ok(())
}
