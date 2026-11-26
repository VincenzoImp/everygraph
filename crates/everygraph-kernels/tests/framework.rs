// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The contract of the edge-map drivers, checked on generated graphs: every source
//! hands the body the same rows; reductions follow the fixed node blocks and frontier
//! runs, not the tasks, the batches, the threads or the source; per-entry columns, worker
//! scratch, random access, filters, corrupt input and the memory budget behave as
//! documented.

#![forbid(unsafe_code)]
// Tests write the files that the mapped and streamed sources read.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
// Small generated graphs: casts cannot truncate.
#![allow(clippy::cast_possible_truncation, clippy::type_complexity)]

use std::num::NonZeroUsize;
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

use everygraph_core::{Budget, Memory, MemorySource, par};
use everygraph_format::adjacency::{Adjacency, AdjacencyIndex, encode};
use everygraph_io::{FileSource, MmapSource};
use everygraph_kernels::edge_map::{
    Compressed, CompressedStream, Csr, Error, FRONTIER_GRAIN, Node, NodeList, PREFIX, Plan,
    RUN_ENTRIES, Reduce, Settings, Source, Stream, Work, all, ids,
};
use everygraph_kernels::{pagerank, proof, traversal};

fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Sorted, duplicate-free lists over `n` nodes: a skewed degree distribution with empty
/// lists, and one hub longer than any batch. With `sparse`, 99 nodes in 100 have no
/// list.
fn graph(n: u32, seed: u64, sparse: bool) -> (Vec<u64>, Vec<u32>) {
    let mut s = seed;
    let (mut offsets, mut targets) = (vec![0_u64], Vec::new());
    for v in 0..n {
        let r = splitmix(&mut s);
        let d = match r % 10 {
            _ if sparse && !r.is_multiple_of(100) => 0,
            0..=2 => 0,
            3..=8 => r >> 8 & 31,
            _ => r >> 8 & 255,
        };
        // Four hubs in a row: their frontier group is more than one run.
        let d = if (n / 2..n / 2 + 4).contains(&v) {
            u64::from(n.min(60_000)) * 3 / 4
        } else {
            d
        };
        let mut list: Vec<u32> = (0..d)
            .map(|_| u32::try_from(splitmix(&mut s) % u64::from(n)).unwrap())
            .collect();
        list.sort_unstable();
        list.dedup();
        targets.extend(list);
        offsets.push(targets.len() as u64);
    }
    (offsets, targets)
}

/// A value of widely varying magnitude per node: any change in how the sums are grouped
/// changes their bits.
fn value(v: u32, list: &[u32]) -> f64 {
    let scale = [1e-12, 1e-6, 1.0, 1e6, 1e12][(v % 5) as usize];
    let sign = if v.is_multiple_of(3) { -1.0 } else { 1.0 };
    sign * scale * (1.0 + f64::from(list.iter().fold(0_u32, |a, &t| a ^ t)) / 7.0)
}

/// A float added in the order of the reduction tree, which the framework gives only
/// to tests: a float that carries a result reduces into `ExactSum`.
#[derive(Clone, Copy, Debug, Default)]
struct Tree(f64);

impl Reduce for Tree {
    const ZERO: Self = Self(0.0);
    fn add(self, other: Self) -> Self {
        Self(self.0 + other.0)
    }
}

/// The per-entry column of the tests: a weight for each entry, in node order.
fn weight(entry: usize) -> f64 {
    1.0 + (entry % 13) as f64 / 16.0
}

/// One graph in every storage.
struct Stored {
    offsets: Vec<u64>,
    targets: Vec<u32>,
    index: AdjacencyIndex,
    data: Vec<u8>,
    map: MmapSource,
    /// The data part, read by positional reads.
    file: FileSource,
    stream: FileSource,
    _files: Temp,
}

struct Temp(Vec<PathBuf>);

impl Drop for Temp {
    fn drop(&mut self) {
        for f in &self.0 {
            let _ = std::fs::remove_file(f);
        }
    }
}

