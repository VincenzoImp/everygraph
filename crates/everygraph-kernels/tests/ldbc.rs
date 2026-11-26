// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The kernels of the crate against the LDBC Graphalytics reference outputs (#177), with
//! the specification's match rules: BFS (direction-optimizing, bottom-up along the
//! in-lists of a directed graph, from the in-adjacency `everygraph-ingest` derives), `PageRank` (f64, the data set's fixed iteration count:
//! `tol = 0`, directed and undirected), SSSP on weighted data sets, and WCC (#48),
//! sampled and in one pass, on the merged lists of a directed graph. Each runs on raw
//! arrays and on targets streamed from a file, at 1 and 8 threads; the results must be
//! identical bit for bit as well.
//!
//! Opt-in: `EVERYGRAPH_LDBC_GRAPHS` names a directory with one unpacked data set per
//! subdirectory (`oracle/ldbc-unpack.sh`; `oracle/run-ci.sh` sets it). Data sets with more
//! than `EVERYGRAPH_LDBC_MAX_EDGES` edges (default 2^27) are skipped.
//! These are not LDBC Benchmark Results.

#![forbid(unsafe_code)]
// The test writes the files the streamed sources read.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
#![allow(clippy::disallowed_macros, clippy::print_stderr)]

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU32;

use everygraph_core::{Memory, par};
use everygraph_io::FileSource;
use everygraph_kernels::components::{WccOptions, wcc};
use everygraph_kernels::edge_map::{Csr, Plan, Settings, Source, Stream};
use everygraph_kernels::pagerank::{Graph, PageRank, pagerank, strength};
use everygraph_kernels::sssp::{Sssp, sssp};
use everygraph_kernels::traversal::{Bfs, UNREACHED, bfs};
use everygraph_oracle::ldbc::read::{Lists, merge_directions};
use everygraph_oracle::ldbc::run::{Row, row};
use everygraph_oracle::ldbc::{Algorithm, Dataset, LdbcGraph, UNREACHABLE, Values};

mod support;

/// One algorithm on one pair of sources: `out` (out-lists, or symmetric lists) and `inn`
/// (in-lists; the same lists on an undirected graph). `sample` is WCC's.
fn run_on<S: Source>(
    (out, inn): (&S, &S),
    weights: Option<&[f64]>,
    sample: bool,
    threads: usize,
    ds: &Dataset,
    g: &LdbcGraph,
    a: Algorithm,
) -> Result<Values, String> {
    let memory = Memory::unlimited();
    let settings = Settings::default();
    let e = |e: everygraph_kernels::edge_map::Error| e.to_string();
    let ids = |labels: Vec<AtomicU32>| {
        Values::Ints(
            labels
                .into_iter()
                .map(|l| g.ids.ids[usize::try_from(l.into_inner()).expect("u32 fits")])
                .collect(),
        )
    };
    let source = || -> Result<u32, String> {
        let id = ds.source(a).map_err(|e| e.to_string())?;
        g.ids
            .index(id)
            .ok_or_else(|| "source is not a vertex".into())
    };
    let threads = NonZeroUsize::new(threads).expect("threads > 0");
    let directed = g.directed;
    let result = Plan::with_pair(out, inn, &memory, settings, |po, pi| {
        par::with_threads(threads, || match a {
            Algorithm::Bfs => {
                let hops = bfs(&po, Some(&pi), source()?, &Bfs::default(), &memory).map_err(e)?;
                Ok(Values::Ints(
                    hops.distance
                        .into_iter()
                        .map(|d| match d.into_inner() {
                            UNREACHED => UNREACHABLE,
                            d => d.into(),
                        })
                        .collect(),
                ))
            }
            Algorithm::Wcc => {
                let options = WccOptions {
                    symmetric: true,
                    sample,
                };
                wcc(&po, &memory, options).map(|(l, _)| ids(l)).map_err(e)
            }
            Algorithm::Pr => {
                let s = strength(&po, None, directed, &memory).map_err(e)?;
                let graph = Graph {
                    lists: &pi,
                    weights: None,
                    directed,
                };
                let p = PageRank {
                    damping: ds.damping().map_err(|e| e.to_string())?,
                    tol: 0.0,
                    max_iter: ds.iterations(a).map_err(|e| e.to_string())?,
                    personalize: None,
                };
                let s = s.as_slice().ok_or("one segment")?;
                let r = pagerank::<_, f64>(&graph, s, &p, &memory).map_err(e)?;
                Ok(Values::Reals(r.values.iter().copied().collect()))
            }
            Algorithm::Sssp => {
                let w = weights.ok_or("skipped: no weights")?;
                let d = sssp(&po, w, source()?, &Sssp::default(), &memory).map_err(e)?;
                let n = po.nodes();
                Ok(Values::Reals(
                    (0..n).map(|v| d.get(v).unwrap_or(f64::INFINITY)).collect(),
                ))
            }
            _ => Err("skipped: not a kernel of this crate".into()),
        })
        .map_err(|t| t.to_string())?
    });
    result.map_err(|e| e.to_string())?
}

