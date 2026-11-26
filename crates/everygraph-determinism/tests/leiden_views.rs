// SPDX-License-Identifier: Apache-2.0 OR MIT

//! What DESIGN D14 promises for Leiden on the same graph stored or viewed in other ways
//! (ADR 0014, "Reproducibility"):
//!
//! - a view as a mask (the graph's ids, the other nodes without edges) and the same
//!   view materialized (its nodes renumbered in order) give the same communities;
//! - nodes without edges change nothing, also past a boundary of the sub-round rule
//!   `clamp(next_pow2(m / 65,536), 4, 1,024)`: `m` counts the nodes with edges;
//! - `compact` keeps the communities of a graph file.
//!
//! The same graph with its ids in another order (`compact --order`, m2-24) is another
//! input: D14 does not promise its communities.
//!
//! The views are compared on the communities of the view's nodes, numbered in the
//! order of their smallest node. The graph is one whose communities change with the
//! seed, so a pass is not an accident of a graph with one clear answer.

#![forbid(unsafe_code)]
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::cast_possible_truncation
)]

use std::collections::HashMap;
use std::fmt::Write as _;
use std::num::NonZeroUsize;
use std::path::PathBuf;

use everygraph_community::{Communities, Options, Refinement, hierarchy};
use everygraph_core::{ByteSource, Memory, MemorySource};
use everygraph_determinism::{Lists, communities, communities_hash, on, planted};
use everygraph_format::adjacency::{Adjacency, AdjacencyIndex};
use everygraph_format::container::{Container, Provenance, Writer, compact};
use everygraph_ingest::build::{BuildOptions, Duplicates, GraphOptions, SelfLoops, Silent};
use everygraph_ingest::native::{self, JobScratch, Output, Written};
use everygraph_ingest::text::{TextOptions, TextReader};
use everygraph_kernels::edge_map::{Compressed, Csr, Settings};

/// Leiden's result on `lists` at 3 threads.
fn run(lists: &Lists, o: &Options) -> Communities {
    let csr = Csr::new(&lists.offsets, &lists.targets).unwrap();
    on(3, || communities(&csr, Settings::default(), o))
}

