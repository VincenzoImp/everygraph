// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The map tree (DESIGN D16, ADR 0016): a partition shaped into regions of bounded
//! fan-out that the map can lay out.
//!
//! - **The partition T** is kept exactly: its communities are regions of kind
//!   [`Kind::Community`], at whatever depth.
//! - **Above T**, a super level when T needs more than `F − 1` communities to cover
//!   90% of its nodes with edges: Leiden on the whole graph at the largest resolution
//!   `2^-j` whose groups need at most `F − 1` (or, when none does, the try of the
//!   smallest cover), each community of T in the group that holds most of its nodes.
//! - **Below T**, every region of more than `leaf` nodes is split again by Leiden on
//!   its own induced subgraph (its own null model: local modularity, resolution 1),
//!   until regions are that small or Leiden finds one community. Each region is solved
//!   alone, so its split does not depend on the rest of the graph or on the order of
//!   the work.
//! - **Tails:** a region with more than `F` children keeps its `F − 1` largest and one
//!   aggregate of the others, "+N smaller communities". The communities without an
//!   edge are one aggregate of their own under the root.
//!
//! Every region has a [`Kind`], so the map never draws a drawing aid as a community.
//! [`store`] keeps the tree in the graph file by its parents, with the exact counts of
//! every region and the graph between siblings (`docs/spec/eg-communities.md` § 6);
//! [`StoredMap`] reads it back.

mod check;
mod read;
mod split;
mod store;
mod tails;
mod upper;

use std::cmp::Reverse;
use std::ops::RangeInclusive;

use everygraph_core::{Memory, MemoryError, SegVec};
use everygraph_kernels::edge_map::{Error, Plan, Settings, Source, all};

use crate::aggregate::{Members, Parts, ix};
use crate::column::Column;
use crate::quality::communities;
use crate::{Options, modularity};

pub(crate) use check::check;
pub use read::StoredMap;
pub use store::{OPERATION, VERSION, key, store};
pub use upper::coarse_leiden;

/// What a region of the map tree is, and so how the map draws it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// The whole graph: region 0, and only it.
    Root,
    /// A group of communities of the partition, found by Leiden on the whole graph at
    /// a lower resolution: the super level.
    Super,
    /// A community of the partition.
    Community,
    /// A part of a community, found by splitting it again for the map: a subdivision
    /// made for readability, never a community of the partition.
    Subcommunity,
    /// The smaller children of one parent shown as one region, "+N smaller
    /// communities". An aggregate inside an aggregate is not drawn.
    Aggregate,
}

/// When the map gets a super level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuperLevel {
    /// When the partition needs more than `F − 1` communities to cover 90% of the
    /// nodes. On a rebuild, `previous` is the decision of the map it replaces, kept
    /// while the count stays within `F ± F/8` (`design/DECISIONS.md`, "The map's super
    /// level").
    Auto {
        /// Whether the previous map of the same graph had one; `None` for a first map.
        previous: Option<bool>,
    },
    /// Always.
    Always,
    /// Never.
    Never,
}

impl Default for SuperLevel {
    fn default() -> Self {
        Self::Auto { previous: None }
    }
}

/// What [`build`] does.
#[derive(Clone, Debug, PartialEq)]
pub struct MapOptions {
    /// The options of the Leiden run that found the partition. The splits and the
    /// super level run Leiden with them, at their own resolution.
    pub leiden: Options,
    /// The target fan-out: no region has more children than its end, and [`Shape`]
    /// reports the share of parents inside it. 8 to 64 by default.
    pub fan_out: RangeInclusive<u32>,
    /// Regions of more nodes than this are split again; 64 by default.
    pub leaf: u32,
    /// Automatic by default.
    pub super_level: SuperLevel,
}

impl Default for MapOptions {
    fn default() -> Self {
        Self {
            leiden: Options::default(),
            fan_out: 8..=64,
            leaf: 64,
            super_level: SuperLevel::default(),
        }
    }
}

/// The map tree. Region 0 is the root; every other region has a parent, and the
/// regions with no children (the leaves) partition the nodes.
#[derive(Debug)]
pub struct MapTree {
    /// The parent of each region; the root is its own.
    pub parent: SegVec<u32>,
    /// The kind of each region.
    pub kind: SegVec<Kind>,
    /// The nodes inside each region.
    pub nodes: SegVec<u32>,
    /// The leaf holding each node.
    pub leaf: Column<u32>,
    /// What the shaping achieved.
    pub shape: Shape,
}