fn csr(l: Lists<'_>) -> Csr<'_> {
    Csr::new(l.offsets, l.targets).expect("csr")
}

fn stream_file(lists: Lists<'_>, path: &Path) -> FileSource {
    let bytes: Vec<u8> = lists.targets.iter().flat_map(|t| t.to_le_bytes()).collect();
    std::fs::write(path, bytes).expect("write targets");
    FileSource::open(path).expect("open targets")
}

fn check_dataset(dir: &Path, tmp: &Path) -> Vec<Row> {
    let ds = Dataset::open(dir).expect("data set");
    let g = ds.load().expect("graph");
    // The in-lists of a directed graph as a graph file stores them, checked against the
    // reader's own transpose.
    let derived_in = g.directed.then(|| {
        let out = g.out.lists();
        let (offsets, targets) = support::in_lists(out.offsets, out.targets);
        let theirs = g.inn();
        assert!(
            offsets == theirs.offsets && targets == theirs.targets,
            "{}: the derived in-adjacency is not the transpose",
            ds.name
        );
        (offsets, targets)
    });
    let inn = || match &derived_in {
        Some((offsets, targets)) => Lists {
            offsets,
            targets,
            weights: g.inn().weights,
        },
        None => g.inn(),
    };
    let merged = g.directed.then(|| merge_directions(g.out.lists(), inn()).0);
    let files = ["out", "in"].map(|w| tmp.join(format!("{}-{}.{w}", std::process::id(), ds.name)));
    let mut rows = Vec::new();
    for a in ds.algorithms() {
        let (out, inn) = match (a, &merged) {
            (Algorithm::Wcc, Some(m)) => (m.lists(), m.lists()),
            (Algorithm::Bfs | Algorithm::Pr | Algorithm::Sssp | Algorithm::Wcc, _) => {
                (g.out.lists(), inn())
            }
            _ => continue,
        };
        let (fo, fi) = (stream_file(out, &files[0]), stream_file(inn, &files[1]));
        let raw = (csr(out), csr(inn));
        let streamed = (
            Stream::new(out.offsets, &fo).expect("stream"),
            Stream::new(inn.offsets, &fi).expect("stream"),
        );
        let mut first: Option<Values> = None;
        let samples: &[bool] = if a == Algorithm::Wcc {
            &[true, false]
        } else {
            &[true]
        };
        let configs = samples.iter().flat_map(|&sample| {
            [("raw", 1), ("raw", 8), ("stream", 1), ("stream", 8)].map(|(n, t)| (sample, n, t))
        });
        for (sample, name, threads) in configs {
            let w = out.weights;
            let got = if name == "raw" {
                run_on((&raw.0, &raw.1), w, sample, threads, &ds, &g, a)
            } else {
                run_on((&streamed.0, &streamed.1), w, sample, threads, &ds, &g, a)
            };
            if let Ok(v) = &got {
                let same = first.get_or_insert_with(|| v.clone()) == v;
                assert!(
                    same,
                    "{} {}: {name} at {threads} threads differs",
                    ds.name,
                    a.name()
                );
            }
            rows.push(row(
                &ds,
                &g.ids,
                a,
                &format!(
                    "kernels/{name}/{threads}{}",
                    if sample { "" } else { "/one-pass" }
                ),
                got,
                0.0,
            ));
        }
    }
    for f in &files {
        let _ = std::fs::remove_file(f);
    }
    rows
}

#[test]
fn kernels_match_the_ldbc_reference_outputs() {
    let Some(root) = std::env::var_os("EVERYGRAPH_LDBC_GRAPHS") else {
        eprintln!("EVERYGRAPH_LDBC_GRAPHS is not set: no LDBC data set checked");
        return;
    };
    let max_edges: u64 = std::env::var("EVERYGRAPH_LDBC_MAX_EDGES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1 << 27);
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root)
        .expect("LDBC directory")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    let mut checked = 0;
    for dir in dirs {
        let (_, edges) = Dataset::open(&dir)
            .expect("data set")
            .declared_size()
            .expect("size");
        if edges > max_edges {
            eprintln!("{}: skipped, {edges} edges", dir.display());
            continue;
        }
        for r in check_dataset(&dir, &tmp) {
            eprintln!("  {}", r.describe());
            if r.status() == "skipped" {
                continue;
            }
            assert_eq!(r.status(), "match", "{}", r.describe());
            checked += 1;
        }
    }
    assert!(checked > 0, "no (graph, kernel) pair checked");
    eprintln!("{checked} (graph, algorithm, configuration) rows match LDBC");
}
