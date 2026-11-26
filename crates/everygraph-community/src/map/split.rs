// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Below the partition: every region of more than `leaf` nodes split again by Leiden on
//! its own induced subgraph, round after round, until none is left to split.
//!
//! Each region is solved alone, so its split is a function of the region: the order of
//! the work cannot change it. Small regions are solved several at a time; a region of
//! [`ALONE`] nodes or more is solved by all threads, one at a time, so memory holds at
//! most one large region's subgraph and Leiden state.

use std::cmp::Reverse;
use std::num::NonZeroUsize;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering::Relaxed;

use everygraph_core::{Memory, par};
use everygraph_kernels::edge_map::{Csr, Error, NodeList, Plan, Settings, Source};

use super::{Kind, Tree};
use crate::aggregate::{Members, Parts, ix};
use crate::column::{Column, GRAIN, offsets};
use crate::{Options, leiden};

/// Nodes from which a region is solved alone: Leiden's own parallel grain, 2^16 nodes
/// per sub-round (`level.rs`).
const ALONE: usize = 1 << 16;

/// Splits the regions from `from` on, and the subcommunities they give, until every
/// leaf has at most `leaf_max` nodes or Leiden finds one community in it. `leaf` holds
/// the leaf of each node and follows the splits. Returns the number of rounds.
#[allow(clippy::too_many_arguments)]
pub(super) fn split<S: Source>(
    source: &S,
    memory: &Memory,
    settings: Settings,
    options: &Options,
    leaf_max: u32,
    tree: &mut Tree,
    leaf: &mut Column<u32>,
    mut from: u32,
) -> Result<u32, Error> {
    let mut rounds = 0;
    loop {
        let to = tree.len();
        let large = (from..to).filter(|&r| tree.nodes[r as usize] > leaf_max);
        let mut active = Column::new(memory, "regions to split", large.clone().count(), |_| 0)?;
        for (a, r) in active.iter_mut().zip(large) {
            *a = r;
        }
        if active.is_empty() {
            return Ok(rounds);
        }
        rounds += 1;
        let members = Members::new(
            memory,
            Parts {
                map: leaf,
                count: to,
            },
        )?;
        let local = places(memory, &members, leaf)?;
        let region = |i: usize| Region {
            id: active[i],
            members: members.of(active[i]),
        };
        let found = Plan::with(source, memory, settings, |plan| {
            let solve = |i: usize| region(i).solve(&plan, memory, settings, options, leaf, &local);
            let small = par::map_ranges(active.len(), NonZeroUsize::MIN, |r| {
                let alone = region(r.start).members.len() >= ALONE;
                if alone { Ok(None) } else { solve(r.start) }
            });
            let mut found = Vec::new();
            for (i, s) in small.into_iter().enumerate() {
                let alone = region(i).members.len() >= ALONE;
                found.push(if alone { solve(i)? } else { s? });
            }
            Ok::<_, Error>(found)
        })?;
        drop(local);
        for (i, found) in found.into_iter().enumerate() {
            if let Some(found) = found {
                region(i).commit(memory, tree, leaf, &found)?;
            }
        }
        from = to;
    }
}

/// The place of every node in its region's members.
fn places(memory: &Memory, members: &Members, leaf: &[u32]) -> Result<Column<u32>, Error> {
    let local = Column::new(memory, "region places", leaf.len(), |_| AtomicU32::new(0))?;
    par::for_each_range(members.ids.len(), GRAIN, |range| {
        for i in range {
            let v = members.ids[i] as usize;
            let first = members.starts[leaf[v] as usize];
            local[v].store(u32::try_from(i as u64 - first).expect("a place"), Relaxed);
        }
    });
    Ok(local.map(AtomicU32::into_inner))
}

/// A region and its nodes, increasing.
struct Region<'a> {
    id: u32,
    members: &'a [u32],
}

/// The communities Leiden found in a region: the community of each member, numbered
/// by smallest node, and their count, 2 or more.
struct Found {
    membership: Column<u32>,
    count: u32,
}

impl Region<'_> {
    /// Leiden on the region's subgraph; `None` when it finds one community.
    fn solve<S: Source>(
        &self,
        plan: &Plan<'_, '_, S>,
        memory: &Memory,
        settings: Settings,
        options: &Options,
        leaf: &[u32],
        local: &[u32],
    ) -> Result<Option<Found>, Error> {
        let (offsets, targets) = self.induced(plan, memory, leaf, local)?;
        let found = leiden(&Csr::new(&offsets, &targets)?, memory, settings, options)?;
        Ok((found.count > 1).then_some(Found {
            membership: found.membership,
            count: found.count,
        }))
    }

    /// Adds the communities found as the region's children, largest first, then by
    /// smallest node, and moves the members into them.
    fn commit(
        &self,
        memory: &Memory,
        tree: &mut Tree,
        leaf: &mut [u32],
        found: &Found,
    ) -> Result<(), Error> {
        let count = found.count as usize;
        let mut size = Column::new(memory, "subcommunity nodes", count, |_| 0_u32)?;
        for &c in found.membership.iter() {
            size[c as usize] += 1;
        }
        let mut order = Column::new(memory, "subcommunity order", count, |c| {
            u32::try_from(c).expect("a community")
        })?;
        order.sort_unstable_by_key(|&c| (Reverse(size[c as usize]), c));
        let mut region = Column::new(memory, "subcommunity regions", count, |_| 0_u32)?;
        for &c in order.iter() {
            region[c as usize] = tree.add(self.id, Kind::Subcommunity, size[c as usize])?;
        }
        for (&v, &c) in self.members.iter().zip(found.membership.iter()) {
            leaf[v as usize] = region[c as usize];
        }
        Ok(())
    }

    /// The subgraph induced by the region, its nodes numbered by their place in
    /// `members`, read by random access to the members' lists: a CSR.
    fn induced<S: Source>(
        &self,
        plan: &Plan<'_, '_, S>,
        memory: &Memory,
        leaf: &[u32],
        local: &[u32],
    ) -> Result<(Column<u64>, Column<u32>), Error> {
        let mut list = NodeList::new(memory);
        for &v in self.members {
            list.push(
                plan.node(v)
                    .ok_or(Error::Invalid("a member is not a node"))?,
            )?;
        }
        let nodes = list.as_slice()?;
        let k = self.members.len();
        let degree = Column::new(memory, "region degrees", k, |_| AtomicU32::new(0))?;
        plan.frontier(nodes, |(): &mut (), row, _| {
            let d = row
                .list
                .iter()
                .filter(|w| leaf[w.index()] == self.id)
                .count();
            let d = u32::try_from(d).expect("a degree");
            degree[local[row.node.index()] as usize].store(d, Relaxed);
        })?;
        let offsets = offsets(memory, "region lists", k, |i| {
            u64::from(degree[i].load(Relaxed))
        })?;
        drop(degree);
        let targets = Column::new(memory, "region lists", ix(offsets[k]), |_| {
            AtomicU32::new(0)
        })?;
        plan.frontier(nodes, |(): &mut (), row, _| {
            let first = ix(offsets[local[row.node.index()] as usize]);
            let inside = row.list.iter().filter(|w| leaf[w.index()] == self.id);
            for (at, w) in (first..).zip(inside) {
                targets[at].store(local[w.index()], Relaxed);
            }
        })?;
        Ok((offsets, targets.map(AtomicU32::into_inner)))
    }
}
