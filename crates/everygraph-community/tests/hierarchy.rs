// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The community hierarchy kept in the graph file (#62).
//!
//! - Oracle: on every graph of the oracle's CI corpus (kept loops in the lists), every
//!   level's node counts and quotient graph equal the oracle's `quotient` of the
//!   membership read back at that level, and its modularity the oracle's. The
//!   modularity of each level's quotient under singletons equals, bit for bit, that of
//!   the level below under the parents' partition (`docs/spec/graph-semantics.md`
//!   § 5.5). The strictest readers find the file intact: `verify --deep` and
//!   `hierarchy::check`.
//! - Metamorphic: storing the membership of any level as a hierarchy of one level, built
//!   from the graph in one step, gives that level's parts byte for byte, though the full
//!   hierarchy built it from the quotient of the level below.
//! - Stable ids: the same input and seed give the same bytes at 1 and 8 threads.
//! - A damaged table is found by `check`; a failing write at any operation leaves the
//!   file without the result or with all of it.

#![forbid(unsafe_code)]
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::print_stderr
)]

use std::collections::BTreeMap;
use std::num::NonZeroUsize;

use everygraph_community::hierarchy::{self, Hierarchy, HierarchyError, store, store_levels};
use everygraph_community::{Communities, Options, leiden};
use everygraph_core::{Budget, ByteSink, ByteSource, Memory, MemorySource, par};
use everygraph_format::adjacency::{IndexBuilder, append_record, encode, encode_events};
use everygraph_format::container::{
    Container, ContainerError, Kind, Provenance, ResultKey, Writer,
};
use everygraph_format::verify::{Depth, Report, verify};
use everygraph_io::faults::{Faulty, Plan};
use everygraph_kernels::edge_map::{Csr, Settings};
use everygraph_oracle::corpus::{Tier, entries};
use everygraph_oracle::kernel::Output;
use everygraph_oracle::projection::Projection;
use everygraph_oracle::reference;
use std::sync::Arc;

type Mem = MemorySource<Vec<u8>>;
/// The entries of a quotient graph, `(a, b, weight)` with `a <= b`.
type Edges = Vec<(u32, u32, u64)>;

/// Lists of an undirected graph: each pair in both lists, a kept loop once.
struct Lists {
    offsets: Vec<u64>,
    targets: Vec<u32>,
}

impl Lists {
    fn of(p: &Projection) -> Self {
        let mut offsets = vec![0];
        let mut targets = Vec::new();
        for u in 0..p.undirected.n() {
            let mut l = p.undirected.list(u).to_vec();
            if p.kept_loops(u) > 0 {
                l.push(u as u32);
                l.sort_unstable();
            }
            targets.extend(l);
            offsets.push(targets.len() as u64);
        }
        Self { offsets, targets }
    }

    fn from_pairs(n: u32, pairs: &[(u32, u32)]) -> Self {
        let mut lists = vec![Vec::new(); n as usize];
        for &(u, v) in pairs {
            lists[u as usize].push(v);
            if u != v {
                lists[v as usize].push(u);
            }
        }
        let mut offsets = vec![0];
        let mut targets = Vec::new();
        for mut l in lists {
            l.sort_unstable();
            l.dedup();
            targets.extend(l);
            offsets.push(targets.len() as u64);
        }
        Self { offsets, targets }
    }

    fn csr(&self) -> Csr<'_> {
        Csr::new(&self.offsets, &self.targets).unwrap()
    }
}

/// A graph file holding `lists` as its undirected adjacency, loops kept.
fn graph_file(lists: &Lists) -> Writer<Mem> {
    let encoded = encode(&lists.offsets, &lists.targets).unwrap();
    let mut w = Writer::create(Mem::default()).unwrap();
    let record = Provenance {
        engine: "everygraph test".into(),
        operation: "ingest".into(),
        parameters: [("directed", "false"), ("self_loops", "keep")]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect(),
        ..Provenance::default()
    };
    w.add_provenance("provenance/ingest", &record).unwrap();
    for (name, kind, bytes) in [
        ("adjacency.out.index", Kind::ADJACENCY_INDEX, &encoded.index),
        ("adjacency.out.data", Kind::ADJACENCY_DATA, &encoded.data),
    ] {
        w.add_section(name, kind, bytes, Some("provenance/ingest"))
            .unwrap();
    }
    w.commit().unwrap();
    w
}