fn store(name: &str, offsets: Vec<u64>, targets: Vec<u32>) -> Stored {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("edge-map-framework");
    std::fs::create_dir_all(&dir).unwrap();
    let files = Temp(vec![
        dir.join(format!("{}-{name}.data", std::process::id())),
        dir.join(format!("{}-{name}.targets", std::process::id())),
    ]);
    let encoded = encode(&offsets, &targets).unwrap();
    std::fs::write(&files.0[0], &encoded.data).unwrap();
    let bytes: Vec<u8> = targets.iter().flat_map(|t| t.to_le_bytes()).collect();
    std::fs::write(&files.0[1], bytes).unwrap();
    Stored {
        index: AdjacencyIndex::parse(&encoded.index).unwrap(),
        data: encoded.data,
        map: MmapSource::open(&files.0[0]).unwrap(),
        file: FileSource::open(&files.0[0]).unwrap(),
        stream: FileSource::open(&files.0[1]).unwrap(),
        offsets,
        targets,
        _files: files,
    }
}

impl Stored {
    fn list(&self, v: u32) -> &[u32] {
        let v = v as usize;
        &self.targets[self.offsets[v] as usize..self.offsets[v + 1] as usize]
    }
}

/// What one run of the probe returns, bit for bit: the scan's float sum, the per-node
/// outputs, the filtered scans and vertex map, the frontier's float sum, the common
/// neighbours counted through random access, and the pieces of a scan that stops early.
type Seen = (u64, Vec<u64>, u64, u64, u64, u64);

/// A small number that changes with each entry of `list` and with `first`, its first
/// entry: sums of it do not overflow.
fn mix(list: &[u32], first: u64) -> u64 {
    let h = list
        .iter()
        .fold(list.len() as u64, |h, &t| h.rotate_left(5) ^ u64::from(t));
    (h ^ first.rotate_left(17)) % 1_000_003
}

/// The filter of the scan that stops early.
fn until_filter(v: u32) -> bool {
    !v.is_multiple_of(3)
}

/// The frontier of the probe: every node but those `≡ 1 (mod 5)`.
fn in_frontier(v: u32) -> bool {
    v % 5 != 1
}

