// SPDX-License-Identifier: Apache-2.0 OR MIT

//! What DESIGN D14 promises for a view (ADR 0014, "Reproducibility"): every kernel of
//! [`kernels`] and Leiden give the same bits on a view run as a mask and run
//! materialized, at every configuration of the grid, over every kind of parent source.
//!
//! The views are those of `everygraph_query::views`: a node view (labels and a node
//! set), a pair view (a weight threshold) and an event view (a time window, with the
//! pair weights summed again from the events it keeps), on a planted graph with weights
//! whose sums depend on their order, and a node view of graphs of the CI corpus.

#![forbid(unsafe_code)]
#![allow(clippy::disallowed_methods, clippy::cast_possible_truncation)]

use std::path::PathBuf;

use everygraph_community::Options;
use everygraph_core::Memory;
use everygraph_determinism::{
    Lists, SplitMix, Stored, communities_hash, corpus, full, grid, kernels, planted, same,
    weighted_communities, with_source,
};
use everygraph_kernels::edge_map::{Settings, Source, View};
use everygraph_kernels::planner::ViewForm;
use everygraph_query::{Filtered, Predicate, Test, Values, ViewSpec};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("determinism-views")
}

/// The hashes of every kernel of [`kernels`], then of Leiden, on `view` with the
/// per-entry `weights`.
fn hashes<S: Source>(
    view: &View<'_, S>,
    settings: Settings,
    weights: &[f64],
) -> Vec<(&'static str, String)> {
    let mut out = kernels(view, settings, weights);
    if view.nodes() > 0 {
        let c = weighted_communities(view, weights, settings, &Options::default());
        out.push(("leiden", communities_hash(&c)));
    }
    out
}

/// The hashes on the view as a mask and materialized, which must be equal.
fn both<S: Source>(
    f: &mut Filtered<'_, '_, S>,
    settings: Settings,
    weights: impl Fn(&Filtered<'_, '_, S>) -> Vec<f64>,
) -> Vec<(&'static str, String)> {
    let memory = Memory::unlimited();
    let w = weights(f);
    assert_eq!(f.hold(1, &memory).unwrap(), ViewForm::Mask);
    let mask = hashes(f.view(), settings, &w);
    assert_eq!(f.hold(u32::MAX, &memory).unwrap(), ViewForm::Materialized);
    assert_eq!(weights(f), w, "the view's weights");
    let materialized = hashes(f.view(), settings, &w);
    assert_eq!(mask, materialized, "mask against materialized");
    mask
}

/// Events of an undirected graph: one or two per pair, the same at both ends, with a
/// time and a weight drawn from the pair. Returns the event offsets, times and weights.
fn events(g: &Lists) -> (Vec<u64>, Vec<i64>, Vec<f64>) {
    let (mut offsets, mut times, mut weights) = (vec![0], Vec::new(), Vec::new());
    for u in 0..g.nodes() {
        for e in g.offsets[u as usize]..g.offsets[u as usize + 1] {
            let v = g.targets[e as usize];
            let mut rng = SplitMix(u64::from(u.min(v)) << 32 | u64::from(u.max(v)));
            for _ in 0..=rng.below(2) {
                times.push(rng.below(100).cast_signed());
                weights.push(rng.wide_weight());
            }
            offsets.push(times.len() as u64);
        }
    }
    (offsets, times, weights)
}

/// Node, pair and event views of a planted graph of 10,000 nodes.
#[test]
fn every_kernel_gives_the_same_bits_on_a_view_as_a_mask_and_materialized() {
    let g = planted(40, 250, 2, 2, 11);
    let n = g.nodes();
    let labels: Vec<u32> = (0..n).map(|v| v % 5).collect();
    let set: Vec<u32> = (0..n).filter(|v| v % 11 != 4).collect();
    let (offsets, times, event_weights) = events(&g);
    let stored = Stored::new(g, &dir());
    let weights = &stored.lists.weights;
    let node = ViewSpec::new()
        .nodes(Predicate::new(Values::U32(&labels), Test::In(&[0, 2, 3])))
        .node_set(&set);
    let pair = ViewSpec::new().pairs(Predicate::new(Values::F64(weights), Test::Lt(1.0.into())));
    let window = ViewSpec::new().events(
        &offsets,
        Predicate::new(
            Values::I64(&times),
            Test::Between(20_i64.into(), 59_i64.into()),
        ),
    );
    let cases = [
        ("node view", node),
        ("pair view", pair),
        ("event view", window.and(ViewSpec::new().node_set(&set))),
    ];
    for (turn, (what, spec)) in cases.iter().enumerate() {
        let events = *what == "event view";
        let first = same(what, &grid(turn), |c| {
            with_source!(stored.source(c.kind), |s| {
                let mut f = spec.build(&s, &Memory::unlimited(), c.settings()).unwrap();
                both(&mut f, c.settings(), |f| {
                    let memory = Memory::unlimited();
                    let w = if events {
                        f.event_sums(&offsets, &event_weights, &memory)
                    } else {
                        f.pair_values(weights, &memory)
                    };
                    w.unwrap().as_slice().unwrap().to_vec()
                })
            })
        });
        assert!(first.len() > 10, "{what}: {first:?}");
    }
}

/// A node view of every 12th graph of the CI corpus (every third with [`full`]), unit
/// weights: loops, isolated nodes and the shapes of the corpus. Graphs of more than
/// 2^20 nodes are left out: the view's node set lists its ids, 4 bytes each, and the
/// sparse ids of the corpus run past 2^28.
#[test]
fn views_of_the_corpus_give_the_same_bits_as_a_mask_and_materialized() {
    let step = if full() { 3 } else { 12 };
    let graphs = corpus(step)
        .into_iter()
        .filter(|(_, g)| g.nodes() <= 1 << 20);
    for (turn, (name, g)) in graphs.enumerate() {
        let keep: Vec<u32> = (0..g.nodes()).filter(|v| v % 4 != 1).collect();
        let stored = Stored::new(g, &dir());
        let spec = ViewSpec::new().node_set(&keep);
        same(&name, &grid(turn), |c| {
            with_source!(stored.source(c.kind), |s| {
                let mut f = spec.build(&s, &Memory::unlimited(), c.settings()).unwrap();
                both(&mut f, c.settings(), |f| {
                    let w = f.pair_values(&stored.lists.weights, &Memory::unlimited());
                    w.unwrap().as_slice().unwrap().to_vec()
                })
            })
        });
    }
}
