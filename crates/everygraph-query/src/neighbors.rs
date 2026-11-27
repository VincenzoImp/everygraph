// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `neighbors`, `degrees` and `has_edge` (`docs/api-sketch.md` § 3.2).

use everygraph_core::{Memory, SegVec, widen};
use everygraph_kernels::edge_map::Source;

use crate::graph::{Shape, Union, Value, column};
use crate::seen::Seen;
use crate::{Cut, Direction, QueryError, Reader, Truncated, rows_of};

/// Rows of pairs: the result of `neighbors`, whose columns are `node` (`source`) and
/// `neighbor` (`target`), and of `edges_between`. Each row is one pair, with the
/// columns the graph has (`docs/api-sketch.md` § 3.4).
#[derive(Debug)]
pub struct EdgeRows {
    /// The node asked about, or the pair's source.
    pub source: SegVec<u32>,
    /// The neighbour, or the pair's target.
    pub target: SegVec<u32>,
    /// The pair's events.
    pub edges: SegVec<u64>,
    /// The sum of the pair's weights, when the graph has a weight.
    pub weight: Option<SegVec<f64>>,
    /// The pair's first time, when the graph has a time.
    pub first: Option<SegVec<i64>>,
    /// The pair's last time.
    pub last: Option<SegVec<i64>>,
    /// Whether, and how, the result was cut short.
    pub truncated: Option<Truncated>,
}

const ROWS: &str = "point query rows";

impl EdgeRows {
    /// No rows, of `shape`.
    pub(crate) fn empty(memory: &Memory, shape: Shape) -> Self {
        Self {
            source: SegVec::new(memory, ROWS),
            target: SegVec::new(memory, ROWS),
            edges: SegVec::new(memory, ROWS),
            weight: shape.weight.then(|| SegVec::new(memory, ROWS)),
            first: shape.time.then(|| SegVec::new(memory, ROWS)),
            last: shape.time.then(|| SegVec::new(memory, ROWS)),
            truncated: None,
        }
    }

    /// Empty rows with room for `rows` of `shape`.
    pub(crate) fn new(memory: &Memory, shape: Shape, rows: usize) -> Result<Self, QueryError> {
        let mut out = Self::empty(memory, shape);
        out.reserve(rows)?;
        Ok(out)
    }

    fn shape(&self) -> Shape {
        Shape {
            weight: self.weight.is_some(),
            time: self.first.is_some(),
        }
    }

    /// Empties the rows; the memory is kept.
    fn clear(&mut self) {
        self.source.clear();
        self.target.clear();
        self.edges.clear();
        if let Some(c) = &mut self.weight {
            c.clear();
        }
        for c in [&mut self.first, &mut self.last].into_iter().flatten() {
            c.clear();
        }
        self.truncated = None;
    }

    /// Makes room for `rows` rows.
    fn reserve(&mut self, rows: usize) -> Result<(), QueryError> {
        let more = rows.saturating_sub(self.len());
        self.source.try_reserve(more)?;
        self.target.try_reserve(more)?;
        self.edges.try_reserve(more)?;
        if let Some(c) = &mut self.weight {
            c.try_reserve(more)?;
        }
        for c in [&mut self.first, &mut self.last].into_iter().flatten() {
            c.try_reserve(more)?;
        }
        Ok(())
    }

    /// Rows.
    #[must_use]
    pub fn len(&self) -> usize {
        self.source.len()
    }

    /// Whether there are no rows.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.source.is_empty()
    }

    pub(crate) fn push(&mut self, source: u32, target: u32, v: Value) -> Result<(), QueryError> {
        self.source.try_push(source)?;
        self.target.try_push(target)?;
        self.edges.try_push(v.edges)?;
        if let Some(w) = &mut self.weight {
            w.try_push(v.weight)?;
        }
        if let (Some(f), Some(l)) = (&mut self.first, &mut self.last) {
            f.try_push(v.first)?;
            l.try_push(v.last)?;
        }
        Ok(())
    }
}

/// `degrees(nodes)`: one row per node given, in that order (`docs/api-sketch.md` § 3.2,
/// `docs/spec/graph-semantics.md` § 6).
#[derive(Debug)]
pub struct Degrees {
    /// The nodes.
    pub node: SegVec<u32>,
    /// Distinct in-neighbours, a kept loop counted once; directed graphs only.
    pub in_degree: Option<SegVec<u32>>,
    /// Distinct out-neighbours, a kept loop counted once; directed graphs only.
    pub out_degree: Option<SegVec<u32>>,
    /// Distinct neighbours in either direction, a kept loop counted twice.
    pub degree: SegVec<u32>,
    /// The sum of the weight over the pairs, a kept loop's twice, when the graph has a
    /// weight: as `everygraph_kernels::degrees::strengths` sums it.
    pub weighted: Option<SegVec<f64>>,
}

