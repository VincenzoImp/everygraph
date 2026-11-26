// SPDX-License-Identifier: Apache-2.0 OR MIT

//! [`store`]: the map tree into the graph file (`docs/spec/eg-communities.md` § 6): the
//! tree by its parents, the children of each region numbered together, the exact counts
//! of every region, and the sibling graph.

use std::cmp::Reverse;

use everygraph_core::{ByteSink, Memory, MemoryKind, SegVec, par};
use everygraph_format::adjacency::{
    IndexBuilder, append_record, encode_events, events_section_bytes,
};
use everygraph_format::container::{
    ContainerError, GraphIdentity, Kind as Section, Provenance, ResultKey, Writer,
};
use everygraph_kernels::edge_map::{Error, Plan, Settings, Source};

use super::{Children, Kind, MapOptions, MapTree, SuperLevel};
use crate::aggregate::{Members, Parts, Quotient, ix, quotient};
use crate::column::{Column, GRAIN};
use crate::hierarchy::{HierarchyError, leiden_key};
use crate::level::{Strengths, Weights};
use crate::table::Tables;

/// The operation of the result (`results/map/<key>`).
pub const OPERATION: &str = "map";
/// The version of the layout and of the algorithm that fills it, in the key.
pub const VERSION: u32 = 1;

pub(super) const PARENT: &str = "parent";
pub(super) const KIND: &str = "kind";
pub(super) const NODES: &str = "nodes";
pub(super) const INTERNAL: &str = "internal";
pub(super) const VOLUME: &str = "volume";
pub(super) const MEMBERSHIP: &str = "membership";
pub(super) const SIBLINGS: &str = "siblings";

/// The key of the map tree built with `options` on the graph `graph`: the options of
/// the partition's Leiden run, and those of the shaping. The decision of a previous map
/// on the super level is not in it: a rebuild replaces the map.
///
/// # Errors
///
/// None in practice: the names are valid.
pub fn key(graph: GraphIdentity, options: &MapOptions) -> Result<ResultKey, ContainerError> {
    let upper = match options.super_level {
        SuperLevel::Auto { .. } => "auto",
        SuperLevel::Always => "always",
        SuperLevel::Never => "never",
    };
    leiden_key(graph, OPERATION, VERSION, &options.leiden)?
        .with("fan_out", &options.fan_out.end().to_string())?
        .with("leaf", &options.leaf.to_string())?
        .with("super", upper)
}

/// The code of a kind in the file.
fn code(kind: Kind) -> u32 {
    match kind {
        Kind::Root => 0,
        Kind::Super => 1,
        Kind::Community => 2,
        Kind::Subcommunity => 3,
        Kind::Aggregate => 4,
    }
}

pub(super) fn decode(code: u32) -> Option<Kind> {
    match code {
        0 => Some(Kind::Root),
        1 => Some(Kind::Super),
        2 => Some(Kind::Community),
        3 => Some(Kind::Subcommunity),
        4 => Some(Kind::Aggregate),
        _ => None,
    }
}

pub(super) fn invalid(what: &str) -> HierarchyError {
    HierarchyError::Invalid(format!("map: {what}"))
}

/// The tree renumbered as the file holds it: breadth first from the root, the children
/// of each region by decreasing node count, then by smallest node.
struct Numbered {
    /// The region of `tree` of each new id.
    old_of: Column<u32>,
    parent: Column<u32>,
    depth: Column<u32>,
    /// The leaf of each node.
    membership: Column<u32>,
}

