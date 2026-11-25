// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Golden files: known answers on small real graphs, computed by at least two independent
//! libraries (`oracle-ref golden --write`) and stored with their provenance under
//! `oracle/golden/`. This module recomputes each value with the in-process references and
//! compares; the engine's kernels will be compared the same way through [`crate::plugin`].
//!
//! A golden file names its source. Zachary's karate club is small enough to be vendored
//! next to its golden file; the SNAP graphs are downloaded by `oracle-ref golden --fetch`
//! into the cache directory (`$EVERYGRAPH_ORACLE_CACHE`, default `oracle/.cache`), and a
//! golden file whose source is not there is skipped, not failed. When the dataset
//! registry (#5) exists, the cache is replaced by its fetcher and `source.registry` names
//! the registry entry.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::graph::Graph;
use crate::projection::Projection;
use crate::{ix, reference, to_u64};

/// Format name of a golden file.
pub const GOLDEN_FORMAT: &str = "everygraph-oracle-golden";

#[derive(Deserialize)]
struct GoldenFile {
    format: String,
    version: u32,
    name: String,
    source: Source,
    directed: bool,
    values: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
struct Source {
    file: String,
    vendored: bool,
}

#[derive(Deserialize)]
struct Value {
    value: serde_json::Value,
}

/// The result of checking one golden file.
#[derive(Clone, Debug)]
pub struct GoldenResult {
    /// The graph's name.
    pub name: String,
    /// Number of values that agree, or what went wrong. A message starting with
    /// `skipped` means the source is not available.
    pub outcome: Result<usize, String>,
}

/// The cache directory for downloaded sources.
#[must_use]
pub fn cache_dir(golden_dir: &Path) -> PathBuf {
    std::env::var_os("EVERYGRAPH_ORACLE_CACHE").map_or_else(
        || golden_dir.parent().unwrap_or(golden_dir).join(".cache"),
        PathBuf::from,
    )
}

/// Reads a whitespace-separated edge list (`#` and `%` start comments) and relabels the
/// ids densely by rank, the convention of the golden files.
///
/// # Errors
///
/// I/O errors and `InvalidData` for a line that is not two integers.
pub fn read_edge_list(path: &Path, directed: bool) -> io::Result<Graph> {
    let text = fs::read_to_string(path)?;
    let mut pairs = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('%') {
            continue;
        }
        let mut f = line.split_whitespace().map(str::parse::<u64>);
        match (f.next(), f.next()) {
            (Some(Ok(u)), Some(Ok(v))) => pairs.push((u, v)),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{}:{}: not an edge", path.display(), i + 1),
                ));
            }
        }
    }
    let mut ids: Vec<u64> = pairs.iter().flat_map(|&(u, v)| [u, v]).collect();
    ids.sort_unstable();
    ids.dedup();
    let rank = |x: u64| u32::try_from(ids.binary_search(&x).unwrap_or(0)).unwrap_or(u32::MAX);
    let name = path
        .file_name()
        .map_or_else(String::new, |f| f.to_string_lossy().into_owned());
    let mut g = Graph::with_nodes(
        &name,
        u32::try_from(ids.len()).unwrap_or(u32::MAX),
        directed,
    );
    for (u, v) in pairs {
        g.src.push(rank(u));
        g.dst.push(rank(v));
    }
    Ok(g)
}

fn ints(v: &[u64]) -> serde_json::Value {
    serde_json::Value::from(v.to_vec())
}

/// Computes the values a golden file may hold.
#[must_use]
pub fn compute(g: &Graph) -> BTreeMap<&'static str, serde_json::Value> {
    let p = Projection::new(g);
    let mut out = BTreeMap::new();
    out.insert("n", g.n.into());
    out.insert("m", p.m().into());
    out.insert("self_loop_rows", p.counts.self_loop_rows.into());
    out.insert("duplicate_rows", p.counts.duplicate_rows.into());
    let class_stats = |labels: &[u32]| {
        let mut size: BTreeMap<u32, u64> = BTreeMap::new();
        for &l in labels {
            *size.entry(l).or_default() += 1;
        }
        (
            to_u64(size.len()),
            size.values().copied().max().unwrap_or(0),
        )
    };
    let (count, largest) = class_stats(&reference::wcc(&p.undirected));
    out.insert("wcc_count", count.into());
    out.insert("largest_wcc", largest.into());
    if g.directed {
        let (count, largest) = class_stats(&reference::scc(&p.out));
        out.insert("scc_count", count.into());
        out.insert("largest_scc", largest.into());
    }
    let t = reference::triangles(&p.undirected);
    out.insert("triangle_total", (t.iter().sum::<u64>() / 3).into());
    out.insert("triangles", ints(&t));
    let core = reference::core_numbers(&p.undirected);
    out.insert("max_core", core.iter().copied().max().unwrap_or(0).into());
    out.insert("core_numbers", ints(&core));
    let mut hist = vec![
        0u64;
        core.iter()
            .map(|&c| ix(u32::try_from(c).unwrap_or(0)) + 1)
            .max()
            .unwrap_or(0)
    ];
    for &c in &core {
        hist[ix(u32::try_from(c).unwrap_or(0))] += 1;
    }
    out.insert("core_histogram", ints(&hist));
    out
}

fn check_file(dir: &Path, path: &Path) -> Result<(String, usize), (String, String)> {
    let text = fs::read_to_string(path).map_err(|e| (path.display().to_string(), e.to_string()))?;
    let f: GoldenFile =
        serde_json::from_str(&text).map_err(|e| (path.display().to_string(), e.to_string()))?;
    let name = f.name.clone();
    if f.format != GOLDEN_FORMAT || f.version != 1 {
        return Err((name, format!("format {} v{}", f.format, f.version)));
    }
    let src = if f.source.vendored {
        dir.join(&f.source.file)
    } else {
        cache_dir(dir).join(&f.source.file)
    };
    if !src.is_file() {
        return Err((
            name,
            format!(
                "skipped: {} not downloaded (oracle-ref golden --fetch)",
                src.display()
            ),
        ));
    }
    let g = read_edge_list(&src, f.directed).map_err(|e| (name.clone(), e.to_string()))?;
    let got = compute(&g);
    let mut agreed = 0;
    let mut bad = Vec::new();
    for (key, want) in &f.values {
        match got.get(key.as_str()) {
            Some(v) if *v == want.value => agreed += 1,
            Some(v) => {
                let show = |x: &serde_json::Value| {
                    let s = x.to_string();
                    if s.len() > 80 {
                        format!("{}…", &s[..80])
                    } else {
                        s
                    }
                };
                bad.push(format!(
                    "{key}: computed {} vs golden {}",
                    show(v),
                    show(&want.value)
                ));
            }
            None => {}
        }
    }
    if bad.is_empty() {
        Ok((name, agreed))
    } else {
        Err((name, bad.join("; ")))
    }
}

/// Checks every `*.json` golden file in `dir`.
///
/// # Errors
///
/// If `dir` cannot be read.
pub fn check_dir(dir: &Path) -> io::Result<Vec<GoldenResult>> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    Ok(files
        .iter()
        .map(|p| match check_file(dir, p) {
            Ok((name, n)) => GoldenResult {
                name,
                outcome: Ok(n),
            },
            Err((name, e)) => GoldenResult {
                name,
                outcome: Err(e),
            },
        })
        .collect())
}