/// A scan with a float reduction, a per-node output and a per-entry column; a filtered
/// scan; a vertex map; a frontier with a float reduction; a scan with scratch and random
/// access. On `threads` threads.
fn probe<S: Source>(source: &S, settings: Settings, threads: usize) -> Seen {
    let memory = Memory::unlimited();
    let weights: Vec<f64> = (0..source.entries() as usize).map(weight).collect();
    let threads = NonZeroUsize::new(threads).unwrap();
    Plan::with(source, &memory, settings, |plan| {
        par::with_threads(threads, || {
            let mut out = vec![0_u64; plan.nodes() as usize];
            let sum: Tree = plan
                .scan(&mut out[..], all, |acc: &mut Tree, out, i, row, _| {
                    acc.0 += value(row.node.get(), ids(row.list));
                    let w = weights[row.entries()].iter().fold(0.0, |s: f64, &x| s + x);
                    let total = ids(row.list).iter().map(|&t| u64::from(t)).sum::<u64>();
                    out[i] = (total << 8 | row.list.len() as u64) ^ w.to_bits();
                })
                .unwrap();
            let filtered: u64 = plan
                .scan(
                    (),
                    |v| v.get() % 7 == 3,
                    |acc: &mut u64, (), _, row, _| {
                        assert_eq!(row.node.get() % 7, 3);
                        *acc += mix(ids(row.list), row.first_entry);
                    },
                )
                .unwrap();
            // Passing nodes far apart: the records between them are skipped.
            let far: u64 = plan
                .scan(
                    (),
                    |v| v.get() % 97 == 5,
                    |acc: &mut u64, (), _, row, _| {
                        assert_eq!(row.first_entry, source.entry_offset(row.node.get()));
                        *acc += mix(ids(row.list), row.first_entry);
                    },
                )
                .unwrap();
            let degrees: u64 = plan
                .vertices((), |acc: &mut u64, (), _, v, d| {
                    *acc += u64::from(d) * u64::from(v.get());
                })
                .unwrap();
            let frontier: Vec<Node<'_>> =
                plan.all_nodes().filter(|v| in_frontier(v.get())).collect();
            let pushed: Tree = plan
                .frontier(&frontier, |acc: &mut Tree, row, _| {
                    acc.0 += value(row.node.get(), ids(row.list));
                })
                .unwrap();
            let serial: Tree = plan
                .frontier_serial_with(
                    &frontier,
                    || (),
                    |acc: &mut Tree, row, _| {
                        acc.0 += value(row.node.get(), ids(row.list));
                    },
                )
                .unwrap();
            assert_eq!(serial.0.to_bits(), pushed.0.to_bits());
            let common: u64 = plan
                .scan_with((), all, Vec::<u32>::new, |acc: &mut u64, (), _, row, w| {
                    let Some(&u) = row.list.first() else { return };
                    let mine = ids(row.list).to_vec();
                    let side = w.lists.get(u).unwrap();
                    w.scratch.clear();
                    w.scratch.extend(
                        ids(side.list)
                            .iter()
                            .filter(|t| mine.binary_search(t).is_ok()),
                    );
                    *acc += w.scratch.len() as u64;
                })
                .unwrap();
            let until: u64 = plan
                .scan_until(
                    (),
                    |v| until_filter(v.get()),
                    |acc: &mut u64, (), _, row| {
                        *acc += mix(ids(row.list), row.first_entry);
                        if row.node.get().is_multiple_of(2) {
                            ControlFlow::Continue(())
                        } else {
                            ControlFlow::Break(())
                        }
                    },
                )
                .unwrap();
            let rest = filtered ^ far << 24 ^ degrees << 44;
            (
                sum.0.to_bits(),
                out,
                rest,
                pushed.0.to_bits(),
                common,
                until,
            )
        })
        .unwrap()
    })
}

/// The runs of a frontier group, as documented: a run ends before the node that would
/// take it past `RUN_ENTRIES`, a node counting its degree plus one.
fn runs<'a>(s: &Stored, group: &'a [u32]) -> Vec<&'a [u32]> {
    let (mut runs, mut start, mut entries) = (Vec::new(), 0, 0);
    for (i, &v) in group.iter().enumerate() {
        let cost = s.list(v).len() + 1;
        if entries > 0 && entries + cost > RUN_ENTRIES {
            runs.push(&group[start..i]);
            (start, entries) = (i, 0);
        }
        entries += cost;
    }
    runs.push(&group[start..]);
    runs
}

/// The probe's float sums and its early-stopping scan computed as documented: per block
/// of `block` nodes in node order, then the blocks in order; per run of each group of
/// `FRONTIER_GRAIN` frontier nodes in order, then the groups in order; the pieces of a
/// list, ending at `PREFIX`, then at four times the end before.
fn reference(s: &Stored, block: u32) -> (u64, u64, u64) {
    let n = (s.offsets.len() - 1) as u32;
    let mut scanned = 0.0;
    for b in (0..n).step_by(block as usize) {
        let mut acc = 0.0;
        for v in b..(b + block).min(n) {
            acc += value(v, s.list(v));
        }
        scanned += acc;
    }
    let frontier: Vec<u32> = (0..n).filter(|&v| in_frontier(v)).collect();
    let mut pushed = 0.0;
    for group in frontier.chunks(FRONTIER_GRAIN) {
        let mut g = 0.0;
        for run in runs(s, group) {
            let mut acc = 0.0;
            for &v in run {
                acc += value(v, s.list(v));
            }
            g += acc;
        }
        pushed += g;
    }
    let mut until = 0;
    for v in (0..n).filter(|&v| until_filter(v)) {
        let (list, first) = (s.list(v), s.offsets[v as usize]);
        let (mut at, mut to) = (0, list.len().min(PREFIX));
        loop {
            until += mix(&list[at..to], first + at as u64);
            if !v.is_multiple_of(2) || to == list.len() {
                break;
            }
            (at, to) = (to, list.len().min(4 * to));
        }
    }
    (scanned.to_bits(), pushed.to_bits(), until)
}

