// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The kernels of the crate through the correctness oracle: `PageRank` (weighted,
//! directed, personalized, with kept loops) and eigenvector centrality (#47), BFS with
//! its parents and SSSP (#49), weak components (#48), strong components (#223), core
//! numbers and degrees (#50).
//! Each runs on the CI corpus under every configuration that must not change a result —
//! 1, 3 and 8 threads,
//! chunks of 64 and 65,536 entries, and five sources: raw arrays, the compressed format
//! in the heap, the compressed format memory-mapped from a file, raw targets streamed
//! from a file, and the compressed format streamed from a file in the smallest window
//! (4 KiB), every second scan in reverse order. Results must be identical bit for bit
//! across all 30, and match the in-process references and the invariants.
//!
//! The adjacency is stored as the engine stores it: a kept loop is the entry `v` in the
//! list of `v`, with the loop's weight, and a cost of 0, which must never shorten a
//! path. On a directed graph the in-lists are the in-adjacency that `everygraph-ingest`
//! derives from the out-adjacency (#38), and the undirected projection (core numbers,
//! degrees) merges the out-lists with it; both are checked equal to the oracle's own.
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
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use everygraph_core::{Memory, par};
use everygraph_format::adjacency::{Adjacency, AdjacencyIndex, encode, merge};
use everygraph_io::{FileSource, MmapSource};
use everygraph_kernels::components::{WccOptions, wcc};
use everygraph_kernels::degrees::{Loop, degrees};
use everygraph_kernels::edge_map::{
    Compressed, CompressedStream, Csr, Plan, Settings, Source, Stream,
};
use everygraph_kernels::kcore::kcore;
use everygraph_kernels::pagerank::{self, Eigenvector, Graph, PageRank};
use everygraph_kernels::scc::scc;
use everygraph_kernels::sssp::{Sssp, sssp};
use everygraph_kernels::traversal::{Bfs, UNREACHED, bfs};
use everygraph_oracle::corpus::{Tier, entries};
use everygraph_oracle::plugin::{
    Case, Config, KernelUnderTest, Options, Prepared, Regime, Storage, run,
};
use everygraph_oracle::projection::{Adjacency as Analytic, Projection};
use everygraph_oracle::{Kernel, Output};

mod support;

/// Iterated to well inside the oracle's 1e-9: the error after stopping is at most
/// `tol · α / (1 − α)`.
const PAGERANK: PageRank<'static> = PageRank {
    damping: 0.85,
    tol: 1e-14,
    max_iter: 1000,
    personalize: None,
};

/// The error after stopping is about `tol / (1 − r)`, `r` the ratio of the two largest
/// eigenvalues of `I + Aᵀ`: about 0.999 on the corpus's paths and grids.
const EIGENVECTOR: Eigenvector = Eigenvector {
    tol: 1e-13,
    max_iter: 200_000,
};

/// One adjacency of the analytic graph with its kept loops written in, and its
/// per-entry columns.
struct Lists {
    offsets: Vec<u64>,
    targets: Vec<u32>,
    /// The pair's summed weight; a loop's weight.
    weight: Vec<f64>,
    /// The pair's smallest event weight; 0 for a loop.
    cost: Vec<f64>,
}

fn with_loops(adj: &Analytic, p: &Projection) -> Lists {
    let mut l = Lists {
        offsets: vec![0],
        targets: Vec::new(),
        weight: Vec::new(),
        cost: Vec::new(),
    };
    for v in 0..adj.n() {
        let r = adj.range(v);
        let at = r.start + adj.list(v).partition_point(|&u| (u as usize) < v);
        let entry = |e: usize| (adj.targets[e], adj.weight_sum[e], adj.weight_min[e]);
        let lower = (r.start..at).map(entry);
        let kept = (p.kept_loops(v) == 1).then_some((v as u32, p.loop_weight[v], 0.0));
        for (t, w, c) in lower.chain(kept).chain((at..r.end).map(entry)) {
            l.targets.push(t);
            l.weight.push(w);
            l.cost.push(c);
        }
        l.offsets.push(l.targets.len() as u64);
    }
    l
}

/// The lists a kernel reads on a directed graph: the in-adjacency derived from the
/// out-adjacency, or the undirected projection merged from both, checked byte-equal to
/// the encoding of the oracle's lists; those lists, with the oracle's per-entry columns.
fn derived(which: Which, p: &Projection) -> Lists {
    let out = with_loops(&p.out, p);
    let out = encode(&out.offsets, &out.targets).unwrap();
    let inn = support::in_adjacency(&out);
    let lists = match which {
        Which::In => inn,
        _ => {
            let (io, ii) = (
                AdjacencyIndex::parse(&out.index).unwrap(),
                AdjacencyIndex::parse(&inn.index).unwrap(),
            );
            let (a, b) = (
                Adjacency::new(&io, &out.data).unwrap(),
                Adjacency::new(&ii, &inn.data).unwrap(),
            );
            merge(&a, &b, &Memory::unlimited()).unwrap().0
        }
    };
    let expected = with_loops(
        if which == Which::In {
            &p.inn
        } else {
            &p.undirected
        },
        p,
    );
    assert!(lists == encode(&expected.offsets, &expected.targets).unwrap());
    expected
}

