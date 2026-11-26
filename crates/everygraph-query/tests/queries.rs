// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Every point query against a model of its graph, on generated graphs with repeated
//! pairs, kept loops and a hub, directed and undirected, on six sources that must agree
//! (raw arrays, the raw copy, the compressed lists in the heap and memory-mapped, raw
//! targets streamed, the compressed lists streamed in the smallest window). The model
//! answers from the specification's wording (`docs/api-sketch.md` § 2.12,
//! `docs/spec/graph-semantics.md` § 6), not from the engine's algorithm: `khop`'s order
//! is a sort by each node's parent and its position in the parent's list.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "test code on small graphs"
)]

use std::cell::Cell;
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::num::NonZeroUsize;

use everygraph_core::{Memory, par};
use everygraph_kernels::edge_map::Source;
use everygraph_query::{
    Common, Cut, Direction, EdgeRows, Graph, Hops, Never, QueryError, Truncated,
};
use proptest::prelude::*;
use support::{Agg, Fixture, Model, Query, on_every_source};

mod support;

/// Graphs of up to 24 nodes whose node 0 is a hub, with repeated rows and loops.
fn graphs() -> impl Strategy<Value = Model> {
    (2_u32..24, any::<bool>()).prop_flat_map(|(n, directed)| {
        let row = (0..n, 0..n, 1_u32..8, -20_i64..20, any::<bool>());
        proptest::collection::vec(row, 0..90).prop_map(move |rows| {
            let rows: Vec<_> = rows
                .into_iter()
                .map(|(s, d, w, t, hub)| (if hub { 0 } else { s }, d, f64::from(w) / 2.0, t))
                .collect();
            Model::new(n, directed, &rows)
        })
    })
}

/// Graphs of up to 400 nodes and two hubs, whose lists are longer than a first prefix
/// and overlap, so that `khop` decodes longer prefixes of a hub whose first neighbours
/// it has seen.
fn hub_graphs() -> impl Strategy<Value = Model> {
    (50_u32..400, any::<bool>()).prop_flat_map(|(n, directed)| {
        let row = (0..n, 0..n, 0_u32..4);
        proptest::collection::vec(row, 0..1500).prop_map(move |rows| {
            let rows: Vec<_> = rows
                .into_iter()
                .map(|(s, d, hub)| (if hub < 2 { hub } else { s }, d, 1.0, 0))
                .collect();
            Model::new(n, directed, &rows)
        })
    })
}

fn directions() -> impl Strategy<Value = Direction> {
    prop_oneof![
        Just(Direction::Out),
        Just(Direction::In),
        Just(Direction::Both)
    ]
}

type Row = (u32, u32, u64, f64, i64, i64);

fn rows(e: &EdgeRows) -> Vec<Row> {
    let (w, f, l) = (e.weight.as_ref(), e.first.as_ref(), e.last.as_ref());
    (0..e.len())
        .map(|i| {
            (
                e.source[i],
                e.target[i],
                e.edges[i],
                w.unwrap()[i],
                f.unwrap()[i],
                l.unwrap()[i],
            )
        })
        .collect()
}

fn row(s: u32, t: u32, a: Agg) -> Row {
    (s, t, a.edges, a.weight, a.first, a.last)
}

/// The neighbours of `v` in `direction`, as the specification lists them.
fn near(m: &Model, v: u32, d: Direction) -> Vec<(u32, Agg)> {
    match d {
        _ if !m.directed => m.list(v, false),
        Direction::Out => m.list(v, false),
        Direction::In => m.list(v, true),
        Direction::Both => m.both(v),
    }
}

fn limit_cut(limit: u64, rows: usize, total: Option<u64>, hop: Option<u32>) -> Truncated {
    Truncated {
        by: Cut::Limit,
        limit,
        rows: rows as u64,
        hop,
        total,
    }
}

