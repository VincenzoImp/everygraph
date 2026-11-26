// SPDX-License-Identifier: Apache-2.0 OR MIT

//! DESIGN D14 across threads, chunkings, sources and repetition (#54): the graph build,
//! every whole-graph kernel, weighted `PageRank`, eigenvector and SSSP with weights
//! over 2^-20 to 2^21, and Leiden, also with those weights; and order probes of the
//! reduction primitives.

#![forbid(unsafe_code)]
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::cast_possible_truncation,
    clippy::print_stderr
)]

use std::io::Write as _;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use everygraph_community::{Objective, Options, Refinement};
use everygraph_core::{Budget, ByteSource, Memory, MemorySource, par};
use everygraph_determinism::{
    Config, Lists, SplitMix, Stored, THREADS, communities, communities_hash, corpus, full, grid,
    kernels, on, planted, same, weighted_communities, with_source,
};
use everygraph_format::container::Provenance;
use everygraph_ingest::build::{BuildOptions, Duplicates, GraphOptions, Regime, SelfLoops, Silent};
use everygraph_ingest::native::{self, JobScratch, Output, Written};
use everygraph_ingest::text::{TextOptions, TextReader};
use everygraph_io::{FileSource, MmapSource};
use everygraph_kernels::edge_map::{Plan, Reduce, all};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("determinism")
}

/// The graphs of the kernel and Leiden checks: every 12th graph of the CI corpus
/// (every third with [`full`]), and planted partitions with wide weights. 10,000
/// nodes give four sub-rounds of about 2,500 nodes on Leiden's level 0, which run on
/// every thread; 400,000 nodes ([`full`]) give eight, and reduction blocks of 128
/// nodes.
fn graphs() -> Vec<(String, Stored)> {
    let step = if full() { 3 } else { 12 };
    let mut out: Vec<(String, Lists)> = corpus(step);
    out.push(("planted 3,000".into(), planted(30, 100, 6, 2, 1)));
    out.push(("planted 10,000".into(), planted(40, 250, 8, 2, 5)));
    if full() {
        out.push(("planted 400,000".into(), planted(400, 1000, 8, 2, 9)));
    }
    out.into_iter()
        .map(|(name, lists)| (name, Stored::new(lists, &dir())))
        .collect()
}

#[test]
fn kernels_give_the_same_bits_everywhere() {
    let mut runs = 0;
    for (i, (name, g)) in graphs().iter().enumerate() {
        let configs = grid(i);
        same(name, &configs, |c: Config| {
            with_source!(g.source(c.kind), |s| kernels(
                &s,
                c.settings(),
                &g.lists.weights
            ))
        });
        runs += configs.len();
    }
    eprintln!("{runs} runs of every kernel, each identical to the first of its graph");
}

#[test]
fn leiden_gives_the_same_bits_everywhere() {
    let randomized = Options {
        refinement: Refinement::Randomized { theta: 0.01 },
        seed: 7,
        ..Options::default()
    };
    let cpm = Options {
        objective: Objective::Cpm,
        resolution: 0.05,
        ..randomized
    };
    // The last two read the graph's weights: 1.0 on the corpus, wide on the planted
    // partitions (#58).
    let variants = [
        ("default", Options::default()),
        ("quality", Options::quality()),
        ("randomized", randomized),
        ("weighted", Options::default()),
        ("weighted, CPM", cpm),
    ];
    let mut runs = 0;
    for (i, (name, g)) in graphs().iter().enumerate() {
        for (k, (variant, o)) in variants.iter().enumerate() {
            if !full() && (i + k) % 3 != 0 {
                continue;
            }
            let configs = grid(i + k);
            let weights = (k >= 3).then_some(&g.lists.weights[..]);
            same(&format!("{name}, {variant}"), &configs, |c: Config| {
                with_source!(g.source(c.kind), |s| communities_hash(&match weights {
                    None => communities(&s, c.settings(), o),
                    Some(w) => weighted_communities(&s, w, c.settings(), o),
                }))
            });
            runs += configs.len();
        }
    }
    eprintln!("{runs} runs of Leiden, each identical to the first of its graph and options");
}

