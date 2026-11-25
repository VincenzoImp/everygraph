// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Golden files: known answers on real graphs, computed by independent libraries
//! (`oracle-ref golden --write`) and stored compactly with their provenance under
//! `oracle/golden/`. This module reads each source with the file's own `parse` rules,
//! recomputes every value with the in-process references and compares. For values that
//! have only one Python witness (core numbers above 600,000 pairs, where NetworkX is not
//! run), this check is the second witness.
//!
//! Sources, by `source.kind`:
//! - `vendored`: next to the golden file (Zachary's karate club);
//! - `registry`: a tier-1 dataset of `datasets/datasets.toml`, under a data root filled
//!   by `everygraph-bench fetch tier1 --data ROOT` (`<root>/<dataset>/<file>`);
//! - `download`: a small SNAP graph outside the registry, in the oracle cache
//!   (`oracle-ref golden --fetch`).
//!
//! A golden file whose source is not available is skipped, not failed.
//!
//! Per-node arrays are compared through the SHA-256 of their little-endian `u64`
//! encoding in node order, the form the golden files store.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::graph::Graph;
use crate::projection::Projection;
use crate::{reference, to_u64};

/// Format name of a golden file.
pub const GOLDEN_FORMAT: &str = "everygraph-oracle-golden";
/// Version of the golden format.
pub const GOLDEN_VERSION: u32 = 2;

/// How a source file becomes rows.
#[derive(Clone, Debug, Deserialize)]
pub struct Parse {
    /// `"whitespace"`, or a single-character separator such as `","` or `"\t"`.
    pub separator: String,
    /// Lines starting with this prefix are skipped.
    pub comment: Option<String>,
    /// Column of the source id.
    pub src: usize,
    /// Column of the target id.
    pub dst: usize,
    /// Keep only rows whose column equals a value.
    pub keep: Option<Keep>,
    /// `"integer"` or `"string"`.
    pub ids: String,
}

/// A row filter.
#[derive(Clone, Debug, Deserialize)]
pub struct Keep {
    /// The column.
    pub column: usize,
    /// The value it must equal.
    pub equals: String,
}

