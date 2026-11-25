// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The contract of the edge-map drivers, checked directly on generated graphs: every
//! source hands the body the same lists; reductions follow the fixed node blocks, not the
//! tasks, the batches, the threads or the source; filters, frontiers, corrupt input and
//! the memory budget behave as documented.

#![forbid(unsafe_code)]
// Tests write the files that the mapped and streamed sources read.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;

use everygraph_core::{Budget, ByteSource, Memory, MemorySource, par};
use everygraph_format::adjacency::{Adjacency, AdjacencyIndex, encode};
use everygraph_io::{FileSource, MmapSource};
use everygraph_kernels::edge_map::{
    Compressed, Csr, Error, Node, NodeList, Plan, Settings, Source, Stream, all, ids,
};

fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Sorted, duplicate-free lists over `n` nodes: a skewed degree distribution with
/// isolated nodes, one hub longer than any batch, and runs of empty lists.
fn graph(n: u32, seed: u64) -> (Vec<u64>, Vec<u32>) {
    let mut s = seed;
    let (mut offsets, mut targets) = (vec![0_u64], Vec::new());
    for v in 0..n {
        let r = splitmix(&mut s);
        let d = match r % 10 {
            0..=2 => 0,
            3..=8 => r >> 8 & 31,
            _ => r >> 8 & 255,
        };
        let d = if v == n / 2 { u64::from(n) * 3 / 4 } else { d };
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

/// A value of wildly varying magnitude per node, so that any change in how the sums are
/// grouped changes their bits.
fn value(v: u32, list: &[u32]) -> f64 {
    let scale = [1e-12, 1e-6, 1.0, 1e6, 1e12][(v % 5) as usize];
    let sign = if v % 3 == 0 { -1.0 } else { 1.0 };
    sign * scale * (1.0 + f64::from(list.iter().fold(0_u32, |a, &t| a ^ t)) / 7.0)
}

/// One graph in every storage.
struct Stored {
    offsets: Vec<u64>,
    targets: Vec<u32>,
    index: AdjacencyIndex,
    data: Vec<u8>,
    map: MmapSource,
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
        stream: FileSource::open(&files.0[1]).unwrap(),
        offsets,
        targets,
        _files: files,
    }
}

/// What one run of the probe kernel returns, bit for bit.
type Seen = (u64, Vec<u64>, u64, u64);

/// A scan with a float reduction and a per-node output, a filtered scan, a vertex map
/// and a frontier pass, under `settings` on `threads` threads.
fn probe<S: Source>(source: &S, settings: Settings) -> Seen {
    let memory = Memory::unlimited();
    Plan::with(source, &memory, settings, |plan| {
        par::with_threads(settings.threads, || {
            let mut out = vec![0_u64; plan.nodes() as usize];
            let sum: f64 = plan
                .scan(&mut out[..], all, |acc: &mut f64, out, i, v, list| {
                    *acc += value(v.get(), ids(list));
                    let total = list.iter().map(|&t| u64::from(t.get())).sum::<u64>();
                    out[i] = total << 8 | list.len() as u64;
                })
                .unwrap();
            let filtered: u64 = plan
                .scan((), |v| v.get() % 7 == 3, |acc: &mut u64, (), _, v, list| {
                    assert_eq!(v.get() % 7, 3);
                    *acc += list.len() as u64 + 1;
                })
                .unwrap();
            let degrees: (u64, f64) = plan
                .vertices((), |acc: &mut (u64, f64), (), _, v, d| {
                    acc.0 += u64::from(d);
                    acc.1 += value(v.get(), &[d]);
                })
                .unwrap();
            let frontier: Vec<Node<'_>> = plan.all_nodes().filter(|v| v.get() % 5 != 1).collect();
            let lists: u64 = plan
                .frontier(&frontier, |acc: &mut u64, v, list| {
                    let total = ids(list).iter().map(|&t| u64::from(t)).sum::<u64>();
                    *acc = acc.wrapping_add(u64::from(v.get()).wrapping_mul(0x9E37) ^ total);
                })
                .unwrap();
            (sum.to_bits(), out, filtered ^ degrees.0 << 32 ^ degrees.1.to_bits(), lists)
        })
        .unwrap()
    })
}

/// The probe's scan sum computed the documented way: per block of `block` nodes in node
/// order, the blocks then added in order.
fn blockwise(offsets: &[u64], targets: &[u32], block: u32) -> u64 {
    let n = u32::try_from(offsets.len() - 1).unwrap();
    let mut total = 0.0;
    for b in (0..n).step_by(block as usize) {
        let mut acc = 0.0;
        for v in b..(b + block).min(n) {
            let v_ = v as usize;
            acc += value(v, &targets[offsets[v_] as usize..offsets[v_ + 1] as usize]);
        }
        total += acc;
    }
    total.to_bits()
}

fn settings(threads: usize, chunk: u64, batch: usize) -> Settings {
    Settings {
        threads: NonZeroUsize::new(threads).unwrap(),
        chunk_entries: chunk,
        batch_entries: batch,
        prefetch: batch % 2 == 1,
    }
}

#[test]
fn every_source_threads_chunking_and_batch_give_the_same_bits() {
    for (k, n) in [1_u32, 63, 700, 20_000].into_iter().enumerate() {
        let (offsets, targets) = graph(n, 7 + k as u64);
        let s = store(&format!("g{k}"), offsets, targets);
        let csr = Csr::new(&s.offsets, &s.targets).unwrap();
        let heap = Compressed::new(Adjacency::new(&s.index, &s.data).unwrap());
        let mapped = Compressed::new(Adjacency::new(&s.index, s.map.as_slice().unwrap()).unwrap())
            .with_hints(&s.map);
        let stream = Stream::new(&s.offsets, &s.stream).unwrap();
        let block = Plan::with(&csr, &Memory::unlimited(), Settings::default(), |p| p.block_nodes());
        let expected = probe(&csr, Settings::default());
        assert_eq!(expected.0, blockwise(&s.offsets, &s.targets, block), "n {n}");
        for threads in [1, 3, 8] {
            for (chunk, batch) in [(1, 1), (64, 7), (1 << 16, 8192), (u64::MAX, usize::MAX)] {
                let c = settings(threads, chunk, batch);
                let what = format!("n {n}, threads {threads}, chunk {chunk}, batch {batch}");
                assert_eq!(probe(&csr, c), expected, "csr, {what}");
                assert_eq!(probe(&heap, c), expected, "compressed, {what}");
                assert_eq!(probe(&mapped, c), expected, "mapped, {what}");
                assert_eq!(probe(&stream, c), expected, "stream, {what}");
            }
        }
    }
}

/// Both ways of listing and clearing: the sparse one (few touched words, on the calling
/// thread) and the dense one (every word, in parallel).
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
        let mut nodes: Vec<Node<'_>> =
            plan.all_nodes().filter(|v| v.get() % 3 == 0 || v.get() % 64 == 63).collect();
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

/// A target outside the graph is an error naming its node, from every source, before the
/// body sees it.
#[test]
fn a_target_outside_the_graph_is_an_error_not_a_panic() {
    let offsets = vec![0, 2, 3, 3, 5];
    let targets = vec![1, 2, 0, 1, 9];
    let e = Csr::new(&offsets, &targets).unwrap_err();
    assert!(matches!(e, Error::Target { node: 3, target: 9 }), "{e}");
    let s = store("bad", offsets, targets);
    let heap = Compressed::new(Adjacency::new(&s.index, &s.data).unwrap());
    let stream = Stream::new(&s.offsets, &s.stream).unwrap();
    let memory = Memory::unlimited();
    fn scan<S: Source>(p: &Plan<'_, '_, S>) -> Error {
        p.scan((), all, |(): &mut (), (), _, _, list| assert!(list.iter().all(|t| t.get() < 4)))
            .unwrap_err()
    }
    let s0 = Settings::default();
    fn frontier<S: Source>(p: &Plan<'_, '_, S>, f: &[u32]) -> Error {
        let f: Vec<Node<'_>> = f.iter().map(|&v| p.node(v).unwrap()).collect();
        p.frontier(&f, |(): &mut (), _, _| ()).unwrap_err()
    }
    for e in [
        Plan::with(&heap, &memory, s0, |p| scan(&p)),
        Plan::with(&stream, &memory, s0, |p| scan(&p)),
        Plan::with(&heap, &memory, s0, |p| frontier(&p, &[3])),
        Plan::with(&stream, &memory, s0, |p| frontier(&p, &[2, 3])),
    ] {
        assert!(matches!(e, Error::Target { node: 3, target: 9 }), "{e}");
    }
    // Offsets that are not a CSR are refused when the source is made.
    assert!(matches!(Csr::new(&[0, 2, 1], &[0]), Err(Error::Invalid(_))));
    let bytes = MemorySource::new(vec![0_u8; 8]);
    assert!(matches!(Stream::new(&[0, 1, 3], &bytes), Err(Error::Invalid(_))));
}

/// Readers are reserved against the budget, and a refusal is an error.
#[test]
fn readers_are_reserved_against_the_budget() {
    let (offsets, targets) = graph(2000, 3);
    let s = store("budget", offsets, targets);
    let heap = Compressed::new(Adjacency::new(&s.index, &s.data).unwrap());
    let budget = Arc::new(Budget::new(1 << 20));
    let memory = Memory::from(budget.clone());
    Plan::with(&heap, &memory, Settings::default(), |plan| {
        let entries: u64 =
            plan.scan((), all, |acc: &mut u64, (), _, _, l| *acc += l.len() as u64).unwrap();
        assert_eq!(entries, s.targets.len() as u64);
        // One reader: the hub's list (1,500 entries) and the batch, as decode buffers.
        assert!(budget.used() >= 8192 * 4, "{}", budget.used());
    });
    assert_eq!(budget.used(), 0);
    let tiny = Memory::from(Arc::new(Budget::new(1000)));
    let e = Plan::with(&heap, &tiny, Settings::default(), |plan| {
        plan.scan((), all, |(): &mut (), (), _, _, _| ()).unwrap_err()
    });
    assert!(matches!(e, Error::Memory(_)), "{e}");
}
