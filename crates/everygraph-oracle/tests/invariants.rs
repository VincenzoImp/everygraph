// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Every invariant can fail: for each one, a hand-made wrong output that it must reject,
//! next to the reference output, which no invariant may reject.

#![forbid(unsafe_code)]

use everygraph_oracle::graph::Graph;
use everygraph_oracle::invariants::check;
use everygraph_oracle::kernel::{Kernel, Output};
use everygraph_oracle::projection::Projection;
use everygraph_oracle::reference;

fn graph(n: u32, edges: &[(u32, u32)], directed: bool) -> Graph {
    let mut g = Graph::with_nodes("t", n, directed);
    for &(u, v) in edges {
        g.src.push(u);
        g.dst.push(v);
    }
    g
}

/// Triangle 0-1-2, pendant 2-3, isolated 4; partition {0,1}, {2,3,4}.
fn sample() -> Graph {
    let mut g = graph(5, &[(0, 1), (1, 2), (2, 0), (2, 3)], false);
    g.partition = Some(vec![0, 0, 1, 1, 1]);
    g
}

fn path() -> Graph {
    graph(4, &[(0, 1), (1, 2), (2, 3)], false)
}

/// The reference output of `kernel` on `g`, after checking that it violates nothing.
fn good(g: &Graph, kernel: &Kernel) -> Output {
    let p = Projection::new(g);
    let out = reference::compute(g, &p, kernel).expect("a reference answer");
    let v = check(kernel, &p, g.partition.as_deref(), &out);
    assert!(v.is_empty(), "the reference output violates {v:?}");
    out
}

/// Asserts that `wrong` violates `invariant` (other invariants may fire too).
fn rejects(g: &Graph, kernel: &Kernel, wrong: &Output, invariant: &str) {
    let p = Projection::new(g);
    let v = check(kernel, &p, g.partition.as_deref(), wrong);
    assert!(
        v.iter().any(|x| x.invariant == invariant),
        "{invariant} did not fire on {wrong:?}; fired: {v:?}"
    );
}

fn csr(g: &Graph) -> (Vec<u64>, Vec<u32>) {
    match good(g, &Kernel::Adjacency) {
        Output::Csr { offsets, targets } => (offsets, targets),
        other => panic!("{other:?}"),
    }
}

fn ints(o: Output) -> Vec<Option<u64>> {
    match o {
        Output::Ints(v) => v,
        other => panic!("{other:?}"),
    }
}

fn reals(o: Output) -> Vec<Option<f64>> {
    match o {
        Output::Reals(v) => v,
        other => panic!("{other:?}"),
    }
}

#[test]
fn shape() {
    let g = sample();
    rejects(&g, &Kernel::Degree, &Output::Ints(vec![Some(1)]), "shape");
}

#[test]
fn offsets_monotone() {
    let g = sample();
    let (mut o, t) = csr(&g);
    o.swap(1, 2);
    rejects(
        &g,
        &Kernel::Adjacency,
        &Output::Csr {
            offsets: o,
            targets: t,
        },
        "offsets_monotone",
    );
}

#[test]
fn targets_in_range() {
    let g = sample();
    let (o, mut t) = csr(&g);
    t[0] = 99;
    rejects(
        &g,
        &Kernel::Adjacency,
        &Output::Csr {
            offsets: o,
            targets: t,
        },
        "targets_in_range",
    );
}

#[test]
fn lists_sorted_unique() {
    let g = sample();
    let (o, mut t) = csr(&g);
    t.swap(0, 1); // node 0's list [1, 2] becomes [2, 1]
    rejects(
        &g,
        &Kernel::Adjacency,
        &Output::Csr {
            offsets: o,
            targets: t,
        },
        "lists_sorted_unique",
    );
}

#[test]
fn no_self_loops() {
    let g = sample();
    let (o, mut t) = csr(&g);
    t[0] = 0; // node 0 lists itself
    rejects(
        &g,
        &Kernel::Adjacency,
        &Output::Csr {
            offsets: o,
            targets: t,
        },
        "no_self_loops",
    );
}

#[test]
fn adjacency_symmetric() {
    let g = sample();
    // Node 3 lists 4 instead of 2: 3→4 has no 4→3, and 2→3 has no 3→2.
    let (o, mut t) = csr(&g);
    let last = t.len() - 1;
    t[last] = 4;
    rejects(
        &g,
        &Kernel::Adjacency,
        &Output::Csr {
            offsets: o,
            targets: t,
        },
        "adjacency_symmetric",
    );
}

