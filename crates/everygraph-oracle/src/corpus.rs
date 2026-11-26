// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The corpus: a fixed list of generated graphs, each with the kernels to check on it.
//!
//! Two tiers. `ci` (a few hundred small graphs, every family and every edge case) runs on
//! every pull request; `nightly` adds more seeds and larger graphs (up to ~10^5 rows).
//! The list is a function of the tier alone, so a failure is reproduced from its name.

use serde::{Deserialize, Serialize};

use crate::generators;
use crate::graph::{Duplicates, Graph, SelfLoops, Semantics};
use crate::kernel::Kernel;
use crate::projection::Projection;
use crate::rng::SplitMix64;
use generators::{Decorations, Family, GraphSpec, Weights};

/// A corpus tier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// Every pull request.
    Ci,
    /// Every night: more seeds, larger graphs.
    Nightly,
}

/// The chunk size, in adjacency entries, that the adversarial families aim at and that
/// chunked configurations of a kernel should use (see [`crate::plugin::Config`]).
pub const ADVERSARIAL_CHUNK: u32 = 64;

/// A graph with the kernels to check on it.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// The graph.
    pub graph: Graph,
    /// The kernels.
    pub kernels: Vec<Kernel>,
}

fn plain(name: String, family: Family, directed: bool, seed: u64) -> GraphSpec {
    GraphSpec {
        name,
        family,
        directed,
        decorations: Decorations::default(),
        semantics: Semantics::default(),
        seed,
    }
}

fn edge_cases() -> Vec<(Family, Decorations, SelfLoops)> {
    let none = Decorations::default;
    let with = |f: fn(&mut Decorations)| {
        let mut d = Decorations::default();
        f(&mut d);
        d
    };
    vec![
        (Family::Empty, none(), SelfLoops::Drop),
        (Family::Isolates { n: 1 }, none(), SelfLoops::Drop),
        (Family::Isolates { n: 2 }, none(), SelfLoops::Drop),
        (Family::SingleEdge, none(), SelfLoops::Drop),
        (
            Family::SingleEdge,
            with(|d| d.parallel = 3),
            SelfLoops::Drop,
        ),
        (
            Family::Isolates { n: 1 },
            with(|d| d.self_loops = 1),
            SelfLoops::Drop,
        ),
        (
            Family::Isolates { n: 1 },
            with(|d| d.self_loops = 1),
            SelfLoops::Keep,
        ),
        (
            Family::SingleEdge,
            with(|d| d.self_loops = 2),
            SelfLoops::Keep,
        ),
        (
            Family::Path { n: 3 },
            with(|d| d.isolates = 2),
            SelfLoops::Drop,
        ),
        (Family::Cycle { n: 3 }, none(), SelfLoops::Drop),
        (
            Family::Star { n: 5 },
            with(|d| d.weights = Weights::Dyadic),
            SelfLoops::Drop,
        ),
        (
            Family::Clique { n: 4 },
            with(|d| d.parallel = 6),
            SelfLoops::Drop,
        ),
    ]
}

fn structured() -> Vec<Family> {
    vec![
        Family::Path { n: 2 },
        Family::Path { n: 17 },
        Family::Cycle { n: 4 },
        Family::Cycle { n: 50 },
        Family::Star { n: 2 },
        Family::Star { n: 100 },
        Family::Clique { n: 3 },
        Family::Clique { n: 12 },
        Family::Grid { rows: 1, cols: 1 },
        Family::Grid { rows: 3, cols: 4 },
        Family::Grid { rows: 10, cols: 10 },
        Family::Tree { n: 30 },
        Family::Tree { n: 200 },
        Family::Union {
            parts: vec![
                Family::Clique { n: 5 },
                Family::Path { n: 4 },
                Family::Isolates { n: 3 },
                Family::Star { n: 6 },
            ],
        },
    ]
}

fn adversarial(chunk: u32) -> Vec<Family> {
    let mut v: Vec<Family> = [-1, 0, 1]
        .iter()
        .map(|&delta| Family::HubNearChunk { chunk, delta })
        .collect();
    // Lists of exactly 8, 16 and 64 entries align with a chunk of 64 entries; 7 and 63 do not.
    for (degree, copies) in [(8, 20), (16, 10), (64, 3), (7, 20), (63, 3)] {
        v.push(Family::BlockAligned { degree, copies });
    }
    v
}

