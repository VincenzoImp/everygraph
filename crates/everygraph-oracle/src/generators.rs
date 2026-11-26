// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Graph generators, deterministic from a seed.
//!
//! A [`GraphSpec`] is a family, a set of decorations (self-loops, parallel edges,
//! isolates, weights, timestamps, id shuffling, reciprocal arcs) and a seed; [`generate`]
//! turns it into a [`Graph`]. The same spec gives the same graph on every platform and in
//! every future version (the generator is [`SplitMix64`]; changing a family's algorithm
//! is a breaking change of the corpus and must regenerate the golden files).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::graph::{Graph, Semantics};
use crate::ix;
use crate::rng::SplitMix64;

/// A family of graphs. Sizes are node counts unless stated.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "family", rename_all = "snake_case")]
pub enum Family {
    /// No nodes at all.
    Empty,
    /// `n` nodes, no edges (`n = 1` is the single-node graph).
    Isolates {
        /// Nodes.
        n: u32,
    },
    /// Two nodes and one edge.
    SingleEdge,
    /// `0 - 1 - ... - (n-1)`.
    Path {
        /// Nodes.
        n: u32,
    },
    /// A path closed into a ring (`n >= 3`).
    Cycle {
        /// Nodes.
        n: u32,
    },
    /// Node 0 joined to all others.
    Star {
        /// Nodes, hub included.
        n: u32,
    },
    /// Every pair joined.
    Clique {
        /// Nodes.
        n: u32,
    },
    /// A `rows × cols` lattice with 4-neighbourhoods.
    Grid {
        /// Rows.
        rows: u32,
        /// Columns.
        cols: u32,
    },
    /// A random recursive tree: node `i > 0` hangs under a uniform node `< i`.
    Tree {
        /// Nodes.
        n: u32,
    },
    /// `m` distinct uniform pairs (fewer if the graph is too small to hold them).
    Gnm {
        /// Nodes.
        n: u32,
        /// Pairs.
        m: u32,
    },
    /// Each pair independently with probability `permille / 1000` (dense for large values).
    Gnp {
        /// Nodes.
        n: u32,
        /// Edge probability in thousandths.
        permille: u32,
    },
    /// Chung–Lu graph with expected degrees following a power law of the given exponent:
    /// `m` rows drawn with both endpoints proportional to `(i + 1)^(-1/(exponent - 1))`.
    /// Has hubs and, naturally, repeated pairs.
    PowerLaw {
        /// Nodes.
        n: u32,
        /// Rows.
        m: u32,
        /// Exponent of the degree distribution, in tenths (21 = 2.1).
        exponent_tenths: u32,
    },
    /// Planted partition: `blocks` blocks of `size` nodes; pairs inside a block with
    /// probability `p_in`, across with `p_out` (thousandths). The partition is the blocks.
    Planted {
        /// Number of blocks.
        blocks: u32,
        /// Nodes per block.
        size: u32,
        /// Inside probability in thousandths.
        p_in_permille: u32,
        /// Across probability in thousandths.
        p_out_permille: u32,
    },
    /// Adversarial for chunked code: a hub whose list has `chunk + delta` entries, its
    /// leaves joined by a sparse random graph.
    HubNearChunk {
        /// The chunk size in adjacency entries the hub is aimed at.
        chunk: u32,
        /// Offset of the hub's degree from the chunk size (−1, 0, +1 in the corpus).
        delta: i32,
    },
    /// Adversarial for chunked code: `copies` disjoint cliques on `degree + 1` nodes, so
    /// that every list has exactly `degree` entries and, when `degree` divides the chunk
    /// size, every chunk boundary falls exactly at the end of a list.
    BlockAligned {
        /// Length of every list.
        degree: u32,
        /// Number of cliques.
        copies: u32,
    },
    /// Adversarial for id widths and variable-length codes: `n` ids (millions), few
    /// edges. For every `k` in 7, 14, 21, 28 and every gap `2^k - 1`, `2^k`, `2^k + 1` that
    /// fits below `n`, one hub lists two neighbours exactly that gap apart; node 1 also
    /// lists node `n - 1`, and `extra` random pairs are spread over all ids. No partition.
    SparseIds {
        /// Nodes (ids), most of them isolated.
        n: u32,
        /// Random pairs added.
        extra: u32,
    },
    /// Disjoint union; node ids are concatenated in order. The partition is the parts.
    Union {
        /// The parts.
        parts: Vec<Family>,
    },
}

