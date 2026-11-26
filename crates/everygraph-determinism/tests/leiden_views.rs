// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Three cases where DESIGN D14 may not hold for Leiden (#54; the state-of-the-art
//! review of 2026-11-26; issue m4-15-determinism-contract decides the promise):
//!
//! - a view as a mask (the graph's ids, the other nodes without edges) and the same
//!   view materialized (its nodes renumbered in order);
//! - a mask that keeps a prefix of the ids, where the two node counts fall on either
//!   side of a boundary of Leiden's sub-round rule `clamp(next_pow2(n / 65,536), 4,
//!   1,024)`;
//! - the ids renumbered by a locality order, as `compact --order` would store them
//!   (m2-24: the order of the Leiden hierarchy), with the result read back in the old
//!   ids.
//!
//! Each compares the communities of the view's nodes, numbered in the order of their
//! smallest node. The graph is one whose communities change with the seed, so a pass
//! is not an accident of a graph with one clear answer.

#![forbid(unsafe_code)]
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::print_stderr,
    clippy::cast_possible_truncation
)]

use std::collections::HashMap;

use everygraph_community::Options;
use everygraph_determinism::{Lists, SplitMix, communities, on, planted};
use everygraph_kernels::edge_map::{Csr, Settings};

/// Leiden's communities of `lists`, default options and `seed`, at 3 threads.
fn membership(lists: &Lists, seed: u64) -> (Vec<u32>, f64) {
    let o = Options {
        seed,
        ..Options::default()
    };
    let csr = Csr::new(&lists.offsets, &lists.targets).unwrap();
    let c = on(3, || communities(&csr, Settings::default(), &o));
    (c.membership.to_vec(), c.modularity)
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
    let g = planted(40, 250, 6, 5, 11);
    assert_ne!(
        membership(&g, 1).0,
        membership(&g, 2).0,
        "the communities do not depend on the seed: the probe would prove nothing"
    );
    g
}

/// Compares the communities of a view of `g` given as a mask (`mask[v]`: kept) with
/// those of the same view given as `lists` over `n` nodes, where `to[v]` is the id of
/// kept node `v` there, and reports both.
fn compare(what: &str, masked: (&Lists, &[bool]), other: (&Lists, &[Option<u32>])) -> bool {
    let (a, qa) = membership(masked.0, 1);
    let (b, qb) = membership(other.0, 1);
    let kept: Vec<usize> = (0..a.len()).filter(|&v| masked.1[v]).collect();
    let left = canonical(kept.iter().map(|&v| a[v]));
    let right = canonical(kept.iter().map(|&v| b[other.1[v].unwrap() as usize]));
    let same = left == right;
    eprintln!(
        "{what}: {} kept nodes, same communities: {same}, {} nodes moved, modularity {qa:.6} \
         as a mask and {qb:.6} the other way",
        kept.len(),
        moved(&left, &right)
    );
    same
}

#[test]
#[ignore = "fails: the masked and the materialized view give other communities (#54 finding \
            1; Leiden draws sub-rounds and ties from stored ids; m4-15 decides)"]
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
    assert!(compare(
        "mask vs materialized",
        (&masked, &keep),
        (&materialized, &to)
    ));
}

/// A prefix of `k` nodes of `g` padded with nodes without edges to `n` nodes, against
/// the prefix alone.
fn padded_prefix(n: u32) -> bool {
    let g = sensitive();
    let k = g.nodes();
    let keep: Vec<bool> = (0..n).map(|v| v < k).collect();
    let to: Vec<Option<u32>> = (0..n).map(|v| (v < k).then_some(v)).collect();
    let padded = g.renamed(n, &to[..k as usize]);
    compare(
        &format!("prefix of {k} in {n} nodes"),
        (&padded, &keep),
        (&g, &to),
    )
}

/// 327,679 nodes: `n / 65,536 = 4`, four sub-rounds, as for the 10,000 nodes alone.
#[test]
fn nodes_without_edges_below_a_sub_round_boundary_change_nothing() {
    assert!(padded_prefix(5 * 65_536 - 1));
}

/// 327,680 nodes: `n / 65,536 = 5`, eight sub-rounds where the prefix alone has four.
#[test]
#[ignore = "fails: past n = 327,680 Leiden runs eight sub-rounds, not four, and the \
            prefix's communities change (#54 finding 2; m4-15 decides)"]
fn nodes_without_edges_past_a_sub_round_boundary_change_nothing() {
    assert!(padded_prefix(5 * 65_536));
}

/// The order of the communities of a Leiden run with `seed`, ids in order inside each.
fn hierarchy_order(g: &Lists, seed: u64) -> Vec<u32> {
    let (c, _) = membership(g, seed);
    let mut order: Vec<u32> = (0..g.nodes()).collect();
    order.sort_by_key(|&v| (c[v as usize], v));
    order
}

/// Five orders: the hierarchy orders of the runs with seeds 0 to 3, and a random one.
#[test]
#[ignore = "fails: Leiden on the ids renumbered by a locality order gives other communities \
            (#54 finding 3; `compact --order` would change the result; m4-15 decides)"]
fn a_locality_order_of_the_ids_gives_the_same_communities() {
    let g = sensitive();
    let n = g.nodes();
    let mut rng = SplitMix(5);
    let mut random: Vec<u32> = (0..n).collect();
    for i in (1..random.len()).rev() {
        random.swap(i, rng.below(i as u64 + 1) as usize);
    }
    let orders = (0..4).map(|seed| hierarchy_order(&g, seed)).chain([random]);
    let all = vec![true; n as usize];
    let mut same = 0;
    for (k, order) in orders.enumerate() {
        let mut to = vec![None; n as usize];
        for (new, &old) in order.iter().enumerate() {
            to[old as usize] = Some(new as u32);
        }
        let renumbered = g.renamed(n, &to);
        same += usize::from(compare(
            &format!("order {k}"),
            (&g, &all),
            (&renumbered, &to),
        ));
    }
    assert_eq!(same, 5, "orders that keep the communities");
}
