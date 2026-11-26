// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The map tree in the graph file (`docs/spec/eg-communities.md` § 6): the levels of a
//! hierarchy by depth, a leaf repeated on every level below its own, and the kind of
//! every region.

use everygraph_core::{ByteSink, ByteSource, Memory, SegVec};
use everygraph_format::container::{ContainerError, GraphIdentity, Provenance, ResultKey, Writer};
use everygraph_kernels::edge_map::{Settings, Source};

use super::{Children, Kind, MapOptions, MapTree, SuperLevel};
use crate::column::Column;
use crate::hierarchy::write::{Run, WINDOW, store_with};
use crate::hierarchy::{Hierarchy, HierarchyError, KIND, leiden_key, part};

/// The operation of the result (`results/map/<key>`).
pub const OPERATION: &str = "map";
/// The version of the layout and of the algorithm that fills it, in the key.
pub const VERSION: u32 = 1;

/// The key of the map tree built with `options` on the graph `graph`: the options of
/// the partition's Leiden run, and those of the shaping.
///
/// # Errors
///
/// None in practice: the names are valid.
pub fn key(graph: GraphIdentity, options: &MapOptions) -> Result<ResultKey, ContainerError> {
    let upper = match options.super_level {
        SuperLevel::Auto { previous: None } => "auto",
        SuperLevel::Auto {
            previous: Some(true),
        } => "auto, built before",
        SuperLevel::Auto {
            previous: Some(false),
        } => "auto, not built before",
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

fn decode(code: u32) -> Option<Kind> {
    match code {
        1 => Some(Kind::Super),
        2 => Some(Kind::Community),
        3 => Some(Kind::Subcommunity),
        4 => Some(Kind::Aggregate),
        _ => None,
    }
}

/// Stores `tree`, made by [`build`](super::build) on `source` (the graph of the file the
/// writer holds), as the result of `key` with the record `about`; as
/// [`hierarchy::store`](crate::hierarchy::store) otherwise. Level 1 holds the root's
/// children; level `ℓ` the regions at depth `ℓ` and the leaves above it.
///
/// # Errors
///
/// [`HierarchyError::Invalid`] for a tree of no region below the root or of another
/// graph; as [`hierarchy::store`](crate::hierarchy::store).
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
        return Err(HierarchyError::Invalid(
            "map: a tree of another graph".into(),
        ));
    }
    let levels = levels(memory, tree)?;
    let maps: Vec<&[u32]> = levels.iter().map(|l| &l.0[..]).collect();
    let kinds: Vec<&[u32]> = levels.iter().map(|l| &l.1[..]).collect();
    let run = Run {
        source,
        memory,
        settings,
        window: WINDOW,
    };
    store_with(writer, key, about, &run, &maps, &kinds)
}

/// A level to store: the map from the level below and the kind code of each region.
type Level = (Column<u32>, Column<u32>);

/// The levels of `tree`, finest first: the map from the level below (the nodes, for
/// the finest), numbered by smallest node, and the kind code of each region.
fn levels(memory: &Memory, tree: &MapTree) -> Result<Vec<Level>, HierarchyError> {
    let r = tree.parent.len();
    let kids = Children::new(memory, &tree.parent)?;
    let order = kids.order(memory)?;
    let mut depth = Column::new(memory, "map depths", r, |_| 0_u32)?;
    for &x in &order[1..] {
        depth[x as usize] = depth[tree.parent[x as usize] as usize] + 1;
    }
    let top = tree
        .leaf
        .iter()
        .map(|&x| depth[x as usize])
        .max()
        .unwrap_or(0);
    let kinds = |regions: &SegVec<u32>| {
        Column::new(memory, "map kinds", regions.len(), |u| {
            code(tree.kind[regions[u] as usize])
        })
    };
    // The unit of each region on the level being numbered, and that level.
    let mut unit = Column::new(memory, "map units", r, |_| (u32::MAX, 0_u32))?;
    let mut out = Vec::new();
    let mut below: Option<SegVec<u32>> = None;
    for d in (1..=top).rev() {
        let mut regions = SegVec::new(memory, "map units");
        let mut next = 0;
        let mut place = |x: u32| {
            let at = &mut unit[x as usize];
            if at.0 != d {
                *at = (d, next);
                next += 1;
                regions.try_push(x)?;
            }
            Ok::<_, HierarchyError>(at.1)
        };
        let map = match &below {
            None => {
                let mut map = Column::new(memory, "map level", tree.leaf.len(), |_| 0_u32)?;
                for (m, &x) in map.iter_mut().zip(tree.leaf.iter()) {
                    *m = place(x)?;
                }
                map
            }
            Some(b) => {
                let mut map = Column::new(memory, "map level", b.len(), |_| 0_u32)?;
                for (m, &x) in map.iter_mut().zip(b.iter()) {
                    let x = if depth[x as usize] > d {
                        tree.parent[x as usize]
                    } else {
                        x
                    };
                    *m = place(x)?;
                }
                map
            }
        };
        out.push((map, kinds(&regions)?));
        below = Some(regions);
    }
    Ok(out)
}

fn invalid(level: u32, what: &str) -> HierarchyError {
    HierarchyError::Invalid(format!("level {level}: {what}"))
}

