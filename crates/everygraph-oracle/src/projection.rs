// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The analytic graph: what kernels see after the semantics of
//! `plan/issues/m2-09-graph-semantics.md` are applied to the events.
//!
//! Built here by the most direct method there is (an ordered map of pairs), so that it can
//! serve as the reference for every adjacency an engine builds. The Python references
//! build the same projection independently (`oracle/src/everygraph_oracle_ref/graph.py`).

use std::collections::BTreeMap;
use std::ops::Range;

use crate::graph::{Duplicates, Graph, SelfLoops};
use crate::to_usize;

/// Adjacency lists with, per entry, the pair's multiplicity and aggregated weight.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Adjacency {
    /// `n + 1` offsets.
    pub offsets: Vec<u64>,
    /// Neighbours, sorted and without duplicates in each list.
    pub targets: Vec<u32>,
    /// Number of events behind each entry.
    pub multiplicity: Vec<u64>,
    /// Sum of the event weights, in row order (the multiplicity if there is no weight).
    pub weight_sum: Vec<f64>,
    /// Smallest event weight (1 if there is no weight column): the cost of a step. Under
    /// `duplicates = sum` the stored sum (§ 4.2); in the undirected projection of a
    /// directed graph, the smaller of the two directions' costs.
    pub weight_min: Vec<f64>,
}

impl Adjacency {
    /// Number of nodes.
    #[must_use]
    pub fn n(&self) -> usize {
        self.offsets.len().saturating_sub(1)
    }

    /// The entry range of node `v`.
    #[must_use]
    pub fn range(&self, v: usize) -> Range<usize> {
        to_usize(self.offsets[v])..to_usize(self.offsets[v + 1])
    }

    /// The neighbours of node `v`.
    #[must_use]
    pub fn list(&self, v: usize) -> &[u32] {
        &self.targets[self.range(v)]
    }

    /// Number of distinct neighbours of `v`.
    #[must_use]
    pub fn degree(&self, v: usize) -> usize {
        self.range(v).len()
    }

    /// Number of entries.
    #[must_use]
    pub fn entries(&self) -> usize {
        self.targets.len()
    }

    /// True if `v` lists `u`.
    #[must_use]
    pub fn has(&self, v: usize, u: u32) -> bool {
        self.list(v).binary_search(&u).is_ok()
    }

    fn from_pairs(n: usize, pairs: &BTreeMap<(u32, u32), Agg>) -> Self {
        let mut offsets = vec![0u64; n + 1];
        for &(u, _) in pairs.keys() {
            offsets[to_usize(u.into()) + 1] += 1;
        }
        for i in 0..n {
            offsets[i + 1] += offsets[i];
        }
        let mut adj = Self {
            offsets,
            targets: Vec::with_capacity(pairs.len()),
            multiplicity: Vec::with_capacity(pairs.len()),
            weight_sum: Vec::with_capacity(pairs.len()),
            weight_min: Vec::with_capacity(pairs.len()),
        };
        // BTreeMap order is (source, target): lists come out grouped and sorted.
        for (&(_, v), a) in pairs {
            adj.targets.push(v);
            adj.multiplicity.push(a.multiplicity);
            adj.weight_sum.push(a.weight_sum);
            adj.weight_min.push(a.weight_min);
        }
        adj
    }
}

/// What ingest did with the rows: `rows = self_loop_rows + duplicate_rows + pairs`.
/// [`Projection::report`] turns them into the ingest report of the specification.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IngestCounts {
    /// Rows in the input.
    pub rows: u64,
    /// Rows from a node to itself (dropped or kept, they are counted here).
    pub self_loop_rows: u64,
    /// Rows, loops excluded, that repeat a pair already seen (for undirected graphs
    /// `(v, u)` repeats `(u, v)`), whatever `duplicates` then does with them.
    pub duplicate_rows: u64,
    /// Distinct pairs (undirected) or arcs (directed), loops excluded: the `m` of the
    /// analytic graph.
    pub pairs: u64,
    /// Nodes without any incident pair or kept loop.
    pub isolates: u64,
}