impl<S: Source> Reader<'_, '_, S> {
    /// The neighbours of each node of `nodes` in `direction`, in graph order: one row
    /// per distinct neighbour, at most `limit` per node. A node given twice is answered
    /// once, at its first place. A node with a kept loop lists itself once. Under
    /// `Both`, a pair's two directions are one row: the sums of both.
    ///
    /// # Errors
    ///
    /// [`QueryError::UnknownNode`], [`QueryError::Invalid`] for a `limit` of 0,
    /// [`QueryError::Unsupported`] for `In` or `Both` without in-lists,
    /// [`QueryError::Budget`] before any read, [`QueryError::Lists`].
    pub fn neighbors(
        &mut self,
        nodes: &[u32],
        direction: Direction,
        limit: u64,
    ) -> Result<EdgeRows, QueryError> {
        let mut out = self.rows();
        self.neighbors_into(nodes, direction, limit, &mut out)?;
        Ok(out)
    }

    /// No rows, with the columns of this graph: for
    /// [`neighbors_into`](Self::neighbors_into).
    #[must_use]
    pub fn rows(&self) -> EdgeRows {
        EdgeRows::empty(&self.memory, self.graph.shape())
    }

    /// [`neighbors`](Self::neighbors) into `out`, whose rows it replaces and whose memory
    /// it reuses: many queries on one thread then allocate only when a result grows.
    ///
    /// # Errors
    ///
    /// As [`neighbors`](Self::neighbors); [`QueryError::Invalid`] for rows made for
    /// another graph's columns.
    pub fn neighbors_into(
        &mut self,
        nodes: &[u32],
        direction: Direction,
        limit: u64,
        out: &mut EdgeRows,
    ) -> Result<(), QueryError> {
        let k = rows_of(limit)?;
        let g = self.graph;
        let direction = g.lists(direction)?;
        let shape = g.shape();
        if out.shape() != shape {
            return Err(QueryError::Invalid("rows made for another graph's columns"));
        }
        let (mut rows, mut longest, mut prefix) = (0_u64, 0_u64, 0_u64);
        for &v in nodes {
            let d = u64::from(g.degree(g.node(v)?, direction));
            rows = rows.saturating_add(d.min(limit));
            longest = longest.max(d);
            prefix = prefix.max(d.min(limit));
        }
        // A few nodes are told apart by comparing them, which allocates nothing.
        let many = nodes.len() > 32;
        let seen = if many {
            Seen::bytes(g.nodes(), nodes.len())
        } else {
            0
        };
        self.admit(
            rows.saturating_mul(shape.row_bytes()) + seen,
            direction,
            longest,
            prefix,
        )?;
        out.clear();
        out.reserve(usize::try_from(rows).unwrap_or(usize::MAX))?;
        let mut cut = false;
        if many {
            self.with_seen(nodes.len(), |r, seen| {
                for &v in nodes {
                    if seen.insert(v)? {
                        cut |= r.gather(v, direction, k, shape, |u, x| out.push(v, u, x))?;
                    }
                }
                Ok(())
            })?;
        } else {
            for (i, &v) in nodes.iter().enumerate() {
                if !nodes[..i].contains(&v) {
                    cut |= self.gather(v, direction, k, shape, |u, x| out.push(v, u, x))?;
                }
            }
        }
        let total = match nodes {
            [v] if direction != Direction::Both => Some(u64::from(g.degree(*v, direction))),
            _ => None,
        };
        out.truncated = cut.then(|| Truncated {
            by: Cut::Limit,
            limit,
            rows: widen(out.len()),
            hop: None,
            total,
        });
        Ok(())
    }

    /// The degrees of each node of `nodes`, in that order.
    ///
    /// # Errors
    ///
    /// [`QueryError::UnknownNode`]; [`QueryError::Unsupported`] on a directed graph
    /// without in-lists; [`QueryError::Budget`] before any read; [`QueryError::Lists`].
    pub fn degrees(&mut self, nodes: &[u32]) -> Result<Degrees, QueryError> {
        let g = self.graph;
        let directed = g.is_directed();
        let direction = g.lists(if directed {
            Direction::Both
        } else {
            Direction::Out
        })?;
        for &v in nodes {
            g.node(v)?;
        }
        let columns = 2 + 2 * u64::from(directed) + 2 * u64::from(g.has_weight());
        let state = widen(nodes.len()).saturating_mul(4 * columns);
        self.admit(state, direction, u64::from(g.longest(direction)), u64::MAX)?;
        let (memory, rows) = (&self.memory, nodes.len());
        let mut out = Degrees {
            node: column(memory, rows)?,
            in_degree: directed.then(|| column(memory, rows)).transpose()?,
            out_degree: directed.then(|| column(memory, rows)).transpose()?,
            degree: column(memory, rows)?,
            weighted: g.has_weight().then(|| column(memory, rows)).transpose()?,
        };
        for &v in nodes {
            let (degree, weighted) = if directed {
                self.directed_degree(v, &mut out)?
            } else {
                self.undirected_degree(v)?
            };
            out.node.try_push(v)?;
            out.degree.try_push(degree)?;
            if let Some(w) = &mut out.weighted {
                w.try_push(weighted)?;
            }
        }
        Ok(out)
    }