/// One adjacency in every storage the sources read.
struct Stored {
    lists: Lists,
    index: AdjacencyIndex,
    data: Vec<u8>,
    map: MmapSource,
    /// The data part, read by positional reads.
    file: FileSource,
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

fn store(lists: Lists) -> Stored {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("kernels-oracle");
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
        lists,
        index: AdjacencyIndex::parse(&encoded.index).unwrap(),
        data: encoded.data,
        map: MmapSource::open(&files[0]).unwrap(),
        file: FileSource::open(&files[0]).unwrap(),
        stream: FileSource::open(&files[1]).unwrap(),
        files,
    }
}

/// Which adjacency a kernel reads.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Which {
    Out,
    In,
    Undirected,
}

/// The kernels as an oracle plug-in.
#[derive(Default)]
struct Kernels {
    /// The last graph's stored adjacencies.
    cache: Mutex<Vec<((String, Which), Arc<Stored>)>>,
}

impl Kernels {
    fn stored(&self, graph: &str, which: Which, p: &Projection) -> Arc<Stored> {
        let mut cache = self.cache.lock().unwrap();
        cache.retain(|((g, _), _)| g == graph);
        if let Some((_, s)) = cache.iter().find(|((_, w), _)| *w == which) {
            return s.clone();
        }
        let adj = match which {
            Which::Out => &p.out,
            Which::In => &p.inn,
            Which::Undirected => &p.undirected,
        };
        let lists = if p.directed && which != Which::Out {
            derived(which, p)
        } else {
            with_loops(adj, p)
        };
        let s = Arc::new(store(lists));
        cache.push(((graph.to_owned(), which), s.clone()));
        s
    }
}

/// The parent rule of `docs/api-sketch.md` § 2.12, checked from the analytic in-lists:
/// the smallest node one hop closer to the source with an edge to the node.
fn check_parents(p: &Projection, depth: &[u32], parent: &[u32]) -> Result<(), String> {
    for v in 0..p.len() {
        let want = match depth[v] {
            0 | UNREACHED => UNREACHED,
            d => p
                .inn
                .list(v)
                .iter()
                .copied()
                .filter(|&u| depth[u as usize] == d - 1)
                .min()
                .unwrap_or(UNREACHED),
        };
        if parent[v] != want {
            return Err(format!("bfs parent of {v}: {} vs {want}", parent[v]));
        }
    }
    Ok(())
}

fn run_kernel<S: Source>(
    out: &S,
    inn: &S,
    stored: (&Stored, &Stored),
    p: &Projection,
    kernel: &Kernel,
    config: &Config,
) -> Result<Output, String> {
    let memory = Memory::unlimited();
    let threads = NonZeroUsize::new(config.threads).unwrap();
    let settings = Settings {
        chunk_entries: config.chunk_entries,
        batch_entries: usize::try_from(config.chunk_entries.min(8192)).unwrap(),
        prefetch: config.regime != Regime::InMemory,
        alternate: config.regime == Regime::Stream,
    };
    let e = |e: everygraph_kernels::edge_map::Error| e.to_string();
    let load = |cells: Vec<AtomicU32>| -> Vec<u32> {
        cells.into_iter().map(AtomicU32::into_inner).collect()
    };
    let reals = |v: &[f64]| Output::Reals(v.iter().map(|&x| Some(x)).collect());
    let directed = p.directed;
    let result = Plan::with_pair(out, inn, &memory, settings, |po, pi| {
        par::with_threads(threads, || match kernel {
            Kernel::PageRank {
                damping,
                weighted,
                personalize,
            } => {
                let w_out = weighted.then_some(&stored.0.lists.weight[..]);
                let s = pagerank::strength(&po, w_out, directed, &memory).map_err(e)?;
                let g = Graph {
                    lists: &pi,
                    weights: weighted.then_some(&stored.1.lists.weight[..]),
                    directed,
                };
                let params = PageRank {
                    damping: *damping,
                    personalize: (!personalize.is_empty()).then_some(&personalize[..]),
                    ..PAGERANK
                };
                let r = pagerank::pagerank::<_, f64>(&g, s.as_slice().unwrap(), &params, &memory)
                    .map_err(e)?;
                Ok(reals(r.values.as_slice().unwrap()))
            }
            Kernel::Eigenvector { weighted } => {
                let g = Graph {
                    lists: &pi,
                    weights: weighted.then_some(&stored.1.lists.weight[..]),
                    directed,
                };
                let r = pagerank::eigenvector(&g, &EIGENVECTOR, &memory).map_err(e)?;
                Ok(reals(r.values.as_slice().unwrap()))
            }
            Kernel::Wcc => {
                // A directed graph's out-lists: not symmetric, so the finish reads them all.
                let options = WccOptions {
                    symmetric: !directed,
                    ..WccOptions::default()
                };
                wcc(&po, &memory, options)
                    .map_err(e)
                    .map(|(l, _)| Output::Labels(load(l)))
            }
            Kernel::Scc => scc(&po, &pi, &memory)
                .map_err(e)
                .map(|(l, _)| Output::Labels(load(l))),
            Kernel::CoreNumber => kcore(&po, &memory)
                .map_err(e)
                .map(|(c, _)| Output::Ints(load(c).into_iter().map(|c| Some(c.into())).collect())),
            Kernel::Degree | Kernel::DegreeOut | Kernel::DegreeIn => {
                let own = if *kernel == Kernel::Degree {
                    Loop::Twice
                } else {
                    Loop::Once
                };
                degrees(&po, &memory, own)
                    .map_err(e)
                    .map(|d| Output::Ints(d.iter().map(|&d| Some(d.into())).collect()))
            }
            Kernel::Bfs { source } => {
                let params = Bfs {
                    parents: true,
                    ..Bfs::default()
                };
                let hops = bfs(&po, Some(&pi), *source, &params, &memory).map_err(e)?;
                let depth = load(hops.distance);
                check_parents(p, &depth, &load(hops.parent.unwrap()))?;
                let d = depth.iter().map(|&d| (d != UNREACHED).then_some(d.into()));
                Ok(Output::Ints(d.collect()))
            }
            Kernel::Sssp { source } => {
                let costs = &stored.0.lists.cost;
                let d = sssp(&po, costs, *source, &Sssp::default(), &memory).map_err(e)?;
                Ok(Output::Reals((0..p.n).map(|v| d.get(v)).collect()))
            }
            _ => Err("unsupported".into()),
        })
        .map_err(|t| t.to_string())?
    });
    result.map_err(|e| e.to_string())?
}

