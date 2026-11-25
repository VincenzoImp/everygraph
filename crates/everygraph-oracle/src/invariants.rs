// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Invariant checkers: properties every correct result has, checked without a reference
//! answer. Each one reads only the analytic graph (counted independently, in
//! [`crate::projection`]) and the result under test.
//!
//! | Invariant | Applies to |
//! |---|---|
//! | `offsets_monotone`, `targets_in_range`, `lists_sorted_unique` | adjacency |
//! | `self_loop_exactly_where_kept` (a node lists itself, once, exactly when it has a kept loop) | adjacency |
//! | `adjacency_symmetric` | adjacency of an undirected graph |
//! | `entries_equal_m` (Σ list lengths = 2m + loops undirected, m + loops directed) | adjacency |
//! | `degree_sum` (Σ degree = 2m + 2·loops, m of the undirected projection; out/in: m + loops) | degrees |
//! | `pagerank_nonnegative`, `pagerank_sums_to_one` (kept loops included) | PageRank |
//! | `pagerank_fixed_point` (one step of the S1 equation, pushed along out-edges, moves the result by at most (1 + α) × its tolerance) | PageRank |
//! | `labels_constant_along_edges`, `labels_are_smallest_ids` | weak components |
//! | `labels_are_smallest_ids` | strong components |
//! | `bfs_source_zero`, `bfs_levels_adjacent` (an edge changes the level by at most 1 forward), `bfs_has_parent` | BFS |
//! | `sssp_source_zero`, `sssp_edges_relaxed`, `sssp_has_tight_parent` | weighted distances |
//! | `core_bounded_by_degree`, `core_is_supported` (v has ≥ core(v) neighbours with core ≥ core(v)) | core numbers |
//! | `triangles_sum_divisible_by_3`, `triangles_bounded_by_pairs` | triangles per node |
//! | `clustering_in_unit_interval`, `transitivity_in_unit_interval`, `modularity_in_range` | coefficients |
//! | `modularity_equals_quotient_modularity` (modularity of the partition = modularity of its quotient graph, loops on the diagonal, under singletons) | modularity |
//! | `community_sizes_sum_to_n`, `quotient_weights_sum_to_m` (m with kept loops) | quotient graphs |
//! | `report_rows_add_up` (rows = rejected + dropped loops + dropped duplicates + merged + edges), `report_edges_add_up` (edges = pairs + repeated) | ingest report |