fn numbered(memory: &Memory, tree: &MapTree) -> Result<Numbered, HierarchyError> {
    let r = tree.parent.len();
    let mut first = Column::new(memory, "map first nodes", r, |_| u32::MAX)?;
    for (v, &l) in tree.leaf.iter().enumerate() {
        let f = &mut first[l as usize];
        *f = (*f).min(u32::try_from(v).expect("a node"));
    }
    let mut kids = Children::new(memory, &tree.parent)?;
    for &x in kids.order(memory)?.iter().rev().filter(|&&x| x != 0) {
        let p = tree.parent[x as usize] as usize;
        first[p] = first[p].min(first[x as usize]);
    }
    let Children { starts, ids } = &mut kids;
    par::for_each_range_mut(&mut ids[..], &starts[..], GRAIN, |_, c| {
        c.sort_unstable_by_key(|&x| (Reverse(tree.nodes[x as usize]), first[x as usize]));
    });
    let order = kids.order(memory)?;
    let mut new_of = Column::new(memory, "map numbers", r, |_| 0_u32)?;
    for (i, &x) in order.iter().enumerate() {
        new_of[x as usize] = u32::try_from(i).expect("a region");
    }
    let parent = Column::new(memory, "map parents", r, |i| {
        new_of[tree.parent[order[i] as usize] as usize]
    })?;
    let mut depth = Column::new(memory, "map depths", r, |_| 0_u32)?;
    for i in 1..r {
        depth[i] = depth[parent[i] as usize] + 1;
    }
    let membership = Column::new(memory, "map membership", tree.leaf.len(), |v| {
        new_of[tree.leaf[v] as usize]
    })?;
    Ok(Numbered {
        old_of: order,
        parent,
        depth,
        membership,
    })
}

/// The exact counts of every region and the sibling graph.
struct Counts {
    internal: Column<u64>,
    volume: Column<u64>,
    /// `(a, b, w)`: `w` pairs of nodes between the siblings `a` and `b`, both ways,
    /// sorted.
    siblings: SegVec<(u32, u32, u64)>,
}

/// The units of one stage, numbered by smallest node: the map from the `len` units of
/// the stage below (the nodes, for the first), given by their region, and the region
/// of each unit.
fn units(
    memory: &Memory,
    regions: usize,
    len: usize,
    below: impl Iterator<Item = u32>,
) -> Result<(Column<u32>, SegVec<u32>), HierarchyError> {
    let mut unit = Column::new(memory, "map units", regions, |_| u32::MAX)?;
    let mut of = SegVec::new(memory, "map units");
    let mut map = Column::new(memory, "map units", len, |_| 0_u32)?;
    for (m, x) in map.iter_mut().zip(below) {
        let x = x as usize;
        if unit[x] == u32::MAX {
            unit[x] = u32::try_from(of.len()).expect("a unit");
            of.try_push(u32::try_from(x).expect("a region"))?;
        }
        *m = unit[x];
    }
    Ok((map, of))
}

/// The quotient of `parts` over `source`, weighted by `w`.
fn stage<S: Source>(
    source: &S,
    memory: &Memory,
    settings: Settings,
    w: Weights<'_>,
    parts: Parts<'_>,
) -> Result<(Quotient, Strengths), Error> {
    Plan::with(source, memory, settings, |plan| {
        let members = Members::new(memory, parts)?;
        quotient(&plan, memory, w, parts, &members, &Tables::new(memory))
    })
}