fn settings(chunk: u64, batch: usize) -> Settings {
    Settings {
        chunk_entries: chunk,
        batch_entries: batch,
        prefetch: batch % 2 == 1,
        alternate: chunk.is_multiple_of(2),
    }
}

#[test]
fn every_source_thread_count_chunk_and_batch_give_the_same_bits() {
    let graphs = [
        (1, false),
        (63, false),
        (700, false),
        (20_000, false),
        (300_001, true),
    ];
    for (k, (n, sparse)) in graphs.into_iter().enumerate() {
        let (offsets, targets) = graph(n, 7 + k as u64, sparse);
        let s = store(&format!("g{k}"), offsets, targets);
        let csr = Csr::new(&s.offsets, &s.targets).unwrap();
        let heap = Compressed::new(Adjacency::new(&s.index, &s.data).unwrap());
        let mapped = Compressed::mapped(&s.index, &s.map).unwrap();
        let stream = Stream::new(&s.offsets, &s.stream).unwrap();
        let small = CompressedStream::new(&s.index, &s.file, 0).unwrap();
        let large = CompressedStream::new(&s.index, &s.file, 1 << 16).unwrap();
        let block = Plan::with(&csr, &Memory::unlimited(), Settings::default(), |p| {
            p.block_nodes()
        });
        let expected = probe(&csr, Settings::default(), 1);
        let documented = (expected.0, expected.3, expected.5);
        assert_eq!(documented, reference(&s, block), "n {n}");
        let configurations: &[(u64, usize)] = if sparse {
            &[(64, 7), (u64::MAX, usize::MAX)]
        } else {
            &[(1, 1), (64, 7), (1 << 16, 8192), (u64::MAX, usize::MAX)]
        };
        for threads in [1, 3, 8] {
            for &(chunk, batch) in configurations {
                let c = settings(chunk, batch);
                let what = format!("n {n}, threads {threads}, chunk {chunk}, batch {batch}");
                assert_eq!(probe(&csr, c, threads), expected, "csr, {what}");
                assert_eq!(probe(&heap, c, threads), expected, "compressed, {what}");
                assert_eq!(probe(&mapped, c, threads), expected, "mapped, {what}");
                assert_eq!(probe(&stream, c, threads), expected, "stream, {what}");
                assert_eq!(probe(&small, c, threads), expected, "4 KiB window, {what}");
                assert_eq!(probe(&large, c, threads), expected, "64 KiB window, {what}");
            }
        }
    }
}

/// Both ways of listing and clearing a node set: the sparse one (few touched words, on
/// the calling thread) and the dense one (every word, in parallel).
#[test]
fn node_sets_list_their_nodes_in_order() {
    for n in [10_000, 1 << 21] {
        node_set_lists_in_order(n);
    }
}

fn node_set_lists_in_order(n: usize) {
    let memory = Memory::unlimited();
    let (offsets, targets) = (vec![0_u64; n + 1], Vec::new());
    let csr = Csr::new(&offsets, &targets).unwrap();
    Plan::with(&csr, &memory, Settings::default(), |plan| {
        let mut set = plan.node_set(&memory).unwrap();
        let mut nodes: Vec<Node<'_>> = plan
            .all_nodes()
            .filter(|v| v.get() % 3 == 0 || v.get() % 64 == 63)
            .collect();
        let sorted = nodes.clone();
        nodes.reverse();
        let grain = NonZeroUsize::new(100).unwrap();
        par::with_threads(NonZeroUsize::new(4).unwrap(), || {
            par::for_each_range(nodes.len(), grain, |r| {
                for &v in &nodes[r] {
                    assert!(set.insert(v));
                }
            });
        })
        .unwrap();
        let node = |id| plan.node(id).unwrap();
        assert!(!set.insert(node(0)) && set.contains(node(9999)) && !set.contains(node(1)));
        assert!(plan.node(u32::try_from(n).unwrap()).is_none());
        let mut out = NodeList::new(&memory);
        set.list_into(&mut out).unwrap();
        assert_eq!(out.as_slice().unwrap(), &sorted[..]);
        set.clear();
        set.list_into(&mut out).unwrap();
        assert!(out.is_empty());
    });
}