/// What [`build`] achieved: the fan-out, the levels, and the top level's modularity.
#[derive(Debug)]
pub struct Shape {
    /// The number of children of each region that has children, increasing.
    pub fan_out: SegVec<u32>,
    /// The target of the options.
    pub target: RangeInclusive<u32>,
    /// Regions on the longest path below the root.
    pub depth: u32,
    /// The communities of the partition, largest first, that cover 90% of its nodes
    /// with edges (communities without an edge left out).
    pub cover: u32,
    /// The resolution of the super level, when there is one.
    pub super_resolution: Option<f64>,
    /// Rounds of splits below the partition.
    pub rounds: u32,
    /// Leaves above the leaf size that Leiden could not split, and their nodes.
    pub unsplit: (u32, u64),
    /// Nodes with an aggregate on their path from the root.
    pub under_aggregate: u64,
    /// The modularity of the top level as drawn: the root's children, each aggregate
    /// as one region.
    pub top_modularity: f64,
}

impl Shape {
    /// The fan-out below which a share `q` (0 to 1) of the parents lie, by the
    /// nearest rank; 0 with no parent.
    #[must_use]
    pub fn quantile(&self, q: f64) -> u32 {
        let len = self.fan_out.len();
        if len == 0 {
            return 0;
        }
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let rank = (q.clamp(0.0, 1.0) * len as f64).ceil() as usize;
        self.fan_out[rank.clamp(1, len) - 1]
    }

    /// The share of parents whose fan-out is in `range`; 0 with no parent.
    #[must_use]
    pub fn share(&self, range: RangeInclusive<u32>) -> f64 {
        let inside = self.fan_out.iter().filter(|f| range.contains(f)).count();
        #[allow(clippy::cast_precision_loss)]
        let share = inside as f64 / self.fan_out.len().max(1) as f64;
        share
    }
}

/// The regions of a tree as it is built.
#[derive(Debug)]
struct Tree {
    parent: SegVec<u32>,
    kind: SegVec<Kind>,
    nodes: SegVec<u32>,
}

impl Tree {
    /// The tree of the root alone.
    fn new(memory: &Memory, n: u32) -> Result<Self, MemoryError> {
        let mut t = Self {
            parent: SegVec::new(memory, "map parents"),
            kind: SegVec::new(memory, "map kinds"),
            nodes: SegVec::new(memory, "map node counts"),
        };
        t.add(0, Kind::Root, n)?;
        Ok(t)
    }

    fn len(&self) -> u32 {
        u32::try_from(self.parent.len()).expect("regions are below 2^32")
    }

    /// Adds a region, and returns its id.
    fn add(&mut self, parent: u32, kind: Kind, nodes: u32) -> Result<u32, MemoryError> {
        let id = self.len();
        self.parent.try_push(parent)?;
        self.kind.try_push(kind)?;
        self.nodes.try_push(nodes)?;
        Ok(id)
    }
}

/// The children of every region, in id order: those of `p` are
/// `ids[starts[p]..starts[p + 1]]`.
struct Children {
    starts: Column<u64>,
    ids: Column<u32>,
}

impl Children {
    fn new(memory: &Memory, parent: &SegVec<u32>) -> Result<Self, MemoryError> {
        let r = parent.len();
        let mut starts = Column::new(memory, "map children", r + 1, |_| 0_u64)?;
        for x in 1..r {
            starts[parent[x] as usize + 1] += 1;
        }
        for p in 0..r {
            starts[p + 1] += starts[p];
        }
        let mut at = Column::new(memory, "map children", r, |p| starts[p])?;
        let mut ids = Column::new(memory, "map children", r.saturating_sub(1), |_| 0_u32)?;
        for x in 1..r {
            let p = parent[x] as usize;
            ids[ix(at[p])] = u32::try_from(x).expect("a region");
            at[p] += 1;
        }
        Ok(Self { starts, ids })
    }

    fn of(&self, p: u32) -> &[u32] {
        let p = p as usize;
        &self.ids[ix(self.starts[p])..ix(self.starts[p + 1])]
    }