fn random_family(rng: &mut SplitMix64, scale: u32) -> Family {
    let n = 2 + rng.below_u32(60 * scale);
    match rng.below(8) {
        0 => Family::Gnm {
            n,
            m: rng.below_u32(3 * n + 1),
        },
        1 => Family::Gnp {
            n: 2 + rng.below_u32(40),
            permille: 300 + rng.below_u32(700),
        },
        2 => Family::PowerLaw {
            n,
            m: 1 + rng.below_u32(4 * n),
            exponent_tenths: 19 + rng.below_u32(12),
        },
        3 => Family::Planted {
            blocks: 2 + rng.below_u32(5),
            size: 3 + rng.below_u32(25),
            p_in_permille: 300 + rng.below_u32(500),
            p_out_permille: rng.below_u32(60),
        },
        4 => Family::Tree { n },
        5 => Family::Grid {
            rows: 1 + rng.below_u32(12),
            cols: 1 + rng.below_u32(12),
        },
        6 => Family::Union {
            parts: vec![
                Family::Gnm { n: n / 2 + 1, m: n },
                Family::Cycle {
                    n: 3 + rng.below_u32(10),
                },
                Family::Isolates {
                    n: rng.below_u32(4),
                },
                Family::Clique {
                    n: 2 + rng.below_u32(6),
                },
            ],
        },
        _ => Family::Star { n },
    }
}

fn random_decorations(rng: &mut SplitMix64, directed: bool) -> Decorations {
    Decorations {
        reciprocal_permille: if directed { rng.below_u32(600) } else { 0 },
        isolates: if rng.chance(400) { rng.below_u32(6) } else { 0 },
        self_loops: if rng.chance(400) { rng.below_u32(6) } else { 0 },
        parallel: if rng.chance(400) {
            rng.below_u32(20)
        } else {
            0
        },
        weights: match rng.below(3) {
            0 => Weights::None,
            1 => Weights::Dyadic,
            _ => Weights::Real,
        },
        timestamps: rng.chance(300),
        shuffle_ids: rng.chance(700),
        shuffle_rows: rng.chance(500),
    }
}

/// The specs of a tier, in a fixed order.
#[must_use]
pub fn specs(tier: Tier) -> Vec<GraphSpec> {
    let mut out = Vec::new();
    let mut push = |family: Family,
                    directed: bool,
                    decorations: Decorations,
                    semantics: Semantics,
                    seed: u64| {
        let tag = serde_json::to_value(&family)
            .ok()
            .and_then(|v| v.get("family").and_then(|f| f.as_str().map(str::to_owned)))
            .unwrap_or_default();
        let name = format!(
            "{:04}-{tag}-{}-s{seed}",
            out.len(),
            if directed { "d" } else { "u" }
        );
        let mut s = plain(name, family, directed, seed);
        s.decorations = decorations;
        s.semantics = semantics;
        out.push(s);
    };
    for directed in [false, true] {
        for (i, (f, d, l)) in edge_cases().into_iter().enumerate() {
            push(f, directed, d, loops(l), i as u64);
        }
        for (i, f) in structured().into_iter().enumerate() {
            push(
                f,
                directed,
                Decorations::default(),
                Semantics::default(),
                100 + i as u64,
            );
        }
        for (i, f) in adversarial(ADVERSARIAL_CHUNK).into_iter().enumerate() {
            push(
                f,
                directed,
                Decorations::default(),
                Semantics::default(),
                200 + i as u64,
            );
        }
    }
    let (count, scale) = match tier {
        Tier::Ci => (220, 1),
        Tier::Nightly => (1500, 8),
    };
    if tier == Tier::Nightly {
        for f in adversarial(1 << 16).into_iter().take(3) {
            push(f, false, Decorations::default(), Semantics::default(), 300);
        }
        push(
            Family::PowerLaw {
                n: 20_000,
                m: 100_000,
                exponent_tenths: 21,
            },
            false,
            Decorations::default(),
            Semantics::default(),
            301,
        );
    }
    let mut rng = SplitMix64::new(0x0EAC_1E00);
    for i in 0..count {
        let seed = 1000 + i;
        let mut r = SplitMix64::new(rng.next_u64());
        let directed = r.chance(450);
        let family = random_family(&mut r, scale);
        let decorations = random_decorations(&mut r, directed);
        let mut semantics = loops(if r.chance(100) {
            SelfLoops::Keep
        } else {
            SelfLoops::Drop
        });
        // Drawn last, so that adding it changed no graph above.
        semantics.duplicates = match r.below(4) {
            0 => Duplicates::Collapse,
            1 => Duplicates::Sum,
            _ => Duplicates::Events,
        };
        push(family, directed, decorations, semantics, seed);
    }
    // Sparse ids last, so that adding them did not rename the graphs above. Gaps cross
    // 2^7, 2^14 and 2^21 (4-byte LEB128 codes); 2^28 would need n > 2^28, whose O(n)
    // reference arrays do not fit the memory limit.
    let sparse: &[(u32, bool)] = match tier {
        // CI: the smallest n whose gaps still cross 2^21 (cost: see PR #131).
        Tier::Ci => &[((1 << 21) + 256, false)],
        Tier::Nightly => &[
            (1 << 22, false),
            (1 << 22, true),
            ((1 << 23) + 3, false),
            (1 << 24, false),
        ],
    };
    for (i, &(n, directed)) in sparse.iter().enumerate() {
        push(
            Family::SparseIds { n, extra: 32 },
            directed,
            Decorations::default(),
            Semantics::default(),
            400 + i as u64,
        );
    }
    // Kept loops and every `duplicates` mode on weighted graphs with repeated rows; last,
    // so that adding them renamed no graph above.
    for directed in [false, true] {
        for (i, (f, d, sem)) in semantic_cases().into_iter().enumerate() {
            push(f, directed, d, sem, 500 + i as u64);
        }
    }
    out
}