#[derive(Deserialize)]
struct GoldenFile {
    format: String,
    version: u32,
    name: String,
    source: Source,
    parse: Parse,
    directed: bool,
    values: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
struct Source {
    kind: String,
    file: String,
    dataset: Option<String>,
}

#[derive(Deserialize)]
struct Value {
    value: serde_json::Value,
}

/// Bytes the in-process check needs per row and per node, measured: web-google (5.1 M
/// rows, 876 k nodes) peaked at 1.76 GB, about 345 bytes per row; the estimate rounds up.
pub const BYTES_PER_ROW: u64 = 400;
/// See [`BYTES_PER_ROW`].
pub const BYTES_PER_NODE: u64 = 160;
/// Default memory limit of the golden check: 4 GiB.
pub const DEFAULT_MAX_MEMORY: u64 = 4 << 30;

/// The memory a graph of `rows` rows and `n` nodes is estimated to need in the
/// in-process references (projection, components, triangles, core numbers).
#[must_use]
pub const fn estimate(rows: u64, n: u64) -> u64 {
    rows * BYTES_PER_ROW + n * BYTES_PER_NODE
}

/// Where the sources are, and how much memory a check may take.
#[derive(Clone, Debug)]
pub struct Roots {
    /// The directory of the golden files (vendored sources live there).
    pub golden: PathBuf,
    /// The oracle cache of downloaded sources.
    pub cache: PathBuf,
    /// The data root of the dataset registry, if any.
    pub data: Option<PathBuf>,
    /// A source whose estimated memory ([`estimate`]) exceeds this is refused.
    pub max_memory: u64,
}

impl Roots {
    /// The defaults: cache `$EVERYGRAPH_ORACLE_CACHE` or `<golden>/../.cache`; data root
    /// `$EVERYGRAPH_ORACLE_DATA` only (an explicit opt-in: `$EVERYGRAPH_DATA` is never read,
    /// so that a plain `cargo test` does not load every dataset on the machine); memory
    /// limit [`DEFAULT_MAX_MEMORY`].
    #[must_use]
    pub fn from_env(golden: &Path) -> Self {
        let var = |k: &str| {
            std::env::var_os(k)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        Self {
            golden: golden.to_path_buf(),
            cache: var("EVERYGRAPH_ORACLE_CACHE")
                .unwrap_or_else(|| golden.parent().unwrap_or(golden).join(".cache")),
            data: var("EVERYGRAPH_ORACLE_DATA"),
            max_memory: DEFAULT_MAX_MEMORY,
        }
    }
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

fn bad(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

fn open(path: &Path) -> io::Result<Box<dyn BufRead>> {
    let file = fs::File::open(path)?;
    let reader: Box<dyn Read> = if path.extension().is_some_and(|x| x == "gz") {
        Box::new(flate2::read::MultiGzDecoder::new(file))
    } else {
        Box::new(file)
    };
    Ok(Box::new(BufReader::with_capacity(1 << 20, reader)))
}

/// Reads `path` under `parse` and relabels the ids densely by rank (numeric order for
/// integer ids, code-point order for strings).
///
/// # Errors
///
/// I/O errors, and `InvalidData` for a row that does not follow `parse`.
pub fn read_rows(path: &Path, parse: &Parse, directed: bool, max_memory: u64) -> io::Result<Graph> {
    let whitespace = parse.separator == "whitespace";
    let sep = parse.separator.chars().next().unwrap_or(' ');
    let integer = parse.ids == "integer";
    let (mut ia, mut ib) = (Vec::new(), Vec::new());
    let (mut sa, mut sb) = (Vec::new(), Vec::new());
    let mut rows = 0u64;
    let refuse = |rows: u64, n: u64| {
        bad(format!(
            "skipped: refused, {} rows and {n} nodes need about {:.1} GiB, more than the limit of {:.1} GiB (--max-memory)",
            rows,
            estimate(rows, n) as f64 / f64::from(1u32 << 30),
            max_memory as f64 / f64::from(1u32 << 30)
        ))
    };
    for (i, line) in open(path)?.lines().enumerate() {
        let line = line?;
        let line = line.trim_end_matches(['\r', '\n']);
        if line.trim().is_empty()
            || parse
                .comment
                .as_deref()
                .is_some_and(|c| line.starts_with(c))
        {
            continue;
        }
        let cols: Vec<&str> = if whitespace {
            line.split_whitespace().collect()
        } else {
            line.split(sep).collect()
        };
        if let Some(k) = &parse.keep {
            if cols.get(k.column) != Some(&k.equals.as_str()) {
                continue;
            }
        }
        let (Some(a), Some(b)) = (cols.get(parse.src), cols.get(parse.dst)) else {
            return Err(bad(format!(
                "{}:{}: too few columns",
                path.display(),
                i + 1
            )));
        };
        let (a, b) = (a.trim(), b.trim());
        rows += 1;
        if rows % 65_536 == 0 && estimate(rows, 0) > max_memory {
            return Err(refuse(rows, 0));
        }
        if integer {
            let p = |s: &str| {
                s.parse::<i64>().map_err(|_| {
                    bad(format!(
                        "{}:{}: not an integer id: {s}",
                        path.display(),
                        i + 1
                    ))
                })
            };
            ia.push(p(a)?);
            ib.push(p(b)?);
        } else {
            sa.push(a.to_owned());
            sb.push(b.to_owned());
        }
    }
    let name = path
        .file_name()
        .map_or_else(String::new, |f| f.to_string_lossy().into_owned());
    let n_estimate = if integer {
        let mut v: Vec<i64> = ia.iter().chain(&ib).copied().collect();
        v.sort_unstable();
        v.dedup();
        to_u64(v.len())
    } else {
        to_u64(sa.len() + sb.len())
    };
    if estimate(rows, n_estimate) > max_memory {
        return Err(refuse(rows, n_estimate));
    }
    let mut g;
    if integer {
        let mut ids: Vec<i64> = ia.iter().chain(&ib).copied().collect();
        ids.sort_unstable();
        ids.dedup();
        let rank = |x: i64| u32::try_from(ids.binary_search(&x).unwrap_or(0)).unwrap_or(u32::MAX);
        g = Graph::with_nodes(
            &name,
            u32::try_from(ids.len()).map_err(|_| bad("more than 2^32 ids"))?,
            directed,
        );
        g.src = ia.iter().map(|&x| rank(x)).collect();
        g.dst = ib.iter().map(|&x| rank(x)).collect();
    } else {
        let mut ids: Vec<&str> = sa.iter().chain(&sb).map(String::as_str).collect();
        ids.sort_unstable();
        ids.dedup();
        let rank = |x: &str| u32::try_from(ids.binary_search(&x).unwrap_or(0)).unwrap_or(u32::MAX);
        g = Graph::with_nodes(
            &name,
            u32::try_from(ids.len()).map_err(|_| bad("more than 2^32 ids"))?,
            directed,
        );
        g.src = sa.iter().map(|x| rank(x)).collect();
        g.dst = sb.iter().map(|x| rank(x)).collect();
    }
    Ok(g)
}

/// SHA-256 of a per-node array as little-endian `u64`, in hex.
#[must_use]
pub fn sha256_u64(values: &[u64]) -> String {
    let mut h = Sha256::new();
    for v in values {
        h.update(v.to_le_bytes());
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Computes every value a golden file may hold.
#[must_use]
pub fn compute(g: &Graph) -> BTreeMap<&'static str, serde_json::Value> {
    use serde_json::json;
    let p = Projection::new(g);
    let n = p.len();
    let mut out = BTreeMap::new();
    out.insert("n", json!(g.n));
    out.insert("rows", json!(p.counts.rows));
    out.insert("self_loop_rows", json!(p.counts.self_loop_rows));
    out.insert("duplicate_rows", json!(p.counts.duplicate_rows));
    out.insert("m", json!(p.m()));
    let deg: Vec<u64> = (0..n).map(|v| to_u64(p.undirected.degree(v))).collect();
    out.insert("max_degree", json!(deg.iter().copied().max().unwrap_or(0)));
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
    out.insert("wcc_count", json!(count));
    out.insert("largest_wcc", json!(largest));
    if g.directed {
        let (count, largest) = class_stats(&reference::scc(&p.out));
        out.insert("scc_count", json!(count));
        out.insert("largest_scc", json!(largest));
    }
    let t = reference::triangles(&p.undirected);
    out.insert("triangle_total", json!(t.iter().sum::<u64>() / 3));
    out.insert("triangles_sha256", json!(sha256_u64(&t)));
    let core = reference::core_numbers(&p.undirected);
    out.insert("core_numbers_sha256", json!(sha256_u64(&core)));
    let max_core = core.iter().copied().max().unwrap_or(0);
    out.insert("max_core", json!(max_core));
    let mut hist = vec![
        0u64;
        if n == 0 {
            0
        } else {
            crate::to_usize(max_core) + 1
        }
    ];
    for &c in &core {
        hist[crate::to_usize(c)] += 1;
    }
    out.insert("core_histogram", json!(hist));
    // First node, last node, and the node of largest degree (smallest id on ties).
    let mut picks: Vec<usize> = Vec::new();
    if n > 0 {
        let top = (0..n)
            .max_by_key(|&v| (deg[v], std::cmp::Reverse(v)))
            .unwrap_or(0);
        picks = vec![0, n - 1, top];
        picks.sort_unstable();
        picks.dedup();
    }
    let samples: Vec<[u64; 4]> = picks
        .iter()
        .map(|&v| [to_u64(v), deg[v], core[v], t[v]])
        .collect();
    out.insert("samples", json!(samples));
    out
}

fn source_path(roots: &Roots, s: &Source) -> Result<PathBuf, String> {
    match s.kind.as_str() {
        "vendored" => Ok(roots.golden.join(&s.file)),
        "download" => Ok(roots.cache.join(&s.file)),
        "registry" => {
            let data = roots.data.as_ref().ok_or(
                "skipped: no data root (--data-root, $EVERYGRAPH_ORACLE_DATA or $EVERYGRAPH_DATA)",
            )?;
            let ds = s
                .dataset
                .as_deref()
                .ok_or("registry source without a dataset")?;
            Ok(data.join(ds).join(&s.file))
        }
        other => Err(format!("unknown source kind {other}")),
    }
}

fn show(x: &serde_json::Value) -> String {
    let s = x.to_string();
    match s.char_indices().nth(80) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s,
    }
}

fn check_file(roots: &Roots, path: &Path) -> GoldenResult {
    let fail = |name: String, e: String| GoldenResult {
        name,
        outcome: Err(e),
    };
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => return fail(path.display().to_string(), e.to_string()),
    };
    let f: GoldenFile = match serde_json::from_str(&text) {
        Ok(f) => f,
        Err(e) => return fail(path.display().to_string(), e.to_string()),
    };
    let name = f.name.clone();
    if f.format != GOLDEN_FORMAT || f.version != GOLDEN_VERSION {
        return fail(name, format!("format {} v{}", f.format, f.version));
    }
    let src = match source_path(roots, &f.source) {
        Ok(p) => p,
        Err(e) => return fail(name, e),
    };
    if !src.is_file() {
        return fail(name, format!("skipped: {} not available", src.display()));
    }
    let g = match read_rows(&src, &f.parse, f.directed, roots.max_memory) {
        Ok(g) => g,
        Err(e) => return fail(name, e.to_string()),
    };
    let got = compute(&g);
    let mut agreed = 0;
    let mut mismatched = Vec::new();
    for (key, want) in &f.values {
        match got.get(key.as_str()) {
            Some(v) if *v == want.value => agreed += 1,
            Some(v) => mismatched.push(format!(
                "{key}: computed {} vs golden {}",
                show(v),
                show(&want.value)
            )),
            None => mismatched.push(format!("{key}: not computed by the Rust check")),
        }
    }
    if mismatched.is_empty() {
        GoldenResult {
            name,
            outcome: Ok(agreed),
        }
    } else {
        GoldenResult {
            name,
            outcome: Err(mismatched.join("; ")),
        }
    }
}

/// Checks every `*.json` golden file in `roots.golden` (or only those named in `only`).
///
/// # Errors
///
/// If the golden directory cannot be read.
pub fn check_dir(roots: &Roots, only: &[String]) -> io::Result<Vec<GoldenResult>> {
    let mut files: Vec<PathBuf> = fs::read_dir(&roots.golden)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter(|p| {
            only.is_empty()
                || p.file_stem()
                    .is_some_and(|s| only.iter().any(|o| *o == s.to_string_lossy()))
        })
        .collect();
    files.sort();
    Ok(files.iter().map(|p| check_file(roots, p)).collect())
}