struct Neighbors {
    nodes: Vec<u32>,
    direction: Direction,
    limit: u64,
}

impl Query for Neighbors {
    type Out = (Vec<Row>, Option<Truncated>);
    fn run<S: Source>(&self, g: &Graph<'_, S>) -> Self::Out {
        let mut r = g.reader(&Memory::unlimited()).unwrap();
        let e = r
            .neighbors(&self.nodes, self.direction, self.limit)
            .unwrap();
        (rows(&e), e.truncated)
    }
}

fn neighbors_of(m: &Model, q: &Neighbors) -> (Vec<Row>, Option<Truncated>) {
    let (mut out, mut cut, mut done) = (Vec::new(), false, BTreeSet::new());
    for &v in &q.nodes {
        if !done.insert(v) {
            continue;
        }
        let list = near(m, v, q.direction);
        cut |= list.len() as u64 > q.limit;
        out.extend(
            list.into_iter()
                .take(q.limit as usize)
                .map(|(u, a)| row(v, u, a)),
        );
    }
    let total = match q.nodes[..] {
        [v] if q.direction != Direction::Both || !m.directed => {
            Some(near(m, v, q.direction).len() as u64)
        }
        _ => None,
    };
    let truncated = cut.then(|| limit_cut(q.limit, out.len(), total, None));
    (out, truncated)
}

struct Khop {
    node: u32,
    k: u32,
    direction: Direction,
    limit: u64,
}

impl Query for Khop {
    type Out = (Vec<(u32, u32)>, Option<Truncated>);
    fn run<S: Source>(&self, g: &Graph<'_, S>) -> Self::Out {
        let mut r = g.reader(&Memory::unlimited()).unwrap();
        let h = r
            .khop(self.node, self.k, self.direction, self.limit, &Never)
            .unwrap();
        (hops(&h), h.truncated)
    }
}

fn hops(h: &Hops) -> Vec<(u32, u32)> {
    h.node
        .iter()
        .copied()
        .zip(h.distance.iter().copied())
        .collect()
}

/// Every node within `k` hops, ordered as § 2.12 says: by distance, then by the
/// position of the node's parent — the first node of the previous distance, in that
/// distance's order, with an edge to it — then by its position in the parent's list.
fn khop_order(m: &Model, q: &Khop) -> Vec<(u32, u32)> {
    let lists = |u| -> Vec<u32> {
        near(m, u, q.direction)
            .into_iter()
            .map(|(x, _)| x)
            .collect()
    };
    let mut dist = HashMap::from([(q.node, 0)]);
    let mut queue = VecDeque::from([q.node]);
    while let Some(u) = queue.pop_front() {
        let d = dist[&u];
        for x in lists(u) {
            if d < q.k && !dist.contains_key(&x) {
                dist.insert(x, d + 1);
                queue.push_back(x);
            }
        }
    }
    let mut levels = vec![vec![q.node]];
    for d in 1..=q.k {
        let previous = &levels[d as usize - 1];
        let mut level: Vec<(usize, usize, u32)> = dist
            .iter()
            .filter(|&(_, &e)| e == d)
            .map(|(&x, _)| {
                let (p, parent) = previous
                    .iter()
                    .enumerate()
                    .find(|(_, p)| lists(**p).contains(&x))
                    .unwrap();
                (p, lists(*parent).iter().position(|&y| y == x).unwrap(), x)
            })
            .collect();
        if level.is_empty() {
            break;
        }
        level.sort_unstable();
        levels.push(level.into_iter().map(|(_, _, x)| x).collect());
    }
    levels
        .iter()
        .enumerate()
        .flat_map(|(d, l)| l.iter().map(move |&x| (x, d as u32)))
        .collect()
}

fn khop_of(m: &Model, q: &Khop) -> (Vec<(u32, u32)>, Option<Truncated>) {
    let mut all = khop_order(m, q);
    let truncated = (all.len() as u64 > q.limit).then(|| {
        let hop = all[q.limit as usize].1;
        limit_cut(q.limit, q.limit as usize, None, Some(hop))
    });
    all.truncate(q.limit as usize);
    (all, truncated)
}