fn loops(self_loops: SelfLoops) -> Semantics {
    Semantics {
        self_loops,
        ..Semantics::default()
    }
}

fn semantic_cases() -> Vec<(Family, Decorations, Semantics)> {
    let mut out = Vec::new();
    for duplicates in [Duplicates::Events, Duplicates::Collapse, Duplicates::Sum] {
        for self_loops in [SelfLoops::Drop, SelfLoops::Keep] {
            let decorations = Decorations {
                self_loops: 4,
                parallel: 8,
                weights: Weights::Dyadic,
                ..Decorations::default()
            };
            let family = Family::Planted {
                blocks: 3,
                size: 6,
                p_in_permille: 700,
                p_out_permille: 100,
            };
            out.push((
                family,
                decorations,
                Semantics {
                    self_loops,
                    duplicates,
                },
            ));
        }
    }
    out
}

/// The kernels the oracle checks on `g`: every kernel, whatever the semantics
/// (`docs/spec/graph-semantics.md` § 8).
#[must_use]
pub fn kernels_for(g: &Graph, p: &Projection) -> Vec<Kernel> {
    let mut k = vec![Kernel::IngestReport, Kernel::Adjacency, Kernel::Degree];
    if g.directed {
        k.extend([Kernel::DegreeOut, Kernel::DegreeIn]);
    }
    k.push(Kernel::Wcc);
    if g.directed {
        k.push(Kernel::Scc);
    }
    if g.n > 0 {
        k.push(Kernel::Bfs { source: 0 });
        if let Some(hub) = crate::compare::max_degree_node(&p.out.offsets) {
            if hub != 0 {
                k.push(Kernel::Bfs { source: hub });
            }
        }
    }
    if g.n > PYTHON_MAX_NODES {
        // Sparse-id graphs (millions of ids, few edges) test id widths and codecs; the
        // per-node kernels above are enough, and they are checked in Rust only.
        return k;
    }
    k.extend([Kernel::Triangles, Kernel::TriangleTotal]);
    k.push(Kernel::PageRank {
        damping: 0.85,
        weighted: false,
    });
    // Weighted kernels where a weight differs from 1: a weight column, repeated rows or a
    // loop of several rows.
    let weighted = g.weight.is_some()
        || p.counts.duplicate_rows > 0
        || (p.keep_loops && p.loops.iter().any(|&r| r > 1));
    if weighted {
        k.push(Kernel::PageRank {
            damping: 0.85,
            weighted: true,
        });
    }
    if g.n % 5 == 1 {
        k.push(Kernel::PageRank {
            damping: 0.5,
            weighted: false,
        });
    }
    if g.weight.is_some() && g.n > 0 {
        k.push(Kernel::Sssp { source: 0 });
    }
    k.extend([Kernel::CoreNumber, Kernel::Clustering, Kernel::Transitivity]);
    if g.partition.is_some() {
        k.push(Kernel::Modularity {
            resolution: 1.0,
            weighted: false,
        });
        if weighted {
            k.push(Kernel::Modularity {
                resolution: 1.0,
                weighted: true,
            });
        }
        k.push(Kernel::Quotient);
    }
    k
}

/// Graphs with more nodes than this are checked by the in-process references and the
/// invariants only: their per-node answers would make `reference.json` hundreds of MB.
pub const PYTHON_MAX_NODES: u32 = 1 << 20;

/// Generates the graphs of a tier with their kernels.
#[must_use]
pub fn entries(tier: Tier) -> Vec<Entry> {
    specs(tier)
        .iter()
        .map(|s| {
            let graph = generators::generate(s);
            let p = Projection::new(&graph);
            let kernels = kernels_for(&graph, &p);
            Entry { graph, kernels }
        })
        .collect()
}