use std::collections::BTreeMap;

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
        (Kernel::PageRank { damping, weighted }, Output::Reals(r)) if r.len() == n => {
            pagerank(&mut c, p, r);
            if r.iter().all(Option::is_some) {
                let x: Vec<f64> = r.iter().flatten().copied().collect();
                pagerank_fixed_point(&mut c, p, *damping, *weighted, &x);
            }
        }
        (Kernel::IngestReport, Output::Counts(r)) => report(&mut c, r),
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
            c.require(
                sum.is_multiple_of(3),
                "triangles_sum_divisible_by_3",
                || format!("sum {sum}"),
            );
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
        (
            Kernel::Modularity {
                resolution,
                weighted,
            },
            Output::Real(q),
        ) => {
            // Q(γ) = Q(1) + (1 − γ) Σ_c (D_c / 2m)², with Q(1) ∈ [−1/2, 1] and the sum in
            // [0, 1]: so Q(γ) ∈ [−1/2 − max(0, γ − 1), 1] for γ ≥ 0.
            let low = -0.5 - (resolution - 1.0).max(0.0);
            c.require((low..=1.0).contains(q), "modularity_in_range", || {
                format!("{q} outside [{low}, 1] at resolution {resolution}")
            });
            if let Some(part) = partition.filter(|part| part.len() == n) {
                let via = quotient_modularity(p, part, *resolution, *weighted);
                c.require(
                    tolerance("modularity").close(*q, via, p.n),
                    "modularity_equals_quotient_modularity",
                    || format!("{q} vs {via} on the quotient graph"),
                );
            }
        }
        (Kernel::Quotient, Output::Quotient { sizes, edges }) => {
            let total: u64 = sizes.iter().sum();
            c.require(total == to_u64(n), "community_sizes_sum_to_n", || {
                format!("Σ sizes = {total}, n = {n}")
            });
            let w: f64 = edges.iter().map(|e| e.2).sum();
            let m = p.undirected_m() + (0..n).map(|v| p.kept_loops(v)).sum::<u64>();
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
        Output::Counts(x) => x.len(),
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
    let loops: u64 = (0..n).map(|v| p.kept_loops(v)).sum();
    let expected = loops + if p.directed { p.m() } else { 2 * p.m() };
    c.require(to_u64(targets.len()) == expected, "entries_equal_m", || {
        format!(
            "{} entries, expected {expected} (m = {}, kept loops = {loops})",
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
        let own = list
            .iter()
            .filter(|&&v| v == u32::try_from(u).unwrap_or(u32::MAX))
            .count();
        c.require(
            to_u64(own) == p.kept_loops(u),
            "self_loop_exactly_where_kept",
            || {
                format!(
                    "node {u} lists itself {own} times, kept loops: {}",
                    p.kept_loops(u)
                )
            },
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
        // `degree` is taken in the undirected projection, directed graphs included.
        _ => (2 * p.undirected_m() + 2 * loops, "2m + 2·loops"),
    };
    c.require(d.iter().all(Option::is_some), "shape", || {
        "a node without a degree".into()
    });
    c.require(sum == expected, "degree_sum", || {
        format!(
            "Σ degree = {sum}, {rule} = {expected} (m = {}, kept loops = {loops})",
            if matches!(kernel, Kernel::Degree) {
                p.undirected_m()
            } else {
                p.m()
            }
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

/// One step of S1 (`docs/spec/graph-semantics.md` § 5.1), pushed along the out-lists
/// (the reference pulls along the in-lists). If every value is within the PageRank
/// tolerance `b(v)` of the fixed point, the step moves the vector by at most
/// `(1 + α) Σ b(v)` in L1, because the step's linear part is α times a stochastic matrix.
fn pagerank_fixed_point(c: &mut Checker, p: &Projection, damping: f64, weighted: bool, x: &[f64]) {
    let n = p.len();
    if n == 0 {
        return;
    }
    let w = |e: usize| if weighted { p.out.weight_sum[e] } else { 1.0 };
    let diag = |v: usize| {
        let l = p.loop_pair_weight(v, weighted);
        if p.directed { l } else { 2.0 * l }
    };
    let s: Vec<f64> = (0..n)
        .map(|u| p.out.range(u).map(w).sum::<f64>() + diag(u))
        .collect();
    let nf = f64::from(p.n);
    let dangling: f64 = (0..n).filter(|&u| s[u] == 0.0).map(|u| x[u]).sum();
    let mut next = vec![(1.0 - damping) / nf + damping * dangling / nf; n];
    for u in (0..n).filter(|&u| s[u] > 0.0) {
        let share = damping * x[u] / s[u];
        for e in p.out.range(u) {
            next[ix(p.out.targets[e])] += share * w(e);
        }
        next[u] += share * diag(u);
    }
    let moved: f64 = x.iter().zip(&next).map(|(a, b)| (a - b).abs()).sum();
    let tol = tolerance("pagerank");
    let limit = (1.0 + damping) * x.iter().map(|&v| tol.bound(v, p.n)).sum::<f64>();
    c.require(moved <= limit, "pagerank_fixed_point", || {
        format!("one step moves the result by {moved:e} in L1, more than {limit:e}")
    });
}

/// Modularity of the quotient graph of `partition` (community pairs and loops, kept input
/// loops on the diagonal) under the singleton partition: `Σ_a [L(a)/W − γ (S(a)/2W)²]`.
fn quotient_modularity(p: &Projection, partition: &[u32], resolution: f64, weighted: bool) -> f64 {
    let sym = &p.undirected;
    let mut edges: BTreeMap<(u32, u32), f64> = BTreeMap::new();
    for u in 0..sym.n() {
        let a = partition[u];
        for e in sym.range(u) {
            let v = ix(sym.targets[e]);
            if v > u {
                let b = partition[v];
                let w = if weighted { sym.weight_sum[e] } else { 1.0 };
                *edges.entry((a.min(b), a.max(b))).or_default() += w;
            }
        }
        *edges.entry((a, a)).or_default() += p.loop_pair_weight(u, weighted);
    }
    let mut inside: BTreeMap<u32, f64> = BTreeMap::new();
    let mut strength: BTreeMap<u32, f64> = BTreeMap::new();
    let mut total = 0.0;
    for (&(a, b), &w) in &edges {
        total += w;
        if a == b {
            *inside.entry(a).or_default() += w;
            *strength.entry(a).or_default() += 2.0 * w;
        } else {
            *strength.entry(a).or_default() += w;
            *strength.entry(b).or_default() += w;
        }
    }
    if total == 0.0 {
        return 0.0;
    }
    strength
        .iter()
        .map(|(a, &s)| {
            inside.get(a).copied().unwrap_or(0.0) / total - resolution * (s / (2.0 * total)).powi(2)
        })
        .sum()
}

/// The two identities of the ingest report (`docs/spec/graph-semantics.md` § 2.8).
fn report(c: &mut Checker, r: &BTreeMap<String, u64>) {
    let keys = [
        "counts.rows",
        "dropped.rejected",
        "dropped.self_loops",
        "dropped.duplicates",
        "counts.merged",
        "counts.edges",
        "counts.repeated",
        "counts.pairs",
    ];
    if let Some(k) = keys.iter().find(|k| !r.contains_key(**k)) {
        c.fail("shape", format!("no {k} in the report"));
        return;
    }
    let k = |name: &str| r[name];
    let parts = k("dropped.rejected")
        + k("dropped.self_loops")
        + k("dropped.duplicates")
        + k("counts.merged")
        + k("counts.edges");
    c.require(parts == k("counts.rows"), "report_rows_add_up", || {
        format!("{} rows, the outcomes add up to {parts}", k("counts.rows"))
    });
    let edges = k("counts.pairs") + k("counts.repeated");
    c.require(edges == k("counts.edges"), "report_edges_add_up", || {
        format!("{} edges, pairs + repeated = {edges}", k("counts.edges"))
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