/// Stage by stage from the deepest: the quotient of the regions at depth `d` and of the
/// leaves above it. The row of a region at its own depth holds its internal weight (on
/// the loop), its volume, and its entries to its siblings; then each region at depth
/// `d` is merged into its parent. One stage's quotient and the next are held at a time.
fn counts<S: Source>(
    source: &S,
    memory: &Memory,
    settings: Settings,
    t: &Numbered,
) -> Result<Counts, HierarchyError> {
    let r = t.parent.len();
    let mut internal = Column::new(memory, "map internal", r, |_| 0_u64)?;
    let mut volume = Column::new(memory, "map volumes", r, |_| 0_u64)?;
    let mut siblings = SegVec::new(memory, "map siblings");
    let top = t
        .membership
        .iter()
        .map(|&x| t.depth[x as usize])
        .max()
        .unwrap_or(0);
    if top > 0 {
        let n = t.membership.len();
        let (mut map, mut of) = units(memory, r, n, t.membership.iter().copied())?;
        let count = |of: &SegVec<u32>| u32::try_from(of.len()).expect("units");
        let parts = Parts {
            map: &map,
            count: count(&of),
        };
        let (mut q, mut k) = stage(source, memory, settings, Weights::Unit, parts)?;
        for d in (1..=top).rev() {
            for u in 0..of.len() {
                let x = of[u] as usize;
                if t.depth[x] != d {
                    continue;
                }
                volume[x] = k.of[u];
                let row = ix(q.offsets[u])..ix(q.offsets[u + 1]);
                for (&b, &w) in q.targets[row.clone()].iter().zip(&q.weights[row]) {
                    let y = of[b as usize];
                    if b as usize == u {
                        internal[x] = w / 2;
                    } else if t.parent[y as usize] == t.parent[x] {
                        siblings.try_push((u32::try_from(x).expect("a region"), y, w))?;
                    }
                }
            }
            if d == 1 {
                break;
            }
            let lift = (0..of.len()).map(|b| {
                let x = of[b];
                if t.depth[x as usize] == d {
                    t.parent[x as usize]
                } else {
                    x
                }
            });
            let (next_map, next_of) = units(memory, r, of.len(), lift)?;
            let csr = q.csr()?;
            let parts = Parts {
                map: &next_map,
                count: count(&next_of),
            };
            let next = stage(&csr, memory, settings, Weights::Entries(&q.weights), parts)?;
            (q, k) = next;
            (map, of) = (next_map, next_of);
        }
        drop((q, k, map));
    }
    let sorted = siblings
        .as_mut_slice()
        .ok_or_else(|| invalid("a sibling graph longer than one segment"))?;
    sorted.sort_unstable();
    Ok(Counts {
        internal,
        volume,
        siblings,
    })
}

/// The counts of every region with children from those of its children and the
/// weights `between` its children in the sibling graph (each pair counted both ways):
/// `internal(p) = Σ internal(c) + between(p) / 2` and `volume(p) = Σ volume(c)`. For
/// the root they are its counts; any other region must have them already, or the first
/// that does not is returned.
pub(super) fn conserved(
    memory: &Memory,
    parent: &[u32],
    internal: &mut [u64],
    volume: &mut [u64],
    children: &[u32],
    between: &[u64],
) -> Result<Option<u32>, HierarchyError> {
    let r = parent.len();
    let mut sums = Column::new(memory, "map check", r, |_| (0_u64, 0_u64))?;
    for x in (1..r).rev() {
        let p = parent[x] as usize;
        sums[p] = (
            sums[p].0.saturating_add(internal[x]),
            sums[p].1.saturating_add(volume[x]),
        );
    }
    for p in (0..r).filter(|&p| children[p] > 0) {
        let made = (sums[p].0.saturating_add(between[p] / 2), sums[p].1);
        if p == 0 {
            (internal[0], volume[0]) = made;
        } else if !between[p].is_multiple_of(2) || (internal[p], volume[p]) != made {
            return Ok(Some(u32::try_from(p).expect("a region")));
        }
    }
    Ok(None)
}

/// The sibling weight under each region: the weights of the entries between its
/// children, `(child, weight)`.
fn between(
    memory: &Memory,
    parent: &[u32],
    entries: impl Iterator<Item = (u32, u64)>,
) -> Result<Column<u64>, HierarchyError> {
    let mut out = Column::new(memory, "map check", parent.len(), |_| 0_u64)?;
    for (a, w) in entries {
        let p = &mut out[parent[a as usize] as usize];
        *p = p.saturating_add(w);
    }
    Ok(out)
}

/// The number of children of every region.
pub(super) fn children_of(memory: &Memory, parent: &[u32]) -> Result<Column<u32>, HierarchyError> {
    let mut c = Column::new(memory, "map children", parent.len(), |_| 0_u32)?;
    for &p in &parent[1.min(parent.len())..] {
        c[p as usize] += 1;
    }
    Ok(c)
}