/// The options compared, seed 1: the default, the quality mode (iterations after the
/// first start from the communities found), randomized refinement, and prolongation
/// (finer levels start from the communities of coarser ones).
fn variants() -> [(&'static str, Options); 4] {
    let o = Options {
        seed: 1,
        ..Options::default()
    };
    [
        ("default", o),
        (
            "quality",
            Options {
                seed: 1,
                ..Options::quality()
            },
        ),
        (
            "randomized",
            Options {
                refinement: Refinement::Randomized { theta: 0.01 },
                ..o
            },
        ),
        ("prolonged", Options { prolong: true, ..o }),
    ]
}

/// `labels` renumbered from 0 in the order of first appearance.
fn canonical(labels: impl IntoIterator<Item = u32>) -> Vec<u32> {
    let mut names = HashMap::new();
    labels
        .into_iter()
        .map(|x| {
            let next = names.len() as u32;
            *names.entry(x).or_insert(next)
        })
        .collect()
}

/// Nodes whose community is not the one most of their community's nodes have in `b`.
fn moved(a: &[u32], b: &[u32]) -> usize {
    let mut votes: HashMap<(u32, u32), usize> = HashMap::new();
    for (&x, &y) in a.iter().zip(b) {
        *votes.entry((x, y)).or_default() += 1;
    }
    let mut best: HashMap<u32, usize> = HashMap::new();
    for (&(x, _), &k) in &votes {
        let b = best.entry(x).or_default();
        *b = (*b).max(k);
    }
    a.len() - best.values().sum::<usize>()
}

/// A planted partition whose Leiden communities change with the seed.
fn sensitive() -> Lists {
    let g = planted(40, 250, 2, 2, 11);
    assert_ne!(
        run(&g, &variants()[0].1).membership[..],
        run(
            &g,
            &Options {
                seed: 2,
                ..Options::default()
            }
        )
        .membership[..],
        "the communities do not depend on the seed: the probe would prove nothing"
    );
    g
}

/// The work of each level: what the sub-round count changes when the communities do
/// not.
fn work(c: &Communities) -> Vec<[u64; 4]> {
    c.stats
        .iter()
        .map(|s| {
            [
                u64::from(s.sweeps),
                s.moves,
                s.parallel_rounds,
                s.serial_rounds,
            ]
        })
        .collect()
}

/// Compares the result on a view of a graph given as a mask (`mask[v]`: kept) with the
/// result on the same view given as other lists, where `to[v]` is the id of kept node
/// `v` there, for each of [`variants`]: the same communities of the kept nodes, the
/// same modularity and the same work on every level.
fn compare(what: &str, masked: (&Lists, &[bool]), other: (&Lists, &[Option<u32>])) {
    let kept: Vec<usize> = (0..masked.1.len()).filter(|&v| masked.1[v]).collect();
    for (variant, o) in variants() {
        let (a, b) = (run(masked.0, &o), run(other.0, &o));
        let left = canonical(kept.iter().map(|&v| a.membership[v]));
        let right = canonical(
            kept.iter()
                .map(|&v| b.membership[other.1[v].unwrap() as usize]),
        );
        let what = format!("{what}, {variant}");
        assert!(
            left == right,
            "{what}: {} of {} kept nodes moved",
            moved(&left, &right),
            kept.len()
        );
        assert_eq!(a.modularity.to_bits(), b.modularity.to_bits(), "{what}");
        assert_eq!(work(&a), work(&b), "{what}: sweeps, moves and sub-rounds per level");
    }
}

#[test]
fn a_view_as_a_mask_and_materialized_give_the_same_communities() {
    let g = sensitive();
    let n = g.nodes();
    let keep: Vec<bool> = (0..n).map(|v| v % 5 != 2).collect();
    let mut rank = 0;
    let to: Vec<Option<u32>> = keep
        .iter()
        .map(|&k| {
            k.then(|| {
                rank += 1;
                rank - 1
            })
        })
        .collect();
    let same_ids: Vec<Option<u32>> = (0..n).map(|v| keep[v as usize].then_some(v)).collect();
    let masked = g.renamed(n, &same_ids);
    let materialized = g.renamed(rank, &to);
    compare(
        "mask vs materialized",
        (&masked, &keep),
        (&materialized, &to),
    );
}

/// The graph padded with nodes without edges to 327,679 and to 327,680 nodes: `n /
/// 65,536` is 4 and then 5, so a rule on `n` would run four and then eight sub-rounds.
/// On its 10,000 nodes with edges, Leiden runs four, as on the graph alone.
#[test]
fn nodes_without_edges_change_neither_the_sub_rounds_nor_the_communities() {
    let g = sensitive();
    let k = g.nodes();
    let to: Vec<Option<u32>> = (0..k).map(Some).collect();
    for n in [5 * 65_536 - 1, 5 * 65_536] {
        let keep: Vec<bool> = (0..n).map(|v| v < k).collect();
        compare(
            &format!("{k} nodes in {n}"),
            (&g.renamed(n, &to), &keep),
            (&g, &to),
        );
    }
}

/// `f` on the undirected adjacency of the graph file `bytes`.
fn in_file<R>(bytes: &[u8], f: impl FnOnce(&Compressed<'_>) -> R) -> R {
    let held = MemorySource::new(bytes);
    let file = Container::open(&held).unwrap();
    let part = |name: &str| {
        let e = file.get(name).unwrap();
        file.source().slice(e.offset..e.offset + e.len).unwrap()
    };
    let index = AdjacencyIndex::parse(part("adjacency.out.index")).unwrap();
    f(&Compressed::new(
        Adjacency::new(&index, part("adjacency.out.data")).unwrap(),
    ))
}

/// The graph built into a graph file, its hierarchy stored twice (the first copy is
/// replaced), and `compact`: the copy gives the communities of the file.
#[test]
fn compact_keeps_the_communities() {
    let mut text = String::new();
    for (u, v, _) in sensitive().pairs() {
        writeln!(text, "{u}\t{v}").unwrap();
    }
    let reader =
        TextReader::open(MemorySource::new(text.as_bytes()), &TextOptions::default()).unwrap();
    let graph = GraphOptions {
        directed: false,
        self_loops: SelfLoops::Drop,
        duplicates: Duplicates::Collapse,
    };
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("leiden-views");
    std::fs::create_dir_all(&dir).unwrap();
    let scratch = JobScratch::create(&dir, &format!("compact-{}", std::process::id())).unwrap();
    let memory = Memory::unlimited();
    let about = Provenance {
        engine: "everygraph determinism suite".into(),
        ..Provenance::default()
    };
    let built = native::write(
        &reader,
        &BuildOptions::new(graph, NonZeroUsize::new(2).unwrap()),
        &memory,
        &scratch,
        Output::Memory,
        about.clone(),
        &Silent,
        &|| true,
    );
    let Ok((Written::Memory(original), _, _)) = built else {
        panic!("the build: {:?}", built.err())
    };
    let o = Options::default();
    let first = in_file(&original, |s| communities(s, Settings::default(), &o));
    let mut bytes = original.clone();
    for _ in 0..2 {
        let mut w = Writer::update(MemorySource::new(bytes)).unwrap();
        let key = hierarchy::key(w.graph_identity(), &o).unwrap();
        in_file(&original, |s| {
            hierarchy::store(&mut w, &key, &about, s, &memory, Settings::default(), &first)
        })
        .unwrap();
        w.commit().unwrap();
        bytes = w.into_sink().into_inner();
    }
    let held = MemorySource::new(&bytes[..]);
    let copy = compact(&Container::open(&held).unwrap(), MemorySource::new(Vec::new()))
        .unwrap()
        .into_inner();
    assert!(copy.len() < bytes.len(), "compact reclaimed nothing");
    let after = in_file(&copy, |s| communities(s, Settings::default(), &o));
    assert_eq!(communities_hash(&first), communities_hash(&after));
}
