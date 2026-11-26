---
title: Flaky test: readers_hold_no_more_heap_than_they_reserve under load
milestone: M3 · Engine and kernels
labels: type:infra, area:engine, P1, risk:low, size:S
---
**Why.** `everygraph-kernels` test `heap::readers_hold_no_more_heap_than_they_reserve` (from PR #202) failed in 1 of 6 runs on an unmodified main head at load about 40, and in 3 of 13 mutant runs during the review of PR #220 (comment 7183716406). A test that fails without a code change hides real failures.

**Method**
- [ ] Find the source of the variance: the counting global allocator also counts allocations of other test threads in the same process (nextest runs one test per process, `cargo test` does not), rayon's pool growing lazily, or a reader pool whose size depends on timing
- [ ] Make the test measure only what it owns (a dedicated process, or allocations tagged by thread), not a looser bound
- [ ] Run it 50 times under load in a loop; zero failures

**Done when.** The cause is named in the PR and the 50-run loop passes.

**Pointers.** `crates/everygraph-memtest/tests/readers.rs` (was `crates/everygraph-kernels/tests/heap.rs`); `crates/everygraph-kernels/src/edge_map/source.rs`.
