// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The Rust side of `tools/check_container_python.py`, which compares it with the
//! independent Python reader (`tools/eg_container.py`):
//!
//! - `write PATH` writes a sample `.eg` container to `PATH` (as `PATH.partial`, then
//!   [`persist`]ed) and prints what it holds;
//! - `list PATH` prints what the container at `PATH` holds, as the Rust reader opens it
//!   (the check script uses it on files it has patched), or `{"error": "…"}`.
//!
//! The listing is JSON on standard output: generation, version, whether the reader
//! fell back to an older commit, the graph identity, and every section.
//!
//! The sample has two commits: a small graph (a provenance record, the two sections of
//! a compressed adjacency, an empty section, one of a kind no specification defines,
//! and sections of 0 to 70,000 bytes, so that BLAKE3's tree has one and several
//! chunks), then a result appended with its own record and one section removed — so
//! it also holds a replaced table of contents and bytes no commit refers to.
//!
//! ```sh
//! cargo run -p everygraph-io --example container_sample -- write sample.eg
//! cargo run -p everygraph-io --example container_sample -- list sample.eg
//! ```

#![forbid(unsafe_code)]

#[path = "../tests/support/bytes.rs"]
mod support;

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use everygraph_format::adjacency;
use everygraph_format::container::{Container, Input, Kind, Provenance, ResultKey, Writer};
use everygraph_io::{FileSink, FileSource, persist};
use support::bytes;

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

/// Writes the sample container to `path`.
fn write(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
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

    let partial = path.with_extension("eg.partial");
    let mut w = Writer::create(FileSink::create(&partial)?)?;
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
    let key = ResultKey::new(w.graph_identity(), "pagerank", 1, true)?.with("damping", "0.85")?;
    let ranks: Vec<f64> = (0..n).map(|v| f64::from(v + 1) / 500_500.0).collect();
    let about = Provenance {
        operation: String::new(),
        parameters: BTreeMap::new(),
        ..record("pagerank")
    };
    w.add_result(&key, &about, &ranks)?;
    w.remove("x.chunk-boundary")?;
    w.commit()?;
    drop(w);
    persist(&partial, path)?;
    Ok(())
}

/// What the container at `path` holds, as JSON.
fn list(path: &Path) -> String {
    let file = match FileSource::open(path)
        .map_err(|e| e.to_string())
        .and_then(|source| Container::open(source).map_err(|e| e.to_string()))
    {
        Ok(file) => file,
        // Debug quoting is valid JSON for the ASCII messages of the reader.
        Err(e) => return format!("{{\"error\": {e:?}}}\n"),
    };
    let mut out = format!(
        "{{\"generation\": {}, \"version\": [{}, {}], \"recovered\": {}, \"identity\": \"{}\", \"sections\": [",
        file.generation(),
        file.version().0,
        file.version().1,
        file.recovered_from().is_some(),
        file.graph_identity(),
    );
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
    out
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let usage = "usage: container_sample write|list PATH";
    let (mode, path) = (args.next().ok_or(usage)?, args.next().ok_or(usage)?);
    let path = Path::new(&path);
    match mode.to_str() {
        Some("write") => write(path)?,
        Some("list") => {}
        _ => return Err(usage.into()),
    }
    std::io::stdout().lock().write_all(list(path).as_bytes())?;
    Ok(())
}
