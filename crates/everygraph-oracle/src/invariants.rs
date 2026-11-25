// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Invariant checkers: properties every correct result has, checked without a reference
//! answer. Each one reads only the analytic graph (counted independently, in
//! [`crate::projection`]) and the result under test.
//!
//! | Invariant | Applies to |
//! |---|---|
//! | `offsets_monotone`, `targets_in_range`, `lists_sorted_unique`, `no_self_loops` | adjacency |
//! | `adjacency_symmetric` | adjacency of an undirected graph |
//! | `entries_equal_m` (Σ list lengths = 2m undirected, m directed) | adjacency |
//! | `degree_sum` (Σ degree = 2m + 2·loops undirected; out/in: m + loops) | degrees |
//! | `pagerank_nonnegative`, `pagerank_sums_to_one` | PageRank |
//! | `labels_constant_along_edges`, `labels_are_smallest_ids` | weak components |
//! | `labels_are_smallest_ids` | strong components |
//! | `bfs_source_zero`, `bfs_levels_adjacent` (an edge changes the level by at most 1 forward), `bfs_has_parent` | BFS |
//! | `sssp_source_zero`, `sssp_edges_relaxed`, `sssp_has_tight_parent` | weighted distances |
//! | `core_bounded_by_degree`, `core_is_supported` (v has ≥ core(v) neighbours with core ≥ core(v)) | core numbers |
//! | `triangles_sum_divisible_by_3`, `triangles_bounded_by_pairs` | triangles per node |
//! | `clustering_in_unit_interval`, `transitivity_in_unit_interval`, `modularity_in_range` | coefficients |
//! | `community_sizes_sum_to_n`, `quotient_weights_sum_to_m` | quotient graphs |

use crate::compare::tolerance;
use crate::kernel::{Kernel, Output};
use crate::projection::{Adjacency, Projection};
use crate::{ix, to_u64};

/// A violated invariant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Violation {
    /// The invariant's name (see the table in the module documentation).
    pub invariant: &'static str,
    /// What was found.
    pub detail: String,
}

struct Checker {
    found: Vec<Violation>,
}

impl Checker {
    fn fail(&mut self, invariant: &'static str, detail: String) {
        if !self.found.iter().any(|v| v.invariant == invariant) {
            self.found.push(Violation { invariant, detail });
        }
    }

    fn require(&mut self, ok: bool, invariant: &'static str, detail: impl FnOnce() -> String) {
        if !ok {
            self.fail(invariant, detail());
        }
    }
}

/// Every invariant that applies to `output` as a result of `kernel` on `p`.
/// An output of the wrong shape or length is reported as `shape`.
#[must_use]
pub fn check(
    kernel: &Kernel,
    p: &Projection,
    partition: Option<&[u32]>,
    output: &Output,
) -> Vec<Violation> {
    let mut c = Checker { found: Vec::new() };
    let n = p.len();
    match (kernel, output) {
        (Kernel::Adjacency, Output::Csr { offsets, targets }) => {
            adjacency(&mut c, p, offsets, targets);
        }
        (Kernel::Degree | Kernel::DegreeOut | Kernel::DegreeIn, Output::Ints(d))
            if d.len() == n =>
        {
            degrees(&mut c, kernel, p, d);
        }
        (Kernel::PageRank { .. }, Output::Reals(r)) if r.len() == n => pagerank(&mut c, p, r),
        (Kernel::Wcc, Output::Labels(l)) if l.len() == n => {
            labels_smallest(&mut c, l);
            for u in 0..n {
                for &v in p.undirected.list(u) {
                    c.require(l[u] == l[ix(v)], "labels_constant_along_edges", || {
                        format!("edge {u}-{v} joins labels {} and {}", l[u], l[ix(v)])
                    });
                }
            }
        }
        (Kernel::Scc, Output::Labels(l)) if l.len() == n => labels_smallest(&mut c, l),
        (Kernel::Bfs { source }, Output::Ints(d)) if d.len() == n => bfs(&mut c, p, ix(*source), d),
        (Kernel::Sssp { source }, Output::Reals(d)) if d.len() == n => {
            sssp(&mut c, p, ix(*source), d);
        }
        (Kernel::CoreNumber, Output::Ints(k)) if k.len() == n => cores(&mut c, &p.undirected, k),
        (Kernel::Triangles, Output::Ints(t)) if t.len() == n => {
            let sum: u64 = t.iter().flatten().sum();
            c.require(t.iter().all(Option::is_some), "shape", || {
                "a node without a count".into()
            });
            c.require(sum % 3 == 0, "triangles_sum_divisible_by_3", || {
                format!("sum {sum}")
            });
            for (v, x) in t.iter().enumerate() {
                let d = to_u64(p.undirected.degree(v));
                let pairs = d * d.saturating_sub(1) / 2;
                c.require(
                    x.unwrap_or(0) <= pairs,
                    "triangles_bounded_by_pairs",
                    || format!("node {v}: {x:?} triangles with degree {d}"),
                );
            }
        }
        (Kernel::TriangleTotal, Output::Count(_)) => {}
        (Kernel::Clustering, Output::Reals(r)) if r.len() == n => {
            for (v, x) in r.iter().enumerate() {
                let ok = x.is_some_and(|x| (0.0..=1.0).contains(&x))
                    && (p.undirected.degree(v) >= 2 || x == &Some(0.0));
                c.require(ok, "clustering_in_unit_interval", || {
                    format!("node {v}: {x:?}")
                });
            }
        }
        (Kernel::Transitivity, Output::Real(x)) => {
            c.require(
                (0.0..=1.0).contains(x),
                "transitivity_in_unit_interval",
                || format!("{x}"),
            );
        }
        (Kernel::Modularity { .. }, Output::Real(q)) => {
            c.require((-0.5..=1.0).contains(q), "modularity_in_range", || {
                format!("{q}")
            });
        }
        (Kernel::Quotient, Output::Quotient { sizes, edges }) => {
            let total: u64 = sizes.iter().sum();
            c.require(total == to_u64(n), "community_sizes_sum_to_n", || {
                format!("Σ sizes = {total}, n = {n}")
            });
            let w: f64 = edges.iter().map(|e| e.2).sum();
            let m = p.undirected_m();
            // By value: an empty sum is -0.0.
            c.require(
                w == crate::exact_f64(m),
                "quotient_weights_sum_to_m",
                || format!("Σ weights = {w}, m = {m}"),
            );
            if let Some(part) = partition {
                let k = sizes.len();
                c.require(
                    part.iter().all(|&l| ix(l) < k)
                        && edges.iter().all(|e| ix(e.0) < k && ix(e.1) < k),
                    "quotient_labels_in_range",
                    || format!("{k} communities"),
                );
            }
        }
        _ => c.fail(
            "shape",
            format!(
                "{} output of length {} for kernel {}",
                output.shape(),
                output_len(output),
                kernel.name()
            ),
        ),
    }
    c.found
}