struct Pairs {
    a: Vec<u32>,
    b: Vec<u32>,
    limit: u64,
}

impl Query for Pairs {
    type Out = (
        Vec<Row>,
        Option<Truncated>,
        Vec<bool>,
        Vec<(u32, u64, u64)>,
        Option<Truncated>,
    );
    fn run<S: Source>(&self, g: &Graph<'_, S>) -> Self::Out {
        let mut r = g.reader(&Memory::unlimited()).unwrap();
        let e = r.edges_between(&self.a, &self.b, self.limit).unwrap();
        let (x, y) = (self.a[0], self.b[0]);
        let common: Common = r.common_neighbors(x, y, self.limit).unwrap();
        let n = self.a.len().min(self.b.len());
        let has = r.has_edges(&self.a[..n], &self.b[..n]).unwrap();
        assert_eq!(r.has_edge(x, y).unwrap(), has[0]);
        let c = (0..common.neighbor.len())
            .map(|i| (common.neighbor[i], common.edges_a[i], common.edges_b[i]))
            .collect();
        (
            rows(&e),
            e.truncated,
            has.iter().copied().collect(),
            c,
            common.truncated,
        )
    }
}

fn pairs_of(m: &Model, q: &Pairs) -> <Pairs as Query>::Out {
    let (a, b): (BTreeSet<u32>, BTreeSet<u32>) =
        (q.a.iter().copied().collect(), q.b.iter().copied().collect());
    let all: Vec<Row> = m
        .out
        .iter()
        .filter(|&(&(u, t), _)| {
            ((a.contains(&u) && b.contains(&t)) || (b.contains(&u) && a.contains(&t)))
                && (m.directed || u <= t)
        })
        .map(|(&(u, t), &g)| row(u, t, g))
        .collect();
    let between =
        (all.len() as u64 > q.limit).then(|| limit_cut(q.limit, q.limit as usize, None, None));
    let n = q.a.len().min(q.b.len());
    let has = (0..n)
        .map(|i| m.out.contains_key(&(q.a[i], q.b[i])))
        .collect();
    let (x, y) = (q.a[0], q.b[0]);
    let (nx, ny) = (near(m, x, Direction::Both), near(m, y, Direction::Both));
    let common: Vec<(u32, u64, u64)> = nx
        .iter()
        .filter_map(|&(u, ax)| {
            let ay = ny.iter().find(|(w, _)| *w == u)?.1;
            (u != x && u != y).then_some((u, ax.edges, ay.edges))
        })
        .collect();
    let cut =
        (common.len() as u64 > q.limit).then(|| limit_cut(q.limit, q.limit as usize, None, None));
    let (mut all, mut common) = (all, common);
    all.truncate(q.limit as usize);
    common.truncate(q.limit as usize);
    (all, between, has, common, cut)
}

/// `degrees`: the columns that the graph has, row by row.
struct Degrees(Vec<u32>);

impl Query for Degrees {
    type Out = Vec<(u32, Option<u32>, Option<u32>, u32, f64)>;
    fn run<S: Source>(&self, g: &Graph<'_, S>) -> Self::Out {
        let mut r = g.reader(&Memory::unlimited()).unwrap();
        let d = r.degrees(&self.0).unwrap();
        (0..d.node.len())
            .map(|i| {
                (
                    d.node[i],
                    d.in_degree.as_ref().map(|c| c[i]),
                    d.out_degree.as_ref().map(|c| c[i]),
                    d.degree[i],
                    d.weighted.as_ref().unwrap()[i],
                )
            })
            .collect()
    }
}

