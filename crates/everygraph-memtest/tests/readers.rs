// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The heap that the readers of the edge-map framework hold, against what they reserve:
//! while a body runs, the bytes in use above those before the plan never exceed the
//! budget's use by more than a fixed slack (the plan's small uncounted vectors). Lists
//! that grow one entry at a time, longer than the batch, and a frontier of more lists
//! than a block are the cases where an amortized buffer would hold more than it reserved.
//!
//! Under Miri (CONTRIBUTING.md, "`unsafe`") it runs the framework's `unsafe` code and the
//! counting allocator's on threads.

#![forbid(unsafe_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use everygraph_core::{Budget, Memory, MemorySource};
use everygraph_format::adjacency::{Adjacency, AdjacencyIndex, encode};
use everygraph_kernels::edge_map::{Compressed, Node, Plan, Settings, Source, Stream, all};
use everygraph_memtest::counting::{self, Counting};
use everygraph_memtest::probe::alone;

#[global_allocator]
static HEAP: Counting = Counting;

/// Bytes the plan holds outside the budget here: its task list, pool and frontier
/// results, for a graph of a few thousand nodes.
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

/// On a pool whose threads have all started ([`alone`]): threads that were still
/// starting allocated their state while the first source was measured, up to 139 KB
/// over the slack in 7 of 72 runs with 64 threads on 32 cores (issue #230).
#[test]
fn readers_hold_no_more_heap_than_they_reserve() {
    alone(|| {
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
    });
}
