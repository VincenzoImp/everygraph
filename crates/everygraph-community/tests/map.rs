// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The map tree (#63, DESIGN D16).
//!
//! - Oracle: on planted, nested, ring and oracle-corpus graphs, every rule of the tree
//!   is checked by code of its own: node counts, fan-out, kinds on every path, the
//!   partition kept exactly, every split equal to Leiden run on the region's subgraph
//!   built here, every leaf above the leaf size one Leiden cannot split, aggregates that
//!   keep the largest children, and the super level equal to Leiden at its resolution
//!   with a majority vote done here, at the largest resolution that meets the cover.
//! - Metamorphic: the subtree of a community equals the map of that community's
//!   subgraph alone.
//! - Stored: the strictest readers find the file intact; the kinds and the partition
//!   read back are the tree's; the bytes are the same at 1, 3 and 8 threads.
//! - The super level's rule on rebuilds; refused options and budgets; a forged kind
//!   found by `check`; a failing write leaves no result or all of it.

#![forbid(unsafe_code)]
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss
)]

mod support;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::num::NonZeroUsize;
use std::sync::Arc;

use everygraph_community::map::{self, Kind, MapOptions, MapTree, StoredMap, SuperLevel};
use everygraph_community::{Options, leiden};
use everygraph_core::{Budget, ByteSink, ByteSource, Memory, MemorySource, par};
use everygraph_format::adjacency::{IndexBuilder, append_record, encode_events};
use everygraph_format::container::{
    Container, Kind as Section, ResultKey, Value, ValueType, Writer,
};
use everygraph_io::faults::{Faulty, Plan};
use everygraph_kernels::edge_map::{Error, Settings};
use everygraph_oracle::corpus::{Tier, entries};
use everygraph_oracle::projection::Projection;
use support::{Lists, about, checked, graph_file, nested, ring};

/// The tree with its children and the nodes of every region.
struct View<'a> {
    t: &'a MapTree,
    kids: Vec<Vec<u32>>,
    members: Vec<Vec<u32>>,
}

impl<'a> View<'a> {
    fn new(t: &'a MapTree) -> Self {
        let r = t.parent.len();
        let mut kids = vec![Vec::new(); r];
        for x in 1..r {
            assert!((t.parent[x] as usize) < r, "parent of {x}");
            kids[t.parent[x] as usize].push(x as u32);
        }
        let mut members = vec![Vec::new(); r];
        for (v, &l) in t.leaf.iter().enumerate() {
            assert!(
                kids[l as usize].is_empty(),
                "node {v} in region {l}, which has children"
            );
            let mut x = l;
            for _ in 0..=r {
                members[x as usize].push(v as u32);
                if x == 0 {
                    break;
                }
                x = t.parent[x as usize];
            }
            assert_eq!(x, 0, "region {l} does not reach the root");
        }
        Self { t, kids, members }
    }

    fn kind(&self, x: u32) -> Kind {
        self.t.kind[x as usize]
    }

    /// The children of `x` with every aggregate replaced by what it holds.
    fn resolved(&self, x: u32) -> Vec<u32> {
        let mut out = Vec::new();
        for &c in &self.kids[x as usize] {
            if self.kind(c) == Kind::Aggregate {
                out.extend(self.resolved(c));
            } else {
                out.push(c);
            }
        }
        out
    }
}

/// The node sets of a partition of `nodes` (`label[i]` for `nodes[i]`), sorted.
fn sets(nodes: &[u32], label: &[u32]) -> Vec<Vec<u32>> {
    let mut by: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for (&v, &l) in nodes.iter().zip(label) {
        by.entry(l).or_default().push(v);
    }
    let mut out: Vec<Vec<u32>> = by.into_values().collect();
    out.sort();
    out
}

/// The 90% cover of `sizes`, as ADR 0016 defines it.
fn cover(sizes: &[u64]) -> u32 {
    let mut s = sizes.to_vec();
    s.sort_unstable_by(|a, b| b.cmp(a));
    let total: u64 = s.iter().sum();
    let mut held = 0;
    (0..=s.len())
        .find(|&k| {
            let ok = 10 * held >= 9 * total;
            held += s.get(k).copied().unwrap_or(0);
            ok
        })
        .unwrap() as u32
}

/// For each community of `partition`, the label of `found` most of its nodes have,
/// the smallest on a tie.
fn vote(partition: &[u32], found: &[u32]) -> HashMap<u32, u32> {
    let mut counts: HashMap<u32, HashMap<u32, u32>> = HashMap::new();
    for (&c, &g) in partition.iter().zip(found) {
        *counts.entry(c).or_default().entry(g).or_default() += 1;
    }
    counts
        .into_iter()
        .map(|(c, by)| {
            let best = by
                .iter()
                .map(|(&g, &k)| (k, std::cmp::Reverse(g)))
                .max()
                .unwrap();
            (c, best.1.0)
        })
        .collect()
}

fn run(lists: &Lists, o: &Options) -> Vec<u32> {
    leiden(&lists.csr(), &Memory::unlimited(), Settings::default(), o)
        .unwrap()
        .membership
        .to_vec()
}

