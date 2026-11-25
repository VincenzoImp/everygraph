// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The on-disk exchange form, read by Rust, by the Python references and by external
//! kernels. One directory per graph:
//!
//! - `graph.json`: the descriptor (format name and version, name, `n`, `directed`, rows,
//!   columns, semantics, the kernels to check, provenance).
//! - `edges.tsv`: one row per event, `src<TAB>dst[<TAB>weight][<TAB>time]`, ids in
//!   `0..n`, preceded by one comment line starting with `#` that names the columns.
//!   Weights are written as the shortest decimal that reads back to the same double.
//! - `partition.txt`: one community label per node, one per line (if the graph has one).
//! - `reference.json`: written by `oracle-ref run`, the results of each Python library.
//!
//! The full description is in `oracle/README.md`, "Exchange formats".

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::corpus::Entry;
use crate::graph::{Graph, Semantics};
use crate::kernel::{Kernel, Output};

/// Format name in `graph.json`.
pub const GRAPH_FORMAT: &str = "everygraph-oracle-graph";
/// Format name in `reference.json`.
pub const REFERENCE_FORMAT: &str = "everygraph-oracle-reference";
/// Version of both formats.
pub const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct Descriptor {
    format: String,
    version: u32,
    name: String,
    n: u32,
    directed: bool,
    rows: u64,
    columns: Vec<String>,
    semantics: Semantics,
    partition: Option<String>,
    kernels: Vec<Kernel>,
    provenance: serde_json::Value,
}

fn bad(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

/// Writes an entry into `dir` (created if needed).
///
/// # Errors
///
/// Any I/O error.
pub fn write_entry(dir: &Path, e: &Entry) -> io::Result<()> {
    let g = &e.graph;
    fs::create_dir_all(dir)?;
    let mut columns = vec!["src".to_owned(), "dst".to_owned()];
    if g.weight.is_some() {
        columns.push("weight".into());
    }
    if g.time.is_some() {
        columns.push("time".into());
    }
    let mut text = format!("# {}\n", columns.join("\t"));
    for i in 0..g.rows() {
        let _ = write!(text, "{}\t{}", g.src[i], g.dst[i]);
        if let Some(w) = &g.weight {
            // `{:?}` is the shortest representation that round-trips.
            let _ = write!(text, "\t{:?}", w[i]);
        }
        if let Some(t) = &g.time {
            let _ = write!(text, "\t{}", t[i]);
        }
        text.push('\n');
    }
    fs::write(dir.join("edges.tsv"), text)?;
    if let Some(p) = &g.partition {
        let mut s = String::with_capacity(p.len() * 3);
        for l in p {
            let _ = writeln!(s, "{l}");
        }
        fs::write(dir.join("partition.txt"), s)?;
    }
    let d = Descriptor {
        format: GRAPH_FORMAT.into(),
        version: VERSION,
        name: g.name.clone(),
        n: g.n,
        directed: g.directed,
        rows: g.rows() as u64,
        columns,
        semantics: g.semantics,
        partition: g.partition.as_ref().map(|_| "partition.txt".to_owned()),
        kernels: e.kernels.clone(),
        provenance: g.provenance.clone(),
    };
    let json = serde_json::to_string_pretty(&d).map_err(|e| bad(e.to_string()))?;
    fs::write(dir.join("graph.json"), json + "\n")
}

fn parse<T: std::str::FromStr>(field: Option<&str>, what: &str, line: usize) -> io::Result<T> {
    field
        .and_then(|f| f.parse().ok())
        .ok_or_else(|| bad(format!("edges.tsv line {line}: bad or missing {what}")))
}

/// Reads an entry from `dir`.
///
/// # Errors
///
/// I/O errors, and `InvalidData` for anything that does not follow the format.
pub fn read_entry(dir: &Path) -> io::Result<Entry> {
    let d: Descriptor = serde_json::from_str(&fs::read_to_string(dir.join("graph.json"))?)
        .map_err(|e| bad(format!("{}: {e}", dir.display())))?;
    if d.format != GRAPH_FORMAT || d.version != VERSION {
        return Err(bad(format!(
            "{}: format {} v{}",
            dir.display(),
            d.format,
            d.version
        )));
    }
    let col = |name: &str| d.columns.iter().position(|c| c == name);
    let (wcol, tcol) = (col("weight"), col("time"));
    let mut g = Graph::with_nodes(&d.name, d.n, d.directed);
    g.semantics = d.semantics;
    g.provenance = d.provenance;
    let mut weight = wcol.map(|_| Vec::new());
    let mut time = tcol.map(|_| Vec::new());
    for (i, line) in fs::read_to_string(dir.join("edges.tsv"))?
        .lines()
        .enumerate()
    {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        g.src.push(parse(f.first().copied(), "src", i + 1)?);
        g.dst.push(parse(f.get(1).copied(), "dst", i + 1)?);
        if let (Some(c), Some(w)) = (wcol, weight.as_mut()) {
            w.push(parse(f.get(c).copied(), "weight", i + 1)?);
        }
        if let (Some(c), Some(t)) = (tcol, time.as_mut()) {
            t.push(parse(f.get(c).copied(), "time", i + 1)?);
        }
    }
    g.weight = weight;
    g.time = time;
    if let Some(p) = &d.partition {
        let text = fs::read_to_string(dir.join(p))?;
        let labels: Result<Vec<u32>, _> = text.lines().map(str::parse).collect();
        g.partition = Some(labels.map_err(|e| bad(format!("{p}: {e}")))?);
    }
    if g.rows() as u64 != d.rows {
        return Err(bad(format!(
            "{}: {} rows, descriptor says {}",
            d.name,
            g.rows(),
            d.rows
        )));
    }
    g.validate().map_err(bad)?;
    Ok(Entry {
        graph: g,
        kernels: d.kernels,
    })
}

/// The answers of the Python references for one graph.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct References {
    /// Library name → version.
    pub libraries: BTreeMap<String, String>,
    /// Per kernel, library name → result.
    pub results: Vec<(Kernel, BTreeMap<String, Output>)>,
}