fn about() -> Provenance {
    Provenance {
        engine: "everygraph test".into(),
        ..Provenance::default()
    }
}

/// `lists` and the hierarchy of `c` in one committed file, and the hierarchy's key.
fn stored(lists: &Lists, c: &Communities, o: &Options) -> (Vec<u8>, ResultKey) {
    let mut w = graph_file(lists);
    let key = hierarchy::key(w.graph_identity(), o).unwrap();
    let memory = Memory::unlimited();
    store(
        &mut w,
        &key,
        &about(),
        &lists.csr(),
        &memory,
        Settings::default(),
        c,
    )
    .unwrap();
    w.commit().unwrap();
    (w.into_sink().into_inner(), key)
}

/// What the strictest readers find in `bytes`.
fn checked(bytes: &[u8]) -> Report {
    let source = MemorySource::new(bytes);
    let memory = Memory::unlimited();
    let mut report = verify(&source, Depth::Deep, &memory).unwrap();
    hierarchy::check(&Container::open(&source).unwrap(), &memory, &mut report).unwrap();
    report
}

/// The entries of a stored quotient graph, `(a, b, weight)` with `a <= b`, sorted.
fn edges<S: everygraph_core::ByteSource>(h: &Hierarchy<'_, S>, level: u32) -> Edges {
    let q = h.quotient(level).unwrap();
    let events = q.events().unwrap();
    let adjacency = q.adjacency().unwrap();
    let mut out = Vec::new();
    let mut buf = Vec::new();
    let mut entry = 0;
    for a in 0..q.index().nodes() {
        for &b in adjacency
            .neighbors(everygraph_core::NodeId::new(a), &mut buf)
            .unwrap()
        {
            let w = events.map_or(1, |e| {
                let r = e.event_range(entry).unwrap();
                r.end - r.start
            });
            if a <= b {
                out.push((a, b, w));
            }
            entry += 1;
        }
    }
    out
}

/// The modularity at resolution 1 of the partition `map` of a weighted graph given by
/// its entries, the arithmetic of `Table::modularity`.
fn modularity(edges: &[(u32, u32, u64)], map: &[u32], count: usize) -> f64 {
    let (mut internal, mut volume) = (vec![0_u64; count], vec![0_u64; count]);
    for &(a, b, w) in edges {
        let (ca, cb) = (map[a as usize] as usize, map[b as usize] as usize);
        volume[ca] += w;
        volume[cb] += w;
        if ca == cb {
            internal[ca] += w;
        }
    }
    let total: u64 = volume.iter().sum();
    if total == 0 {
        return 0.0;
    }
    let inside: u64 = internal.iter().sum();
    let squares: u128 = volume.iter().map(|&s| u128::from(s) * u128::from(s)).sum();
    let total = total as f64;
    2.0 * inside as f64 / total - squares as f64 / (total * total)
}

fn identity(n: usize) -> Vec<u32> {
    (0..n as u32).collect()
}