fn degrees_of(m: &Model, nodes: &[u32]) -> <Degrees as Query>::Out {
    let sum = |l: &[(u32, Agg)], v: u32, own: f64| {
        l.iter()
            .map(|&(u, a)| if u == v { own * a.weight } else { a.weight })
            .sum::<f64>()
    };
    nodes
        .iter()
        .map(|&v| {
            let (out, inn) = (m.list(v, false), m.list(v, true));
            let looped = u32::from(out.iter().any(|&(u, _)| u == v));
            if m.directed {
                let degree = m.both(v).len() as u32 + looped;
                let w = sum(&out, v, 1.0) + sum(&inn, v, 1.0);
                (v, Some(inn.len() as u32), Some(out.len() as u32), degree, w)
            } else {
                (v, None, None, out.len() as u32 + looped, sum(&out, v, 2.0))
            }
        })
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// One reader and one set of rows reused for a run of queries answer each as a
    /// fresh reader does: what a query keeps for the next changes no answer.
    #[test]
    fn a_reused_reader_answers_as_a_fresh_one(
        m in hub_graphs(),
        asks in proptest::collection::vec((0_u32..400, 0_u32..4, directions(), 1_u64..300), 1..12),
    ) {
        let f = Fixture::new(m);
        let (out, inn) = (f.out.heap(), f.inn.as_ref().map(support::Stored::heap));
        let g = f.graph(&out, inn.as_ref());
        let memory = Memory::unlimited();
        let mut r = g.reader(&memory).unwrap();
        let (mut edges, mut ball) = (r.rows(), Hops::new(&memory));
        for (pick, k, direction, limit) in asks {
            let node = pick % f.model.n;
            let fresh = Khop { node, k, direction, limit }.run(&g);
            r.khop_into(node, k, direction, limit, &Never, &mut ball).unwrap();
            prop_assert_eq!((hops(&ball), ball.truncated), fresh);
            let nodes = [node, 0, 1, node];
            let fresh = Neighbors { nodes: nodes.to_vec(), direction, limit }.run(&g);
            r.neighbors_into(&nodes, direction, limit, &mut edges).unwrap();
            prop_assert_eq!((rows(&edges), edges.truncated), fresh);
        }
    }

    #[test]
    fn neighbors_follow_the_model(
        m in graphs(),
        picks in proptest::collection::vec(any::<u32>(), 1..50),
        direction in directions(),
        limit in 1_u64..12,
    ) {
        let nodes: Vec<u32> = picks.iter().map(|p| p % m.n).collect();
        let f = Fixture::new(m);
        let q = Neighbors { nodes, direction, limit };
        prop_assert_eq!(on_every_source(&f, &q), neighbors_of(&f.model, &q));
        let degrees = Degrees(q.nodes.clone());
        prop_assert_eq!(on_every_source(&f, &degrees), degrees_of(&f.model, &q.nodes));
    }

    #[test]
    fn khop_follows_the_order_of_the_specification(
        m in graphs(),
        pick in any::<u32>(),
        k in 0_u32..4,
        direction in directions(),
        limit in 1_u64..30,
    ) {
        let f = Fixture::new(m);
        let q = Khop { node: pick % f.model.n, k, direction, limit };
        let got = on_every_source(&f, &q);
        prop_assert_eq!(&got, &khop_of(&f.model, &q));
        // A khop cut at its limit gives the same rows at any thread count.
        for t in [1, 4, 16] {
            let at = par::with_threads(NonZeroUsize::new(t).unwrap(), || {
                q.run(&f.graph(&f.out.heap(), f.inn.as_ref().map(support::Stored::heap).as_ref()))
            })
            .unwrap();
            prop_assert_eq!(&at, &got);
        }
    }

    #[test]
    fn khop_on_hubs_follows_the_order_of_the_specification(
        m in hub_graphs(),
        pick in 0_u32..3,
        k in 1_u32..4,
        direction in directions(),
        limit in 1_u64..400,
    ) {
        let f = Fixture::new(m);
        let q = Khop { node: pick, k, direction, limit };
        prop_assert_eq!(on_every_source(&f, &q), khop_of(&f.model, &q));
        let n = Neighbors { nodes: vec![0, 1, pick], direction, limit: limit / 4 + 1 };
        prop_assert_eq!(on_every_source(&f, &n), neighbors_of(&f.model, &n));
    }

    #[test]
    fn pairs_follow_the_model(
        m in graphs(),
        a in proptest::collection::vec(any::<u32>(), 1..6),
        b in proptest::collection::vec(any::<u32>(), 1..6),
        limit in 1_u64..20,
    ) {
        let n = m.n;
        let q = Pairs {
            a: a.iter().map(|x| x % n).collect(),
            b: b.iter().map(|x| x % n).collect(),
            limit,
        };
        let f = Fixture::new(m);
        prop_assert_eq!(on_every_source(&f, &q), pairs_of(&f.model, &q));
    }

    #[test]
    fn a_timeout_returns_a_prefix_of_the_rows(
        m in graphs(),
        pick in any::<u32>(),
        after in 0_u32..6,
    ) {
        let f = Fixture::new(m);
        let q = Khop { node: pick % f.model.n, k: 3, direction: Direction::Both, limit: 1000 };
        let (all, _) = khop_of(&f.model, &q);
        let source = f.out.heap();
        let inn = f.inn.as_ref().map(support::Stored::heap);
        let g = f.graph(&source, inn.as_ref());
        let asked = Cell::new(0);
        let deadline = || {
            asked.set(asked.get() + 1);
            asked.get() > after
        };
        let h = g.reader(&Memory::unlimited()).unwrap().khop(q.node, q.k, q.direction, q.limit, &deadline).unwrap();
        let got = hops(&h);
        prop_assert_eq!(&got[..], &all[..got.len()]);
        match h.truncated {
            Some(t) => prop_assert!(t.by == Cut::Timeout && t.rows == got.len() as u64),
            None => prop_assert_eq!(got.len(), all.len()),
        }
    }
}