/// Weights attached to the rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Weights {
    /// No weight column.
    #[default]
    None,
    /// Multiples of 1/16 in `(0, 4]`: sums are exact in any order, so weighted results
    /// can be compared bit for bit.
    Dyadic,
    /// Arbitrary doubles in `(0.001, 10)`: sums depend on their order.
    Real,
}

/// Decorations applied after the family is generated, in this order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decorations {
    /// Directed graphs: thousandths of arcs that get a reverse arc.
    pub reciprocal_permille: u32,
    /// Extra nodes without edges.
    pub isolates: u32,
    /// Rows from a node to itself.
    pub self_loops: u32,
    /// Extra rows repeating a random existing row (reversed half of the time when
    /// undirected).
    pub parallel: u32,
    /// The weight column.
    pub weights: Weights,
    /// Add a timestamp column.
    pub timestamps: bool,
    /// Relabel the nodes by a random permutation, so structure is not aligned with ids.
    pub shuffle_ids: bool,
    /// Put the rows in random order.
    pub shuffle_rows: bool,
}

/// Everything that determines a generated graph.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GraphSpec {
    /// Name of the graph (unique in a corpus).
    pub name: String,
    /// The family.
    pub family: Family,
    /// Directed or undirected.
    pub directed: bool,
    /// Decorations.
    pub decorations: Decorations,
    /// Semantics the graph is analysed under.
    pub semantics: Semantics,
    /// The seed.
    pub seed: u64,
}

/// Undirected pairs (u != v), node count and the family's natural partition.
struct Skeleton {
    n: u32,
    pairs: Vec<(u32, u32)>,
    partition: Option<Vec<u32>>,
}

fn u32_of(x: usize) -> u32 {
    u32::try_from(x).expect("generated graphs have u32 node ids")
}