/// Every level against the oracle, and each level's modularity against the level
/// below's.
fn against_oracle(name: &str, p: &Projection, bytes: &[u8], key: &ResultKey, c: &Communities) {
    let source = MemorySource::new(bytes);
    let file = Container::open(&source).unwrap();
    let memory = Memory::unlimited();
    let h = Hierarchy::find(&file, key).unwrap().unwrap();
    let mut finer: Option<(Edges, Vec<u32>)> = None;
    for level in (1..=h.levels()).rev() {
        let m = h.membership(level, &memory).unwrap();
        let table = h.table(level, &memory).unwrap();
        let count = h.count(level).unwrap() as usize;
        let Some(Output::Quotient { sizes, edges: want }) = reference::quotient(p, &m) else {
            panic!("{name}: the oracle has no quotient");
        };
        let mine = edges(&h, level);
        let sizes_mine: Vec<u64> = table.nodes.iter().map(|&x| u64::from(x)).collect();
        assert_eq!(sizes, sizes_mine, "{name} level {level}: node counts");
        let as_f64: Vec<(u32, u32, f64)> = mine.iter().map(|&(a, b, w)| (a, b, w as f64)).collect();
        assert_eq!(want, as_f64, "{name} level {level}: quotient graph");
        for c in 0..count {
            assert_eq!(table.cut(c) + 2 * table.internal[c], table.volume[c]);
        }
        let q = modularity(&mine, &identity(count), count);
        let oracle = reference::modularity(p, &m, 1.0, false);
        assert!(
            (q - oracle).abs() < 1e-12,
            "{name} level {level}: {q} against {oracle}"
        );
        assert_eq!(
            q.to_bits(),
            table.modularity(1.0).to_bits(),
            "{name} level {level}"
        );
        if let Some((below, parents)) = finer.take() {
            let from_below = modularity(&below, &parents, count);
            assert_eq!(
                q.to_bits(),
                from_below.to_bits(),
                "{name} level {level}: modularity differs from the level below's"
            );
        }
        if level == 1 {
            assert_eq!(q.to_bits(), c.modularity.to_bits(), "{name}: the partition");
        } else {
            finer = Some((mine, h.parents(level, &memory).unwrap().unwrap().to_vec()));
        }
    }
}

#[test]
fn every_level_matches_the_oracle_and_conserves_modularity() {
    let memory = Memory::unlimited();
    let mut loops = 0;
    let mut levels = 0;
    for e in entries(Tier::Ci) {
        let p = Projection::new(&e.graph);
        let lists = Lists::of(&p);
        loops += usize::from((0..p.undirected.n()).any(|v| p.kept_loops(v) > 0));
        for o in [Options::default(), Options::quality()] {
            let c = leiden(&lists.csr(), &memory, Settings::default(), &o).unwrap();
            let (bytes, key) = stored(&lists, &c, &o);
            let report = checked(&bytes);
            let name = &e.graph.name;
            assert!(report.is_intact(), "{name}: {:?}", report.problems);
            assert!(
                !report
                    .unchecked
                    .iter()
                    .any(|u| u.starts_with("communities")),
                "{name}: {:?}",
                report.unchecked
            );
            against_oracle(name, &p, &bytes, &key, &c);
            levels += c.levels.len();
        }
    }
    assert!(loops > 0, "no graph of the corpus keeps a loop");
    assert!(levels > 0);
}

/// A graph with many levels: planted blocks of a few nodes, joined sparsely.
fn planted(blocks: u32, size: u32) -> (u32, Vec<(u32, u32)>) {
    let n = blocks * size;
    let mut pairs = Vec::new();
    let mut x = 0x9E37_79B9_7F4A_7C15_u64;
    let mut next = |m: u32| {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        (x % u64::from(m)) as u32
    };
    for b in 0..blocks {
        for i in 0..size {
            for j in i + 1..size {
                if next(3) > 0 {
                    pairs.push((b * size + i, b * size + j));
                }
            }
        }
        pairs.push((b * size, next(n)));
        pairs.push((b * size + 1, ((b + 1) % blocks) * size));
    }
    pairs.push((0, 0));
    (n, pairs)
}

/// A membership renumbered by smallest node.
fn by_smallest(m: &[u32]) -> Vec<u32> {
    let mut names = BTreeMap::new();
    m.iter()
        .map(|&c| {
            let next = names.len() as u32;
            *names.entry(c).or_insert(next)
        })
        .collect()
}

/// The names and hashes of the parts of level `level` of the result of `key`.
fn parts<S: everygraph_core::ByteSource>(
    file: &Container<S>,
    key: &ResultKey,
    level: u32,
) -> BTreeMap<String, [u8; 32]> {
    let root = file.find_result(key).unwrap().unwrap();
    let prefix = format!("{}/{level}/", root.name());
    file.entries()
        .iter()
        .filter_map(|e| e.name.strip_prefix(&prefix).map(|p| (p.to_owned(), e.hash)))
        .filter(|(p, _)| p != "parent")
        .collect()
}