/// The kind of each region of `level` of a stored map tree.
///
/// # Errors
///
/// [`HierarchyError::Invalid`] for a kind part missing, of another length than the
/// level, or holding a code that is not a kind; the file's errors and the budget's.
pub fn kinds<S: ByteSource>(
    map: &Hierarchy<'_, S>,
    level: u32,
    memory: &Memory,
) -> Result<Column<Kind>, HierarchyError> {
    let codes = map.column::<u32>(memory, &part(level, KIND))?;
    if codes.len() != map.count(level)? as usize {
        return Err(invalid(level, "kinds of another length than the level"));
    }
    if codes.iter().any(|&c| decode(c).is_none()) {
        return Err(invalid(level, "a kind that is not one"));
    }
    Ok(Column::new(memory, "map kinds", codes.len(), |u| {
        decode(codes[u]).unwrap_or(Kind::Root)
    })?)
}

/// The partition a stored map tree was built from, read from its kinds: the region of
/// kind [`Kind::Community`] that holds each node, numbered by level, then by id.
///
/// # Errors
///
/// [`HierarchyError::Invalid`] for a node under no community; as [`kinds`].
pub fn partition<S: ByteSource>(
    map: &Hierarchy<'_, S>,
    memory: &Memory,
) -> Result<Column<u32>, HierarchyError> {
    let mut next = 0;
    let mut label: Option<Column<u32>> = None;
    for level in 1..=map.levels() {
        let kinds = kinds(map, level, memory)?;
        let parents = map.parents(level, memory)?;
        let mut here = Column::new(memory, "map labels", kinds.len(), |_| u32::MAX)?;
        for (u, h) in here.iter_mut().enumerate() {
            let above = match (&label, &parents) {
                (Some(l), Some(p)) => l[p[u] as usize],
                _ => u32::MAX,
            };
            *h = if above != u32::MAX {
                above
            } else if kinds[u] == Kind::Community {
                next += 1;
                next - 1
            } else {
                u32::MAX
            };
        }
        label = Some(here);
    }
    let label = label.ok_or_else(|| invalid(1, "no level"))?;
    let membership = map.membership(map.levels(), memory)?;
    if membership.iter().any(|&u| label[u as usize] == u32::MAX) {
        return Err(invalid(map.levels(), "a node under no community"));
    }
    Ok(Column::new(memory, "partition", membership.len(), |v| {
        label[membership[v] as usize]
    })?)
}

/// The rules of a stored map tree that `everygraph verify --deep` checks beyond those
/// of every hierarchy: at most `fan_out` (from the record) regions under the root and
/// under any region; a region with one child at the next level is the same region and
/// keeps its kind; on every path, communities only below super groups, exactly one
/// community, and subcommunities only below it.
pub(crate) fn check<S: ByteSource>(
    map: &Hierarchy<'_, S>,
    record: &Provenance,
    memory: &Memory,
) -> Result<(), HierarchyError> {
    let fan_out: u32 = record
        .parameters
        .get("fan_out")
        .and_then(|f| f.parse().ok())
        .ok_or_else(|| invalid(1, "a map tree without its fan-out"))?;
    if map.count(1)? > fan_out {
        return Err(invalid(1, "more regions under the root than the fan-out"));
    }
    // Whether a community is on the path to each region, and the kinds of the level.
    let mut above: Option<(Column<bool>, Column<Kind>)> = None;
    for level in 1..=map.levels() {
        let kinds = kinds(map, level, memory)?;
        let mut under = Column::new(memory, "map check", kinds.len(), |_| false)?;
        match (&above, map.parents(level, memory)?) {
            (Some((state, up)), Some(parents)) => {
                let mut children = Column::new(memory, "map check", up.len(), |_| 0_u32)?;
                for &p in parents.iter() {
                    children[p as usize] += 1;
                }
                if children.iter().any(|&c| c > fan_out) {
                    return Err(invalid(
                        level - 1,
                        "a region with more children than the fan-out",
                    ));
                }
                for (u, &p) in parents.iter().enumerate() {
                    let (p, k) = (p as usize, kinds[u]);
                    under[u] = if children[p] == 1 {
                        if k != up[p] {
                            return Err(invalid(level, "a region repeated with another kind"));
                        }
                        state[p]
                    } else {
                        ordered(state[p], k).ok_or_else(|| invalid(level, "kinds out of order"))?
                    };
                }
            }
            _ => {
                for (u, &k) in kinds.iter().enumerate() {
                    under[u] =
                        ordered(false, k).ok_or_else(|| invalid(level, "kinds out of order"))?;
                }
            }
        }
        above = Some((under, kinds));
    }
    if above.is_some_and(|(under, _)| under.contains(&false)) {
        return Err(invalid(map.levels(), "a region under no community"));
    }
    Ok(())
}

/// Whether a community is on the path once a region of kind `kind` follows a path with
/// one (`under`) or without; `None` when the kind may not follow it.
fn ordered(under: bool, kind: Kind) -> Option<bool> {
    match (kind, under) {
        (Kind::Super, false) | (Kind::Aggregate, _) => Some(under),
        (Kind::Community, false) => Some(true),
        (Kind::Subcommunity, true) => Some(true),
        _ => None,
    }
}