#[test]
fn entries_equal_m() {
    let g = sample();
    // Drop the pair 2-3 in both directions: still symmetric and sorted, one pair short.
    let offsets = vec![0, 2, 4, 6, 6, 6];
    let targets = vec![1, 2, 0, 2, 0, 1];
    rejects(
        &g,
        &Kernel::Adjacency,
        &Output::Csr { offsets, targets },
        "entries_equal_m",
    );
}

#[test]
fn degree_sum() {
    let g = sample();
    let mut d = ints(good(&g, &Kernel::Degree));
    d[4] = Some(1);
    rejects(&g, &Kernel::Degree, &Output::Ints(d), "degree_sum");
}

const PR: Kernel = Kernel::PageRank {
    damping: 0.85,
    weighted: false,
};

#[test]
fn pagerank_nonnegative() {
    let g = sample();
    let mut r = reals(good(&g, &PR));
    let x = r[0].expect("value");
    r[0] = Some(-x);
    r[1] = r[1].map(|y| y + 2.0 * x); // keep the sum at 1
    rejects(&g, &PR, &Output::Reals(r), "pagerank_nonnegative");
}

#[test]
fn pagerank_sums_to_one() {
    let g = sample();
    let r: Vec<Option<f64>> = reals(good(&g, &PR))
        .into_iter()
        .map(|x| x.map(|y| y * 1.01))
        .collect();
    rejects(&g, &PR, &Output::Reals(r), "pagerank_sums_to_one");
}

#[test]
fn labels_constant_along_edges() {
    let g = sample();
    good(&g, &Kernel::Wcc);
    // Node 3 gets a class of its own, although 2-3 is an edge.
    rejects(
        &g,
        &Kernel::Wcc,
        &Output::Labels(vec![0, 0, 0, 3, 4]),
        "labels_constant_along_edges",
    );
}

#[test]
fn labels_are_smallest_ids() {
    let g = sample();
    // The right classes, labelled by their largest member.
    rejects(
        &g,
        &Kernel::Wcc,
        &Output::Labels(vec![3, 3, 3, 3, 4]),
        "labels_are_smallest_ids",
    );
}

#[test]
fn bfs_source_zero() {
    let g = path();
    let k = Kernel::Bfs { source: 0 };
    good(&g, &k);
    rejects(
        &g,
        &k,
        &Output::Ints(vec![Some(1), Some(2), Some(3), Some(4)]),
        "bfs_source_zero",
    );
}

#[test]
fn bfs_levels_adjacent() {
    let g = path();
    rejects(
        &g,
        &Kernel::Bfs { source: 0 },
        &Output::Ints(vec![Some(0), Some(1), Some(2), Some(5)]),
        "bfs_levels_adjacent",
    );
}

#[test]
fn bfs_has_parent() {
    let g = path();
    // Node 3 at level 2 like node 2: no neighbour at level 1.
    rejects(
        &g,
        &Kernel::Bfs { source: 0 },
        &Output::Ints(vec![Some(0), Some(1), Some(2), Some(2)]),
        "bfs_has_parent",
    );
}

fn weighted_path() -> Graph {
    let mut g = path();
    g.weight = Some(vec![1.0, 2.0, 0.5]);
    g
}

#[test]
fn sssp_source_zero() {
    let g = weighted_path();
    let k = Kernel::Sssp { source: 0 };
    good(&g, &k);
    rejects(
        &g,
        &k,
        &Output::Reals(vec![Some(0.5), Some(1.0), Some(3.0), Some(3.5)]),
        "sssp_source_zero",
    );
}

#[test]
fn sssp_edges_relaxed() {
    let g = weighted_path();
    // d(3) = 10 > d(2) + 0.5.
    rejects(
        &g,
        &Kernel::Sssp { source: 0 },
        &Output::Reals(vec![Some(0.0), Some(1.0), Some(3.0), Some(10.0)]),
        "sssp_edges_relaxed",
    );
}

#[test]
fn sssp_has_tight_parent() {
    let g = weighted_path();
    // d(3) = 2 is reachable by no edge.
    rejects(
        &g,
        &Kernel::Sssp { source: 0 },
        &Output::Reals(vec![Some(0.0), Some(1.0), Some(3.0), Some(2.0)]),
        "sssp_has_tight_parent",
    );
}

#[test]
fn core_bounded_by_degree() {
    let g = sample();
    let mut c = ints(good(&g, &Kernel::CoreNumber));
    c[3] = Some(5);
    rejects(
        &g,
        &Kernel::CoreNumber,
        &Output::Ints(c),
        "core_bounded_by_degree",
    );
}