/// Every rule of the tree of `partition` on `lists`, by code independent of the
/// builder's.
fn rules(name: &str, lists: &Lists, partition: &[u32], t: &MapTree, o: &MapOptions) {
    let v = View::new(t);
    let f = *o.fan_out.end() as usize;
    let r = t.parent.len() as u32;
    assert_eq!(
        (t.kind[0], t.parent[0]),
        (Kind::Root, 0),
        "{name}: the root"
    );
    let sub = Options {
        resolution: 1.0,
        ..o.leiden
    };
    for x in 0..r {
        let (kids, members) = (&v.kids[x as usize], &v.members[x as usize]);
        assert_eq!(
            members.len(),
            t.nodes[x as usize] as usize,
            "{name}: nodes of {x}"
        );
        assert!(kids.len() <= f, "{name}: {x} has {} children", kids.len());
        assert!(x == 0 || kids.len() != 1, "{name}: {x} has one child");
        let kind = v.kind(x);
        assert!((kind == Kind::Root) == (x == 0), "{name}: kind of {x}");
        if x == 0 {
            continue;
        }
        // Kinds on the path: super groups only above the communities, exactly one
        // community, subcommunities only below it.
        let path: Vec<Kind> =
            std::iter::successors(Some(x), |&y| (y != 0).then(|| t.parent[y as usize]))
                .map(|y| v.kind(y))
                .collect();
        let communities = path.iter().filter(|&&k| k == Kind::Community).count();
        match kind {
            Kind::Super => assert_eq!(communities, 0, "{name}: super {x} under a community"),
            Kind::Subcommunity => assert_eq!(communities, 1, "{name}: subcommunity {x}"),
            Kind::Community => assert_eq!(communities, 1, "{name}: community {x}"),
            _ => assert!(communities <= 1, "{name}: two communities above {x}"),
        }
        if kids.is_empty() {
            assert_eq!(communities, 1, "{name}: leaf {x} under no community");
        }
        // Splits: Leiden on the region's subgraph, built here.
        if matches!(kind, Kind::Community | Kind::Subcommunity) {
            let parts = v.resolved(x);
            if members.len() <= o.leaf as usize {
                assert!(parts.is_empty(), "{name}: {x} is small and split");
                continue;
            }
            let inside = |u: u32| {
                lists
                    .list(u)
                    .iter()
                    .any(|&w| w != u && members.binary_search(&w).is_ok())
            };
            if !members.iter().any(|&u| inside(u)) {
                assert!(
                    parts.is_empty(),
                    "{name}: {x} has no edge inside and is split"
                );
                continue;
            }
            let found = run(&lists.induced(members), &sub);
            let mut got: Vec<Vec<u32>> = parts
                .iter()
                .map(|&c| v.members[c as usize].clone())
                .collect();
            got.sort();
            let want = sets(members, &found);
            if want.len() == 1 {
                assert!(
                    got.is_empty(),
                    "{name}: {x} split where Leiden finds one community"
                );
            } else {
                assert_eq!(got, want, "{name}: the split of {x}");
                for &c in &parts {
                    assert_eq!(v.kind(c), Kind::Subcommunity, "{name}: kind of {c}");
                }
                // Numbered largest first, then by smallest node.
                let rank = |&c: &u32| {
                    (
                        std::cmp::Reverse(t.nodes[c as usize]),
                        v.members[c as usize][0],
                    )
                };
                let mut by_rank = parts.clone();
                by_rank.sort_by_key(rank);
                assert!(
                    by_rank.windows(2).all(|w| w[0] < w[1]),
                    "{name}: order under {x}"
                );
            }
        }
    }
    // Communities without an edge: two or more are one aggregate under the root.
    let lonely = |c: &[u32]| c.iter().all(|&u| lists.list(u).iter().all(|&w| w == u));
    let edgeless: Vec<u32> = (0..r)
        .filter(|&x| v.kind(x) == Kind::Community && lonely(&v.members[x as usize]))
        .collect();
    let is_edgeless: HashSet<u32> = edgeless.iter().copied().collect();
    let apart = v.kids[0].iter().copied().find(|&c| {
        v.kind(c) == Kind::Aggregate && v.resolved(c).iter().all(|y| is_edgeless.contains(y))
    });
    if edgeless.len() > 1 {
        let mut held = v.resolved(apart.expect("an aggregate of communities without edges"));
        held.sort_unstable();
        assert_eq!(held, edgeless, "{name}: communities without edges");
    }
    // Aggregates: under each parent, the fan-out minus one largest stay outside.
    for x in 0..r {
        if v.kind(x) == Kind::Aggregate {
            continue;
        }
        let direct: Vec<u32> = v.kids[x as usize]
            .iter()
            .copied()
            .filter(|&c| v.kind(c) != Kind::Aggregate)
            .collect();
        let mut all = v.resolved(x);
        let mut f = f;
        if let (0, Some(_), true) = (x, apart, edgeless.len() > 1) {
            all.retain(|c| !is_edgeless.contains(c));
            f -= 1;
        }
        if all.len() <= f {
            assert_eq!(direct.len(), all.len(), "{name}: an aggregate under {x}");
            continue;
        }
        assert_eq!(direct.len(), f - 1, "{name}: children kept by {x}");
        let smallest = direct.iter().map(|&c| t.nodes[c as usize]).min().unwrap();
        let inside = all.iter().filter(|c| !direct.contains(c));
        assert!(
            inside.map(|&c| t.nodes[c as usize]).all(|n| n <= smallest),
            "{name}: {x}"
        );
    }
    // The partition is kept: its communities are the regions of that kind.
    let mut communities: Vec<Vec<u32>> = (0..r)
        .filter(|&x| v.kind(x) == Kind::Community)
        .map(|x| v.members[x as usize].clone())
        .collect();
    communities.sort();
    let all: Vec<u32> = (0..lists.nodes()).collect();
    assert_eq!(communities, sets(&all, partition), "{name}: the partition");
    // The super level: Leiden at its resolutions and a vote, here, without the
    // communities without an edge.
    let sizes: Vec<u64> = communities
        .iter()
        .filter(|c| !lonely(c))
        .map(|c| c.len() as u64)
        .collect();
    assert_eq!(t.shape.cover, cover(&sizes), "{name}: cover");
    let group_of = |x: u32| -> Option<u32> {
        std::iter::successors(Some(x), |&y| (y != 0).then(|| t.parent[y as usize]))
            .find(|&y| v.kind(y) == Kind::Super)
    };
    let region_of: HashMap<u32, u32> = (0..r)
        .filter(|&x| v.kind(x) == Kind::Community && !is_edgeless.contains(&x))
        .map(|x| (partition[v.members[x as usize][0] as usize], x))
        .collect();
    let groups_at = |gamma: f64| {
        let memory = Memory::unlimited();
        let found =
            map::coarse_leiden(&lists.csr(), &memory, Settings::default(), &o.leiden, gamma);
        let mut groups = vote(partition, &found.unwrap().membership);
        groups.retain(|c, _| region_of.contains_key(c));
        let mut nodes: HashMap<u32, u64> = HashMap::new();
        for (c, g) in &groups {
            *nodes.entry(*g).or_default() += u64::from(t.nodes[region_of[c] as usize]);
        }
        (groups, cover(&nodes.into_values().collect::<Vec<_>>()))
    };
    let f32 = f as u32;
    match t.shape.super_resolution {
        None => assert!(
            o.super_level == SuperLevel::Never
                || (o.super_level != SuperLevel::Always && cover(&sizes) < f32),
            "{name}: no super level"
        ),
        Some(gamma) => {
            // The first 2^-j that meets the cover; or, once the cover stops falling or at
            // 2^-16, the smallest cover, the larger resolution on a tie.
            let (mut best, mut last, mut at) = (u32::MAX, u32::MAX, 1.0);
            let mut chosen = 0.0;
            for _ in 0..16 {
                at /= 2.0;
                let c = groups_at(at).1;
                if c < best {
                    (best, chosen) = (c, at);
                }
                if c < f32 || c >= last {
                    break;
                }
                last = c;
            }
            assert_eq!(gamma, chosen, "{name}: the resolution of the super level");
            let (groups, _) = groups_at(gamma);
            let mut by: HashMap<u32, Vec<u32>> = HashMap::new();
            for (&c, &g) in &groups {
                by.entry(g).or_default().push(c);
            }
            let mut seen = Vec::new();
            for members in by.values() {
                let supers: Vec<Option<u32>> =
                    members.iter().map(|c| group_of(region_of[c])).collect();
                if members.len() == 1 {
                    assert_eq!(supers, vec![None], "{name}: a group of one community");
                } else {
                    assert!(
                        supers[0].is_some() && supers.iter().all(|s| *s == supers[0]),
                        "{name}: a group"
                    );
                    seen.push(supers[0]);
                }
            }
            let groups = seen.len();
            seen.sort_unstable();
            seen.dedup();
            assert_eq!(seen.len(), groups, "{name}: two groups in one super region");
            assert_eq!(
                groups,
                (0..r).filter(|&x| v.kind(x) == Kind::Super).count(),
                "{name}"
            );
        }
    }
    for &x in &edgeless {
        assert_eq!(
            group_of(x),
            None,
            "{name}: a community without edges in a group"
        );
    }
}