/// Stores `tree`, made by [`build`](super::build) on `source` (the graph of the file the
/// writer holds), as the result of `key` with the record `about`; a map stored under
/// `key` is replaced. Nothing is stored until the writer commits.
///
/// # Errors
///
/// [`HierarchyError::Invalid`] for a tree of another graph, or counts that do not add
/// up; the errors of the source, the budget and the writer. An error found before
/// anything is written leaves the writer as it was; any later one poisons it.
pub fn store<G: Source, K: ByteSink>(
    writer: &mut Writer<K>,
    key: &ResultKey,
    about: &Provenance,
    source: &G,
    memory: &Memory,
    settings: Settings,
    tree: &MapTree,
) -> Result<(), HierarchyError> {
    if tree.leaf.len() != source.nodes() as usize {
        return Err(invalid("a tree of another graph"));
    }
    let t = numbered(memory, tree)?;
    let mut c = counts(source, memory, settings, &t)?;
    let children = children_of(memory, &t.parent)?;
    let between = between(
        memory,
        &t.parent,
        c.siblings.iter().map(|&(a, _, w)| (a, w)),
    )?;
    if let Some(p) = conserved(
        memory,
        &t.parent,
        &mut c.internal,
        &mut c.volume,
        &children,
        &between,
    )? {
        return Err(invalid(&format!(
            "region {p}: its counts are not those of its children"
        )));
    }
    let written = write(writer, key, about, tree, &t, &c, memory);
    if written.is_err() {
        writer.poison();
    }
    written
}

fn write<K: ByteSink>(
    writer: &mut Writer<K>,
    key: &ResultKey,
    about: &Provenance,
    tree: &MapTree,
    t: &Numbered,
    c: &Counts,
    memory: &Memory,
) -> Result<(), HierarchyError> {
    let r = t.parent.len();
    let built = u64::from(tree.shape.super_resolution.is_some());
    writer.add_result(key, about, &[r as u64, tree.leaf.len() as u64, built])?;
    let old = |i: usize| t.old_of[i] as usize;
    writer.add_result_part(key, PARENT, t.parent.iter().copied())?;
    writer.add_result_part(key, KIND, (0..r).map(|i| code(tree.kind[old(i)])))?;
    writer.add_result_part(key, NODES, (0..r).map(|i| tree.nodes[old(i)]))?;
    writer.add_result_part(key, INTERNAL, c.internal.iter().copied())?;
    writer.add_result_part(key, VOLUME, c.volume.iter().copied())?;
    writer.add_result_part(key, MEMBERSHIP, t.membership.iter().copied())?;
    let siblings = c.siblings.as_slice().unwrap_or(&[]);
    let mut index = IndexBuilder::new(memory, u32::try_from(r).expect("regions"))?;
    let mut data = writer.begin_result_part(
        key,
        &format!("{SIBLINGS}.data"),
        Section::RESULT_ADJACENCY_DATA,
    )?;
    let mut record = Vec::new();
    let mut at = 0;
    for a in 0..u32::try_from(r).expect("regions") {
        let len = siblings[at..].iter().take_while(|e| e.0 == a).count();
        record.clear();
        let degree = append_record(a, siblings[at..at + len].iter().map(|e| e.1), &mut record)?;
        data.write(&record)?;
        index.push(degree, record.len() as u64)?;
        at += len;
    }
    data.finish()?;
    let entries = siblings.len() as u64;
    let events: u64 = siblings.iter().map(|e| e.2).sum();
    let events_bytes = if events > entries {
        let bytes = events_section_bytes(entries, events)
            .ok_or_else(|| invalid("a sibling graph too large"))?;
        let _held = memory
            .reserve(
                "map sibling events",
                MemoryKind::Anonymous,
                bytes.saturating_mul(2),
            )
            .map_err(Error::from)?;
        Some(encode_events(
            entries,
            events,
            siblings.iter().map(|e| e.2),
        )?)
    } else {
        None
    };
    let (index_bytes, _held) = index.finish(events_bytes.is_some())?;
    let parts = [
        ("index", Section::RESULT_ADJACENCY_INDEX, Some(index_bytes)),
        ("events", Section::RESULT_ADJACENCY_EVENTS, events_bytes),
    ];
    for (suffix, kind, bytes) in parts {
        if let Some(bytes) = bytes {
            let mut s = writer.begin_result_part(key, &format!("{SIBLINGS}.{suffix}"), kind)?;
            s.write(&bytes)?;
            s.finish()?;
        }
    }
    Ok(())
}