    /// Every region, parents before their children: breadth first from the root.
    fn order(&self, memory: &Memory) -> Result<Column<u32>, MemoryError> {
        let mut order = Column::new(memory, "map order", self.ids.len() + 1, |_| 0_u32)?;
        let (mut read, mut write) = (0, 1);
        while read < write {
            for &c in self.of(order[read]) {
                order[write] = c;
                write += 1;
            }
            read += 1;
        }
        Ok(order)
    }
}

/// The map tree of `partition` (one community per node, below the node count), found
/// on `source` by Leiden with `options.leiden`. Deterministic Leiden gives the same tree
/// at any thread count, chunking and source (DESIGN D14).
///
/// # Errors
///
/// [`Error::Invalid`] for a partition that is not one value per node below the node
/// count, or options out of range: a fan-out that does not end at 4 or more or that
/// starts after it ends, a leaf size of 0; the errors of [`leiden`](crate::leiden).
///
/// # Panics
///
/// Never for a source that keeps the contract of [`Source`].
pub fn build<S: Source>(
    source: &S,
    memory: &Memory,
    settings: Settings,
    partition: &[u32],
    options: &MapOptions,
) -> Result<MapTree, Error> {
    let fan_out = *options.fan_out.end();
    if fan_out < 4 || options.fan_out.start() > options.fan_out.end() || options.leaf == 0 {
        return Err(Error::Invalid(
            "map options: the fan-out must end at 4 or more, after its start, and the leaf \
             size be above 0",
        ));
    }
    let n = source.nodes();
    let count = communities(n, partition)?;
    let t = Members::new(
        memory,
        Parts {
            map: partition,
            count,
        },
    )?;
    let size = |c: u32| u32::try_from(t.of(c).len()).expect("a community of nodes");
    let edgeless = edgeless(source, memory, settings, &t, count)?;
    let cover = upper::cover(memory, count as usize, |c| {
        let c = u32::try_from(c).expect("a community");
        if edgeless[c as usize] {
            0
        } else {
            u64::from(size(c))
        }
    })?;
    let groups = if upper::wanted(cover, fan_out, options.super_level) {
        Some(upper::groups(
            source,
            memory,
            settings,
            &options.leiden,
            &t,
            count,
            &edgeless,
            fan_out,
        )?)
    } else {
        None
    };
    let mut tree = Tree::new(memory, n)?;
    let mut order = Column::new(memory, "communities by size", count as usize, |c| {
        u32::try_from(c).expect("a community")
    })?;
    order.sort_unstable_by_key(|&c| (Reverse(size(c)), t.of(c).first().copied()));
    let nonempty = order.partition_point(|&c| size(c) > 0);
    // The region above each community: the root, or its group's region.
    let mut above = Column::new(memory, "community parents", count as usize, |_| 0_u32)?;
    let mut resolution = None;
    if let Some(g) = &groups {
        resolution = Some(g.resolution);
        let mut regions = Column::new(memory, "super regions", g.count as usize, |_| 0_u32)?;
        // Communities, nodes and smallest node of each group.
        let mut held = Column::new(memory, "super sizes", g.count as usize, |_| {
            (0_u32, 0_u32, u32::MAX)
        })?;
        let grouped = || order[..nonempty].iter().filter(|&&c| !edgeless[c as usize]);
        for &c in grouped() {
            let h = &mut held[g.of[c as usize] as usize];
            *h = (h.0 + 1, h.1 + size(c), h.2.min(t.of(c)[0]));
        }
        let mut by_size = Column::new(memory, "super order", g.count as usize, |x| {
            u32::try_from(x).expect("a group")
        })?;
        by_size.sort_unstable_by_key(|&x| (Reverse(held[x as usize].1), held[x as usize].2));
        for &x in by_size.iter() {
            let (communities, nodes, _) = held[x as usize];
            if communities > 1 {
                regions[x as usize] = tree.add(0, Kind::Super, nodes)?;
            }
        }
        for &c in grouped() {
            above[c as usize] = regions[g.of[c as usize] as usize];
        }
    }
    let mut region = Column::new(memory, "community regions", count as usize, |_| 0_u32)?;
    let first = tree.len();
    for &c in &order[..nonempty] {
        region[c as usize] = tree.add(above[c as usize], Kind::Community, size(c))?;
    }
    // The communities without an edge, in one aggregate of their own under the root.
    let alone = order[..nonempty].iter().filter(|&&c| edgeless[c as usize]);
    let mut alone = Column::new(memory, "communities without edges", alone.count(), |_| 0)?;
    let edgeless_regions = order[..nonempty].iter().filter(|&&c| edgeless[c as usize]);
    for (a, &c) in alone.iter_mut().zip(edgeless_regions) {
        *a = region[c as usize];
    }
    if alone.len() > 1 {
        tails::aggregate(&mut tree, &alone, 0, fan_out as usize)?;
    }
    drop((order, above, groups, t, edgeless, alone));
    let mut leaf = Column::new(memory, "map leaves", n as usize, |v| {
        region[partition[v] as usize]
    })?;
    drop(region);
    let sub = Options {
        resolution: 1.0,
        ..options.leiden
    };
    let rounds = split::split(
        source,
        memory,
        settings,
        &sub,
        options.leaf,
        &mut tree,
        &mut leaf,
        first,
    )?;
    tails::bound(memory, &mut tree, fan_out)?;
    let shape = Shape {
        target: options.fan_out.clone(),
        cover,
        super_resolution: resolution,
        rounds,
        ..measure(source, memory, settings, &tree, &leaf, options.leaf)?
    };
    Ok(MapTree {
        parent: tree.parent,
        kind: tree.kind,
        nodes: tree.nodes,
        leaf,
        shape,
    })
}