/// A target outside the graph is an error naming its node, from every source and
/// driver, before any body sees the list.
#[test]
fn a_target_outside_the_graph_is_an_error_not_a_panic() {
    fn scan<S: Source>(p: &Plan<'_, '_, S>) -> Error {
        p.scan((), all, |(): &mut (), (), _, row, _| {
            assert!(row.list.iter().all(|t| t.get() < 4));
        })
        .unwrap_err()
    }
    fn frontier<S: Source>(p: &Plan<'_, '_, S>, f: &[u32]) -> Error {
        let f: Vec<Node<'_>> = f.iter().map(|&v| p.node(v).unwrap()).collect();
        p.frontier(&f, |(): &mut (), _, _| ()).unwrap_err()
    }
    fn side<S: Source>(p: &Plan<'_, '_, S>) -> Error {
        let (zero, three) = (p.node(0).unwrap(), p.node(3).unwrap());
        let bad: u64 = p
            .frontier(&[zero], |acc: &mut u64, _, w| {
                *acc += w.lists.get(three).map_or(1, |_| 0);
            })
            .unwrap();
        assert_eq!(bad, 1);
        Error::Invalid("checked above")
    }
    let offsets = vec![0, 2, 3, 3, 5];
    let targets = vec![1, 2, 0, 1, 9];
    let e = Csr::new(&offsets, &targets).unwrap_err();
    assert!(matches!(e, Error::Target { node: 3, target: 9 }), "{e}");
    let s = store("bad", offsets, targets);
    let heap = Compressed::new(Adjacency::new(&s.index, &s.data).unwrap());
    let stream = Stream::new(&s.offsets, &s.stream).unwrap();
    let streamed = CompressedStream::new(&s.index, &s.file, 0).unwrap();
    let memory = Memory::unlimited();
    let s0 = Settings::default();
    for e in [
        Plan::with(&heap, &memory, s0, |p| scan(&p)),
        Plan::with(&stream, &memory, s0, |p| scan(&p)),
        Plan::with(&streamed, &memory, s0, |p| scan(&p)),
        Plan::with(&heap, &memory, s0, |p| frontier(&p, &[3])),
        Plan::with(&stream, &memory, s0, |p| frontier(&p, &[2, 3])),
        Plan::with(&streamed, &memory, s0, |p| frontier(&p, &[1, 3])),
    ] {
        assert!(matches!(e, Error::Target { node: 3, target: 9 }), "{e}");
    }
    Plan::with(&heap, &memory, s0, |p| side(&p));
    Plan::with(&stream, &memory, s0, |p| side(&p));
    Plan::with(&streamed, &memory, s0, |p| side(&p));
    assert!(matches!(Csr::new(&[0, 2, 1], &[0]), Err(Error::Invalid(_))));
    let bytes = MemorySource::new(vec![0_u8; 8]);
    assert!(matches!(
        Stream::new(&[0, 1, 3], &bytes),
        Err(Error::Invalid(_))
    ));
    let wrong = MemorySource::new(vec![0_u8; s.data.len() + 1]);
    assert!(matches!(
        Compressed::mapped(&s.index, &wrong),
        Err(Error::Format(_))
    ));
    assert!(matches!(
        CompressedStream::new(&s.index, &wrong, 0),
        Err(Error::Format(_))
    ));
}