fn skeleton(f: &Family, rng: &mut SplitMix64) -> Skeleton {
    let mut pairs = Vec::new();
    let mut partition = None;
    let n = match *f {
        Family::Empty => 0,
        Family::Isolates { n } => n,
        Family::SingleEdge => {
            pairs.push((0, 1));
            2
        }
        Family::Path { n } => {
            pairs.extend((1..n).map(|i| (i - 1, i)));
            n
        }
        Family::Cycle { n } => {
            pairs.extend((1..n).map(|i| (i - 1, i)));
            if n >= 3 {
                pairs.push((n - 1, 0));
            }
            n
        }
        Family::Star { n } => {
            pairs.extend((1..n).map(|i| (0, i)));
            n
        }
        Family::Clique { n } => {
            for u in 0..n {
                pairs.extend((u + 1..n).map(|v| (u, v)));
            }
            n
        }
        Family::Grid { rows, cols } => {
            let id = |r: u32, c: u32| r * cols + c;
            for r in 0..rows {
                for c in 0..cols {
                    if c + 1 < cols {
                        pairs.push((id(r, c), id(r, c + 1)));
                    }
                    if r + 1 < rows {
                        pairs.push((id(r, c), id(r + 1, c)));
                    }
                }
            }
            rows * cols
        }
        Family::Tree { n } => {
            pairs.extend((1..n).map(|i| (rng.below_u32(i), i)));
            n
        }
        Family::Gnm { n, m } => {
            let possible = u64::from(n) * u64::from(n.saturating_sub(1)) / 2;
            let want = u64::from(m).min(possible);
            let mut seen = BTreeSet::new();
            let mut attempts = 0u64;
            while (seen.len() as u64) < want && attempts < 20 * want + 100 {
                attempts += 1;
                let (u, v) = (rng.below_u32(n), rng.below_u32(n));
                if u != v && seen.insert((u.min(v), u.max(v))) {
                    pairs.push((u, v));
                }
            }
            n
        }
        Family::Gnp { n, permille } => {
            for u in 0..n {
                for v in u + 1..n {
                    if rng.chance(permille) {
                        pairs.push((u, v));
                    }
                }
            }
            n
        }
        Family::PowerLaw {
            n,
            m,
            exponent_tenths,
        } => {
            if n >= 2 {
                let gamma = f64::from(exponent_tenths.max(11)) / 10.0;
                let mut cum = Vec::with_capacity(ix(n));
                let mut total = 0.0;
                for i in 0..n {
                    // (i + 1)^(-1 / (gamma - 1)) through the pinned libm, not `powf`: the
                    // platform's maths library would make the graph platform-dependent.
                    total += libm::exp(-libm::log(f64::from(i + 1)) / (gamma - 1.0));
                    cum.push(total);
                }
                let draw = |rng: &mut SplitMix64| {
                    let x = rng.unit() * total;
                    u32_of(cum.partition_point(|&c| c <= x).min(ix(n) - 1))
                };
                for _ in 0..m {
                    let u = draw(rng);
                    let mut v = draw(rng);
                    let mut tries = 0;
                    while v == u && tries < 8 {
                        v = draw(rng);
                        tries += 1;
                    }
                    if v != u {
                        pairs.push((u, v));
                    }
                }
            }
            n
        }
        Family::Planted {
            blocks,
            size,
            p_in_permille,
            p_out_permille,
        } => {
            let n = blocks * size;
            for u in 0..n {
                for v in u + 1..n {
                    let p = if u / size.max(1) == v / size.max(1) {
                        p_in_permille
                    } else {
                        p_out_permille
                    };
                    if rng.chance(p) {
                        pairs.push((u, v));
                    }
                }
            }
            partition = Some((0..n).map(|v| v / size.max(1)).collect());
            n
        }
        Family::HubNearChunk { chunk, delta } => {
            let leaves =
                u32::try_from((i64::from(chunk) + i64::from(delta)).max(0)).expect("leaf count");
            pairs.extend((1..=leaves).map(|i| (0, i)));
            for _ in 0..leaves {
                let (u, v) = (
                    1 + rng.below_u32(leaves.max(1)),
                    1 + rng.below_u32(leaves.max(1)),
                );
                if u != v && u <= leaves && v <= leaves {
                    pairs.push((u, v));
                }
            }
            leaves + 1
        }
        Family::BlockAligned { degree, copies } => {
            let k = degree + 1;
            for c in 0..copies {
                for u in 0..k {
                    pairs.extend((u + 1..k).map(|v| (c * k + u, c * k + v)));
                }
            }
            partition = Some((0..k * copies).map(|v| v / k).collect());
            k * copies
        }
        Family::SparseIds { n, extra } => {
            const BASE: u64 = 64; // above the hub ids
            let mut hub = 2u32;
            for k in [7u32, 14, 21, 28] {
                for gap in [(1u64 << k) - 1, 1u64 << k, (1u64 << k) + 1] {
                    if BASE + gap < u64::from(n) {
                        let far = u32::try_from(BASE + gap).expect("below n");
                        let base = u32::try_from(BASE).expect("small");
                        pairs.extend([(hub, base), (hub, far)]);
                        hub += 1;
                    }
                }
            }
            if n > 2 {
                pairs.push((1, n - 1));
            }
            for _ in 0..extra {
                let (u, v) = (rng.below_u32(n.max(1)), rng.below_u32(n.max(1)));
                if u != v {
                    pairs.push((u, v));
                }
            }
            n
        }
        Family::Union { ref parts } => {
            let mut base = 0u32;
            let mut labels = Vec::new();
            for (i, part) in parts.iter().enumerate() {
                let s = skeleton(part, rng);
                pairs.extend(s.pairs.iter().map(|&(u, v)| (u + base, v + base)));
                labels.extend(std::iter::repeat_n(u32_of(i), ix(s.n)));
                base += s.n;
            }
            partition = Some(labels);
            base
        }
    };
    Skeleton {
        n,
        pairs,
        partition,
    }
}

/// Families whose directed version is oriented low id → high id (so they stay acyclic,
/// except the cycle, which closes); all others get a random orientation per pair.
const fn oriented_forward(f: &Family) -> bool {
    matches!(
        f,
        Family::Path { .. } | Family::Cycle { .. } | Family::Star { .. } | Family::Tree { .. }
    )
}

/// The default partition when the family has none: `k = ⌈√n / 2⌉` contiguous blocks.
fn default_partition(n: u32) -> Vec<u32> {
    if n == 0 {
        return Vec::new();
    }
    let mut k = 1u32;
    while 4 * k * k < n {
        k += 1;
    }
    (0..n)
        .map(|v| u32::try_from(u64::from(v) * u64::from(k) / u64::from(n)).expect("block below k"))
        .collect()
}