/// The map tree of `lists` and its partition (Leiden's when `partition` is `None`).
fn tree(lists: &Lists, partition: Option<&[u32]>, o: &MapOptions) -> (Vec<u32>, MapTree) {
    let memory = Memory::unlimited();
    let partition = partition.map_or_else(|| run(lists, &o.leiden), <[u32]>::to_vec);
    let t = map::build(&lists.csr(), &memory, Settings::default(), &partition, o).unwrap();
    (partition, t)
}

/// A star of 300 leaves joined to a clique of 29 nodes: a region Leiden cannot split.
fn star_and_clique() -> Lists {
    let mut pairs: Vec<(u32, u32)> = (1..=300).map(|v| (0, v)).collect();
    for a in 301..330 {
        for b in a + 1..330 {
            pairs.push((a, b));
        }
    }
    pairs.push((0, 301));
    Lists::from_pairs(330, &pairs)
}

/// The graphs of the oracle tests, with a partition of their own or `None`.
fn graphs() -> Vec<(String, Lists, Option<Vec<u32>>)> {
    let mut out = Vec::new();
    for (name, (n, pairs)) in [
        ("nested 12x6x30", nested(&[30, 6, 12], &[5, 2, 1, 0], 1)),
        ("nested 8x400", nested(&[400, 8], &[6, 1, 0], 2)),
        ("sparse 3000", nested(&[3000], &[4, 0], 3)),
    ] {
        out.push((name.to_owned(), Lists::from_pairs(n, &pairs), None));
    }
    // 90 cliques as the partition: a super level.
    let (n, pairs) = ring(90, 4);
    let cliques = (0..n).map(|v| v / 4).collect();
    out.push((
        "ring of 90 cliques".to_owned(),
        Lists::from_pairs(n, &pairs),
        Some(cliques),
    ));
    out.push(("star and clique".to_owned(), star_and_clique(), None));
    // Nodes without edges: one community of 40 of them, and 30 alone beside a super level.
    for (cliques, extra, together) in [(20, 40, true), (90, 30, false)] {
        let (n, pairs) = ring(cliques, 4);
        let mut partition: Vec<u32> = (0..n).map(|v| v / 4).collect();
        partition.extend((0..extra).map(|i| if together { n } else { n + i }));
        let name = format!("ring of {cliques} cliques and {extra} nodes without edges");
        out.push((name, Lists::from_pairs(n + extra, &pairs), Some(partition)));
    }
    for e in entries(Tier::Ci) {
        let p = Projection::new(&e.graph);
        let n = p.undirected.n() as u32;
        let mut pairs = Vec::new();
        for u in 0..n {
            pairs.extend(p.undirected.list(u as usize).iter().map(|&w| (u, w)));
            if p.kept_loops(u as usize) > 0 {
                pairs.push((u, u));
            }
        }
        out.push((e.graph.name.clone(), Lists::from_pairs(n, &pairs), None));
    }
    out
}

#[test]
fn every_map_tree_keeps_its_rules_and_matches_the_oracle() {
    let small = MapOptions {
        fan_out: 2..=8,
        leaf: 16,
        ..MapOptions::default()
    };
    for (name, lists, partition) in graphs() {
        for o in [MapOptions::default(), small.clone()] {
            let (partition, t) = tree(&lists, partition.as_deref(), &o);
            rules(&name, &lists, &partition, &t, &o);
            let s = &t.shape;
            assert!(s.quantile(1.0) <= *o.fan_out.end(), "{name}");
            assert!((0.0..=1.0).contains(&s.share(o.fan_out.clone())), "{name}");
        }
    }
}

#[test]
fn the_super_level_and_aggregates_appear_where_the_rules_say() {
    let (n, pairs) = ring(90, 4);
    let lists = Lists::from_pairs(n, &pairs);
    let cliques: Vec<u32> = (0..n).map(|v| v / 4).collect();
    let (_, t) = tree(&lists, Some(&cliques), &MapOptions::default());
    assert_eq!(t.shape.cover, 81);
    assert!(t.shape.super_resolution.is_some());
    assert!(t.kind.iter().any(|&k| k == Kind::Super));
    let never = MapOptions {
        super_level: SuperLevel::Never,
        ..MapOptions::default()
    };
    let (_, t) = tree(&lists, Some(&cliques), &never);
    assert!(t.shape.super_resolution.is_none());
    // 90 children of the root: 63 kept, 27 in one aggregate.
    let aggregates: Vec<usize> = (0..t.parent.len())
        .filter(|&x| t.kind[x] == Kind::Aggregate)
        .collect();
    assert_eq!(aggregates.len(), 1);
    assert_eq!(t.nodes[aggregates[0]], 27 * 4);
    assert_eq!(t.shape.under_aggregate, 27 * 4);
    let (_, t) = tree(&star_and_clique(), None, &MapOptions::default());
    assert_eq!(t.shape.unsplit, (1, 301));
}

