// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The proof kernels of the edge-map framework against the LDBC Graphalytics reference
//! outputs (#177), with the specification's match rules: BFS on every graph, WCC on the
//! merged lists of a directed graph, `PageRank` (f64, the data set's iteration count) on
//! undirected graphs. Each runs on raw arrays and on targets streamed from a file, at 1
//! and 8 threads; the results must be identical bit for bit as well.
//!
//! Opt-in: `EVERYGRAPH_LDBC_GRAPHS` names a directory with one unpacked data set per
//! subdirectory (`oracle/ldbc-unpack.sh`; `oracle/run-ci.sh` sets it). Data sets with more
//! than `EVERYGRAPH_LDBC_MAX_EDGES` edges (default 2^27) are skipped.
//! These are not LDBC Benchmark Results.

#![forbid(unsafe_code)]
// The test writes the file the streamed source reads.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
#![allow(clippy::disallowed_macros, clippy::print_stderr)]

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU32;

use everygraph_core::{Memory, par};
use everygraph_io::FileSource;
use everygraph_kernels::edge_map::{Csr, Plan, Settings, Source, Stream};
use everygraph_kernels::proof::{UNREACHED, bfs, pagerank, wcc};
use everygraph_oracle::ldbc::read::merge_directions;
use everygraph_oracle::ldbc::run::{Row, row};
use everygraph_oracle::ldbc::{Algorithm, Dataset, LdbcGraph, UNREACHABLE, Values};

fn run_on<S: Source>(
    source: &S,
    threads: usize,
    ds: &Dataset,
    g: &LdbcGraph,
    a: Algorithm,
) -> Result<Values, String> {
    let memory = Memory::unlimited();
    let settings = Settings {
        chunk_entries: 1 << 16,
        batch_entries: 8192,
        prefetch: false,
    };
    let e = |e: everygraph_kernels::edge_map::Error| e.to_string();
    let ids = |labels: Vec<AtomicU32>| {
        Values::Ints(
            labels
                .into_iter()
                .map(|l| g.ids.ids[usize::try_from(l.into_inner()).expect("u32 fits")])
                .collect(),
        )
    };
    let threads = NonZeroUsize::new(threads).expect("threads > 0");
    Plan::with(source, &memory, settings, |plan| {
        par::with_threads(threads, || match a {
            Algorithm::Bfs => {
                let id = ds.source(a).map_err(|e| e.to_string())?;
                let s = g.ids.index(id).ok_or("source is not a vertex")?;
                let (d, _) = bfs(&plan, &memory, s, !g.directed).map_err(e)?;
                Ok(Values::Ints(
                    d.into_iter()
                        .map(|d| match d.into_inner() {
                            UNREACHED => UNREACHABLE,
                            d => d.into(),
                        })
                        .collect(),
                ))
            }
            Algorithm::Wcc => wcc(&plan, &memory).map(|(l, _)| ids(l)).map_err(e),
            Algorithm::Pr => {
                let d = ds.damping().map_err(|e| e.to_string())?;
                let it = ds.iterations(a).map_err(|e| e.to_string())?;
                let r = pagerank::<_, f64>(&plan, &memory, d, it).map_err(e)?;
                Ok(Values::Reals(r.iter().copied().collect()))
            }
            _ => Err("not a proof kernel".into()),
        })
    })
    .map_err(|e| e.to_string())?
}

fn check_dataset(dir: &Path, tmp: &Path) -> Vec<Row> {
    let ds = Dataset::open(dir).expect("data set");
    let g = ds.load().expect("graph");
    let merged = g
        .directed
        .then(|| merge_directions(g.out.lists(), g.inn()).0);
    let targets_file = tmp.join(format!("{}-{}.targets", std::process::id(), ds.name));
    let mut rows = Vec::new();
    for a in ds.algorithms() {
        let lists = match (a, &merged) {
            (Algorithm::Pr, _) if g.directed => continue,
            (Algorithm::Bfs | Algorithm::Pr, _) | (Algorithm::Wcc, None) => &g.out,
            (Algorithm::Wcc, Some(m)) => m,
            _ => continue,
        };
        let bytes: Vec<u8> = lists.targets.iter().flat_map(|t| t.to_le_bytes()).collect();
        std::fs::write(&targets_file, bytes).expect("write targets");
        let file = FileSource::open(&targets_file).expect("open targets");
        let raw = Csr::new(&lists.offsets, &lists.targets).expect("csr");
        let streamed = Stream::new(&lists.offsets, &file).expect("stream");
        let mut first: Option<Values> = None;
        for (name, threads) in [("raw", 1), ("raw", 8), ("stream", 1), ("stream", 8)] {
            let got = if name == "raw" {
                run_on(&raw, threads, &ds, &g, a)
            } else {
                run_on(&streamed, threads, &ds, &g, a)
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
                &format!("proof/{name}/{threads}"),
                got,
                0.0,
            ));
        }
    }
    let _ = std::fs::remove_file(&targets_file);
    rows
}

#[test]
fn proof_kernels_match_the_ldbc_reference_outputs() {
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
            // The lab's PageRank, which these kernels port, needs symmetric lists; directed
            // graphs were skipped above. Everything else must match.
            assert_eq!(r.status(), "match", "{}", r.describe());
            checked += 1;
        }
    }
    assert!(checked > 0, "no (graph, kernel) pair checked");
    eprintln!("{checked} (graph, algorithm, configuration) rows match LDBC");
}