/// How the build reads its input.
#[derive(Clone, Copy, Debug)]
enum Input {
    Memory,
    File,
    Mapped,
    Gzip,
    Zstd,
}

/// An edge list of `rows` rows over sparse integer ids, with repeated rows and loops.
fn edge_list(rows: u64, seed: u64) -> Vec<u8> {
    let mut rng = SplitMix(seed);
    let mut text = Vec::new();
    let nodes = rows / 4;
    for _ in 0..rows {
        let u = rng.below(nodes);
        let v = match rng.below(20) {
            0 => u,
            1..=4 => (u + 1) % nodes,
            _ => rng.below(nodes),
        };
        writeln!(text, "{}\t{}", u * 7 + 3, v * 7 + 3).unwrap();
    }
    text
}

/// Writes `bytes` to a file of its own and returns its path.
fn file(name: &str, bytes: &[u8]) -> PathBuf {
    let d = dir();
    std::fs::create_dir_all(&d).unwrap();
    let path = d.join(format!("{}-{name}", std::process::id()));
    std::fs::write(&path, bytes).unwrap();
    path
}

/// The bytes of the graph built from `reader` on the current pool, and the regime.
fn build<S: ByteSource + Sync>(
    reader: &TextReader<S>,
    graph: GraphOptions,
    threads: usize,
    memory: &Memory,
    scratch: &JobScratch,
) -> Option<(Vec<u8>, Regime)> {
    let options = BuildOptions::new(graph, NonZeroUsize::new(threads).unwrap());
    let provenance = Provenance {
        engine: "everygraph determinism suite".into(),
        ..Provenance::default()
    };
    match native::write(
        reader,
        &options,
        memory,
        scratch,
        Output::Memory,
        provenance,
        &Silent,
        &|| true,
    ) {
        Ok((Written::Memory(bytes), report, _)) => Some((bytes, report.regime)),
        Ok(_) => unreachable!("built in memory"),
        Err(native::WriteError::Build(everygraph_ingest::build::BuildError::Budget(_))) => None,
        Err(e) => panic!("{e:?}"),
    }
}

/// A plain-text reader of `input`, through a copy in `scratch` when it is compressed.
fn reader_and_build(
    input: Input,
    paths: &Paths,
    graph: GraphOptions,
    threads: usize,
    memory: &Memory,
) -> Option<(Vec<u8>, Regime)> {
    let options = TextOptions::default();
    let job = format!("{input:?}-{threads}-{}", std::process::id());
    let scratch = JobScratch::create(&dir(), &job).unwrap();
    let open = |p: &Path| TextReader::open(FileSource::open(p).unwrap(), &options).unwrap();
    match input {
        Input::Memory => {
            let r = TextReader::open(MemorySource::new(&paths.text[..]), &options).unwrap();
            build(&r, graph, threads, memory, &scratch)
        }
        Input::File => build(&open(&paths.plain), graph, threads, memory, &scratch),
        Input::Mapped => {
            let r = TextReader::open(MmapSource::open(&paths.plain).unwrap(), &options).unwrap();
            build(&r, graph, threads, memory, &scratch)
        }
        Input::Gzip | Input::Zstd => {
            let p = if matches!(input, Input::Gzip) {
                &paths.gzip
            } else {
                &paths.zstd
            };
            let plain = native::decompress(&open(p), &scratch, &options).unwrap();
            build(&plain, graph, threads, memory, &scratch)
        }
    }
}

struct Paths {
    text: Vec<u8>,
    plain: PathBuf,
    gzip: PathBuf,
    zstd: PathBuf,
}