#[test]
fn the_rebuild_rule_keeps_the_previous_decision_inside_its_band() {
    // A partition of k cliques of 4 nodes needs ceil(0.9 k) of them to cover 90%.
    for (k, first, built, not_built) in [
        (61, false, false, false), // cover 55: below 56..=72
        (62, false, true, false),  // cover 56: inside
        (70, false, true, false),  // cover 63
        (71, true, true, false),   // cover 64
        (80, true, true, false),   // cover 72: inside
        (81, true, true, true),    // cover 73: above
    ] {
        let (n, pairs) = ring(k, 4);
        let lists = Lists::from_pairs(n, &pairs);
        let cliques: Vec<u32> = (0..n).map(|v| v / 4).collect();
        for (previous, want) in [(None, first), (Some(true), built), (Some(false), not_built)] {
            let o = MapOptions {
                super_level: SuperLevel::Auto { previous },
                ..MapOptions::default()
            };
            let (_, t) = tree(&lists, Some(&cliques), &o);
            assert_eq!(
                t.shape.super_resolution.is_some(),
                want,
                "{k} cliques, {previous:?}"
            );
        }
    }
}

/// The node sets and kinds of the subtree under `root`, nodes mapped by `name`.
fn subtree(t: &MapTree, root: u32, name: impl Fn(u32) -> u32) -> Vec<(Vec<u32>, String, Vec<u32>)> {
    let v = View::new(t);
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(x) = stack.pop() {
        let mut nodes: Vec<u32> = v.members[x as usize].iter().map(|&u| name(u)).collect();
        nodes.sort_unstable();
        let mut kids: Vec<u32> = v.kids[x as usize]
            .iter()
            .map(|&c| {
                v.members[c as usize]
                    .iter()
                    .map(|&u| name(u))
                    .min()
                    .unwrap()
            })
            .collect();
        kids.sort_unstable();
        out.push((nodes, format!("{:?}", v.kind(x)), kids));
        stack.extend(&v.kids[x as usize]);
    }
    out.sort();
    out
}

#[test]
fn a_community_alone_gives_the_subtree_it_has_in_the_whole_map() {
    let (n, pairs) = nested(&[40, 8, 6], &[6, 2, 1, 0], 7);
    let lists = Lists::from_pairs(n, &pairs);
    let o = MapOptions {
        fan_out: 4..=6,
        leaf: 20,
        super_level: SuperLevel::Never,
        ..MapOptions::default()
    };
    let (_, whole) = tree(&lists, None, &o);
    let v = View::new(&whole);
    let mut communities: Vec<u32> = (0..whole.parent.len() as u32)
        .filter(|&x| whole.kind[x as usize] == Kind::Community)
        .collect();
    communities.sort_by_key(|&x| std::cmp::Reverse(whole.nodes[x as usize]));
    let mut compared = 0;
    for &c in communities.iter().take(3) {
        let members = v.members[c as usize].clone();
        if members.len() <= o.leaf as usize {
            continue;
        }
        let alone = lists.induced(&members);
        let (_, part) = tree(&alone, Some(&vec![0; members.len()]), &o);
        let first = (0..part.parent.len() as u32)
            .find(|&x| part.kind[x as usize] == Kind::Community)
            .unwrap();
        assert_eq!(
            subtree(&part, first, |u| members[u as usize]),
            subtree(&whole, c, |u| u),
            "community {c}"
        );
        compared += 1;
    }
    assert!(compared > 0);
}

/// `lists` and the map tree of `t` in one committed file, and its key.
fn stored(lists: &Lists, t: &MapTree, o: &MapOptions) -> (Vec<u8>, ResultKey) {
    let mut w = graph_file(lists);
    let key = map::key(w.graph_identity(), o).unwrap();
    map::store(
        &mut w,
        &key,
        &about(),
        &lists.csr(),
        &Memory::unlimited(),
        Settings::default(),
        t,
    )
    .unwrap();
    w.commit().unwrap();
    (w.into_sink().into_inner(), key)
}

/// A stored map read back: per region its parent, kind and nodes, and per node its leaf.
struct Read {
    parent: Vec<u32>,
    kind: Vec<Kind>,
    members: Vec<Vec<u32>>,
}

impl Read {
    fn of(m: &StoredMap<'_, MemorySource<&[u8]>>) -> Self {
        let memory = Memory::unlimited();
        let parent = m.parents(&memory).unwrap().to_vec();
        let kind = m.kinds(&memory).unwrap().to_vec();
        let mut members = vec![Vec::new(); parent.len()];
        for (v, &l) in m.membership(&memory).unwrap().iter().enumerate() {
            for x in Self::path(&parent, l) {
                members[x as usize].push(v as u32);
            }
        }
        Self {
            parent,
            kind,
            members,
        }
    }

    /// `x` and its ancestors, the root last.
    fn path(parent: &[u32], x: u32) -> Vec<u32> {
        std::iter::successors(Some(x), |&y| (y != 0).then(|| parent[y as usize])).collect()
    }
}

