// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The proof kernels of the edge-map framework through the correctness oracle (#44):
//! `PageRank`, components and BFS, written once, on the CI corpus, under every
//! configuration that must not change a result — 1, 3 and 8 threads, chunks of 64 and
//! 65,536 entries, and four sources: raw arrays, the compressed format in the heap, the
//! compressed format memory-mapped from a file, and raw targets streamed from a file.
//! Results must be identical bit for bit across all 24, and match the in-process
//! references and the invariants.
//!
//! With `EVERYGRAPH_ORACLE_CORPUS` set to a corpus directory that `oracle-ref run` has
//! filled (`oracle/run-ci.sh` does), the graphs are read from there and compared with
//! NetworkX, python-igraph and scipy as well.

#![forbid(unsafe_code)]
// Tests write the files that the mapped and streamed sources read.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
// Test code on small, generated graphs: casts cannot truncate, and a summary line is printed.
#![allow(
    clippy::cast_possible_truncation,
    clippy::disallowed_macros,
    clippy::print_stderr,
    clippy::doc_markdown,
    clippy::type_complexity
)]

use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use everygraph_core::{Memory, par};
use everygraph_format::adjacency::{Adjacency, AdjacencyIndex, encode};
use everygraph_io::{FileSource, MmapSource};
use everygraph_kernels::edge_map::{Compressed, Csr, Plan, Settings, Source, Stream};
use everygraph_kernels::proof::{UNREACHED, bfs, pagerank, wcc};
use everygraph_oracle::corpus::{Tier, entries};
use everygraph_oracle::plugin::{
    Case, Config, KernelUnderTest, Options, Prepared, Regime, Storage, run,
};
use everygraph_oracle::projection::Adjacency as Lists;
use everygraph_oracle::{Kernel, Output};

/// Iterations of `PageRank`: past the oracle's convergence (241 at damping 0.85).
const ITERATIONS: u32 = 300;

/// One adjacency in every storage the sources read.
struct Stored {
    offsets: Vec<u64>,
    targets: Vec<u32>,
    index: AdjacencyIndex,
    data: Vec<u8>,
    map: MmapSource,
    stream: FileSource,
    files: [PathBuf; 2],
}

impl Drop for Stored {
    fn drop(&mut self) {
        for f in &self.files {
            let _ = std::fs::remove_file(f);
        }
    }
}

fn store(lists: &Lists) -> Stored {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("edge-map-oracle");
    std::fs::create_dir_all(&dir).unwrap();
    let k = NEXT.fetch_add(1, Ordering::Relaxed);
    let files = [
        dir.join(format!("{}-{k}.data", std::process::id())),
        dir.join(format!("{}-{k}.targets", std::process::id())),
    ];
    let encoded = encode(&lists.offsets, &lists.targets).unwrap();
    std::fs::write(&files[0], &encoded.data).unwrap();
    let bytes: Vec<u8> = lists.targets.iter().flat_map(|t| t.to_le_bytes()).collect();
    std::fs::write(&files[1], bytes).unwrap();
    Stored {
        offsets: lists.offsets.clone(),
        targets: lists.targets.clone(),
        index: AdjacencyIndex::parse(&encoded.index).unwrap(),
        data: encoded.data,
        map: MmapSource::open(&files[0]).unwrap(),
        stream: FileSource::open(&files[1]).unwrap(),
        files,
    }
}

