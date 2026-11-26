// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tails: a region with more than `F` children keeps its `F − 1` largest and puts the
//! others in one aggregate (ADR 0016).

use std::cmp::Reverse;

use everygraph_core::{Memory, MemoryError};

use super::{Children, Kind, Tree};
use crate::column::Column;

/// Bounds the fan-out of every region at `fan_out` (4 or more). Children are ranked by
/// decreasing node count, then by id.
pub(super) fn bound(memory: &Memory, tree: &mut Tree, fan_out: u32) -> Result<(), MemoryError> {
    let kids = Children::new(memory, &tree.parent)?;
    let f = fan_out as usize;
    for p in 0..tree.len() {
        let children = kids.of(p);
        if children.len() <= f {
            continue;
        }
        let mut ranked = Column::new(memory, "ranked children", children.len(), |i| children[i])?;
        ranked.sort_unstable_by_key(|&c| (Reverse(tree.nodes[c as usize]), c));
        aggregate(tree, &ranked[f - 1..], p, f)?;
    }
    Ok(())
}

/// One aggregate under `parent` holding `members`, ranked. Above `f` members, it
/// holds the first `f / 2` directly and the others in at most `f / 2` runs of
/// consecutive ranks, each run of two or more an aggregate of its own: at most `f`
/// children everywhere, at a depth logarithmic in the members.
fn aggregate(tree: &mut Tree, members: &[u32], parent: u32, f: usize) -> Result<(), MemoryError> {
    let nodes: u32 = members.iter().map(|&c| tree.nodes[c as usize]).sum();
    let me = tree.add(parent, Kind::Aggregate, nodes)?;
    let (direct, rest) = if members.len() <= f {
        (members, &[][..])
    } else {
        members.split_at(f / 2)
    };
    for &c in direct {
        tree.parent[c as usize] = me;
    }
    if rest.is_empty() {
        return Ok(());
    }
    let runs = (f / 2).min(rest.len().div_ceil(f / 2));
    let (base, longer) = (rest.len() / runs, rest.len() % runs);
    let mut at = 0;
    for i in 0..runs {
        let run = &rest[at..at + base + usize::from(i < longer)];
        at += run.len();
        if let [c] = run {
            tree.parent[*c as usize] = me;
        } else {
            aggregate(tree, run, me, f)?;
        }
    }
    Ok(())
}
