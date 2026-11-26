// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `common_neighbors` and `edges_between` (`docs/api-sketch.md` § 3.2).

use everygraph_core::{Memory, SegVec, usize_of, widen};
use everygraph_kernels::edge_map::{Source, one_segment, one_segment_mut};

use crate::graph::{Shape, column};
use crate::neighbors::EdgeRows;
use crate::{Cut, Direction, QueryError, Reader, Truncated, rows_of};

/// `common_neighbors(a, b)`: `neighbor`, `edges_a`, `edges_b`.
#[derive(Debug)]
pub struct Common {
    /// The common neighbours, in graph order.
    pub neighbor: SegVec<u32>,
    /// The events joining each to `a`, both directions.
    pub edges_a: SegVec<u64>,
    /// The events joining each to `b`, both directions.
    pub edges_b: SegVec<u64>,
    /// Whether, and how, the result was cut short.
    pub truncated: Option<Truncated>,
}

fn cut_at(limit: u64, rows: usize) -> Option<Truncated> {
    Some(Truncated {
        by: Cut::Limit,
        limit,
        rows: widen(rows),
        hop: None,
        total: None,
    })
}

impl<S: Source> Reader<'_, '_, S> {
    /// The neighbours that `a` and `b` share in the undirected projection, in graph
    /// order, at most `limit`; never `a` or `b` themselves.
    ///
    /// # Errors
    ///
    /// [`QueryError::UnknownNode`], [`QueryError::Invalid`] for a `limit` of 0,
    /// [`QueryError::Unsupported`] on a directed graph without in-lists,
    /// [`QueryError::Budget`] before any read, [`QueryError::Lists`].
    pub fn common_neighbors(&mut self, a: u32, b: u32, limit: u64) -> Result<Common, QueryError> {
        let k = rows_of(limit)?;
        let g = self.graph;
        let (a, b) = (g.node(a)?, g.node(b)?);
        let direction = g.lists(Direction::Both)?;
        let (da, db) = (g.degree(a, direction), g.degree(b, direction));
        let rows = u64::from(da.min(db)).min(limit);
        let state = 12 * u64::from(da) + 16 * rows;
        self.admit(state, direction, u64::from(da.max(db)))?;
        let memory = self.memory.clone();
        let rows = usize::try_from(rows).unwrap_or(usize::MAX);
        let mut out = Common {
            neighbor: column(&memory, rows)?,
            edges_a: column(&memory, rows)?,
            edges_b: column(&memory, rows)?,
            truncated: None,
        };
        let (mut near, mut events) = (
            column(&memory, usize_of(da))?,
            SegVec::new(&memory, "point query rows"),
        );
        let mut cut = false;
        let mut keep = |u, x, y, out: &mut Common| -> Result<(), QueryError> {
            if u == a || u == b {
                return Ok(());
            }
            if out.neighbor.len() == k {
                cut = true;
                return Ok(());
            }
            out.neighbor.try_push(u)?;
            out.edges_a.try_push(x)?;
            Ok(out.edges_b.try_push(y)?)
        };
        if direction == Direction::Out {
            // One adjacency: the two lists, merged as slices.
            let (first_a, list) = self.list(a, direction)?;
            near.try_extend_from_slice(list)?;
            let near = one_segment(&near)?;
            let (first_b, list) = self.list(b, direction)?;
            let mut i = 0;
            for (e, &u) in (first_b..).zip(list) {
                while near.get(i).is_some_and(|&x| x < u) {
                    i += 1;
                }
                if near.get(i) == Some(&u) {
                    let x = g.value(false, first_a + widen(i), Shape::default())?.edges;
                    let y = g.value(false, e, Shape::default())?.edges;
                    keep(u, x, y, &mut out)?;
                }
            }
        } else {
            events.try_reserve(usize_of(da))?;
            self.gather(a, direction, usize::MAX, Shape::default(), |u, value| {
                near.try_push(u)?;
                Ok(events.try_push(value.edges)?)
            })?;
            let (near, events) = (one_segment(&near)?, one_segment(&events)?);
            let mut i = 0;
            self.gather(b, direction, usize::MAX, Shape::default(), |u, value| {
                while near.get(i).is_some_and(|&x| x < u) {
                    i += 1;
                }
                if near.get(i) == Some(&u) {
                    keep(u, events[i], value.edges, &mut out)?;
                }
                Ok(())
            })?;
        }
        if cut {
            out.truncated = cut_at(limit, out.neighbor.len());
        }
        Ok(out)
    }

    /// The pairs with one end in `a` and the other in `b`: one row per pair, as stored
    /// on a directed graph (`source → target`), once with `source ≤ target` on an
    /// undirected one; a kept loop at `v` when `v` is in both sets. By `source`, then
    /// `target`, at most `limit`.
    ///
    /// # Errors
    ///
    /// [`QueryError::UnknownNode`], [`QueryError::Invalid`] for a `limit` of 0,
    /// [`QueryError::Budget`] before any read, [`QueryError::Lists`].
    pub fn edges_between(
        &mut self,
        a: &[u32],
        b: &[u32],
        limit: u64,
    ) -> Result<EdgeRows, QueryError> {
        let k = rows_of(limit)?;
        let g = self.graph;
        for &v in a.iter().chain(b) {
            g.node(v)?;
        }
        let sets = 8 * widen(a.len() + b.len());
        let (mut rows, mut longest) = (0_u64, 0);
        for &v in a.iter().chain(b) {
            let d = g.degree(v, Direction::Out);
            rows = rows.saturating_add(u64::from(d));
            longest = longest.max(d);
        }
        let rows = rows.min(limit);
        let shape = g.shape();
        self.admit(
            sets + rows * shape.row_bytes(),
            Direction::Out,
            u64::from(longest),
        )?;
        let (a, b) = (sorted(&self.memory, a)?, sorted(&self.memory, b)?);
        let (a, b) = (one_segment(&a)?, one_segment(&b)?);
        let has = |set: &[u32], v: u32| set.binary_search(&v).is_ok();
        let mut out = EdgeRows::new(&self.memory, shape, usize::try_from(rows).unwrap_or(0))?;
        let (mut i, mut j) = (0, 0);
        while i < a.len() || j < b.len() {
            let u = a
                .get(i)
                .copied()
                .unwrap_or(u32::MAX)
                .min(b.get(j).copied().unwrap_or(u32::MAX));
            let (in_a, in_b) = (a.get(i) == Some(&u), b.get(j) == Some(&u));
            i += usize::from(in_a);
            j += usize::from(in_b);
            let (first, list) = self.list(u, Direction::Out)?;
            for (e, &t) in (first..).zip(list) {
                let keep = (in_a && has(b, t)) || (in_b && has(a, t));
                if !keep || (!g.is_directed() && t < u) {
                    continue;
                }
                if out.len() == k {
                    out.truncated = cut_at(limit, k);
                    return Ok(out);
                }
                out.push(u, t, g.value(false, e, shape)?)?;
            }
        }
        Ok(out)
    }
}

/// `nodes`, sorted, each once.
fn sorted(memory: &Memory, nodes: &[u32]) -> Result<SegVec<u32>, QueryError> {
    let mut set = SegVec::new(memory, "point query sets");
    set.try_extend_from_slice(nodes)?;
    let s = one_segment_mut(&mut set)?;
    s.sort_unstable();
    let mut len = 0;
    for i in 0..s.len() {
        if i == 0 || s[i] != s[len - 1] {
            s[len] = s[i];
            len += 1;
        }
    }
    set.truncate(len);
    Ok(set)
}
