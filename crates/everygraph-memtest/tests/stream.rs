// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The semi-external regime under the counting allocator: `PageRank`, components, BFS,
//! k-core and degrees on a compressed stream, under a budget of half its data part, give
//! the in-memory bits, never hold more heap than the budget's peak use and a small
//! slack, and give all of it back.

#![forbid(unsafe_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

use everygraph_core::{Budget, Memory, MemorySource};
use everygraph_format::adjacency::{Adjacency, AdjacencyIndex, encode};
use everygraph_kernels::edge_map::{Compressed, CompressedStream, Plan, Settings, Source};
use everygraph_memtest::counting::{self, Counting};
use everygraph_memtest::probe::alone;

#[global_allocator]
static HEAP: Counting = Counting;

/// Bytes the plans hold outside the budget: their task lists, pools and partial values.
const SLACK: usize = 16 << 10;

/// A symmetric graph without loops: each node joined to 50 drawn nodes.
fn symmetric(n: u32) -> (Vec<u64>, Vec<u32>) {
    let mut pairs = Vec::new();
    let mut state = 7_u64;
    for v in 0..n {
        for _ in 0..50 {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            let u = u32::try_from((state >> 33) % u64::from(n)).unwrap();
            if u != v {
                pairs.extend([(v, u), (u, v)]);
            }
        }
    }
    pairs.sort_unstable();
    pairs.dedup();
    let mut offsets: Vec<u64> = (0..=n).map(|_| 0).collect();
    for &(v, _) in &pairs {
        offsets[usize::try_from(v).unwrap() + 1] += 1;
    }
    for v in 0..offsets.len() - 1 {
        offsets[v + 1] += offsets[v];
    }
    (offsets, pairs.iter().map(|p| p.1).collect())
}

/// A hash of `values`, so that results kept for the comparison take no heap.
fn hash(values: impl Iterator<Item = u32>) -> u64 {
    values.fold(0xCBF2_9CE4_8422_2325, |h, x| {
        (h ^ u64::from(x)).wrapping_mul(0x0000_0100_0000_01B3)
    })
}

/// `PageRank` (f32), components, BFS, k-core and degrees on `source`, hashed.
fn kernels<S: Source>(source: &S, memory: &Memory, settings: Settings) -> [u64; 5] {
    use everygraph_kernels::components::{WccOptions, wcc};
    use everygraph_kernels::degrees::{Loop, degrees};
    use everygraph_kernels::kcore::kcore;
    use everygraph_kernels::pagerank::{self, Graph, PageRank};
    use everygraph_kernels::traversal::{Bfs, bfs};
    let load = |a: &[AtomicU32]| hash(a.iter().map(|x| x.load(Relaxed)));
    Plan::with(source, memory, settings, |plan| {
        let s = pagerank::strength(&plan, None, false, memory).unwrap();
        let g = Graph {
            lists: &plan,
            weights: None,
            directed: false,
        };
        let p = PageRank {
            tol: 0.0,
            max_iter: 5,
            ..PageRank::default()
        };
        let rank = pagerank::pagerank::<_, f32>(&g, s.as_slice().unwrap(), &p, memory).unwrap();
        drop(s);
        let rank = hash(rank.values.iter().map(|r| r.to_bits()));
        let options = WccOptions {
            sample: false,
            ..WccOptions::default()
        };
        let labels = load(&wcc(&plan, memory, options).unwrap().0);
        let depth = load(
            &bfs(&plan, Some(&plan), 1, &Bfs::default(), memory)
                .unwrap()
                .distance,
        );
        let core = load(&kcore(&plan, memory).unwrap().0);
        let degree = hash(degrees(&plan, memory, Loop::Once).unwrap().iter().copied());
        [rank, labels, depth, core, degree]
    })
}

/// On a pool of 4 threads that have all run once ([`alone`]), with 4 KiB windows: the
/// stream gives the bits of the compressed format in memory; the heap above the baseline
/// never exceeds the budget's peak use by more than the slack; after the run the heap and
/// the budget are back at the baseline. The baseline is taken after one run of the
/// kernels, which leaves heap on the pool's threads for good.
#[test]
fn kernels_on_a_stream_hold_no_more_heap_than_they_reserve() {
    let (offsets, targets) = symmetric(20_000);
    let encoded = encode(&offsets, &targets).unwrap();
    let index = AdjacencyIndex::parse(&encoded.index).unwrap();
    let heap = Compressed::new(Adjacency::new(&index, &encoded.data).unwrap());
    let data = MemorySource::new(&encoded.data[..]);
    let streamed = CompressedStream::new(&index, &data, 0).unwrap();
    let limit = encoded.data.len() as u64 / 2;
    let budget = Arc::new(Budget::new(limit));
    let memory = Memory::from(budget.clone());
    let (expected, got, over, after) = alone(|| {
        let expected = kernels(&heap, &Memory::unlimited(), Settings::default());
        kernels(&streamed, &memory, streamed.settings());
        let base = counting::start();
        let got = kernels(&streamed, &memory, streamed.settings());
        (
            expected,
            got,
            counting::peak() - base,
            counting::live().abs_diff(base),
        )
    });
    assert_eq!(got, expected);
    let peak = usize::try_from(budget.peak()).unwrap();
    assert!(
        over <= peak + SLACK,
        "{over} B of heap, {peak} B reserved at most"
    );
    assert_eq!(
        (after, budget.used()),
        (0, 0),
        "heap and budget after the run"
    );
}