fn output_len(o: &Output) -> usize {
    match o {
        Output::Ints(x) => x.len(),
        Output::Reals(x) => x.len(),
        Output::Labels(x) => x.len(),
        Output::Csr { offsets, .. } => offsets.len().saturating_sub(1),
        Output::Quotient { sizes, .. } => sizes.len(),
        Output::Count(_) | Output::Real(_) => 1,
    }
}

fn adjacency(c: &mut Checker, p: &Projection, offsets: &[u64], targets: &[u32]) {
    let n = p.len();
    c.require(offsets.len() == n + 1, "shape", || {
        format!("{} offsets for n = {n}", offsets.len())
    });
    if offsets.len() != n + 1 {
        return;
    }
    let monotone = offsets.first() == Some(&0)
        && offsets.windows(2).all(|w| w[0] <= w[1])
        && offsets.last().copied() == Some(to_u64(targets.len()));
    c.require(monotone, "offsets_monotone", || {
        let bad = offsets.windows(2).position(|w| w[0] > w[1]);
        format!(
            "first {:?}, last {:?}, entries {}, first decrease at node {bad:?}",
            offsets.first(),
            offsets.last(),
            targets.len()
        )
    });
    if !monotone {
        return;
    }
    let adj = Adjacency {
        offsets: offsets.to_vec(),
        targets: targets.to_vec(),
        ..Adjacency::default()
    };
    let expected = if p.directed { p.m() } else { 2 * p.m() };
    c.require(to_u64(targets.len()) == expected, "entries_equal_m", || {
        format!(
            "{} entries, expected {expected} (m = {})",
            targets.len(),
            p.m()
        )
    });
    for u in 0..n {
        let list = adj.list(u);
        c.require(list.iter().all(|&v| ix(v) < n), "targets_in_range", || {
            format!("node {u}: {list:?}")
        });
        c.require(
            list.windows(2).all(|w| w[0] < w[1]),
            "lists_sorted_unique",
            || format!("node {u}: {list:?}"),
        );
        c.require(
            !list.contains(&u32::try_from(u).unwrap_or(u32::MAX)),
            "no_self_loops",
            || format!("node {u} lists itself"),
        );
        if !p.directed {
            for &v in list {
                let back = ix(v) < n
                    && adj
                        .list(ix(v))
                        .contains(&u32::try_from(u).unwrap_or(u32::MAX));
                c.require(back, "adjacency_symmetric", || {
                    format!("{u} lists {v}, {v} does not list {u}")
                });
            }
        }
    }
}