/// The analytic graph.
#[derive(Clone, Debug, PartialEq)]
pub struct Projection {
    /// Number of nodes.
    pub n: u32,
    /// Directed or not.
    pub directed: bool,
    /// Out-adjacency (symmetric if undirected).
    pub out: Adjacency,
    /// In-adjacency of a directed graph; equal to `out` for an undirected one.
    pub inn: Adjacency,
    /// Undirected projection: `{u, v}` present if either arc is, events of both
    /// directions aggregated. Equal to `out` for an undirected graph.
    pub undirected: Adjacency,
    /// Self-loop rows at each node.
    pub loops: Vec<u64>,
    /// Weight of each node's kept loop: the sum of its rows' weights in row order (their
    /// number without a weight column), the first row's under `duplicates = collapse`;
    /// 0 without a kept loop. It enters the adjacency as `A[v][v] = 2w` undirected, `w`
    /// directed (`docs/spec/graph-semantics.md` § 3).
    pub loop_weight: Vec<f64>,
    /// Whether loops are part of the analytic graph.
    pub keep_loops: bool,
    /// What repeated rows became.
    pub duplicates: Duplicates,
    /// The ingest report.
    pub counts: IngestCounts,
}

#[derive(Clone, Copy, Debug)]
struct Agg {
    multiplicity: u64,
    weight_sum: f64,
    weight_min: f64,
}

impl Agg {
    fn add(&mut self, w: f64) {
        self.multiplicity += 1;
        self.weight_sum += w;
        self.weight_min = self.weight_min.min(w);
    }
}

fn insert(map: &mut BTreeMap<(u32, u32), Agg>, key: (u32, u32), w: f64) -> bool {
    match map.get_mut(&key) {
        Some(a) => {
            a.add(w);
            false
        }
        None => {
            map.insert(
                key,
                Agg {
                    multiplicity: 1,
                    weight_sum: w,
                    weight_min: w,
                },
            );
            true
        }
    }
}

impl Projection {
    /// Applies the graph's semantics to its events.
    ///
    /// # Panics
    ///
    /// If the graph does not pass [`Graph::validate`].
    #[must_use]
    pub fn new(g: &Graph) -> Self {
        g.validate().expect("valid graph");
        let n = to_usize(g.n.into());
        let keep_loops = g.semantics.self_loops == SelfLoops::Keep;
        let duplicates = g.semantics.duplicates;
        let collapse = duplicates == Duplicates::Collapse;
        let mut loops = vec![0u64; n];
        let mut loop_weight = vec![0.0f64; n];
        // Undirected: key (min, max) for counting, mirrored below. Directed: key (u, v).
        let mut own: BTreeMap<(u32, u32), Agg> = BTreeMap::new();
        let mut und: BTreeMap<(u32, u32), Agg> = BTreeMap::new();
        let mut counts = IngestCounts {
            rows: g.rows() as u64,
            ..IngestCounts::default()
        };
        for i in 0..g.rows() {
            let (u, v) = (g.src[i], g.dst[i]);
            let w = g.weight.as_ref().map_or(1.0, |w| w[i]);
            if u == v {
                let x = to_usize(u.into());
                loops[x] += 1;
                counts.self_loop_rows += 1;
                if keep_loops && (loops[x] == 1 || !collapse) {
                    loop_weight[x] += w;
                }
                continue;
            }
            let key = if g.directed {
                (u, v)
            } else {
                (u.min(v), u.max(v))
            };
            if own.contains_key(&key) {
                counts.duplicate_rows += 1;
                if collapse {
                    continue;
                }
            }
            insert(&mut own, key, w);
            if g.directed {
                insert(&mut und, (u.min(v), u.max(v)), w);
            }
        }
        counts.pairs = own.len() as u64;
        if duplicates == Duplicates::Sum {
            // The events are gone: a step costs the stored sum.
            for a in own.values_mut() {
                a.weight_min = a.weight_sum;
            }
            for (&(u, v), a) in &mut und {
                a.weight_min = [(u, v), (v, u)]
                    .iter()
                    .filter_map(|k| own.get(k))
                    .map(|b| b.weight_sum)
                    .fold(f64::INFINITY, f64::min);
            }
        }
        let mirror = |m: &BTreeMap<(u32, u32), Agg>| {
            let mut sym = BTreeMap::new();
            for (&(u, v), &a) in m {
                sym.insert((u, v), a);
                sym.insert((v, u), a);
            }
            sym
        };
        let (out, inn, undirected) = if g.directed {
            let rev: BTreeMap<(u32, u32), Agg> =
                own.iter().map(|(&(u, v), &a)| ((v, u), a)).collect();
            (
                Adjacency::from_pairs(n, &own),
                Adjacency::from_pairs(n, &rev),
                Adjacency::from_pairs(n, &mirror(&und)),
            )
        } else {
            let sym = Adjacency::from_pairs(n, &mirror(&own));
            (sym.clone(), sym.clone(), sym)
        };
        counts.isolates = (0..n)
            .filter(|&v| undirected.degree(v) == 0 && !(keep_loops && loops[v] > 0))
            .count() as u64;
        Self {
            n: g.n,
            directed: g.directed,
            out,
            inn,
            undirected,
            loops,
            loop_weight,
            keep_loops,
            duplicates,
            counts,
        }
    }