#[test]
fn a_stored_map_reads_back_the_tree_its_counts_and_its_siblings_and_verifies() {
    let memory = Memory::unlimited();
    for (name, lists, partition) in graphs() {
        let o = MapOptions {
            fan_out: 2..=8,
            leaf: 16,
            ..MapOptions::default()
        };
        let (partition, t) = tree(&lists, partition.as_deref(), &o);
        let (bytes, key) = stored(&lists, &t, &o);
        let report = checked(&bytes);
        assert!(
            report.is_intact() && report.unchecked.is_empty(),
            "{name}: {report:?}"
        );
        let file = Container::open(MemorySource::new(&bytes[..])).unwrap();
        let m = StoredMap::find(&file, &key).unwrap().unwrap();
        assert_eq!(
            m.super_level(),
            t.shape.super_resolution.is_some(),
            "{name}"
        );
        let read = Read::of(&m);
        // The same tree: every region by its nodes, kind and parent's nodes.
        let v = View::new(&t);
        let regions = |members: &[Vec<u32>],
                       kind: &dyn Fn(usize) -> Kind,
                       parent: &dyn Fn(usize) -> usize| {
            let mut out: Vec<(Vec<u32>, String, Vec<u32>)> = (1..members.len())
                .map(|x| {
                    (
                        members[x].clone(),
                        format!("{:?}", kind(x)),
                        members[parent(x)].clone(),
                    )
                })
                .collect();
            out.sort();
            out
        };
        assert_eq!(
            regions(&read.members, &|x| read.kind[x], &|x| read.parent[x]
                as usize),
            regions(&v.members, &|x| t.kind[x], &|x| t.parent[x] as usize),
            "{name}: the tree"
        );
        // The partition, read from the kinds.
        let mut communities: Vec<Vec<u32>> = (0..read.kind.len())
            .filter(|&x| read.kind[x] == Kind::Community)
            .map(|x| read.members[x].clone())
            .collect();
        communities.sort();
        let all: Vec<u32> = (0..lists.nodes()).collect();
        assert_eq!(communities, sets(&all, &partition), "{name}: the partition");
        // Counts and siblings, from the lists: each pair at its lowest common ancestor.
        let r = read.parent.len();
        let leaf = m.membership(&memory).unwrap();
        let (mut internal, mut volume) = (vec![0_u64; r], vec![0_u64; r]);
        let mut siblings: BTreeMap<(u32, u32), u64> = BTreeMap::new();
        for u in 0..lists.nodes() {
            let pu = Read::path(&read.parent, leaf[u as usize]);
            for &w in lists.list(u) {
                for &x in &pu {
                    volume[x as usize] += if w == u { 2 } else { 1 };
                }
                if w < u {
                    continue;
                }
                let pw = Read::path(&read.parent, leaf[w as usize]);
                let common: Vec<u32> = pu.iter().copied().filter(|x| pw.contains(x)).collect();
                for &x in &common {
                    internal[x as usize] += 1;
                }
                let below = |p: &[u32]| {
                    p.iter()
                        .copied()
                        .find(|y| read.parent[*y as usize] == common[0] && *y != common[0])
                };
                if let (Some(a), Some(b)) = (below(&pu), below(&pw)) {
                    *siblings.entry((a, b)).or_default() += 1;
                    *siblings.entry((b, a)).or_default() += 1;
                }
            }
        }
        let table = m.table(&memory).unwrap();
        assert_eq!(table.internal.to_vec(), internal, "{name}: internal");
        assert_eq!(table.volume.to_vec(), volume, "{name}: volume");
        assert_eq!(stored_siblings(&m), siblings, "{name}: siblings");
    }
}

/// The entries of a stored sibling graph, `(a, b) → weight`.
fn stored_siblings(m: &StoredMap<'_, MemorySource<&[u8]>>) -> BTreeMap<(u32, u32), u64> {
    let s = m.siblings().unwrap();
    let adjacency = s.adjacency().unwrap();
    let events = s.events().unwrap();
    let mut out = BTreeMap::new();
    let (mut buf, mut entry) = (Vec::new(), 0);
    for a in 0..m.regions() {
        for &b in adjacency
            .neighbors(everygraph_core::NodeId::new(a), &mut buf)
            .unwrap()
        {
            let w = events.map_or(1, |e| {
                let r = e.event_range(entry).unwrap();
                r.end - r.start
            });
            out.insert((a, b), w);
            entry += 1;
        }
    }
    out
}

#[test]
fn the_same_input_gives_the_same_bytes_at_any_thread_count() {
    // One community of 4.5 M entries: a region solved alone, then many solved together.
    let (n, pairs) = nested(&[500, 520], &[8, 1, 0], 11);
    let lists = Lists::from_pairs(n, &pairs);
    let everything = vec![0; n as usize];
    let o = MapOptions::default();
    let bytes = |threads: usize| {
        par::with_threads(NonZeroUsize::new(threads).unwrap(), || {
            let (_, t) = tree(&lists, Some(&everything), &o);
            assert!(t.shape.rounds > 1);
            stored(&lists, &t, &o).0
        })
        .unwrap()
    };
    let one = bytes(1);
    assert!(checked(&one).is_intact());
    for t in [3, 8] {
        assert!(bytes(t) == one, "{t} threads");
    }
}

#[test]
fn bad_options_and_small_budgets_are_refused() {
    let (n, pairs) = nested(&[100, 4], &[5, 1, 0], 5);
    let lists = Lists::from_pairs(n, &pairs);
    let partition = run(&lists, &Options::default());
    let memory = Memory::unlimited();
    for o in [
        MapOptions {
            fan_out: 2..=3,
            ..MapOptions::default()
        },
        MapOptions {
            fan_out: std::ops::RangeInclusive::new(9, 8),
            ..MapOptions::default()
        },
        MapOptions {
            leaf: 0,
            ..MapOptions::default()
        },
    ] {
        let e = map::build(&lists.csr(), &memory, Settings::default(), &partition, &o);
        assert!(matches!(e, Err(Error::Invalid(_))), "{o:?}");
    }
    let e = map::build(
        &lists.csr(),
        &memory,
        Settings::default(),
        &partition[1..],
        &MapOptions::default(),
    );
    assert!(matches!(e, Err(Error::Invalid(_))));
    for limit in [0, 1 << 10, 1 << 14] {
        let small = Memory::new(Arc::new(Budget::new(limit)));
        let e = map::build(
            &lists.csr(),
            &small,
            Settings::default(),
            &partition,
            &MapOptions::default(),
        );
        assert!(matches!(e, Err(Error::Memory(_))), "{limit}");
    }
}

