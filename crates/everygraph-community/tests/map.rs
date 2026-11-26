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

use std::collections::{BTreeMap, HashMap};
use std::num::NonZeroUsize;
use std::sync::Arc;

use everygraph_community::hierarchy::Hierarchy;
use everygraph_community::map::{self, Kind, MapOptions, MapTree, SuperLevel};
use everygraph_community::{Options, leiden};
use everygraph_core::{Budget, ByteSink, ByteSource, Memory, MemorySource, par};
use everygraph_format::container::{Container, Kind as Section, ResultKey, Writer};
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
        let all = v.resolved(x);
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
    // The super level: Leiden at its resolution and a vote, here.
    let sizes: Vec<u64> = communities.iter().map(|c| c.len() as u64).collect();
    assert_eq!(t.shape.cover, cover(&sizes), "{name}: cover");
    let group_of = |x: u32| -> Option<u32> {
        std::iter::successors(Some(x), |&y| (y != 0).then(|| t.parent[y as usize]))
            .find(|&y| v.kind(y) == Kind::Super)
    };
    let region_of: HashMap<u32, u32> = (0..r)
        .filter(|&x| v.kind(x) == Kind::Community)
        .map(|x| (partition[v.members[x as usize][0] as usize], x))
        .collect();
    let groups_at = |gamma: f64| {
        let memory = Memory::unlimited();
        let found =
            map::coarse_leiden(&lists.csr(), &memory, Settings::default(), &o.leiden, gamma);
        let groups = vote(partition, &found.unwrap().membership);
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
            // The largest resolution 2^-j whose groups meet the cover, or 2^-16.
            let (groups, c) = groups_at(gamma);
            assert!(
                c < f32 || gamma == 0.5_f64.powi(16),
                "{name}: cover {c} at {gamma}"
            );
            if gamma < 0.5 {
                assert!(
                    groups_at(2.0 * gamma).1 >= f32,
                    "{name}: a larger resolution meets it"
                );
            }
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
    // The corpus's graphs of millions of isolated nodes are left to the Leiden tests.
    for e in entries(Tier::Ci) {
        let p = Projection::new(&e.graph);
        let n = p.undirected.n() as u32;
        if n > 100_000 {
            continue;
        }
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

#[test]
fn a_stored_map_reads_back_its_kinds_and_its_partition_and_verifies() {
    let memory = Memory::unlimited();
    for (name, lists, partition) in graphs() {
        let o = MapOptions {
            fan_out: 2..=8,
            leaf: 16,
            ..MapOptions::default()
        };
        let (partition, t) = tree(&lists, partition.as_deref(), &o);
        if lists.nodes() == 0 {
            continue;
        }
        let (bytes, key) = stored(&lists, &t, &o);
        let report = checked(&bytes);
        assert!(
            report.is_intact() && report.unchecked.is_empty(),
            "{name}: {report:?}"
        );
        let file = Container::open(MemorySource::new(&bytes[..])).unwrap();
        let h = Hierarchy::find(&file, &key).unwrap().unwrap();
        let read = map::partition(&h, &memory).unwrap();
        let all: Vec<u32> = (0..lists.nodes()).collect();
        assert_eq!(sets(&all, &read), sets(&all, &partition), "{name}");
        // At every level, each node's region has the kind of the tree region it
        // stands for: its ancestor at that depth, or its leaf when that is shallower.
        let depth = |x: u32| {
            std::iter::successors(Some(x), |&y| (y != 0).then(|| t.parent[y as usize])).count()
                as u32
                - 1
        };
        for level in 1..=h.levels() {
            let kinds = map::kinds(&h, level, &memory).unwrap();
            let m = h.membership(level, &memory).unwrap();
            for (u, &l) in t.leaf.iter().enumerate() {
                let mut x = l;
                while depth(x) > level {
                    x = t.parent[x as usize];
                }
                assert_eq!(
                    kinds[m[u] as usize], t.kind[x as usize],
                    "{name}: level {level}, node {u}"
                );
            }
        }
        assert_eq!(
            h.count(1).unwrap() as usize,
            View::new(&t).kids[0].len(),
            "{name}"
        );
    }
}

#[test]
fn the_same_input_gives_the_same_bytes_at_any_thread_count() {
    // One community of 70,000 nodes: a region solved alone, then many solved together.
    let (n, pairs) = nested(&[500, 140], &[8, 1, 0], 11);
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
fn check_finds_kinds_that_break_the_rules() {
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
    let h = Hierarchy::find(&file, &key).unwrap().unwrap();
    let top = h.levels();
    let codes = |level: u32| -> Vec<u32> {
        map::kinds(&h, level, &Memory::unlimited())
            .unwrap()
            .iter()
            .map(|&k| match k {
                Kind::Super => 1,
                Kind::Community => 2,
                Kind::Subcommunity => 3,
                Kind::Aggregate => 4,
                Kind::Root => 0,
            })
            .collect()
    };
    // A region of `level` whose parent has two children or more (any at level 1).
    let split_at = |level: u32| -> u32 {
        let Some(parents) = h.parents(level, &Memory::unlimited()).unwrap() else {
            return 0;
        };
        (0..parents.len())
            .find(|&u| parents.iter().filter(|&&p| p == parents[u]).count() > 1)
            .unwrap() as u32
    };
    // A community of level 1 split into subcommunities.
    let parents = h.parents(2, &Memory::unlimited()).unwrap().unwrap();
    let wide = (0..h.count(1).unwrap())
        .find(|&p| codes(1)[p as usize] == 2 && parents.iter().filter(|&&q| q == p).count() > 1)
        .unwrap();
    for (level, at, code, what) in [
        (1, wide, 3, "kinds out of order"),
        (top, split_at(top), 2, "kinds out of order"),
        (top, 0, 9, "a kind that is not one"),
        (1, 0, 0, "a kind that is not one"),
        (top, split_at(top), 1, "kinds out of order"),
    ] {
        let mut values = codes(level);
        values[at as usize] = code;
        let bad = forged(&bytes, &key, &format!("{level}/kind"), &values);
        let report = checked(&bad);
        assert!(
            report.problems.iter().any(|p| p.what.contains(what)),
            "level {level}, code {code}: {:?}",
            report.problems
        );
    }
}

/// A copy of `bytes` with part `name` of the result of `key` holding `values`.
fn forged(bytes: &[u8], key: &ResultKey, name: &str, values: &[u32]) -> Vec<u8> {
    let source = MemorySource::new(bytes.to_vec());
    let file = Container::open(&source).unwrap();
    let root = file.find_result(key).unwrap().unwrap();
    let mut head = [0_u64; 2];
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
        if part == name {
            w.add_result_part(key, &part, values.iter().copied())
                .unwrap();
        } else {
            let mut s = w.begin_result_part(key, &part, kind).unwrap();
            s.write(&buf).unwrap();
            s.finish().unwrap();
        }
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