    /// The ingest report of `docs/spec/graph-semantics.md` § 2.8, keyed as printed
    /// (`counts.rows`, `dropped.self_loops`, ...). `dropped.rejected` is always 0: the
    /// oracle's rows are all readable.
    #[must_use]
    pub fn report(&self) -> BTreeMap<String, u64> {
        let c = self.counts;
        let kept: u64 = (0..self.len()).map(|v| self.kept_loops(v)).sum();
        let (dropped_loops, loop_repeats) = if self.keep_loops {
            (0, c.self_loop_rows - kept)
        } else {
            (c.self_loop_rows, 0)
        };
        let repeats = c.duplicate_rows + loop_repeats;
        let (dropped, merged, repeated) = match self.duplicates {
            Duplicates::Events => (0, 0, repeats),
            Duplicates::Collapse => (repeats, 0, 0),
            Duplicates::Sum => (0, repeats, 0),
        };
        let pairs = c.pairs + kept;
        let stored_loops = if self.duplicates == Duplicates::Events {
            kept + loop_repeats
        } else {
            kept
        };
        [
            ("counts.rows", c.rows),
            ("dropped.rejected", 0),
            ("dropped.self_loops", dropped_loops),
            ("dropped.duplicates", dropped),
            ("counts.merged", merged),
            ("counts.edges", pairs + repeated),
            ("counts.self_loops", stored_loops),
            ("counts.repeated", repeated),
            ("counts.pairs", pairs),
            ("counts.nodes", u64::from(self.n)),
            ("counts.isolates", c.isolates),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect()
    }

    /// Number of nodes as an index.
    #[must_use]
    pub fn len(&self) -> usize {
        to_usize(self.n.into())
    }

    /// True if there are no nodes.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// `m`: distinct pairs (undirected) or arcs (directed), loops excluded.
    #[must_use]
    pub const fn m(&self) -> u64 {
        self.counts.pairs
    }

    /// Kept loops at `v`: 1 if loops are kept and `v` has any (repeated loop rows are
    /// one pair, like any repeated pair), else 0.
    #[must_use]
    pub fn kept_loops(&self, v: usize) -> u64 {
        u64::from(self.keep_loops && self.loops[v] > 0)
    }

    /// `w({v, v})` of `v`'s kept loop: 1 unweighted, [`Self::loop_weight`] weighted, 0
    /// without a kept loop.
    #[must_use]
    pub fn loop_pair_weight(&self, v: usize, weighted: bool) -> f64 {
        if weighted {
            self.loop_weight[v]
        } else if self.kept_loops(v) == 1 {
            1.0
        } else {
            0.0
        }
    }

    /// The teleport vector of PageRank: uniform over the distinct `seeds`, or over every
    /// node when there are none (`docs/spec/graph-semantics.md` § 5.1).
    #[must_use]
    pub fn teleport(&self, seeds: &[u32]) -> Vec<f64> {
        if seeds.is_empty() {
            return vec![1.0 / f64::from(self.n); self.len()];
        }
        let mut t = vec![0.0; self.len()];
        let set: std::collections::BTreeSet<u32> = seeds.iter().copied().collect();
        #[expect(clippy::cast_precision_loss, reason = "a small count")]
        let share = 1.0 / set.len() as f64;
        for s in set {
            t[to_usize(s.into())] = share;
        }
        t
    }

    /// Number of pairs of the undirected projection.
    #[must_use]
    pub fn undirected_m(&self) -> u64 {
        self.undirected.entries() as u64 / 2
    }
}

#[cfg(test)]
mod tests {
    use super::Projection;
    use crate::graph::{Duplicates, Graph, SelfLoops};

