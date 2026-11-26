// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Communities numbered by their smallest node, their members, and the quotient graph
//! (`docs/spec/graph-semantics.md` § 5.5, S6).
//!
//! The quotient has one node per community. Its entry between communities `a` and `b`
//! holds the sum of `A` over the pairs between them, and its diagonal entry the sum of
//! `A` inside `a`, kept loops included: twice the internal weight, as every loop is held
//! in this crate. So its total weight and the modularity of its singletons equal those
//! of the partition. A row lists its targets in the order its members' lists first name
//! them: a function of the input, so the quotient is the same array for every schedule.
//!
//! In-memory and member-ordered: a community's row is built at its smallest node, from
//! its members' lists read by random access. Out of core (#60) the same quotient comes
//! from one sequential pass into buckets (`lab/m1-02-ooc-leiden`, FINDINGS § M1-02); the
//! partition does not depend on which, because every sum is an integer and no step
//! depends on the order of a list.

use std::num::NonZeroUsize;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicU32, AtomicU64};

use everygraph_core::{Memory, par};
use everygraph_kernels::edge_map::{Csr, Error, Node, Plan, Row, Source, Split, Worker};

use crate::column::{Column, GRAIN, offsets};
use crate::level::{Strengths, Weights};
use crate::table::{Lent, Table, Tables};

/// Nodes per block when members are moved to their buckets.
const BLOCK: usize = 1 << 16;

/// A `u64` as an index into memory.
pub(crate) fn ix(x: u64) -> usize {
    usize::try_from(x).expect("an index into memory")
}

/// The node of id `v` of a plan.
fn node<'g, S: Source>(plan: &Plan<'_, 'g, S>, v: u32) -> Node<'g> {
    plan.node(v).expect("a member is a node")
}

/// A partition with its communities numbered `0..count` in the order of their
/// smallest node.
#[derive(Debug)]
pub(crate) struct Dense {
    pub(crate) map: Column<u32>,
    pub(crate) count: u32,
}

impl Dense {
    pub(crate) fn parts(&self) -> Parts<'_> {
        Parts {
            map: &self.map,
            count: self.count,
        }
    }
}

/// A [`Dense`] partition borrowed: its map may be held by someone else.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Parts<'a> {
    pub(crate) map: &'a [u32],
    pub(crate) count: u32,
}

/// Numbers the communities of `label` (values below the node count) by their smallest
/// node.
pub(crate) fn dense<'g, S: Source>(
    plan: &Plan<'_, 'g, S>,
    memory: &Memory,
    label: impl Fn(Node<'g>) -> u32 + Sync,
) -> Result<Dense, Error> {
    let n = plan.nodes() as usize;
    let first = Column::new(memory, "first members", n, |_| AtomicU32::new(u32::MAX))?;
    plan.vertices((), |(): &mut (), (), _, v, _| {
        first[label(v) as usize].fetch_min(v.get(), Relaxed);
    })?;
    let first_of = |v: Node<'g>| first[label(v) as usize].load(Relaxed);
    let rank = offsets(memory, "community ranks", n, |v| {
        let v = node(plan, u32::try_from(v).expect("a node id"));
        u64::from(first_of(v) == v.get())
    })?;
    let mut map = Column::new(memory, "communities", n, |_| 0_u32)?;
    plan.vertices(&mut map[..], |(): &mut (), out, i, v, _| {
        out[i] = u32::try_from(rank[first_of(v) as usize]).expect("below the node count");
    })?;
    let count = u32::try_from(rank[n]).expect("below the node count");
    Ok(Dense { map, count })
}

/// The members of each community of a [`Dense`] partition: community `c` holds
/// `ids[starts[c]..starts[c + 1]]`, increasing.
#[derive(Debug)]
pub(crate) struct Members {
    pub(crate) ids: Column<u32>,
    pub(crate) starts: Column<u64>,
}