#[test]
fn core_is_supported() {
    let g = path();
    good(&g, &Kernel::CoreNumber);
    // Node 1 has degree 2, but no neighbour with core >= 2.
    rejects(
        &g,
        &Kernel::CoreNumber,
        &Output::Ints(vec![Some(1), Some(2), Some(1), Some(1)]),
        "core_is_supported",
    );
}

#[test]
fn triangles_sum_divisible_by_3() {
    let g = sample();
    good(&g, &Kernel::Triangles);
    rejects(
        &g,
        &Kernel::Triangles,
        &Output::Ints(vec![Some(1), Some(1), Some(0), Some(0), Some(0)]),
        "triangles_sum_divisible_by_3",
    );
}

#[test]
fn triangles_bounded_by_pairs() {
    let g = path();
    // Node 0 has one neighbour, so no pair to close.
    rejects(
        &g,
        &Kernel::Triangles,
        &Output::Ints(vec![Some(3), Some(0), Some(0), Some(0)]),
        "triangles_bounded_by_pairs",
    );
}

#[test]
fn clustering_in_unit_interval() {
    let g = sample();
    let mut c = reals(good(&g, &Kernel::Clustering));
    c[0] = Some(1.5);
    rejects(
        &g,
        &Kernel::Clustering,
        &Output::Reals(c),
        "clustering_in_unit_interval",
    );
    // Degree below 2 must give exactly 0.
    let mut c = reals(good(&g, &Kernel::Clustering));
    c[3] = Some(0.5);
    rejects(
        &g,
        &Kernel::Clustering,
        &Output::Reals(c),
        "clustering_in_unit_interval",
    );
}

#[test]
fn transitivity_in_unit_interval() {
    let g = sample();
    good(&g, &Kernel::Transitivity);
    rejects(
        &g,
        &Kernel::Transitivity,
        &Output::Real(1.5),
        "transitivity_in_unit_interval",
    );
}

#[test]
fn modularity_in_range() {
    let g = sample();
    let k = Kernel::Modularity { resolution: 1.0 };
    good(&g, &k);
    rejects(&g, &k, &Output::Real(1.5), "modularity_in_range");
    rejects(&g, &k, &Output::Real(-0.6), "modularity_in_range");
    // At resolution 2 the bound is −1.5: −1.2 is possible, −1.6 is not.
    let k2 = Kernel::Modularity { resolution: 2.0 };
    good(&g, &k2);
    let p = Projection::new(&g);
    assert!(check(&k2, &p, g.partition.as_deref(), &Output::Real(-1.2)).is_empty());
    rejects(&g, &k2, &Output::Real(-1.6), "modularity_in_range");
}

fn quotient(g: &Graph) -> (Vec<u64>, Vec<(u32, u32, f64)>) {
    match good(g, &Kernel::Quotient) {
        Output::Quotient { sizes, edges } => (sizes, edges),
        other => panic!("{other:?}"),
    }
}

#[test]
fn community_sizes_sum_to_n() {
    let g = sample();
    let (mut sizes, edges) = quotient(&g);
    sizes[1] -= 1;
    rejects(
        &g,
        &Kernel::Quotient,
        &Output::Quotient { sizes, edges },
        "community_sizes_sum_to_n",
    );
}

#[test]
fn quotient_weights_sum_to_m() {
    let g = sample();
    let (sizes, mut edges) = quotient(&g);
    edges[0].2 += 1.0;
    rejects(
        &g,
        &Kernel::Quotient,
        &Output::Quotient { sizes, edges },
        "quotient_weights_sum_to_m",
    );
}

#[test]
fn quotient_labels_in_range() {
    let g = sample();
    let (sizes, mut edges) = quotient(&g);
    edges[0].1 = 7;
    rejects(
        &g,
        &Kernel::Quotient,
        &Output::Quotient { sizes, edges },
        "quotient_labels_in_range",
    );
}

/// The list of invariants above is complete: every name the checker can report has a test.
#[test]
fn every_invariant_has_a_negative_test() {
    let source = include_str!("../src/invariants.rs");
    let this = include_str!("invariants.rs");
    let mut names: Vec<&str> = source
        .split('"')
        .skip(1)
        .step_by(2)
        .filter(|s| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        })
        .filter(|s| !["pagerank_sum", "sssp"].contains(s)) // tolerance names, not invariants
        .collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), 28, "27 invariants plus `shape`: {names:?}");
    for name in names {
        assert!(
            this.contains(&format!("fn {name}()")),
            "no negative test for {name}"
        );
    }
}