fn degrees(c: &mut Checker, kernel: &Kernel, p: &Projection, d: &[Option<u64>]) {
    let loops: u64 = (0..p.len()).map(|v| p.kept_loops(v)).sum();
    let sum: u64 = d.iter().flatten().sum();
    let (expected, rule) = match kernel {
        Kernel::DegreeOut | Kernel::DegreeIn => (p.m() + loops, "m + loops"),
        _ => (2 * p.m() + 2 * loops, "2m + 2·loops"),
    };
    c.require(d.iter().all(Option::is_some), "shape", || {
        "a node without a degree".into()
    });
    c.require(sum == expected, "degree_sum", || {
        format!(
            "Σ degree = {sum}, {rule} = {expected} (m = {}, kept loops = {loops})",
            p.m()
        )
    });
}

fn pagerank(c: &mut Checker, p: &Projection, r: &[Option<f64>]) {
    c.require(
        r.iter()
            .all(|x| x.is_some_and(|x| x.is_finite() && x >= 0.0)),
        "pagerank_nonnegative",
        || {
            let v = r
                .iter()
                .position(|x| !x.is_some_and(|x| x.is_finite() && x >= 0.0));
            format!("node {v:?}: {:?}", v.map(|v| r[v]))
        },
    );
    if p.is_empty() {
        return;
    }
    let sum: f64 = r.iter().flatten().sum();
    let tol = tolerance("pagerank_sum");
    c.require(tol.close(sum, 1.0, p.n), "pagerank_sums_to_one", || {
        format!("Σ = {sum:.17}")
    });
}

fn labels_smallest(c: &mut Checker, l: &[u32]) {
    for (v, &x) in l.iter().enumerate() {
        let ok = ix(x) <= v && l.get(ix(x)) == Some(&x);
        c.require(ok, "labels_are_smallest_ids", || {
            format!("node {v} has label {x}, which is not the smallest id of its class")
        });
    }
}

fn bfs(c: &mut Checker, p: &Projection, s: usize, d: &[Option<u64>]) {
    if s >= d.len() {
        c.fail("shape", format!("source {s} out of range"));
        return;
    }
    c.require(d[s] == Some(0), "bfs_source_zero", || {
        format!("source has {:?}", d[s])
    });
    for u in 0..d.len() {
        if let Some(du) = d[u] {
            for &v in p.out.list(u) {
                let ok = d[ix(v)].is_some_and(|dv| dv <= du + 1);
                c.require(ok, "bfs_levels_adjacent", || {
                    format!("edge {u}->{v}: {du} then {:?}", d[ix(v)])
                });
            }
            if u != s {
                let parent = p
                    .inn
                    .list(u)
                    .iter()
                    .any(|&w| d[ix(w)].is_some_and(|dw| dw + 1 == du));
                c.require(du > 0 && parent, "bfs_has_parent", || {
                    format!("node {u} at {du} has no parent")
                });
            }
        }
    }
}

fn sssp(c: &mut Checker, p: &Projection, s: usize, d: &[Option<f64>]) {
    if s >= d.len() {
        c.fail("shape", format!("source {s} out of range"));
        return;
    }
    let tol = tolerance("sssp");
    c.require(d[s] == Some(0.0), "sssp_source_zero", || {
        format!("source has {:?}", d[s])
    });
    for u in 0..d.len() {
        let Some(du) = d[u] else { continue };
        for e in p.out.range(u) {
            let v = ix(p.out.targets[e]);
            let via = du + p.out.weight_min[e];
            let ok = d[v].is_some_and(|dv| dv <= via + tol.bound(via, p.n));
            c.require(ok, "sssp_edges_relaxed", || {
                format!("edge {u}->{v}: {du} + w > {:?}", d[v])
            });
        }
        if u != s {
            let tight = p.inn.range(u).any(|e| {
                let w = ix(p.inn.targets[e]);
                d[w].is_some_and(|dw| tol.close(dw + p.inn.weight_min[e], du, p.n))
            });
            c.require(tight, "sssp_has_tight_parent", || {
                format!("node {u} at {du} has no tight parent")
            });
        }
    }
}

fn cores(c: &mut Checker, sym: &Adjacency, k: &[Option<u64>]) {
    for v in 0..k.len() {
        let Some(kv) = k[v] else {
            c.fail("shape", format!("node {v} has no core number"));
            continue;
        };
        let d = to_u64(sym.degree(v));
        c.require(kv <= d, "core_bounded_by_degree", || {
            format!("node {v}: core {kv} > degree {d}")
        });
        let support = sym
            .list(v)
            .iter()
            .filter(|&&u| k[ix(u)].is_some_and(|ku| ku >= kv))
            .count();
        c.require(to_u64(support) >= kv, "core_is_supported", || {
            format!("node {v}: core {kv}, only {support} neighbours with core >= {kv}")
        });
    }
}