/// Generates the graph described by `spec`.
///
/// # Panics
///
/// If the spec describes more than 2^32 nodes.
#[must_use]
pub fn generate(spec: &GraphSpec) -> Graph {
    let mut rng = SplitMix64::new(spec.seed);
    let s = skeleton(&spec.family, &mut rng);
    let d = &spec.decorations;
    let mut rows: Vec<(u32, u32)> = Vec::with_capacity(s.pairs.len());
    for &(u, v) in &s.pairs {
        let (u, v) = (u.min(v), u.max(v));
        let forward = !spec.directed || oriented_forward(&spec.family) || rng.chance(500);
        rows.push(if forward { (u, v) } else { (v, u) });
        if spec.directed && rng.chance(d.reciprocal_permille) {
            rows.push(if forward { (v, u) } else { (u, v) });
        }
    }
    let n = s.n + d.isolates;
    let sparse = matches!(spec.family, Family::SparseIds { .. });
    let mut partition = s.partition.unwrap_or_else(|| default_partition(s.n));
    partition.extend(std::iter::repeat_n(0, ix(d.isolates)));
    if n > 0 {
        for _ in 0..d.self_loops {
            let v = rng.below_u32(n);
            rows.push((v, v));
        }
    }
    if !rows.is_empty() {
        for _ in 0..d.parallel {
            let (u, v) = rows[ix(rng.below_u32(u32_of(rows.len())))];
            rows.push(if !spec.directed && rng.chance(500) {
                (v, u)
            } else {
                (u, v)
            });
        }
    }
    if d.shuffle_rows {
        for i in (1..rows.len()).rev() {
            rows.swap(i, ix(rng.below_u32(u32_of(i + 1))));
        }
    }
    if d.shuffle_ids && n > 1 {
        let mut perm: Vec<u32> = (0..n).collect();
        for i in (1..perm.len()).rev() {
            perm.swap(i, ix(rng.below_u32(u32_of(i + 1))));
        }
        for r in &mut rows {
            *r = (perm[ix(r.0)], perm[ix(r.1)]);
        }
        let mut moved = vec![0; ix(n)];
        for (old, &new) in perm.iter().enumerate() {
            moved[ix(new)] = partition[old];
        }
        partition = moved;
    }
    if !spec.directed && d.shuffle_ids {
        // Undirected rows carry no orientation: write some of them the other way round.
        for r in &mut rows {
            if rng.chance(500) {
                *r = (r.1, r.0);
            }
        }
    }
    let weight = match d.weights {
        Weights::None => None,
        Weights::Dyadic => Some(
            rows.iter()
                .map(|_| f64::from(1 + rng.below_u32(64)) / 16.0)
                .collect(),
        ),
        Weights::Real => Some(rows.iter().map(|_| 0.001 + rng.unit() * 10.0).collect()),
    };
    let time = d.timestamps.then(|| {
        rows.iter()
            .map(|_| i64::try_from(rng.below(1_000_000_000)).expect("below 1e9"))
            .collect()
    });
    Graph {
        name: spec.name.clone(),
        n,
        directed: spec.directed,
        src: rows.iter().map(|r| r.0).collect(),
        dst: rows.iter().map(|r| r.1).collect(),
        weight,
        time,
        partition: (!sparse).then_some(partition),
        semantics: spec.semantics,
        provenance: serde_json::json!({ "generator": "everygraph-oracle", "spec": spec }),
    }
}

#[cfg(test)]
mod tests {
    use super::{Decorations, Family, GraphSpec, Weights, generate};
    use crate::graph::Semantics;

    fn spec(family: Family, directed: bool, seed: u64) -> GraphSpec {
        GraphSpec {
            name: "t".into(),
            family,
            directed,
            decorations: Decorations {
                reciprocal_permille: 300,
                isolates: 3,
                self_loops: 2,
                parallel: 4,
                weights: Weights::Real,
                timestamps: true,
                shuffle_ids: true,
                shuffle_rows: true,
            },
            semantics: Semantics::default(),
            seed,
        }
    }

    #[test]
    fn same_seed_same_graph_other_seed_other_graph() {
        let f = Family::PowerLaw {
            n: 200,
            m: 800,
            exponent_tenths: 21,
        };
        assert_eq!(
            generate(&spec(f.clone(), true, 9)),
            generate(&spec(f.clone(), true, 9))
        );
        assert_ne!(
            generate(&spec(f.clone(), true, 9)).src,
            generate(&spec(f, true, 10)).src
        );
    }

    #[test]
    fn families_have_their_shape() {
        let g = generate(&spec(Family::Clique { n: 5 }, false, 1));
        assert_eq!(g.n, 8);
        assert_eq!(g.rows(), 10 + 2 + 4);
        g.validate().expect("valid");
        let g = generate(&spec(
            Family::BlockAligned {
                degree: 7,
                copies: 3,
            },
            false,
            1,
        ));
        assert_eq!(g.n, 24 + 3);
        let mut empty = spec(Family::Empty, false, 1);
        empty.decorations = Decorations::default();
        let g = generate(&empty);
        assert_eq!((g.n, g.rows()), (0, 0));
    }
}