/// Budget peaks of a frontier with and without the hub, on one source.
fn peaks<S: Source>(source: &S, hub: u32) -> (u64, u64, u64) {
    let budget = Arc::new(Budget::unlimited());
    let memory = Memory::from(budget.clone());
    let s = Settings {
        batch_entries: 16,
        ..Settings::default()
    };
    Plan::with(source, &memory, s, |plan| {
        let small: Vec<Node<'_>> = (0..8).filter_map(|v| plan.node(v)).collect();
        plan.frontier(&small, |(): &mut (), _, _| ()).unwrap();
        let low = budget.peak();
        plan.frontier(&[plan.node(hub).unwrap()], |(): &mut (), _, _| ())
            .unwrap();
        let high = budget.peak();
        (low, high, budget.used())
    })
}

/// A run under a budget of `limit` bytes: the small frontier, then the hub.
fn under<S: Source>(source: &S, hub: u32, limit: u64) -> (Result<(), Error>, Result<(), Error>) {
    let memory = Memory::from(Arc::new(Budget::new(limit)));
    let s = Settings {
        batch_entries: 16,
        ..Settings::default()
    };
    Plan::with(source, &memory, s, |plan| {
        let small: Vec<Node<'_>> = (0..8).filter_map(|v| plan.node(v)).collect();
        let a = plan.frontier(&small, |(): &mut (), _, _| ());
        let b = plan.frontier(&[plan.node(hub).unwrap()], |(): &mut (), _, _| ());
        (a, b)
    })
}

/// Readers reserve their buffers, a hub's buffer is refused cleanly when it does not
/// fit, and a reader gives the hub's buffer and its reservation back after the call.
#[test]
fn readers_are_reserved_against_the_budget() {
    let (offsets, targets) = graph(4000, 3, false);
    let s = store("budget", offsets, targets);
    // Longer than a reader's first buffer of list ends.
    let hub = 2000;
    assert!(s.list(hub).len() > 2000);
    let heap = Compressed::new(Adjacency::new(&s.index, &s.data).unwrap());
    let stream = Stream::new(&s.offsets, &s.stream).unwrap();
    let streamed = CompressedStream::new(&s.index, &s.file, 0).unwrap();
    for (name, (low, high, after)) in [
        ("compressed", peaks(&heap, hub)),
        ("stream", peaks(&stream, hub)),
        ("streamed", peaks(&streamed, hub)),
    ] {
        let hub_bytes = s.list(hub).len() as u64 * 4;
        assert!(
            high > low && high >= hub_bytes,
            "{name}: {low} → {high}, hub {hub_bytes} B"
        );
        assert!(
            after <= low,
            "{name}: {after} B held after the hub, {low} before"
        );
        let limit = (low + high) / 2;
        let (a, b) = match name {
            "compressed" => under(&heap, hub, limit),
            "stream" => under(&stream, hub, limit),
            _ => under(&streamed, hub, limit),
        };
        assert!(a.is_ok(), "{name}: {a:?}");
        assert!(matches!(b, Err(Error::Memory(_))), "{name}: {b:?}");
    }
}

/// What `PageRank`, the proof WCC and BFS return on one source, bit for bit, with the
/// BFS's work.
fn kernels<S: Source>(
    source: &S,
    settings: Settings,
    threads: usize,
) -> (Vec<u64>, Vec<u32>, Vec<u32>, Work) {
    let memory = Memory::unlimited();
    Plan::with(source, &memory, settings, |plan| {
        par::with_threads(NonZeroUsize::new(threads).unwrap(), || {
            let load = |a: &AtomicU32| a.load(Relaxed);
            let g = pagerank::Graph {
                lists: &plan,
                weights: None,
                directed: false,
            };
            let s = pagerank::strength(&plan, None, false, &memory).unwrap();
            let p = pagerank::PageRank {
                tol: 0.0,
                max_iter: 10,
                ..pagerank::PageRank::default()
            };
            let rank = pagerank::pagerank::<_, f64>(&g, s.as_slice().unwrap(), &p, &memory)
                .unwrap()
                .values;
            let (labels, _) = proof::wcc(&plan, &memory).unwrap();
            let before = plan.work();
            let bfs = traversal::Bfs::default();
            let depth = traversal::bfs(&plan, Some(&plan), plan.nodes() / 2, &bfs, &memory)
                .unwrap()
                .distance;
            let after = plan.work();
            let work = Work {
                lists: after.lists - before.lists,
                entries: after.entries - before.entries,
            };
            (
                rank.iter().map(|r| r.to_bits()).collect(),
                labels.iter().map(load).collect(),
                depth.iter().map(load).collect(),
                work,
            )
        })
        .unwrap()
    })
}

