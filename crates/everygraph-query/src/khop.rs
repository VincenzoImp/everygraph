// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `khop` (`docs/api-sketch.md` § 3.2, the order of § 2.12).

use everygraph_core::{Memory, SegVec, usize_of, widen};
use everygraph_kernels::edge_map::Source;

use crate::seen::Seen;
use crate::{Cut, Deadline, Direction, QueryError, Reader, Truncated, rows_of};

/// The nodes within `k` hops of a node, nearest first: `node`, `distance`.
#[derive(Debug)]
pub struct Hops {
    /// The nodes.
    pub node: SegVec<u32>,
    /// Their distance from the start.
    pub distance: SegVec<u32>,
    /// Whether, and how, the result was cut short.
    pub truncated: Option<Truncated>,
}

/// The fewest entries of a list decoded at once: a smaller prefix costs about the same.
const MIN_PREFIX: usize = 32;
/// Entries between two looks at the deadline inside one list.
const DEADLINE_EVERY: usize = 4096;

impl Hops {
    /// No rows, held under `memory`: for [`Reader::khop_into`].
    #[must_use]
    pub fn new(memory: &Memory) -> Self {
        Self {
            node: SegVec::new(memory, "point query rows"),
            distance: SegVec::new(memory, "point query rows"),
            truncated: None,
        }
    }
}

impl<S: Source> Reader<'_, '_, S> {
    /// The nodes within `k` hops of `node` in `direction`, with their distance: by
    /// distance, then in discovery order — the position of the node's parent in the
    /// previous distance's order, then the node's position in that parent's list. The
    /// traversal stops as soon as `limit` rows exist and one more node is found, so its
    /// cost is bounded by what it returns: a list is decoded in prefixes, doubling, only
    /// as far as needed. `deadline` is asked between lists and every 4,096 entries.
    ///
    /// # Errors
    ///
    /// [`QueryError::UnknownNode`], [`QueryError::Invalid`] for a `limit` of 0,
    /// [`QueryError::Unsupported`] for `In` or `Both` without in-lists,
    /// [`QueryError::Budget`] before any read, [`QueryError::Lists`].
    pub fn khop(
        &mut self,
        node: u32,
        k: u32,
        direction: Direction,
        limit: u64,
        deadline: &dyn Deadline,
    ) -> Result<Hops, QueryError> {
        let mut out = Hops::new(&self.memory);
        self.khop_into(node, k, direction, limit, deadline, &mut out)?;
        Ok(out)
    }

    /// [`khop`](Self::khop) into `out`, whose rows it replaces and whose memory it
    /// reuses: many queries on one thread then allocate only when a result grows.
    ///
    /// # Errors
    ///
    /// As [`khop`](Self::khop).
    pub fn khop_into(
        &mut self,
        node: u32,
        k: u32,
        direction: Direction,
        limit: u64,
        deadline: &dyn Deadline,
        out: &mut Hops,
    ) -> Result<(), QueryError> {
        let most = rows_of(limit)?;
        let g = self.graph;
        let (v, direction) = (g.node(node)?, g.lists(direction)?);
        let rows = most.min(usize_of(g.nodes()));
        let state = 8 * widen(rows) + Seen::bytes(g.nodes(), rows.saturating_add(1));
        self.admit(state, direction, u64::from(g.longest(direction)))?;
        out.node.clear();
        out.distance.clear();
        out.truncated = None;
        out.node.try_reserve(rows)?;
        out.distance.try_reserve(rows)?;
        self.with_seen(rows.saturating_add(1), |r, seen| {
            seen.insert(v)?;
            out.node.try_push(v)?;
            out.distance.try_push(0)?;
            let cut = r.traverse(k, direction, most, deadline, seen, out)?;
            out.truncated = cut.map(|t| Truncated { limit, ..t });
            Ok(())
        })
    }

    /// The hops after the first row of `out`; the cut, if any, its `limit` left to the
    /// caller.
    fn traverse(
        &mut self,
        k: u32,
        direction: Direction,
        most: usize,
        deadline: &dyn Deadline,
        seen: &mut Seen,
        out: &mut Hops,
    ) -> Result<Option<Truncated>, QueryError> {
        let cut = |by, d, out: &Hops| {
            Some(Truncated {
                by,
                limit: 0,
                rows: widen(out.node.len()),
                hop: Some(d),
                total: None,
            })
        };
        let (mut start, mut d) = (0, 0);
        while d < k && start < out.node.len() {
            let end = out.node.len();
            d += 1;
            for i in start..end {
                if deadline.passed() {
                    return Ok(cut(Cut::Timeout, d, out));
                }
                let parent = out.node[i];
                let (mut at, mut want) = (0, (most - out.node.len() + 1).max(MIN_PREFIX));
                loop {
                    let (list, whole) = self.prefix(parent, direction, want)?;
                    for (c, &u) in list.iter().enumerate().skip(at) {
                        if c % DEADLINE_EVERY == DEADLINE_EVERY - 1 && deadline.passed() {
                            return Ok(cut(Cut::Timeout, d, out));
                        }
                        if !seen.insert(u)? {
                            continue;
                        }
                        if out.node.len() == most {
                            return Ok(cut(Cut::Limit, d, out));
                        }
                        out.node.try_push(u)?;
                        out.distance.try_push(d)?;
                    }
                    if whole {
                        break;
                    }
                    at = list.len();
                    want = want.saturating_mul(2);
                }
            }
            start = end;
        }
        Ok(None)
    }
}