impl Members {
    pub(crate) fn new(memory: &Memory, d: Parts<'_>) -> Result<Self, Error> {
        let (n, count) = (d.map.len(), d.count as usize);
        let sizes = Column::new(memory, "community sizes", count, |_| AtomicU32::new(0))?;
        par::for_each_range(n, GRAIN, |r| {
            for v in r {
                sizes[d.map[v] as usize].fetch_add(1, Relaxed);
            }
        });
        let starts = offsets(memory, "members", count, |c| {
            u64::from(sizes[c].load(Relaxed))
        })?;
        drop(sizes);
        let mut ids = Column::new(memory, "members", n, |_| 0_u32)?;
        let buckets = count.div_ceil(256).clamp(1, 1024);
        let per = count.div_ceil(buckets).max(1);
        let bounds = par::scatter(
            memory,
            "members",
            n.div_ceil(BLOCK),
            NonZeroUsize::new(buckets).expect("non-zero"),
            |b, sink| {
                let nodes = b * BLOCK..(b * BLOCK + BLOCK).min(n);
                for (v, &c) in nodes.clone().zip(&d.map[nodes]) {
                    let id = u32::try_from(v).expect("a node id");
                    sink.push(c as usize / per, id);
                }
                Ok::<(), Error>(())
            },
            [&mut ids[..]],
        )?;
        par::for_each_range_mut(&mut ids[..], &bounds, NonZeroUsize::MIN, |_, s| {
            s.sort_unstable_by_key(|&v| (d.map[v as usize], v));
        });
        Ok(Self { ids, starts })
    }

    /// The members of community `c`.
    pub(crate) fn of(&self, c: u32) -> &[u32] {
        let c = c as usize;
        &self.ids[ix(self.starts[c])..ix(self.starts[c + 1])]
    }
}

/// A weighted graph in memory: the quotient graph of a partition.
#[derive(Debug)]
pub(crate) struct Quotient {
    /// `nodes + 1` offsets into `targets`.
    pub(crate) offsets: Column<u64>,
    /// Each node's neighbours.
    pub(crate) targets: Column<u32>,
    /// The weight of each entry, in the units of `A` (twice the weight inside a
    /// community on the diagonal).
    pub(crate) weights: Column<u64>,
}

impl Quotient {
    /// Its number of nodes.
    #[cfg(test)]
    pub(crate) fn nodes(&self) -> u32 {
        u32::try_from(self.offsets.len() - 1).expect("below 2^32 nodes")
    }

    /// The graph as a source of the edge-map framework; the arrays are checked again.
    pub(crate) fn csr(&self) -> Result<Csr<'_>, Error> {
        Csr::new(&self.offsets, &self.targets)
    }
}

/// Per-node rows of the quotient as the framework's outputs: node `i` of a part writes
/// entries `offsets[i] - offsets[0] ..` of its part.
struct Rows<'a> {
    offsets: &'a [u64],
    targets: &'a mut [u32],
    weights: &'a mut [u64],
}

impl Split for Rows<'_> {
    fn split_at(self, mid: usize) -> (Self, Self) {
        let cut = ix(self.offsets[mid] - self.offsets[0]);
        let (t0, t1) = self.targets.split_at_mut(cut);
        let (w0, w1) = self.weights.split_at_mut(cut);
        (
            Rows {
                offsets: &self.offsets[..=mid],
                targets: t0,
                weights: w0,
            },
            Rows {
                offsets: &self.offsets[mid..],
                targets: t1,
                weights: w1,
            },
        )
    }
}

impl Rows<'_> {
    fn row(&mut self, i: usize) -> (&mut [u32], &mut [u64]) {
        let base = self.offsets[0];
        let r = ix(self.offsets[i] - base)..ix(self.offsets[i + 1] - base);
        (&mut self.targets[r.clone()], &mut self.weights[r])
    }
}

/// The sums of `A` from the community of `row.node` to every community, in the lent
/// table; `None` if a list could not be read or the table did not fit (recorded in
/// `tables`).
fn combine<'t, 'g, S: Source>(
    plan: &Plan<'_, 'g, S>,
    row: &Row<'_, 'g>,
    worker: &'t mut Worker<'_, 'g, S, Lent<'_>>,
    w: Weights<'_>,
    d: Parts<'_>,
    members: &Members,
    tables: &Tables,
) -> Option<&'t mut Table> {
    let map = d.map;
    let mine = members.of(map[row.node.index()]);
    let keys: usize = mine
        .iter()
        .map(|&x| plan.degree(node(plan, x)) as usize)
        .sum();
    let Worker { scratch, lists } = worker;
    let table = scratch.begin(keys.min(d.count as usize))?;
    for &x in mine {
        if x == row.node.get() {
            w.each(row, |v, y| table.add(map[v.index()], y));
            continue;
        }
        match lists.get(node(plan, x)) {
            Ok(other) => w.each(&other, |v, y| table.add(map[v.index()], y)),
            Err(e) => {
                tables.fail(e);
                return None;
            }
        }
    }
    Some(table)
}