    fn graph(directed: bool, edges: &[(u32, u32)], n: u32) -> Graph {
        let mut g = Graph::with_nodes("t", n, directed);
        for &(u, v) in edges {
            g.src.push(u);
            g.dst.push(v);
        }
        g
    }

    #[test]
    fn undirected_counts_add_up() {
        let g = graph(false, &[(0, 1), (1, 0), (1, 1), (1, 2), (2, 1), (2, 1)], 4);
        let p = Projection::new(&g);
        let c = p.counts;
        assert_eq!(
            (
                c.rows,
                c.self_loop_rows,
                c.duplicate_rows,
                c.pairs,
                c.isolates
            ),
            (6, 1, 3, 2, 1)
        );
        assert_eq!(c.self_loop_rows + c.duplicate_rows + c.pairs, c.rows);
        assert_eq!(p.out.list(1), &[0, 2]);
        assert_eq!(p.out.multiplicity[p.out.range(1)], [2, 3]);
    }

    #[test]
    fn directed_has_three_views() {
        let mut g = graph(true, &[(0, 1), (1, 0), (2, 1), (2, 2)], 3);
        g.semantics.self_loops = SelfLoops::Keep;
        let p = Projection::new(&g);
        assert_eq!(p.m(), 3);
        assert_eq!(p.out.list(2), &[1]);
        assert_eq!(p.inn.list(1), &[0, 2]);
        assert_eq!(p.undirected.list(1), &[0, 2]);
        assert_eq!(p.undirected.multiplicity[p.undirected.range(0)], [2]);
        assert_eq!(p.kept_loops(2), 1);
        assert_eq!(p.kept_loops(1), 0);
        assert_eq!(p.counts.isolates, 0);
    }

    /// Pair 0-1 as rows weighted 4, 1, 2 (the third reversed) and loop rows 3, 5 at 2.
    fn repeated(duplicates: Duplicates) -> Projection {
        let mut g = graph(false, &[(0, 1), (0, 1), (1, 0), (2, 2), (2, 2)], 4);
        g.weight = Some(vec![4.0, 1.0, 2.0, 3.0, 5.0]);
        g.semantics.self_loops = SelfLoops::Keep;
        g.semantics.duplicates = duplicates;
        Projection::new(&g)
    }

    #[test]
    fn duplicates_modes_store_what_the_spec_says() {
        let at = |p: &Projection| {
            let e = p.out.range(0).start;
            (
                p.out.multiplicity[e],
                p.out.weight_sum[e],
                p.out.weight_min[e],
                p.loop_weight[2],
            )
        };
        assert_eq!(at(&repeated(Duplicates::Events)), (3, 7.0, 1.0, 8.0));
        assert_eq!(at(&repeated(Duplicates::Sum)), (3, 7.0, 7.0, 8.0));
        assert_eq!(at(&repeated(Duplicates::Collapse)), (1, 4.0, 4.0, 3.0));
    }

    #[test]
    fn the_report_adds_up_in_every_mode() {
        for (mode, dropped, merged, stored) in [
            (Duplicates::Events, 0, 0, 3),
            (Duplicates::Collapse, 3, 0, 0),
            (Duplicates::Sum, 0, 3, 0),
        ] {
            let r = repeated(mode).report();
            let k = |s: &str| r[s];
            assert_eq!(
                (
                    k("dropped.duplicates"),
                    k("counts.merged"),
                    k("counts.repeated")
                ),
                (dropped, merged, stored),
                "{mode:?}"
            );
            assert_eq!(k("counts.pairs"), 2, "the pair and the loop");
            assert_eq!(k("counts.edges"), k("counts.pairs") + k("counts.repeated"));
            assert_eq!(k("counts.rows"), 5);
            assert_eq!(
                k("counts.rows"),
                k("dropped.self_loops")
                    + k("dropped.duplicates")
                    + k("counts.merged")
                    + k("counts.edges")
            );
            assert_eq!(k("counts.isolates"), 1);
        }
    }
}