/// The kernels on every source and thread count: the same bits, and the same BFS
/// work counters (its filter reads only a node's own depth). The union-find cells and
/// the frontier sets are shared between threads here, so `tools/check_tsan.py` runs this
/// file.
#[test]
fn proof_kernels_and_their_work_do_not_depend_on_source_or_threads() {
    let n = 5000_u32;
    let (offsets, targets) = graph(n, 11, false);
    // Symmetric, as the kernels require: every entry and its reverse, without loops.
    let mut pairs: Vec<(u32, u32)> = Vec::new();
    for v in 0..n {
        let list = &targets[offsets[v as usize] as usize..offsets[v as usize + 1] as usize];
        for &u in list.iter().filter(|&&u| u != v) {
            pairs.extend([(v, u), (u, v)]);
        }
    }
    pairs.sort_unstable();
    pairs.dedup();
    let mut offsets = vec![0_u64; n as usize + 1];
    for &(v, _) in &pairs {
        offsets[v as usize + 1] += 1;
    }
    for v in 0..n as usize {
        offsets[v + 1] += offsets[v];
    }
    let targets: Vec<u32> = pairs.iter().map(|p| p.1).collect();
    let s = store("kernels", offsets, targets);
    let csr = Csr::new(&s.offsets, &s.targets).unwrap();
    let heap = Compressed::new(Adjacency::new(&s.index, &s.data).unwrap());
    let stream = Stream::new(&s.offsets, &s.stream).unwrap();
    let streamed = CompressedStream::new(&s.index, &s.file, 0).unwrap();
    let expected = kernels(&csr, Settings::default(), 1);
    assert!(expected.3.lists > 0 && expected.3.entries > 0);
    for threads in [1, 3, 8] {
        for (chunk, batch) in [(64, 7), (1 << 16, 8192)] {
            let c = settings(chunk, batch);
            let what = format!("threads {threads}, chunk {chunk}");
            assert_eq!(kernels(&csr, c, threads), expected, "csr, {what}");
            assert_eq!(kernels(&heap, c, threads), expected, "compressed, {what}");
            assert_eq!(kernels(&stream, c, threads), expected, "stream, {what}");
            assert_eq!(kernels(&streamed, c, threads), expected, "streamed, {what}");
        }
    }
}

/// With scans in alternating order, the second scan runs its tasks from the last, and
/// still reports the first corrupt list in node order, not the first one it reached.
#[test]
fn an_alternating_scan_reports_the_first_corrupt_list_in_node_order() {
    let n = 2000_u32;
    let (mut offsets, mut targets) = (vec![0_u64], Vec::new());
    for v in 0..n {
        targets.push(if v == 100 || v == 1900 {
            n + v
        } else {
            (v + 1) % n
        });
        offsets.push(targets.len() as u64);
    }
    let encoded = encode(&offsets, &targets).unwrap();
    let index = AdjacencyIndex::parse(&encoded.index).unwrap();
    let heap = Compressed::new(Adjacency::new(&index, &encoded.data).unwrap());
    let settings = Settings {
        chunk_entries: 64,
        alternate: true,
        ..Settings::default()
    };
    for threads in [1, 4] {
        Plan::with(&heap, &Memory::unlimited(), settings, |plan| {
            par::with_threads(NonZeroUsize::new(threads).unwrap(), || {
                for scan in 0..2 {
                    let e = plan
                        .scan((), all, |(): &mut (), (), _, _, _| ())
                        .unwrap_err();
                    assert!(
                        matches!(e, Error::Target { node: 100, .. }),
                        "scan {scan}, {threads} threads: {e}"
                    );
                }
            })
            .unwrap();
        });
    }
}