impl Drop for Paths {
    fn drop(&mut self) {
        for p in [&self.plain, &self.gzip, &self.zstd] {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// The `.eg` bytes are the same from plain text in memory, in a file or mapped, and
/// from gzip and zstd, at every thread count, without a budget and at the smallest
/// that builds. Spilling needs smaller buffers than the public build has at these
/// sizes; `everygraph-ingest`'s own test `output_does_not_depend_on_threads_or_budget`
/// covers it with its test tuning.
#[test]
fn the_build_gives_the_same_bytes_everywhere() {
    let rows = if full() { 400_000 } else { 30_000 };
    let text = edge_list(rows, 3);
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gz.write_all(&text).unwrap();
    let paths = Paths {
        plain: file("edges.tsv", &text),
        gzip: file("edges.tsv.gz", &gz.finish().unwrap()),
        zstd: file(
            "edges.tsv.zst",
            &ruzstd::encoding::compress_to_vec(
                &text[..],
                ruzstd::encoding::CompressionLevel::Fastest,
            ),
        ),
        text,
    };
    let inputs = [
        Input::Memory,
        Input::File,
        Input::Mapped,
        Input::Gzip,
        Input::Zstd,
    ];
    let undirected = GraphOptions {
        directed: false,
        self_loops: SelfLoops::Keep,
        duplicates: Duplicates::Events,
    };
    let directed = GraphOptions {
        directed: true,
        self_loops: SelfLoops::Drop,
        duplicates: Duplicates::Collapse,
    };
    for graph in [undirected, directed] {
        let unlimited = Memory::unlimited();
        let (expected, _) = on(1, || {
            reader_and_build(Input::Memory, &paths, graph, 1, &unlimited)
        })
        .unwrap();
        // The smallest power of two that builds on one thread. More threads hold more
        // buffers, and may be refused there.
        let small = (20..)
            .map(|shift| 1_u64 << shift)
            .find(|&b| {
                let memory = Memory::from(Arc::new(Budget::new(b)));
                on(1, || {
                    reader_and_build(Input::Memory, &paths, graph, 1, &memory)
                })
                .is_some()
            })
            .unwrap();
        let (mut spilled, mut refused) = (0, 0);
        for (i, threads) in THREADS.into_iter().enumerate() {
            for (k, input) in inputs.into_iter().enumerate() {
                for budget in [None, Some(small)] {
                    if !full() && (i + k) % inputs.len() != 0 {
                        continue;
                    }
                    let memory = budget.map_or_else(Memory::unlimited, |b| {
                        Memory::from(Arc::new(Budget::new(b)))
                    });
                    let what =
                        format!("{graph:?}, {input:?}, {threads} threads, budget {budget:?}");
                    let Some((bytes, regime)) = on(threads, || {
                        reader_and_build(input, &paths, graph, threads, &memory)
                    }) else {
                        assert!(budget.is_some(), "{what}: refused without a budget");
                        refused += 1;
                        continue;
                    };
                    spilled += usize::from(regime == Regime::OnDisk);
                    assert!(bytes == expected, "{what}: other bytes");
                }
            }
        }
        eprintln!("{graph:?}: budget {small}, {spilled} builds spilled, {refused} refused");
    }
}

/// Lanes of an order probe: one sum per pair `(i, j)` under test.
const LANES: usize = 64;
/// The large value of `FPRev` (Xie et al., ATC'25): its unit in the last place is 4,096, so
/// adding a count of ones below 2,048 to it leaves it unchanged. A sum of ones with
/// `+M` at `i` and `−M` at `j` is then the number of ones added after the subtree that
/// joins `i` and `j`: the results over all pairs reveal the summation tree.
const M: f64 = 18_446_744_073_709_551_616.0;

#[derive(Clone, Copy)]
struct Lanes([f64; LANES]);

impl Reduce for Lanes {
    const ZERO: Self = Self([0.0; LANES]);
    fn add(mut self, other: Self) -> Self {
        for (a, b) in self.0.iter_mut().zip(other.0) {
            *a += b;
        }
        self
    }
}

/// Adds the probe input at position `v` to each lane: `+M` where `v` is the lane's
/// `i`, `−M` where it is its `j`, 1 otherwise.
fn add_probe(acc: &mut Lanes, pairs: &[(usize, usize)], v: usize) {
    for (k, x) in acc.0.iter_mut().enumerate() {
        *x += match pairs.get(k) {
            Some(&(i, _)) if v == i => M,
            Some(&(_, j)) if v == j => -M,
            _ => 1.0,
        };
    }
}

/// Every pair `i < j` below `n` (with [`full`]) or a seventh of them, in lanes.
fn probe_pairs(n: usize) -> Vec<Vec<(usize, usize)>> {
    let all = (0..n).flat_map(|i| (i + 1..n).map(move |j| (i, j)));
    let pairs: Vec<_> = all.filter(|(i, j)| full() || (i + j) % 7 == 0).collect();
    pairs.chunks(LANES).map(<[_]>::to_vec).collect()
}

/// The specified tree of a sum over cuts of `cut` consecutive values: a left fold from
/// +0.0 inside each cut, then a left fold of the cuts' sums from +0.0
/// (`everygraph_kernels::edge_map`, "Reduction blocks"; `par::map_ranges`).
fn specified(pairs: &[(usize, usize)], n: usize, cut: usize) -> Lanes {
    let mut total = Lanes::ZERO;
    for start in (0..n).step_by(cut) {
        let mut acc = Lanes::ZERO;
        for v in start..(start + cut).min(n) {
            add_probe(&mut acc, pairs, v);
        }
        total = total.add(acc);
    }
    total
}

fn bits(l: &Lanes) -> Vec<u64> {
    l.0.iter().map(|x| x.to_bits()).collect()
}

/// R15b of `docs/research/sota-numerics.md`: the summation tree of the edge-map
/// framework's reduction (every scan, as `PageRank`'s float sums would use it) and of
/// `par::map_ranges` is the specified one at every thread count, chunking and source.
#[test]
fn order_probes_reveal_the_specified_summation_tree() {
    let n = if full() { 1000 } else { 300 };
    let path = Lists::undirected(n as u32, (0..n as u32 - 1).map(|v| (v, v + 1, 1.0)));
    let g = Stored::new(path, &dir());
    let lanes = probe_pairs(n);
    let configs: Vec<Config> = if full() {
        grid(0)
    } else {
        grid(0).into_iter().chain(grid(3)).collect()
    };
    for c in configs {
        let wrong = on(c.threads, || {
            with_source!(g.source(c.kind), |s| {
                Plan::with(&s, &Memory::unlimited(), c.settings(), |plan| {
                    let cut = plan.block_nodes() as usize;
                    lanes
                        .iter()
                        .filter(|pairs| {
                            let sum: Lanes = plan
                                .scan((), all, |acc: &mut Lanes, (), _, row, _| {
                                    add_probe(acc, pairs, row.node.index());
                                })
                                .unwrap();
                            bits(&sum) != bits(&specified(pairs, n, cut))
                        })
                        .count()
                })
            })
        });
        assert_eq!(wrong, 0, "edge-map reduction, {c:?}");
    }
    for threads in THREADS {
        for grain in [1, 7, 64] {
            let g = NonZeroUsize::new(grain).unwrap();
            let wrong = on(threads, || {
                lanes
                    .iter()
                    .filter(|pairs| {
                        let parts = par::map_ranges(n, g, |r| {
                            let mut acc = Lanes::ZERO;
                            for v in r {
                                add_probe(&mut acc, pairs, v);
                            }
                            acc
                        });
                        let sum = parts.into_iter().fold(Lanes::ZERO, Lanes::add);
                        bits(&sum) != bits(&specified(pairs, n, grain))
                    })
                    .count()
            });
            assert_eq!(wrong, 0, "map_ranges, {threads} threads, grain {grain}");
        }
    }
}
