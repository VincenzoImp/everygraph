// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The heap that the readers of the edge-map framework hold, against what they reserve:
//! while a body runs, the bytes in use above those before the plan never exceed the
//! budget's use by more than a fixed slack (the plan's small uncounted vectors). Lists
//! that grow one entry at a time, longer than the batch, and a frontier of more lists
//! than a block are the cases where an amortized buffer would hold more than it reserved.
//! And the semi-external regime: kernels on a compressed stream under a budget of half
//! the data part never hold more heap than the budget and the slack.
//!
//! Counting needs a global allocator, which needs `unsafe` (module `counting`,
//! CONTRIBUTING.md, "`unsafe`").

// The test's graphs are buffers outside the budget, made before the counting starts.
#![allow(clippy::disallowed_methods)]

use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, Mutex, PoisonError};

use everygraph_core::{Budget, Memory, MemorySource, par};
use everygraph_format::adjacency::{Adjacency, AdjacencyIndex, encode};
use everygraph_kernels::edge_map::{
    Compressed, CompressedStream, Node, Plan, Settings, Source, Stream, all,
};

mod counting {
    //! A global allocator that passes everything to `System` and counts the bytes in
    //! use. Safe code cannot replace the global allocator, hence the module's `unsafe`.
    #![allow(unsafe_code)]

    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static LIVE: AtomicUsize = AtomicUsize::new(0);
    static PEAK: AtomicUsize = AtomicUsize::new(0);

    /// Bytes in use now.
    pub fn live() -> usize {
        LIVE.load(Ordering::Relaxed)
    }

    /// The most bytes in use since the last [`reset_peak`].
    pub fn peak() -> usize {
        PEAK.load(Ordering::Relaxed)
    }

    pub fn reset_peak() {
        PEAK.store(live(), Ordering::Relaxed);
    }

    fn grew(bytes: usize) {
        let now = LIVE.fetch_add(bytes, Ordering::Relaxed) + bytes;
        PEAK.fetch_max(now, Ordering::Relaxed);
    }

    pub struct Counting;

    // SAFETY: every block comes from `System` and goes back to it with the layout it
    // was allocated with; the counter does not touch the blocks.
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            // SAFETY: the caller's contract for `alloc` is passed on unchanged.
            let p = unsafe { System.alloc(layout) };
            if !p.is_null() {
                grew(layout.size());
            }
            p
        }

        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            // SAFETY: as for `alloc`.
            let p = unsafe { System.alloc_zeroed(layout) };
            if !p.is_null() {
                grew(layout.size());
            }
            p
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
            // SAFETY: `ptr` was allocated by `System` with `layout`.
            unsafe { System.dealloc(ptr, layout) };
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            // SAFETY: `ptr` was allocated by `System` with `layout`, and the caller
            // guarantees that `new_size` is valid for it.
            let p = unsafe { System.realloc(ptr, layout, new_size) };
            if !p.is_null() {
                grew(new_size);
                LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
            }
            p
        }
    }

    #[global_allocator]
    static GLOBAL: Counting = Counting;
}

/// Bytes the plan holds outside the budget here: its task list, pool, partial values and
/// frontier results, for a graph of a few thousand nodes.
const SLACK: usize = 16 << 10;

/// Entries of the shortest long list. Under Miri, which only checks the `unsafe` code,
/// the lists are shorter than the slack and the test checks nothing else.
const LONG: u32 = if cfg!(miri) { 300 } else { 10_000 };

/// Nodes 0, 1 and 2 list `LONG`, `LONG + 1` and `LONG + 2` nodes; the others a few.
fn graph(n: u32) -> (Vec<u64>, Vec<u32>) {
    let (mut offsets, mut targets) = (vec![0_u64], Vec::new());
    for v in 0..n {
        let len = if v < 3 { LONG + v } else { v % 4 };
        targets.extend((0..len).map(|k| (v + 1 + k) % n));
        targets[usize::try_from(offsets[usize::try_from(v).unwrap()]).unwrap()..].sort_unstable();
        offsets.push(targets.len() as u64);
    }
    (offsets, targets)
}