/// The framework's proof kernels as an oracle plug-in.
#[derive(Default)]
struct Proof {
    /// The last graph's stored adjacencies, by (graph, which adjacency).
    cache: Mutex<Vec<((String, &'static str), std::sync::Arc<Stored>)>>,
}

impl Proof {
    fn stored(&self, graph: &str, which: &'static str, lists: &Lists) -> std::sync::Arc<Stored> {
        let mut cache = self.cache.lock().unwrap();
        cache.retain(|((g, _), _)| g == graph);
        if let Some((_, s)) = cache.iter().find(|((_, w), _)| *w == which) {
            return s.clone();
        }
        let s = std::sync::Arc::new(store(lists));
        cache.push(((graph.to_owned(), which), s.clone()));
        s
    }
}

fn kernel<S: Source>(
    source: &S,
    kernel: &Kernel,
    config: &Config,
    symmetric: bool,
) -> Result<Output, String> {
    let memory = Memory::unlimited();
    let threads = NonZeroUsize::new(config.threads).unwrap();
    let settings = Settings {
        chunk_entries: config.chunk_entries,
        batch_entries: usize::try_from(config.chunk_entries.min(8192)).unwrap(),
        prefetch: config.regime != Regime::InMemory,
    };
    let e = |e: everygraph_kernels::edge_map::Error| e.to_string();
    let out = Plan::with(source, &memory, settings, |plan| {
        par::with_threads(threads, || match kernel {
            Kernel::PageRank { damping, .. } => {
                pagerank::<_, f64>(&plan, &memory, *damping, ITERATIONS)
                    .map_err(e)
                    .map(|r| Output::Reals(r.iter().map(|&x| Some(x)).collect()))
            }
            Kernel::Wcc => wcc(&plan, &memory)
                .map_err(e)
                .map(|(l, _)| Output::Labels(l.into_iter().map(AtomicU32::into_inner).collect())),
            Kernel::Bfs { source } => {
                bfs(&plan, &memory, *source, symmetric)
                    .map_err(e)
                    .map(|(d, _)| {
                        let d = d.into_iter().map(AtomicU32::into_inner);
                        Output::Ints(d.map(|d| (d != UNREACHED).then_some(d.into())).collect())
                    })
            }
            _ => Err("unsupported".into()),
        })
    });
    out.map_err(|e| e.to_string())?
}

impl KernelUnderTest for Proof {
    fn name(&self) -> String {
        "edge-map proof kernels".into()
    }

    fn supports(&self, kernel: &Kernel, case: &Case<'_>) -> bool {
        let p = case.projection;
        match kernel {
            Kernel::Wcc | Kernel::Bfs { .. } => true,
            Kernel::PageRank { weighted, .. } => {
                !weighted && !p.directed && !(p.keep_loops && p.loops.iter().any(|&l| l > 0))
            }
            _ => false,
        }
    }

    fn configurations(&self) -> Vec<Config> {
        let sources = [
            (Storage::Raw, Regime::InMemory),
            (Storage::Compressed, Regime::InMemory),
            (Storage::Compressed, Regime::Mmap),
            (Storage::Raw, Regime::Stream),
        ];
        Config::grid(
            &[1, 3, 8],
            &[64, 1 << 16],
            &[Storage::Raw, Storage::Compressed],
            &[Regime::InMemory, Regime::Mmap, Regime::Stream],
        )
        .into_iter()
        .filter(|c| sources.contains(&(c.storage, c.regime)))
        .collect()
    }

    fn run(&self, case: &Case<'_>, k: &Kernel, config: &Config) -> Result<Output, String> {
        let p = case.projection;
        let (which, lists) = match k {
            Kernel::Wcc => ("undirected", &p.undirected),
            _ => ("out", &p.out),
        };
        let s = self.stored(&case.graph.name, which, lists);
        let symmetric = !p.directed;
        match (config.storage, config.regime) {
            (Storage::Raw, Regime::InMemory) => {
                let csr = Csr::new(&s.offsets, &s.targets).map_err(|e| e.to_string())?;
                kernel(&csr, k, config, symmetric)
            }
            (Storage::Compressed, Regime::Mmap) => {
                let source = Compressed::mapped(&s.index, &s.map).map_err(|e| e.to_string())?;
                kernel(&source, k, config, symmetric)
            }
            (Storage::Compressed, _) => {
                let adj = Adjacency::new(&s.index, &s.data).map_err(|e| e.to_string())?;
                kernel(&Compressed::new(adj), k, config, symmetric)
            }
            (Storage::Raw, _) => {
                let stream = Stream::new(&s.offsets, &s.stream).map_err(|e| e.to_string())?;
                kernel(&stream, k, config, symmetric)
            }
        }
    }
}

fn graphs() -> (Vec<Prepared>, bool) {
    match std::env::var_os("EVERYGRAPH_ORACLE_CORPUS") {
        Some(dir) => {
            let dirs = everygraph_oracle::exchange::list_corpus(dir.as_ref()).unwrap();
            (
                dirs.iter().map(|d| Prepared::load(d).unwrap()).collect(),
                true,
            )
        }
        None => (
            entries(Tier::Ci).into_iter().map(Prepared::new).collect(),
            false,
        ),
    }
}

#[test]
fn proof_kernels_pass_the_oracle_under_every_configuration() {
    let (graphs, python) = graphs();
    let report = run(&Proof::default(), &graphs, Options::default());
    let shown: Vec<String> = report
        .findings
        .iter()
        .take(20)
        .map(ToString::to_string)
        .collect();
    assert!(
        report.findings.is_empty(),
        "{} findings:\n{}",
        report.findings.len(),
        shown.join("\n")
    );
    assert!(report.checked > 3 * graphs.len() / 2, "{report:?}");
    assert_eq!(report.runs, report.checked * 24);
    if python {
        assert!(
            report.python_comparisons > 0 && report.python_missing == 0,
            "{report:?}"
        );
    }
    eprintln!(
        "{} graphs, {} (graph, kernel) pairs, {} runs, {} Python comparisons, 0 findings",
        report.graphs, report.checked, report.runs, report.python_comparisons
    );
}