/// Writes the row of each community at its smallest node `v`, into `rows` at
/// `offsets[v]`, and its sum into `strength`. Returns the number of communities whose
/// row names no other community.
fn write<S: Source>(
    plan: &Plan<'_, '_, S>,
    w: Weights<'_>,
    d: Parts<'_>,
    members: &Members,
    tables: &Tables,
    rows: Rows<'_>,
    strength: &[AtomicU64],
) -> Result<u32, Error> {
    let first = |v: Node<'_>| members.of(d.map[v.index()])[0] == v.get();
    let alone = plan.scan_with(
        rows,
        first,
        || tables.lend(),
        |alone: &mut u32, rows, i, row, worker| {
            let c = d.map[row.node.index()];
            if let Some(table) = combine(plan, &row, worker, w, d, members, tables) {
                let (t, x) = rows.row(i);
                let mut s = 0;
                for ((t, x), (key, sum)) in t.iter_mut().zip(x).zip(table.entries()) {
                    (*t, *x) = (key, sum);
                    s += sum;
                }
                strength[c as usize].store(s, Relaxed);
                *alone += u32::from(table.entries().all(|(key, _)| key == c));
            }
        },
    )?;
    tables.check()?;
    Ok(alone)
}

/// The quotient graph of the partition `d` of the plan's graph, and its strengths: the
/// rows are combined twice, once to count them and once to write them in place.
pub(crate) fn quotient<S: Source>(
    plan: &Plan<'_, '_, S>,
    memory: &Memory,
    w: Weights<'_>,
    d: Parts<'_>,
    members: &Members,
    tables: &Tables,
) -> Result<(Quotient, Strengths), Error> {
    let n = plan.nodes() as usize;
    let count = d.count as usize;
    let strength = Column::new(memory, "strengths", count, |_| AtomicU64::new(0))?;
    let first_member = |c: usize| members.ids[ix(members.starts[c])] as usize;
    let first = |v: Node<'_>| first_member(d.map[v.index()] as usize) == v.index();
    let mut degree = Column::new(memory, "quotient degrees", n, |_| 0_u32)?;
    plan.scan_with(
        &mut degree[..],
        first,
        || tables.lend(),
        |(): &mut (), out, i, row, worker| {
            if let Some(table) = combine(plan, &row, worker, w, d, members, tables) {
                out[i] = u32::try_from(table.len()).expect("below the node count");
            }
        },
    )?;
    tables.check()?;
    let rows_at = offsets(memory, "quotient rows", n, |v| u64::from(degree[v]))?;
    let entries = ix(rows_at[n]);
    let mut targets = Column::new(memory, "quotient targets", entries, |_| 0_u32)?;
    let mut weights = Column::new(memory, "quotient weights", entries, |_| 0_u64)?;
    let rows = Rows {
        offsets: &rows_at,
        targets: &mut targets[..],
        weights: &mut weights[..],
    };
    let alone = write(plan, w, d, members, tables, rows, &strength)?;
    let offsets = Column::new(memory, "quotient offsets", count + 1, |c| {
        rows_at[if c < count { first_member(c) } else { n }]
    })?;
    let strength = strength.map(AtomicU64::into_inner);
    let total = strength.iter().sum();
    Ok((
        Quotient {
            offsets,
            targets,
            weights,
        },
        Strengths {
            of: strength,
            total,
            alone,
            mass: None,
            skew: None,
        },
    ))
}

/// The quotient graph of a partition of a graph with `weights` (`membership` as
/// [`table`](crate::table) takes it). Node `c` of the quotient is the `c`-th community
/// in the order of its smallest node.
#[cfg(test)]
pub(crate) fn quotient_of<S: Source>(
    source: &S,
    weights: crate::EdgeWeights<'_>,
    memory: &Memory,
    settings: everygraph_kernels::edge_map::Settings,
    membership: &[u32],
) -> Result<Quotient, Error> {
    crate::quality::communities(source.nodes(), membership)?;
    let (w, _) = crate::weights::scaled(source, weights)?;
    Plan::with(source, memory, settings, |plan| {
        let d = dense(&plan, memory, |v| membership[v.index()])?;
        let members = Members::new(memory, d.parts())?;
        let q = quotient(&plan, memory, w, d.parts(), &members, &Tables::new(memory))?;
        Ok(q.0)
    })
}