/// Whether each community of `t` has no edge but loops: none of its nodes has a
/// neighbour other than itself.
fn edgeless<S: Source>(
    source: &S,
    memory: &Memory,
    settings: Settings,
    t: &Members,
    count: u32,
) -> Result<Column<bool>, Error> {
    let mut lonely = Column::new(
        memory,
        "nodes without edges",
        source.nodes() as usize,
        |_| false,
    )?;
    Plan::with(source, memory, settings, |plan| {
        plan.scan(&mut lonely[..], all, |(): &mut (), out, i, row, _| {
            out[i] = row.list.iter().all(|&w| w == row.node);
        })
    })?;
    Ok(Column::new(
        memory,
        "communities without edges",
        count as usize,
        |c| {
            t.of(u32::try_from(c).expect("a community"))
                .iter()
                .all(|&v| lonely[v as usize])
        },
    )?)
}

/// The parts of the shape that come from the tree alone.
fn measure<S: Source>(
    source: &S,
    memory: &Memory,
    settings: Settings,
    tree: &Tree,
    leaf: &[u32],
    leaf_size: u32,
) -> Result<Shape, Error> {
    let r = tree.len() as usize;
    let kids = Children::new(memory, &tree.parent)?;
    let order = kids.order(memory)?;
    // For each region: its depth, its ancestor among the root's children (numbered in
    // id order), and whether an aggregate is on its path.
    let mut at = Column::new(memory, "map depths", r, |_| (0_u32, 0_u32, false))?;
    for (i, &c) in kids.of(0).iter().enumerate() {
        let i = u32::try_from(i).expect("a child");
        at[c as usize] = (1, i, tree.kind[c as usize] == Kind::Aggregate);
    }
    for &x in &order[1..] {
        let (depth, top, under) = at[x as usize];
        for &c in kids.of(x) {
            at[c as usize] = (
                depth + 1,
                top,
                under || tree.kind[c as usize] == Kind::Aggregate,
            );
        }
    }
    let mut fan_out = SegVec::new(memory, "map fan-out");
    let (mut unsplit, mut under_aggregate, mut depth) = ((0, 0), 0, 0);
    for x in 0..tree.len() {
        let children = kids.of(x).len();
        let (d, _, under) = at[x as usize];
        depth = depth.max(d);
        if children > 0 {
            fan_out.try_push(u32::try_from(children).expect("a fan-out"))?;
        } else {
            let nodes = u64::from(tree.nodes[x as usize]);
            if nodes > u64::from(leaf_size) {
                unsplit = (unsplit.0 + 1, unsplit.1 + nodes);
            }
            if under {
                under_aggregate += nodes;
            }
        }
    }
    if let Some(values) = fan_out.as_mut_slice() {
        values.sort_unstable();
    }
    let top = Column::new(memory, "map top level", leaf.len(), |v| {
        at[leaf[v] as usize].1
    })?;
    Ok(Shape {
        fan_out,
        target: 0..=0,
        depth,
        cover: 0,
        super_resolution: None,
        rounds: 0,
        unsplit,
        under_aggregate,
        top_modularity: modularity(source, memory, settings, &top, 1.0)?,
    })
}