#[test]
fn a_level_stored_alone_from_the_graph_is_the_same_level() {
    let (n, pairs) = planted(300, 6);
    let lists = Lists::from_pairs(n, &pairs);
    let memory = Memory::unlimited();
    let o = Options::default();
    let c = leiden(&lists.csr(), &memory, Settings::default(), &o).unwrap();
    assert!(
        c.levels.len() >= 2,
        "the graph gives {} levels",
        c.levels.len()
    );
    let (bytes, key) = stored(&lists, &c, &o);
    let file = Container::open(MemorySource::new(&bytes[..])).unwrap();
    let h = Hierarchy::find(&file, &key).unwrap().unwrap();
    let mut w = Writer::update(MemorySource::new(bytes.clone())).unwrap();
    let mut alone = Vec::new();
    for level in 1..=h.levels() {
        let m = by_smallest(&h.membership(level, &memory).unwrap());
        let k = ResultKey::new(w.graph_identity(), "communities", 1, true)
            .unwrap()
            .with("level", &level.to_string())
            .unwrap();
        store_levels(
            &mut w,
            &k,
            &about(),
            &lists.csr(),
            &memory,
            Settings::default(),
            &[&m],
        )
        .unwrap();
        alone.push(k);
    }
    w.commit().unwrap();
    let bytes = w.into_sink().into_inner();
    let file2 = Container::open(MemorySource::new(&bytes[..])).unwrap();
    for (level, k) in (1..=h.levels()).zip(&alone) {
        let full = parts(&file2, &key, level);
        let one = parts(&file2, k, 1);
        assert_eq!(full, one, "level {level}");
        assert!(full.contains_key("quotient.data"));
    }
    assert!(checked(&bytes).is_intact());
}

#[test]
fn the_same_input_and_seed_give_the_same_bytes_at_any_thread_count() {
    let (n, pairs) = planted(200, 5);
    let lists = Lists::from_pairs(n, &pairs);
    let o = Options::quality();
    let run = |threads: usize| {
        par::with_threads(NonZeroUsize::new(threads).unwrap(), || {
            let memory = Memory::unlimited();
            let c = leiden(&lists.csr(), &memory, Settings::default(), &o).unwrap();
            stored(&lists, &c, &o).0
        })
        .unwrap()
    };
    assert_eq!(run(1), run(8));
}

#[test]
fn the_api_reads_what_the_levels_hold() {
    let (n, pairs) = planted(120, 6);
    let lists = Lists::from_pairs(n, &pairs);
    let memory = Memory::unlimited();
    let o = Options::default();
    let c = leiden(&lists.csr(), &memory, Settings::default(), &o).unwrap();
    let (bytes, key) = stored(&lists, &c, &o);
    let file = Container::open(MemorySource::new(&bytes[..])).unwrap();
    let h = Hierarchy::find(&file, &key).unwrap().unwrap();
    let top = h.levels();
    assert_eq!(top as usize, c.levels.len());
    assert_eq!(h.nodes(), n);
    assert_eq!(
        by_smallest(&h.membership(1, &memory).unwrap()),
        c.membership[..]
    );
    for level in 1..=top {
        let m = h.membership(level, &memory).unwrap();
        let table = h.table(level, &memory).unwrap();
        // Ids by decreasing size.
        assert!(table.nodes.windows(2).all(|w| w[0] >= w[1]));
        for community in 0..h.count(level).unwrap() {
            let members = h.members(level, community, &memory).unwrap();
            let want: Vec<u32> = (0..n).filter(|&v| m[v as usize] == community).collect();
            assert_eq!(members[..], want[..]);
            let up = h.ancestors(level, community).unwrap();
            assert_eq!(up.len() as u32, level - 1);
            for (i, &a) in up.iter().enumerate() {
                let at = level - 1 - i as u32;
                let inside = h.descendants(at, a, level, &memory).unwrap();
                assert!(inside.contains(&community));
                let am = h.membership(at, &memory).unwrap();
                assert!(members.iter().all(|&v| am[v as usize] == a));
            }
        }
    }
    assert!(h.count(0).is_err() && h.count(top + 1).is_err());
    assert!(h.members(1, h.count(1).unwrap(), &memory).is_err());
    assert!(h.descendants(2, 0, 1, &memory).is_err());
}