#[test]
fn check_finds_a_stored_map_that_breaks_its_rules() {
    let (n, pairs) = nested(&[30, 6, 12], &[5, 2, 1, 0], 1);
    let lists = Lists::from_pairs(n, &pairs);
    let o = MapOptions {
        leaf: 16,
        super_level: SuperLevel::Never,
        ..MapOptions::default()
    };
    let (_, t) = tree(&lists, None, &o);
    let (bytes, key) = stored(&lists, &t, &o);
    let file = Container::open(MemorySource::new(&bytes[..])).unwrap();
    let m = StoredMap::find(&file, &key).unwrap().unwrap();
    let memory = Memory::unlimited();
    let read = Read::of(&m);
    let codes: Vec<u32> = read
        .kind
        .iter()
        .map(|&k| match k {
            Kind::Root => 0,
            Kind::Super => 1,
            Kind::Community => 2,
            Kind::Subcommunity => 3,
            Kind::Aggregate => 4,
        })
        .collect();
    let r = codes.len();
    let kids = |p: usize| (1..r).filter(|&x| read.parent[x] as usize == p).count();
    // A community with subcommunities, and a leaf below a community.
    let wide = (1..r).find(|&x| codes[x] == 2 && kids(x) > 1).unwrap();
    let low = (1..r)
        .rev()
        .find(|&x| codes[x] == 3 && kids(x) == 0)
        .unwrap();
    let with = |at: usize, code: u32| {
        let mut c = codes.clone();
        c[at] = code;
        c
    };
    let parents = read.parent.clone();
    let mut swapped = parents.clone();
    swapped[r - 1] = 0;
    let table = m.table(&memory).unwrap();
    let mut internal = table.internal.to_vec();
    internal[wide] += 1;
    let mut volume = table.volume.to_vec();
    volume[low] += 2;
    let mut nodes = table.nodes.to_vec();
    nodes[low] += 1;
    let mut membership = m.membership(&memory).unwrap().to_vec();
    let other = membership.iter().position(|&l| l != membership[0]).unwrap();
    membership.swap(0, other);
    let cases: Vec<(&str, Vec<u8>, &str)> = vec![
        ("kind", typed(&with(wide, 3)), "kinds out of order"),
        ("kind", typed(&with(low, 2)), "kinds out of order"),
        ("kind", typed(&with(0, 2)), "kinds out of order"),
        ("kind", typed(&with(low, 9)), "a kind that is not one"),
        ("parent", typed(&swapped), "breadth first"),
        ("internal", typed(&internal), "counts that are not"),
        ("volume", typed(&volume), "counts"),
        ("nodes", typed(&nodes), "node counts"),
        ("membership", typed(&membership), "map:"),
    ];
    let whole = Parts::of(&m, &file, &key);
    let r32 = r as u32;
    // Two sibling leaves of different sizes, numbered the wrong way round.
    let leaves = (2..r32).find(|&x| {
        let (a, b) = (x as usize - 1, x as usize);
        read.parent[a] == read.parent[b]
            && kids(a) == 0
            && kids(b) == 0
            && whole.nodes[a] > whole.nodes[b]
    });
    let forged_whole = vec![(
        whole.swapped(leaves.unwrap() - 1, leaves.unwrap()),
        "numbered",
    )];
    // One pair moved around a cycle of four siblings: `a–b` and `c–d` gain one, `a–d` and
    // `c–b` lose one, so every row keeps its sum and every region its counts.
    let w = |x: u32, y: u32| whole.siblings.get(&(x, y)).copied().unwrap_or(0);
    let groups = (0..r32).map(|p| {
        (1..r32)
            .filter(|&x| read.parent[x as usize] == p)
            .collect::<Vec<_>>()
    });
    let cycle = groups
        .flat_map(|g| {
            let g2 = g.clone();
            g.into_iter().flat_map(move |a| {
                let g3 = g2.clone();
                g2.clone()
                    .into_iter()
                    .flat_map(move |b| g3.clone().into_iter().map(move |c| (a, b, c)))
            })
        })
        .find_map(|(a, b, c)| {
            let distinct = a != b && a != c && b != c;
            let d = (1..r32).find(|&d| {
                d != a
                    && d != b
                    && d != c
                    && read.parent[d as usize] == read.parent[a as usize]
                    && w(a, b) >= 1
                    && w(c, d) >= 1
                    && w(a, d) >= 2
                    && w(c, b) >= 2
            });
            d.filter(|_| distinct).map(|d| (a, b, c, d))
        })
        .expect("four siblings with pairs between them");
    let (a, b, c, d) = cycle;
    let mut moved = whole.clone();
    for (x, y, change) in [(a, b, 1_i64), (c, d, 1), (a, d, -1), (c, b, -1)] {
        for key in [(x, y), (y, x)] {
            let e = moved.siblings.get_mut(&key).unwrap();
            *e = e.checked_add_signed(change).unwrap();
        }
    }
    let mut forged_whole = forged_whole;
    forged_whole.push((moved, "not what the tree makes"));
    // One more pair inside a leaf and each of its ancestors: counts that add up, but
    // not to the graph's.
    let mut inside = whole.clone();
    for x in Read::path(&read.parent, low as u32) {
        inside.internal[x as usize] += 1;
    }
    forged_whole.push((inside, "not those of the graph"));
    for (forged, what) in forged_whole {
        let report = checked(&forged.written(&bytes, &key));
        assert!(
            report.problems.iter().any(|p| p.what.contains(what)),
            "{what}: {:?}",
            report.problems
        );
    }
    assert!(checked(&whole.written(&bytes, &key)).is_intact());
    for (part, values, what) in cases {
        let report = checked(&forged(&bytes, &key, part, values));
        assert!(
            report.problems.iter().any(|p| p
                .section
                .as_deref()
                .is_some_and(|s| s.starts_with("results/map/"))
                && p.what.contains(what)),
            "{part}, {what}: {:?}",
            report.problems
        );
    }
}

/// Every part of a stored map, to forge one.
#[derive(Clone)]
struct Parts {
    head: Vec<u64>,
    parent: Vec<u32>,
    kind: Vec<u32>,
    nodes: Vec<u32>,
    internal: Vec<u64>,
    volume: Vec<u64>,
    membership: Vec<u32>,
    siblings: BTreeMap<(u32, u32), u64>,
}

impl Parts {
    fn of(
        m: &StoredMap<'_, MemorySource<&[u8]>>,
        file: &Container<MemorySource<&[u8]>>,
        key: &ResultKey,
    ) -> Self {
        let memory = Memory::unlimited();
        let root = file.find_result(key).unwrap().unwrap();
        let mut head = vec![0_u64; root.len() as usize];
        root.read(0, &mut head).unwrap();
        let table = m.table(&memory).unwrap();
        let read = Read::of(m);
        Self {
            head,
            parent: read.parent,
            kind: read.kind.iter().map(|&k| code(k)).collect(),
            nodes: table.nodes.to_vec(),
            internal: table.internal.to_vec(),
            volume: table.volume.to_vec(),
            membership: m.membership(&memory).unwrap().to_vec(),
            siblings: stored_siblings(m),
        }
    }