#[derive(Deserialize)]
struct RefFile {
    format: String,
    version: u32,
    libraries: BTreeMap<String, String>,
    results: Vec<RefResult>,
}

#[derive(Deserialize)]
struct RefResult {
    kernel: Kernel,
    values: BTreeMap<String, Output>,
}

/// Reads `reference.json` from `dir`; `Ok(None)` if there is none.
///
/// # Errors
///
/// I/O errors other than a missing file, and `InvalidData` for a malformed file.
pub fn read_references(dir: &Path) -> io::Result<Option<References>> {
    let text = match fs::read_to_string(dir.join("reference.json")) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let f: RefFile =
        serde_json::from_str(&text).map_err(|e| bad(format!("{}: {e}", dir.display())))?;
    if f.format != REFERENCE_FORMAT || f.version != VERSION {
        return Err(bad(format!(
            "reference.json format {} v{}",
            f.format, f.version
        )));
    }
    Ok(Some(References {
        libraries: f.libraries,
        results: f
            .results
            .into_iter()
            .map(|r| (r.kernel, r.values))
            .collect(),
    }))
}

/// The graph directories under `corpus`, sorted by name.
///
/// # Errors
///
/// Any I/O error.
pub fn list_corpus(corpus: &Path) -> io::Result<Vec<std::path::PathBuf>> {
    let mut dirs: Vec<_> = fs::read_dir(corpus)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.join("graph.json").is_file())
        .collect();
    dirs.sort();
    Ok(dirs)
}

#[cfg(test)]
mod tests {
    use super::{read_entry, write_entry};
    use crate::corpus::{Tier, entries};

    #[test]
    fn round_trip_of_every_ci_graph_with_weights() {
        let dir = std::env::temp_dir().join(format!("everygraph-oracle-rt-{}", std::process::id()));
        for e in entries(Tier::Ci)
            .into_iter()
            .filter(|e| e.graph.weight.is_some())
            .take(25)
        {
            let d = dir.join(&e.graph.name);
            write_entry(&d, &e).expect("write");
            assert_eq!(read_entry(&d).expect("read"), e, "{}", e.graph.name);
        }
        std::fs::remove_dir_all(&dir).expect("clean up");
    }
}