/// One test at a time: the counters are global.
static SERIAL: Mutex<()> = Mutex::new(());

/// The largest excess of the heap over the budget's use, seen from inside the bodies of
/// a scan, a frontier of 256 consecutive nodes (more lists than a block), and random
/// access to the three long lists.
fn excess<S: Source>(source: &S) -> usize {
    let budget = Arc::new(Budget::unlimited());
    let memory = Memory::from(budget.clone());
    let settings = Settings {
        batch_entries: 1,
        ..Settings::default()
    };
    let worst = AtomicUsize::new(0);
    let base = counting::live();
    let seen = || {
        let over = counting::live().saturating_sub(base);
        let used = usize::try_from(budget.used()).unwrap();
        worst.fetch_max(over.saturating_sub(used), Relaxed);
    };
    Plan::with(source, &memory, settings, |plan| {
        plan.scan((), all, |(): &mut (), (), _, _, _| seen())
            .unwrap();
        let run: Vec<Node<'_>> = (0..256).filter_map(|v| plan.node(v)).collect();
        plan.frontier(&run, |(): &mut (), _, _| seen()).unwrap();
        let long: Vec<Node<'_>> = (0..3).filter_map(|v| plan.node(v)).collect();
        plan.frontier(&[run[10]], |(): &mut (), _, w| {
            for &v in &long {
                w.lists.get(v).unwrap();
                seen();
            }
        })
        .unwrap();
    });
    worst.into_inner()
}

#[test]
fn readers_hold_no_more_heap_than_they_reserve() {
    let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
    let n = 2 * LONG;
    let (offsets, targets) = graph(n);
    let encoded = encode(&offsets, &targets).unwrap();
    let index = AdjacencyIndex::parse(&encoded.index).unwrap();
    let compressed = Compressed::new(Adjacency::new(&index, &encoded.data).unwrap());
    let bytes = MemorySource::new(
        targets
            .iter()
            .flat_map(|t| t.to_le_bytes())
            .collect::<Vec<u8>>(),
    );
    let stream = Stream::new(&offsets, &bytes).unwrap();
    let data = MemorySource::new(&encoded.data[..]);
    let streamed = CompressedStream::new(&index, &data, 0).unwrap();
    for (name, over) in [
        ("compressed", excess(&compressed)),
        ("stream", excess(&stream)),
        ("compressed stream", excess(&streamed)),
    ] {
        assert!(
            over <= SLACK,
            "{name}: {over} B of heap above the budget's use"
        );
    }
}

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
    let mut offsets = vec![0_u64; usize::try_from(n).unwrap() + 1];
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

/// Under a budget of half the data part, with 4 threads and 4 KiB windows, the kernels
/// give the in-memory bits, and the heap above what was in use before never exceeds the
/// budget by more than the slack.
#[test]
#[cfg_attr(
    miri,
    ignore = "minutes under Miri; the test above reaches the same `unsafe` code"
)]
fn kernels_on_a_stream_hold_no_more_heap_than_the_budget() {
    let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
    let n = 10_000;
    let (offsets, targets) = symmetric(n);
    let encoded = encode(&offsets, &targets).unwrap();
    let index = AdjacencyIndex::parse(&encoded.index).unwrap();
    let heap = Compressed::new(Adjacency::new(&index, &encoded.data).unwrap());
    let expected = kernels(&heap, &Memory::unlimited(), Settings::default());
    let data = MemorySource::new(&encoded.data[..]);
    let streamed = CompressedStream::new(&index, &data, 0).unwrap();
    let limit = encoded.data.len() as u64 / 2;
    let memory = Memory::from(Arc::new(Budget::new(limit)));
    let threads = NonZeroUsize::new(4).unwrap();
    let base = counting::live();
    counting::reset_peak();
    let got =
        par::with_threads(threads, || kernels(&streamed, &memory, streamed.settings())).unwrap();
    let over = counting::peak() - base;
    assert_eq!(got, expected);
    assert!(
        over <= usize::try_from(limit).unwrap() + SLACK,
        "{over} B of heap, budget {limit} B"
    );
}