    /// The degree and strength of `v` on an undirected graph.
    fn undirected_degree(&mut self, v: u32) -> Result<(u32, f64), QueryError> {
        let g = self.graph;
        let (first, list) = self.list(v, Direction::Out)?;
        let (mut loops, mut sum) = (0, 0.0);
        for (e, &u) in (first..).zip(list) {
            let w = g.weight(false, e);
            if u == v {
                loops += 1;
                sum += 2.0 * w;
            } else {
                sum += w;
            }
        }
        Ok((u32::try_from(list.len() + loops).unwrap_or(u32::MAX), sum))
    }

    /// The degree and strength of `v` on a directed graph; its in- and out-degree go
    /// to `out`.
    fn directed_degree(&mut self, v: u32, out: &mut Degrees) -> Result<(u32, f64), QueryError> {
        let g = self.graph;
        let [(fo, lo), (fi, li)] = self.both(v)?;
        let strength = |inn, first: u64, list: &[u32]| {
            (first..first + widen(list.len())).fold(0.0, |sum, e| sum + g.weight(inn, e))
        };
        let weighted = strength(false, fo, lo) + strength(true, fi, li);
        let union = Union::new(lo, li, v).count();
        let looped = usize::from(lo.binary_search(&v).is_ok());
        let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        if let (Some(o), Some(n)) = (&mut out.out_degree, &mut out.in_degree) {
            o.try_push(count(lo.len()))?;
            n.try_push(count(li.len()))?;
        }
        Ok((count(union + looped), weighted))
    }

    /// Whether `source` has an edge to `target` (either way on an undirected graph);
    /// `has_edge(a, a)` is true exactly when `a` has a kept loop. Reads the shorter of
    /// the two lists that can tell.
    ///
    /// # Errors
    ///
    /// [`QueryError::UnknownNode`]; [`QueryError::Budget`] before any read;
    /// [`QueryError::Lists`].
    pub fn has_edge(&mut self, source: u32, target: u32) -> Result<bool, QueryError> {
        let got = self.has_edges(&[source], &[target])?;
        Ok(got.get(0).copied().unwrap_or(false))
    }

    /// [`has_edge`](Self::has_edge) of each pair `(sources[i], targets[i])`, in that
    /// order.
    ///
    /// # Errors
    ///
    /// As [`has_edge`](Self::has_edge); [`QueryError::Invalid`] for columns of different
    /// lengths.
    pub fn has_edges(
        &mut self,
        sources: &[u32],
        targets: &[u32],
    ) -> Result<SegVec<bool>, QueryError> {
        if sources.len() != targets.len() {
            return Err(QueryError::Invalid(
                "the source and target columns differ in length",
            ));
        }
        let g = self.graph;
        let mut longest = 0;
        for (&a, &b) in sources.iter().zip(targets) {
            let (read, _, direction) = self.side_for(g.node(a)?, g.node(b)?);
            longest = longest.max(g.degree(read, direction));
        }
        let both = if g.is_directed() {
            Direction::Both
        } else {
            Direction::Out
        };
        self.admit(widen(sources.len()), both, u64::from(longest), u64::MAX)?;
        let mut out = column(&self.memory, sources.len())?;
        for (&a, &b) in sources.iter().zip(targets) {
            let (read, find, direction) = self.side_for(a, b);
            let (_, list) = self.list(read, direction)?;
            out.try_push(list.binary_search(&find).is_ok())?;
        }
        Ok(out)
    }

    /// The list to read and the node to find in it for `has_edge(a, b)`: the shorter one.
    fn side_for(&self, a: u32, b: u32) -> (u32, u32, Direction) {
        let g = self.graph;
        let out = g.degree(a, Direction::Out);
        if !g.is_directed() {
            return if g.degree(b, Direction::Out) < out {
                (b, a, Direction::Out)
            } else {
                (a, b, Direction::Out)
            };
        }
        match g.lists(Direction::In) {
            Ok(_) if g.degree(b, Direction::In) < out => (b, a, Direction::In),
            _ => (a, b, Direction::Out),
        }
    }
}
