// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The rules of a stored map tree that `everygraph verify --deep` checks
//! (`docs/spec/eg-communities.md` § 6).

use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering::Relaxed;

use everygraph_core::rng::{Purpose, draw};
use everygraph_core::{ByteSource, Memory};
use everygraph_format::adjacency::Adjacency;
use everygraph_format::container::{Container, Provenance, StoredResult};
use everygraph_kernels::edge_map::{Compressed, Error, Plan, Settings, Source, all};

use super::Kind;
use super::read::StoredMap;
use super::store::{children_of, conserved, invalid};
use crate::column::Column;
use crate::hierarchy::HierarchyError;
use crate::hierarchy::check::{PRIME, add, undirected};
use crate::level::Weights;
use crate::quality::tally;

/// Whether a community is on the path once a region of kind `kind` follows a path with
/// one (`under`) or without; `None` when the kind may not follow it.
fn ordered(under: bool, kind: Kind) -> Option<bool> {
    match (kind, under) {
        (Kind::Super, false) | (Kind::Aggregate, _) => Some(under),
        (Kind::Community, false) | (Kind::Subcommunity, true) => Some(true),
        _ => None,
    }
}

/// The rules of a stored map tree (`docs/spec/eg-communities.md` § 6) that
/// `everygraph verify --deep` checks. Returns whether the graph of the file was in
/// memory to check the leaves and the sibling graph against.
pub(crate) fn check<S: ByteSource>(
    file: &Container<S>,
    root: StoredResult<'_, S>,
    record: &Provenance,
    memory: &Memory,
) -> Result<bool, HierarchyError> {
    let m = StoredMap::of(file, root)?;
    let fan_out: u32 = record
        .parameters
        .get("fan_out")
        .and_then(|f| f.parse().ok())
        .ok_or_else(|| invalid("no fan-out in its record"))?;
    let r = m.regions() as usize;
    let parent = m.parents(memory)?;
    let kind = m.kinds(memory)?;
    let ordered_tree = parent[0] == 0
        && (1..r).all(|x| (parent[x] as usize) < x && (x == 1 || parent[x] >= parent[x - 1]));
    if !ordered_tree {
        return Err(invalid(
            "regions not numbered breadth first, children together",
        ));
    }
    let children = children_of(memory, &parent)?;
    if children.iter().any(|&c| c > fan_out) {
        return Err(invalid("a region with more children than the fan-out"));
    }
    let mut under = Column::new(memory, "map check", r, |_| false)?;
    if kind[0] != Kind::Root {
        return Err(invalid("kinds out of order"));
    }
    for x in 1..r {
        let p = under[parent[x] as usize];
        under[x] = ordered(p, kind[x]).ok_or_else(|| invalid("kinds out of order"))?;
    }
    let table = m.table(memory)?;
    let membership = m.membership(memory)?;
    let mut count = Column::new(memory, "map check", r, |_| 0_u32)?;
    let mut first = Column::new(memory, "map check", r, |_| u32::MAX)?;
    for (v, &l) in membership.iter().enumerate() {
        let l = l as usize;
        if children[l] > 0 || !under[l] {
            return Err(invalid(
                "a node in a region with children or under no community",
            ));
        }
        count[l] += 1;
        first[l] = first[l].min(u32::try_from(v).unwrap_or(u32::MAX));
    }
    for x in (1..r).rev() {
        let p = parent[x] as usize;
        count[p] = count[p].saturating_add(count[x]);
        first[p] = first[p].min(first[x]);
    }
    if count
        .iter()
        .zip(&table.nodes[..])
        .any(|(&c, &t)| c != t || t == 0 && r > 1)
    {
        return Err(invalid("node counts that do not add up"));
    }
    let numbered = (2..r).all(|x| {
        let (a, b) = (table.nodes[x - 1], table.nodes[x]);
        parent[x] != parent[x - 1] || a > b || (a == b && first[x - 1] < first[x])
    });
    if !numbered {
        return Err(invalid(
            "children not numbered by decreasing node count, then smallest node",
        ));
    }
    // The sibling graph: entries between siblings only, and its random-weight sums.
    let s = m.siblings()?;
    let adjacency = s.adjacency()?;
    let events = s.events()?;
    let h = weights(memory, m.regions())?;
    let mut stored_h = Column::new(memory, "map check", r, |_| 0_u64)?;
    let mut buf = Vec::new();
    let mut entry = 0_u64;
    let mut sibling = Column::new(memory, "map check", r, |_| 0_u64)?;
    for a in 0..m.regions() {
        let row = adjacency
            .neighbors(everygraph_core::NodeId::new(a), &mut buf)
            .map_err(Error::from)?;
        for &b in row {
            let w = match events {
                Some(e) => {
                    let range = e.event_range(entry)?;
                    range.end - range.start
                }
                None => 1,
            };
            entry += 1;
            let (a, b) = (a as usize, b as usize);
            if a == 0 || b == a || b >= r || parent[b] != parent[a] {
                return Err(invalid(
                    "a sibling entry between regions that are not siblings",
                ));
            }
            let term = u128::from(w % PRIME) * u128::from(h[b]) % u128::from(PRIME);
            stored_h[a] = add(stored_h[a], u64::try_from(term).unwrap_or(0));
            let p = &mut sibling[parent[a] as usize];
            *p = p.saturating_add(w);
        }
    }
    drop(buf);
    let mut made = m.table(memory)?;
    let conserved = conserved(
        memory,
        &parent,
        &mut made.internal,
        &mut made.volume,
        &children,
        &sibling,
    )?;
    if conserved.is_some()
        || made.internal[0] != table.internal[0]
        || made.volume[0] != table.volume[0]
    {
        return Err(invalid(
            "counts that are not those of the children and the sibling graph",
        ));
    }
    drop(made);
    let Some((index, data)) = undirected(file)? else {
        return Ok(false);
    };
    let source = Compressed::new(Adjacency::new(&index, data)?);
    if source.nodes() != m.nodes() {
        return Err(invalid("a membership of another length than the graph"));
    }
    let mut depth = Column::new(memory, "map check", r, |_| 0_u32)?;
    for x in 1..r {
        depth[x] = depth[parent[x] as usize] + 1;
    }
    // The pair of children of the lowest common ancestor of the leaves `a` and `b`.
    let pair = |mut a: u32, mut b: u32| {
        while depth[a as usize] > depth[b as usize] {
            a = parent[a as usize];
        }
        while depth[b as usize] > depth[a as usize] {
            b = parent[b as usize];
        }
        while parent[a as usize] != parent[b as usize] {
            (a, b) = (parent[a as usize], parent[b as usize]);
        }
        (a, b)
    };
    let made_h = Column::new(memory, "map check", r, |_| AtomicU64::new(0))?;
    let leaves = Plan::with(&source, memory, Settings::default(), |plan| {
        plan.scan((), all, |(): &mut (), (), _, row, _| {
            let lu = membership[row.node.index()];
            for &v in row.list {
                let lv = membership[v.index()];
                if lu != lv {
                    let (a, b) = pair(lu, lv);
                    let cell = &made_h[a as usize];
                    let _ = cell.try_update(Relaxed, Relaxed, |old| Some(add(old, h[b as usize])));
                }
            }
        })?;
        tally(&plan, memory, Weights::Unit, &membership, m.regions())
    })?;
    let same_leaves = (0..r)
        .filter(|&x| children[x] == 0)
        .all(|x| leaves.internal[x] == table.internal[x] && leaves.volume[x] == table.volume[x]);
    if !same_leaves {
        return Err(invalid("leaves whose counts are not those of the graph"));
    }
    let same = (0..r).all(|x| made_h[x].load(Relaxed) == stored_h[x]);
    if !same {
        return Err(invalid(
            "a sibling graph that is not what the tree makes of the graph",
        ));
    }
    Ok(true)
}

/// The random weight of each region for the check of the sibling graph.
fn weights(memory: &Memory, regions: u32) -> Result<Column<u64>, HierarchyError> {
    Ok(Column::new(
        memory,
        "check weights",
        regions as usize,
        |x| {
            let x = u32::try_from(x).unwrap_or(u32::MAX);
            draw(0, Purpose::HIERARCHY_CHECK, [x, 0, 0, 1]) % PRIME
        },
    )?)
}
