---
title: Flaky tests under load: heap and retention bounds, file locks in cargo test
milestone: M3 · Engine and kernels
labels: type:infra, area:engine, P1, risk:low, size:S
---
**Why.** Test `readers_hold_no_more_heap_than_they_reserve` (from PR #202, now in `everygraph-memtest`) failed in 1 of 6 runs on an unmodified main head at load about 40, and in 3 of 13 mutant runs during the review of PR #220 (comment 7183716406). A test that fails without a code change hides real failures. Two more seen during PR #236: `everygraph-ingest` `tests/retention.rs` went about 10 MB over the ledger on a CI runner against an 8 MB limit (2.7 MB or less in three local runs); and three tests in `everygraph-io` `tests/container.rs` got `ResourceBusy` under `cargo test` (all tests in one process) and passed when rerun alone. A likely cause is that a child process spawned by another test inherits the open file and its lock until it calls `exec`.

**Status.** PR #299 fixed three other tests that failed without a code change: the Leiden fast-mode gate on `planted-20x100-80-8` (the fast mode depends on the schedule), `a_failed_warm_up_writes_its_measurement` (the OOM killer ended the measuring helper after the command, on GitHub's arm64 runners), and `a_timed_out_run_counts_the_reads_of_its_orphaned_children` (it counted other tests' reads). The method below is still open.

**Method**
- [ ] Find the source of the variance: the counting global allocator also counts allocations of other test threads in the same process (nextest runs one test per process, `cargo test` does not), rayon's pool growing lazily, or a reader pool whose size depends on timing
- [ ] Make the test measure only what it owns (a dedicated process, or allocations tagged by thread), not a looser bound
- [ ] Same for the retention bound; for the lock tests, open files close-on-exec or spawn children before files are opened, and show `cargo test` passes
- [ ] Run each 50 times under load in a loop; zero failures

**Done when.** The cause is named in the PR and the 50-run loop passes.

**Pointers.** `crates/everygraph-memtest/tests/readers.rs`; `crates/everygraph-kernels/src/edge_map/source.rs`.