#[test]
fn queries_refuse_what_they_cannot_do() {
    let m = Model::new(3, true, &[(0, 1, 1.0, 0), (1, 1, 1.0, 0)]);
    let f = Fixture::new(m);
    let out = f.out.heap();
    let g = Graph::directed(&out, None).unwrap();
    let mut r = g.reader(&Memory::unlimited()).unwrap();
    assert!(matches!(
        r.neighbors(&[0], Direction::In, 5),
        Err(QueryError::Unsupported(_))
    ));
    assert!(matches!(
        r.common_neighbors(0, 1, 5),
        Err(QueryError::Unsupported(_))
    ));
    assert!(matches!(
        r.neighbors(&[3], Direction::Out, 5),
        Err(QueryError::UnknownNode(3))
    ));
    assert!(matches!(
        r.neighbors(&[0], Direction::Out, 0),
        Err(QueryError::Invalid(_))
    ));
    assert!(matches!(
        r.khop(7, 1, Direction::Out, 5, &Never),
        Err(QueryError::UnknownNode(7))
    ));
    assert!(matches!(
        r.has_edges(&[0], &[]),
        Err(QueryError::Invalid(_))
    ));
    // A loop: `has_edge(a, a)`, and `v` its own neighbour once.
    assert!(r.has_edge(1, 1).unwrap() && !r.has_edge(0, 0).unwrap());
    let e = r.neighbors(&[1], Direction::Out, 5).unwrap();
    assert_eq!(e.target.iter().copied().collect::<Vec<_>>(), [1]);
    let short = vec![1_u64; 1];
    let bad = everygraph_query::Columns {
        edges: Some(&short),
        ..Default::default()
    };
    assert!(matches!(
        Graph::undirected(&out).with_columns(bad, bad),
        Err(QueryError::Invalid(_))
    ));
}