    /// `bytes` with the map of `key` written from these parts.
    fn written(&self, bytes: &[u8], key: &ResultKey) -> Vec<u8> {
        let r = self.parent.len();
        let mut lists = vec![Vec::new(); r];
        for (&(a, b), &w) in &self.siblings {
            lists[a as usize].push((b, w));
        }
        let mut index = IndexBuilder::new(&Memory::unlimited(), r as u32).unwrap();
        let (mut data, mut weights) = (Vec::new(), Vec::new());
        for (a, l) in lists.iter().enumerate() {
            let before = data.len();
            let d = append_record(a as u32, l.iter().map(|e| e.0), &mut data).unwrap();
            index.push(d, (data.len() - before) as u64).unwrap();
            weights.extend(l.iter().map(|e| e.1));
        }
        let (entries, events) = (weights.len() as u64, weights.iter().sum::<u64>());
        let index = index.finish(events > entries).unwrap().0;
        let mut w = Writer::update(MemorySource::new(bytes.to_vec())).unwrap();
        w.add_result(key, &about(), &self.head).unwrap();
        w.add_result_part(key, "parent", self.parent.iter().copied())
            .unwrap();
        w.add_result_part(key, "kind", self.kind.iter().copied())
            .unwrap();
        w.add_result_part(key, "nodes", self.nodes.iter().copied())
            .unwrap();
        w.add_result_part(key, "internal", self.internal.iter().copied())
            .unwrap();
        w.add_result_part(key, "volume", self.volume.iter().copied())
            .unwrap();
        w.add_result_part(key, "membership", self.membership.iter().copied())
            .unwrap();
        let mut parts = vec![
            ("index", Section::RESULT_ADJACENCY_INDEX, index),
            ("data", Section::RESULT_ADJACENCY_DATA, data),
        ];
        if events > entries {
            let e = encode_events(entries, events, weights).unwrap();
            parts.push(("events", Section::RESULT_ADJACENCY_EVENTS, e));
        }
        for (suffix, kind, b) in parts {
            let mut s = w
                .begin_result_part(key, &format!("siblings.{suffix}"), kind)
                .unwrap();
            s.write(&b).unwrap();
            s.finish().unwrap();
        }
        w.commit().unwrap();
        w.into_sink().into_inner()
    }

    /// The parts with regions `a` and `b` swapped.
    fn swapped(&self, a: u32, b: u32) -> Self {
        let map = |x: u32| {
            if x == a {
                b
            } else if x == b {
                a
            } else {
                x
            }
        };
        let mut p = self.clone();
        for v in [&mut p.kind, &mut p.nodes] {
            v.swap(a as usize, b as usize);
        }
        p.internal.swap(a as usize, b as usize);
        p.volume.swap(a as usize, b as usize);
        p.parent.swap(a as usize, b as usize);
        for x in &mut p.parent {
            *x = map(*x);
        }
        for x in &mut p.membership {
            *x = map(*x);
        }
        p.siblings = self
            .siblings
            .iter()
            .map(|(&(x, y), &w)| ((map(x), map(y)), w))
            .collect();
        p
    }
}

fn code(k: Kind) -> u32 {
    match k {
        Kind::Root => 0,
        Kind::Super => 1,
        Kind::Community => 2,
        Kind::Subcommunity => 3,
        Kind::Aggregate => 4,
    }
}

/// The bytes of a result section holding `values`.
fn typed<T: Value>(values: &[T]) -> Vec<u8> {
    let mut out = b"EGRES\0\0\0".to_vec();
    out.extend_from_slice(
        &if T::TYPE == ValueType::U32 {
            1_u32
        } else {
            2_u32
        }
        .to_le_bytes(),
    );
    out.extend_from_slice(&[0; 4]);
    let mut buf = [0; 8];
    for &v in values {
        let len = T::TYPE.size();
        v.to_le(&mut buf[..len]);
        out.extend_from_slice(&buf[..len]);
    }
    out
}

/// A copy of `bytes` with part `name` of the result of `key` holding `section` instead.
fn forged(bytes: &[u8], key: &ResultKey, name: &str, section: Vec<u8>) -> Vec<u8> {
    let source = MemorySource::new(bytes.to_vec());
    let file = Container::open(&source).unwrap();
    let root = file.find_result(key).unwrap().unwrap();
    let mut head = vec![0_u64; root.len() as usize];
    root.read(0, &mut head).unwrap();
    let prefix = format!("{}/", root.name());
    let parts: Vec<(String, Section, Vec<u8>)> = file
        .entries()
        .iter()
        .filter_map(|e| {
            let part = e.name.strip_prefix(&prefix)?;
            let mut buf = vec![0; e.len as usize];
            file.section(&e.name).unwrap().read_at(0, &mut buf).unwrap();
            Some((part.to_owned(), e.kind, buf))
        })
        .collect();
    let mut w = Writer::update(MemorySource::new(bytes.to_vec())).unwrap();
    w.add_result(key, &about(), &head).unwrap();
    for (part, kind, buf) in parts {
        let mut s = w.begin_result_part(key, &part, kind).unwrap();
        s.write(if part == name { &section } else { &buf }).unwrap();
        s.finish().unwrap();
    }
    w.commit().unwrap();
    w.into_sink().into_inner()
}

/// The map tree stored and committed through `sink`, which holds the graph file.
fn work<S: ByteSink>(sink: S, lists: &Lists, t: &MapTree, key: &ResultKey) -> Result<(), String> {
    let mut w = Writer::update(sink).map_err(|e| e.to_string())?;
    map::store(
        &mut w,
        key,
        &about(),
        &lists.csr(),
        &Memory::unlimited(),
        Settings::default(),
        t,
    )
    .map_err(|e| e.to_string())?;
    w.commit().map(drop).map_err(|e| e.to_string())
}