#[test]
fn the_hierarchy_of_a_graph_without_edges_is_its_nodes() {
    let lists = Lists::from_pairs(5, &[]);
    let memory = Memory::unlimited();
    let o = Options::default();
    let c = leiden(&lists.csr(), &memory, Settings::default(), &o).unwrap();
    let (bytes, key) = stored(&lists, &c, &o);
    assert!(checked(&bytes).is_intact());
    let file = Container::open(MemorySource::new(&bytes[..])).unwrap();
    let h = Hierarchy::find(&file, &key).unwrap().unwrap();
    assert_eq!((h.levels(), h.count(1).unwrap()), (1, 5));
    assert!(edges(&h, 1).is_empty());
}

#[test]
fn communities_that_are_not_nested_partitions_are_refused() {
    let lists = Lists::from_pairs(4, &[(0, 1), (2, 3)]);
    let memory = Memory::unlimited();
    let mut w = graph_file(&lists);
    let key = ResultKey::new(w.graph_identity(), "communities", 1, true).unwrap();
    for maps in [
        vec![&[1_u32, 0, 0, 1][..]],
        vec![&[0, 0, 1][..]],
        vec![&[0, 0, 1, 1][..], &[0, 1, 1][..]],
        vec![],
    ] {
        let e = store_levels(
            &mut w,
            &key,
            &about(),
            &lists.csr(),
            &memory,
            Settings::default(),
            &maps,
        );
        assert!(
            matches!(e, Err(hierarchy::HierarchyError::Invalid(_))),
            "{maps:?}"
        );
    }
}

/// A copy of the result of `key` in `bytes` with the parts of `replace` (name, kind,
/// bytes) written instead of the stored ones; a part with no bytes is left out.
fn replaced(bytes: &[u8], key: &ResultKey, replace: &[(String, Kind, Option<Vec<u8>>)]) -> Vec<u8> {
    let source = MemorySource::new(bytes.to_vec());
    let file = Container::open(&source).unwrap();
    let root = file.find_result(key).unwrap().unwrap();
    let mut top = [0_u64; 2];
    root.read(0, &mut top).unwrap();
    let prefix = format!("{}/", root.name());
    let mut parts: BTreeMap<String, (Kind, Option<Vec<u8>>)> = file
        .entries()
        .iter()
        .filter_map(|e| {
            let part = e.name.strip_prefix(&prefix)?;
            let mut buf = vec![0; e.len as usize];
            file.section(&e.name).unwrap().read_at(0, &mut buf).unwrap();
            Some((part.to_owned(), (e.kind, Some(buf))))
        })
        .collect();
    for (name, kind, bytes) in replace {
        parts.insert(name.clone(), (*kind, bytes.clone()));
    }
    let mut w = Writer::update(MemorySource::new(bytes.to_vec())).unwrap();
    w.add_result(key, &about(), &top).unwrap();
    for (part, (kind, bytes)) in parts {
        if let Some(bytes) = bytes {
            let mut s = w.begin_result_part(key, &part, kind).unwrap();
            s.write(&bytes).unwrap();
            s.finish().unwrap();
        }
    }
    w.commit().unwrap();
    w.into_sink().into_inner()
}

