// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `proptest` strategies over [`GraphSpec`]s, for property-based tests in any crate.
//!
//! A strategy produces a spec (family parameters, decorations, seed), not raw edges, so a
//! failing case shrinks towards a smaller family and fewer decorations, and the reported
//! value is a spec that [`crate::generators::generate`] reproduces exactly.
//!
//! ```
//! use everygraph_oracle::strategies::arb_graph;
//! use proptest::test_runner::{Config, TestRunner};
//!
//! let mut runner = TestRunner::new(Config::with_cases(16));
//! runner
//!     .run(&arb_graph(40), |g| {
//!         assert!(g.validate().is_ok());
//!         Ok(())
//!     })
//!     .expect("every generated graph is valid");
//! ```

use proptest::prelude::*;

use crate::generators::{Decorations, Family, GraphSpec, Weights, generate};
use crate::graph::{Graph, SelfLoops, Semantics};

/// A family with at most about `max_n` nodes.
pub fn arb_family(max_n: u32) -> impl Strategy<Value = Family> {
    let max_n = max_n.max(2);
    let leaf = prop_oneof![
        Just(Family::Empty),
        (0..=max_n.min(4)).prop_map(|n| Family::Isolates { n }),
        Just(Family::SingleEdge),
        (1..=max_n).prop_map(|n| Family::Path { n }),
        (3..=max_n.max(3)).prop_map(|n| Family::Cycle { n }),
        (1..=max_n).prop_map(|n| Family::Star { n }),
        (1..=max_n.min(16)).prop_map(|n| Family::Clique { n }),
        (1..=8u32, 1..=8u32).prop_map(|(rows, cols)| Family::Grid { rows, cols }),
        (1..=max_n).prop_map(|n| Family::Tree { n }),
        (2..=max_n, 0..=3 * max_n).prop_map(|(n, m)| Family::Gnm { n, m }),
        (2..=max_n.min(30), 0..=1000u32).prop_map(|(n, permille)| Family::Gnp { n, permille }),
        (2..=max_n, 1..=4 * max_n, 15..=30u32).prop_map(|(n, m, exponent_tenths)| {
            Family::PowerLaw {
                n,
                m,
                exponent_tenths,
            }
        }),
        (2..=4u32, 2..=8u32, 200..=900u32, 0..=100u32).prop_map(
            |(blocks, size, p_in_permille, p_out_permille)| {
                Family::Planted {
                    blocks,
                    size,
                    p_in_permille,
                    p_out_permille,
                }
            }
        ),
        (prop_oneof![Just(4u32), Just(8), Just(16)], -1..=1i32)
            .prop_map(|(chunk, delta)| Family::HubNearChunk { chunk, delta }),
        (1..=8u32, 1..=4u32).prop_map(|(degree, copies)| Family::BlockAligned { degree, copies }),
    ];
    leaf.prop_recursive(1, 4, 4, |inner| {
        prop::collection::vec(inner, 1..4).prop_map(|parts| Family::Union { parts })
    })
}

/// Decorations, all of them small.
pub fn arb_decorations(directed: bool) -> impl Strategy<Value = Decorations> {
    (
        0..=if directed { 800u32 } else { 0 },
        0..=4u32,
        0..=4u32,
        0..=8u32,
        prop_oneof![
            Just(Weights::None),
            Just(Weights::Dyadic),
            Just(Weights::Real)
        ],
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
    )
        .prop_map(
            |(
                reciprocal_permille,
                isolates,
                self_loops,
                parallel,
                weights,
                timestamps,
                shuffle_ids,
                shuffle_rows,
            )| {
                Decorations {
                    reciprocal_permille,
                    isolates,
                    self_loops,
                    parallel,
                    weights,
                    timestamps,
                    shuffle_ids,
                    shuffle_rows,
                }
            },
        )
}

/// A spec, directed or not, with the default semantics or kept self-loops.
pub fn arb_spec(max_n: u32) -> impl Strategy<Value = GraphSpec> {
    (any::<bool>(), any::<u64>(), prop::bool::weighted(0.15)).prop_flat_map(
        move |(directed, seed, keep)| {
            (arb_family(max_n), arb_decorations(directed)).prop_map(move |(family, decorations)| {
                GraphSpec {
                    name: format!("prop-s{seed}"),
                    family,
                    directed,
                    decorations,
                    semantics: Semantics {
                        self_loops: if keep {
                            SelfLoops::Keep
                        } else {
                            SelfLoops::Drop
                        },
                    },
                    seed,
                }
            })
        },
    )
}

/// A generated graph.
pub fn arb_graph(max_n: u32) -> impl Strategy<Value = Graph> {
    arb_spec(max_n).prop_map(|s| generate(&s))
}

/// An undirected generated graph with the default semantics.
pub fn arb_undirected_graph(max_n: u32) -> impl Strategy<Value = Graph> {
    arb_spec(max_n).prop_map(|mut s| {
        s.directed = false;
        s.decorations.reciprocal_permille = 0;
        s.semantics = Semantics::default();
        generate(&s)
    })
}
