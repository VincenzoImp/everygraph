// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The heap that the readers of the edge-map framework hold, against what they reserve:
//! while a body runs, the bytes in use above those before the plan never exceed the
//! budget's use by more than a fixed slack (the plan's small uncounted vectors). Lists
//! that grow one entry at a time, longer than the batch, and a frontier of more lists
//! than a block are the cases where an amortized buffer would hold more than it reserved.
//!
//! Counting needs a global allocator, which needs `unsafe` (module `counting`,
//! CONTRIBUTING.md, "`unsafe`").

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use everygraph_core::{Budget, Memory, MemorySource};
use everygraph_format::adjacency::{Adjacency, AdjacencyIndex, encode};
use everygraph_kernels::edge_map::{Compressed, Node, Plan, Settings, Source, Stream, all};

mod counting {
    //! A global allocator that passes everything to `System` and counts the bytes in
    //! use. Safe code cannot replace the global allocator, hence the module's `unsafe`.
    #![allow(unsafe_code)]

    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static LIVE: AtomicUsize = AtomicUsize::new(0);

    /// Bytes in use now.
    pub fn live() -> usize {
        LIVE.load(Ordering::Relaxed)
    }

    pub struct Counting;

    // SAFETY: every block comes from `System` and goes back to it with the layout it
    // was allocated with; the counter does not touch the blocks.
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            // SAFETY: the caller's contract for `alloc` is passed on unchanged.
            let p = unsafe { System.alloc(layout) };
            if !p.is_null() {
                LIVE.fetch_add(layout.size(), Ordering::Relaxed);
            }
            p
        }

        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            // SAFETY: as for `alloc`.
            let p = unsafe { System.alloc_zeroed(layout) };
            if !p.is_null() {
                LIVE.fetch_add(layout.size(), Ordering::Relaxed);
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
                LIVE.fetch_add(new_size, Ordering::Relaxed);
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
    for (name, over) in [
        ("compressed", excess(&compressed)),
        ("stream", excess(&stream)),
    ] {
        assert!(
            over <= SLACK,
            "{name}: {over} B of heap above the budget's use"
        );
    }
}