/// The bytes of a result section of `u64` values (`docs/spec/eg-container.md` § 6).
fn typed(values: &[u64]) -> Vec<u8> {
    let mut out = b"EGRES\0\0\0".to_vec();
    out.extend_from_slice(&2_u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend(values.iter().flat_map(|v| v.to_le_bytes()));
    out
}

/// A copy of the result of `key` in `bytes` with part `name` holding `values` instead.
fn forged(bytes: &[u8], key: &ResultKey, name: &str, values: &[u64]) -> Vec<u8> {
    replaced(
        bytes,
        key,
        &[(name.to_owned(), Kind::RESULT, Some(typed(values)))],
    )
}

/// The quotient parts of `level` holding the graph of `edges` (`a <= b`) on `nodes`.
fn quotient_parts(
    level: u32,
    nodes: u32,
    edges: &BTreeMap<(u32, u32), u64>,
) -> Vec<(String, Kind, Option<Vec<u8>>)> {
    let mut lists = vec![Vec::new(); nodes as usize];
    for (&(a, b), &w) in edges {
        lists[a as usize].push((b, w));
        if a != b {
            lists[b as usize].push((a, w));
        }
    }
    let (mut offsets, mut targets, mut weights) = (vec![0_u64], Vec::new(), Vec::new());
    for mut l in lists {
        l.sort_unstable();
        targets.extend(l.iter().map(|p| p.0));
        weights.extend(l.iter().map(|p| p.1));
        offsets.push(targets.len() as u64);
    }
    let (entries, events) = (weights.len() as u64, weights.iter().sum::<u64>());
    let mut index = IndexBuilder::new(&Memory::unlimited(), nodes).unwrap();
    let mut data = Vec::new();
    for v in 0..nodes as usize {
        let list = &targets[offsets[v] as usize..offsets[v + 1] as usize];
        let before = data.len();
        let d = append_record(v as u32, list.iter().copied(), &mut data).unwrap();
        index.push(d, (data.len() - before) as u64).unwrap();
    }
    let has_events = events > entries;
    let index = index.finish(has_events).unwrap().0;
    let name = |s: &str| format!("{level}/quotient.{s}");
    vec![
        (name("index"), Kind::RESULT_ADJACENCY_INDEX, Some(index)),
        (name("data"), Kind::RESULT_ADJACENCY_DATA, Some(data)),
        (
            name("events"),
            Kind::RESULT_ADJACENCY_EVENTS,
            has_events.then(|| encode_events(entries, events, weights).unwrap()),
        ),
    ]
}

#[test]
fn check_finds_a_quotient_graph_with_its_entries_swapped_at_every_level() {
    let (n, pairs) = planted(300, 6);
    let lists = Lists::from_pairs(n, &pairs);
    let memory = Memory::unlimited();
    let o = Options::default();
    let c = leiden(&lists.csr(), &memory, Settings::default(), &o).unwrap();
    let (bytes, key) = stored(&lists, &c, &o);
    let file = Container::open(MemorySource::new(&bytes[..])).unwrap();
    let h = Hierarchy::find(&file, &key).unwrap().unwrap();
    assert!(h.levels() >= 3, "{} levels", h.levels());
    for level in 1..=h.levels() {
        let mut edges: BTreeMap<(u32, u32), u64> = edges(&h, level)
            .into_iter()
            .map(|(a, b, w)| ((a, b), w))
            .collect();
        let off: Vec<(u32, u32)> = edges.keys().copied().filter(|&(a, b)| a != b).collect();
        // (a, b) and (c, d) lose one each, (a, d) and (c, b) gain one: every row sum and
        // every loop stays.
        let (a, b, c, d) = off
            .iter()
            .flat_map(|&(a, b)| off.iter().map(move |&(c, d)| (a, b, c, d)))
            .find(|&(a, b, c, d)| a != c && a != d && b != c && b != d)
            .expect("two disjoint entries");
        let key_of = |x: u32, y: u32| (x.min(y), x.max(y));
        for (x, y, gain) in [(a, b, false), (c, d, false), (a, d, true), (c, b, true)] {
            let w = edges.entry(key_of(x, y)).or_insert(0);
            *w = if gain { *w + 1 } else { *w - 1 };
        }
        edges.retain(|_, w| *w > 0);
        let bad = replaced(
            &bytes,
            &key,
            &quotient_parts(level, h.count(level).unwrap(), &edges),
        );
        let report = checked(&bad);
        assert!(
            report.problems.iter().any(|p| p
                .what
                .starts_with(&format!("level {level}: its quotient graph is not"))),
            "level {level}: {:?}",
            report.problems
        );
    }
}

#[test]
fn a_quotient_too_heavy_to_sum_is_refused() {
    // One community of the path's four nodes, whose loop says 2^63 + 1 events.
    let lists = Lists::from_pairs(4, &[(0, 1), (1, 2), (2, 3)]);
    let bytes = graph_file(&lists).into_sink().into_inner();
    let heavy = BTreeMap::from([((0, 0), (1_u64 << 63) + 1)]);
    let mut w = Writer::update(MemorySource::new(bytes.clone())).unwrap();
    let k = ResultKey::new(w.graph_identity(), "communities", 1, true)
        .unwrap()
        .with("heavy", "1")
        .unwrap();
    w.add_result(&k, &about(), &[1_u64, 4]).unwrap();
    w.add_result_part(&k, "membership", [0_u32; 4]).unwrap();
    w.add_result_part(&k, "1/nodes", [4_u32]).unwrap();
    w.add_result_part(&k, "1/internal", [3_u64]).unwrap();
    w.add_result_part(&k, "1/volume", [6_u64]).unwrap();
    for (part, kind, bytes) in quotient_parts(1, 1, &heavy) {
        let mut s = w.begin_result_part(&k, &part, kind).unwrap();
        s.write(&bytes.unwrap()).unwrap();
        s.finish().unwrap();
    }
    w.commit().unwrap();
    let report = checked(&w.into_sink().into_inner());
    assert!(
        report.problems.iter().any(|p| p.what.contains("2^62")),
        "{:?}",
        report.problems
    );
}

#[test]
fn a_store_that_fails_half_way_cannot_be_committed() {
    let (n, pairs) = planted(60, 5);
    let lists = Lists::from_pairs(n, &pairs);
    let o = Options::default();
    let c = leiden(&lists.csr(), &Memory::unlimited(), Settings::default(), &o).unwrap();
    let (bytes, key) = stored(&lists, &c, &o);
    let mut poisoned = 0;
    for limit in (0..).map(|k| 4096 * k) {
        let mut w = Writer::update(MemorySource::new(bytes.clone())).unwrap();
        let memory = Memory::new(Arc::new(Budget::new(limit)));
        let outcome = store(
            &mut w,
            &key,
            &about(),
            &lists.csr(),
            &memory,
            Settings::default(),
            &c,
        );
        let Err(e) = outcome else {
            break;
        };
        assert!(matches!(e, HierarchyError::Graph(_)), "{e}");
        match w.commit() {
            Err(ContainerError::Poisoned) => poisoned += 1,
            Ok(_) => {}
            Err(e) => panic!("{e}"),
        }
        // The file reads as before: the hierarchy stored first, whole.
        let after = w.into_sink().into_inner();
        let source = MemorySource::new(&after[..]);
        let file = Container::open(&source).unwrap();
        assert!(
            Hierarchy::find(&file, &key).unwrap().is_some(),
            "limit {limit}"
        );
        assert!(checked(&after).is_intact(), "limit {limit}");
    }
    assert!(poisoned > 0);
}

#[test]
fn check_finds_a_table_that_does_not_add_up() {
    let (n, pairs) = planted(50, 5);
    let lists = Lists::from_pairs(n, &pairs);
    let memory = Memory::unlimited();
    let o = Options::default();
    let c = leiden(&lists.csr(), &memory, Settings::default(), &o).unwrap();
    let (bytes, key) = stored(&lists, &c, &o);
    let file = Container::open(MemorySource::new(&bytes[..])).unwrap();
    let h = Hierarchy::find(&file, &key).unwrap().unwrap();
    let top = h.levels();
    for level in [1, top] {
        let table = h.table(level, &memory).unwrap();
        let mut volume = table.volume.to_vec();
        volume[0] += 1;
        let mut internal = table.internal.to_vec();
        let last = internal.len() - 1;
        internal.swap(0, last);
        for (name, values) in [("volume", volume), ("internal", internal)] {
            let bad = forged(&bytes, &key, &format!("{level}/{name}"), &values);
            let report = checked(&bad);
            assert!(
                report.problems.iter().any(|p| p
                    .section
                    .as_deref()
                    .is_some_and(|s| s.starts_with("results/communities/"))),
                "{level}/{name}: {:?}",
                report.problems
            );
        }
    }
    assert!(checked(&forged(&bytes, &key, "none", &[])).is_intact());
}

/// The graph file, then the hierarchy stored and committed through `sink`.
fn work<S: ByteSink>(
    sink: S,
    lists: &Lists,
    c: &Communities,
    key: &ResultKey,
) -> Result<(), String> {
    let mut w = Writer::update(sink).map_err(|e| e.to_string())?;
    let memory = Memory::unlimited();
    store(
        &mut w,
        key,
        &about(),
        &lists.csr(),
        &memory,
        Settings::default(),
        c,
    )
    .map_err(|e| e.to_string())?;
    w.commit()
        .map(drop)
        .map_err(|e: ContainerError| e.to_string())
}

#[test]
fn a_failing_write_leaves_no_result_or_all_of_it() {
    let (n, pairs) = planted(30, 5);
    let lists = Lists::from_pairs(n, &pairs);
    let memory = Memory::unlimited();
    let o = Options::default();
    let c = leiden(&lists.csr(), &memory, Settings::default(), &o).unwrap();
    let base = graph_file(&lists).into_sink().into_inner();
    let key = hierarchy::key(
        Container::open(MemorySource::new(&base[..]))
            .unwrap()
            .graph_identity(),
        &o,
    )
    .unwrap();
    let mut whole = MemorySource::new(base.clone());
    work(&mut whole, &lists, &c, &key).unwrap();
    let whole = whole.into_inner();
    let ops = {
        let mut counter = Faulty::new(MemorySource::new(base.clone()), Plan::NONE);
        work(&mut counter, &lists, &c, &key).unwrap();
        counter.ops()
    };
    for k in 0..ops {
        let mut sink = Faulty::new(MemorySource::new(base.clone()), Plan::at(k));
        let outcome = work(&mut sink, &lists, &c, &key);
        for bytes in [sink.durable().to_vec(), sink.into_inner().into_inner()] {
            let file = Container::open(MemorySource::new(&bytes[..])).unwrap();
            let found = file.find_result(&key).unwrap().is_some();
            assert!(outcome.is_ok() || !found || bytes != base, "k = {k}");
            assert!(checked(&bytes).is_intact(), "k = {k}");
            if found {
                let whole_file = Container::open(MemorySource::new(&whole[..])).unwrap();
                let names = |f: &Container<MemorySource<&[u8]>>| -> Vec<(String, [u8; 32])> {
                    f.entries()
                        .iter()
                        .map(|e| (e.name.to_string(), e.hash))
                        .collect()
                };
                assert_eq!(names(&file), names(&whole_file), "k = {k}");
            }
        }
    }
}

#[test]
fn damaged_bytes_give_problems_never_a_panic() {
    let (n, pairs) = planted(20, 5);
    let lists = Lists::from_pairs(n, &pairs);
    let memory = Memory::unlimited();
    let o = Options::default();
    let c = leiden(&lists.csr(), &memory, Settings::default(), &o).unwrap();
    let (bytes, _) = stored(&lists, &c, &o);
    let file = Container::open(MemorySource::new(&bytes[..])).unwrap();
    let parts: Vec<(u64, u64)> = file
        .entries()
        .iter()
        .filter(|e| e.name.starts_with("results/") && e.kind != Kind::PROVENANCE)
        .map(|e| (e.offset, e.len))
        .collect();
    let (mut flipped, mut found) = (0, 0);
    for (offset, len) in parts {
        for at in (offset..offset + len).step_by(3) {
            let mut bad = bytes.clone();
            bad[at as usize] ^= 0x5A;
            let source = MemorySource::new(&bad[..]);
            let Ok(file) = Container::open(&source) else {
                continue;
            };
            let mut report = Report::default();
            hierarchy::check(&file, &memory, &mut report).unwrap();
            flipped += 1;
            found += usize::from(!report.problems.is_empty());
        }
    }
    eprintln!("{found} of {flipped} flips found by the conservation checks alone");
    assert!(
        flipped > 1000 && found * 10 > flipped * 9,
        "{found} of {flipped}"
    );
}