impl KernelUnderTest for Kernels {
    fn name(&self) -> String {
        "everygraph-kernels".into()
    }

    fn supports(&self, kernel: &Kernel, _: &Case<'_>) -> bool {
        matches!(
            kernel,
            Kernel::PageRank { .. }
                | Kernel::Eigenvector { .. }
                | Kernel::Wcc
                | Kernel::Scc
                | Kernel::CoreNumber
                | Kernel::Degree
                | Kernel::DegreeOut
                | Kernel::DegreeIn
                | Kernel::Bfs { .. }
                | Kernel::Sssp { .. }
        )
    }

    fn configurations(&self) -> Vec<Config> {
        let sources = [
            (Storage::Raw, Regime::InMemory),
            (Storage::Compressed, Regime::InMemory),
            (Storage::Compressed, Regime::Mmap),
            (Storage::Raw, Regime::Stream),
            (Storage::Compressed, Regime::Stream),
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
        let name = &case.graph.name;
        let (a, b) = match k {
            Kernel::CoreNumber | Kernel::Degree => (Which::Undirected, Which::Undirected),
            Kernel::Wcc if !p.directed => (Which::Undirected, Which::Undirected),
            Kernel::DegreeIn => (Which::In, Which::In),
            _ if !p.directed => (Which::Out, Which::Out),
            _ => (Which::Out, Which::In),
        };
        let (so, si) = (self.stored(name, a, p), self.stored(name, b, p));
        let stored = (&*so, &*si);
        match (config.storage, config.regime) {
            (Storage::Raw, Regime::InMemory) => {
                run_kernel(&csr(&so)?, &csr(&si)?, stored, p, k, config)
            }
            (Storage::Compressed, Regime::Mmap) => {
                run_kernel(&mapped(&so)?, &mapped(&si)?, stored, p, k, config)
            }
            (Storage::Compressed, Regime::Stream) => {
                run_kernel(&streamed(&so)?, &streamed(&si)?, stored, p, k, config)
            }
            (Storage::Compressed, _) => run_kernel(&heap(&so)?, &heap(&si)?, stored, p, k, config),
            (Storage::Raw, _) => run_kernel(&stream(&so)?, &stream(&si)?, stored, p, k, config),
        }
    }
}

fn csr(s: &Stored) -> Result<Csr<'_>, String> {
    Csr::new(&s.lists.offsets, &s.lists.targets).map_err(|e| e.to_string())
}

fn mapped(s: &Stored) -> Result<Compressed<'_>, String> {
    Compressed::mapped(&s.index, &s.map).map_err(|e| e.to_string())
}

fn heap(s: &Stored) -> Result<Compressed<'_>, String> {
    Adjacency::new(&s.index, &s.data)
        .map(Compressed::new)
        .map_err(|e| e.to_string())
}

fn streamed(s: &Stored) -> Result<CompressedStream<'_>, String> {
    CompressedStream::new(&s.index, &s.file, 0).map_err(|e| e.to_string())
}

fn stream(s: &Stored) -> Result<Stream<'_>, String> {
    Stream::new(&s.lists.offsets, &s.stream).map_err(|e| e.to_string())
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
#[cfg_attr(
    debug_assertions,
    ignore = "minutes in a debug build; `oracle/run-ci.sh` (the oracle job) runs it in release"
)]
fn kernels_pass_the_oracle_under_every_configuration() {
    let (graphs, python) = graphs();
    let report = run(&Kernels::default(), &graphs, Options::default());
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
    assert!(report.checked > 3 * graphs.len(), "{report:?}");
    assert_eq!(report.runs, report.checked * 30);
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