#[test]
fn a_failing_write_leaves_no_map_or_all_of_it() {
    let (n, pairs) = nested(&[40, 5], &[5, 1, 0], 9);
    let lists = Lists::from_pairs(n, &pairs);
    let o = MapOptions {
        fan_out: 2..=4,
        leaf: 10,
        ..MapOptions::default()
    };
    let (_, t) = tree(&lists, None, &o);
    let base = graph_file(&lists).into_sink().into_inner();
    let key = map::key(
        Container::open(MemorySource::new(&base[..]))
            .unwrap()
            .graph_identity(),
        &o,
    )
    .unwrap();
    let ops = {
        let mut counter = Faulty::new(MemorySource::new(base.clone()), Plan::NONE);
        work(&mut counter, &lists, &t, &key).unwrap();
        counter.ops()
    };
    assert!(ops > 0);
    for k in 0..ops {
        let mut sink = Faulty::new(MemorySource::new(base.clone()), Plan::at(k));
        let outcome = work(&mut sink, &lists, &t, &key);
        for bytes in [sink.durable().to_vec(), sink.into_inner().into_inner()] {
            let file = Container::open(MemorySource::new(&bytes[..])).unwrap();
            let found = file.find_result(&key).unwrap().is_some();
            assert!(outcome.is_ok() || !found || bytes != base, "k = {k}");
            assert!(checked(&bytes).is_intact(), "k = {k}");
        }
    }
}

#[test]
fn a_graph_of_one_node_and_a_graph_without_edges_have_trees() {
    for n in [1, 50] {
        let lists = Lists::from_pairs(n, &[]);
        let o = MapOptions::default();
        let (partition, t) = tree(&lists, None, &o);
        rules("no edges", &lists, &partition, &t, &o);
        let (bytes, _) = stored(&lists, &t, &o);
        assert!(checked(&bytes).is_intact());
    }
}

/// A grid of `side` × `side` nodes.
fn grid(side: u32) -> (u32, Vec<(u32, u32)>) {
    let mut pairs = Vec::new();
    for y in 0..side {
        for x in 0..side {
            let v = y * side + x;
            if x + 1 < side {
                pairs.push((v, v + 1));
            }
            if y + 1 < side {
                pairs.push((v, v + side));
            }
        }
    }
    (side * side, pairs)
}

/// The super level's Leiden coarsens on: on grids at a low resolution, the usual rule
/// of `leiden` stops with 15 to 17 communities, the super level's with 2 or 3.
#[test]
fn the_super_level_leiden_coarsens_further_than_leiden() {
    let memory = Memory::unlimited();
    for (name, (n, pairs), gamma) in [
        ("grid of 300 x 300", grid(300), 1.0 / 128.0),
        ("grid of 400 x 400", grid(400), 1.0 / 128.0),
    ] {
        let lists = Lists::from_pairs(n, &pairs);
        let o = Options::default();
        let usual = leiden(
            &lists.csr(),
            &memory,
            Settings::default(),
            &Options {
                resolution: gamma,
                ..o
            },
        )
        .unwrap()
        .count;
        let coarse = map::coarse_leiden(&lists.csr(), &memory, Settings::default(), &o, gamma)
            .unwrap()
            .count;
        assert!(2 * coarse < usual, "{name}: usual {usual}, coarse {coarse}");
    }
}

/// Nodes without edges change no region: the map of a graph with 2,000,000 of them
/// added is the map of the graph alone, and one aggregate of those nodes under the root.
#[test]
fn nodes_without_edges_add_one_region_and_change_no_other() {
    let (n, pairs) = nested(&[40, 8, 6], &[6, 2, 1, 0], 7);
    let lists = Lists::from_pairs(n, &pairs);
    let o = MapOptions::default();
    let (partition, alone) = tree(&lists, None, &o);
    assert!(alone.shape.super_resolution.is_none() && View::new(&alone).kids[0].len() < 64);
    let extra = 2_000_000;
    let mut offsets = lists.offsets.clone();
    offsets.extend(std::iter::repeat_n(*offsets.last().unwrap(), extra));
    let wide = Lists {
        offsets,
        targets: lists.targets.clone(),
    };
    let mut wider = partition.clone();
    wider.extend(n..n + extra as u32);
    let (_, t) = tree(&wide, Some(&wider), &o);
    assert_eq!(t.shape.cover, alone.shape.cover);
    assert!(t.shape.super_resolution.is_none());
    let v = View::new(&t);
    let apart: Vec<u32> = v.kids[0]
        .iter()
        .copied()
        .filter(|&c| v.members[c as usize].first().is_some_and(|&u| u >= n))
        .collect();
    assert_eq!(apart.len(), 1);
    let apart = apart[0];
    assert_eq!(t.kind[apart as usize], Kind::Aggregate);
    assert_eq!(v.members[apart as usize].len(), extra);
    let mut graph: Vec<_> = subtree(&t, 0, |u| u)
        .into_iter()
        .filter(|(nodes, _, _)| nodes.iter().all(|&u| u < n) || nodes.len() > extra)
        .collect();
    // The root holds the extra nodes too, and one more child.
    let root = graph
        .iter()
        .position(|(nodes, _, _)| nodes.len() > extra)
        .unwrap();
    let (_, kind, mut kids) = graph.remove(root);
    kids.retain(|&k| k < n);
    graph.push(((0..n).collect(), kind, kids));
    graph.sort();
    assert_eq!(graph, subtree(&alone, 0, |u| u));
}

/// When the budget refuses regions solved together, they are solved again one at a
/// time: just under the peak of a run on eight threads, the same tree is built.
#[test]
fn regions_refused_together_are_solved_one_at_a_time() {
    let (n, pairs) = nested(&[4000, 25], &[8, 1, 0], 13);
    let lists = Lists::from_pairs(n, &pairs);
    let blocks: Vec<u32> = (0..n).map(|v| v / 4000).collect();
    let o = MapOptions::default();
    let build = |budget: &Arc<Budget>| {
        par::with_threads(NonZeroUsize::new(8).unwrap(), || {
            let memory = Memory::from(Arc::clone(budget));
            map::build(&lists.csr(), &memory, Settings::default(), &blocks, &o)
                .map(|t| t.parent.iter().copied().collect::<Vec<u32>>())
        })
        .unwrap()
    };
    let unlimited = Arc::new(Budget::unlimited());
    let free = build(&unlimited).unwrap();
    let tight = Arc::new(Budget::new(unlimited.peak() - 1));
    assert_eq!(build(&tight).unwrap(), free);
}
