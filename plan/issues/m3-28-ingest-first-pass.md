---
title: "Ingest: reserve the first pass's text blocks and the spill read-back, and refuse below the estimate before any work"
milestone: M3 · Engine and kernels
labels: type:feature, area:ingest, P1, risk:medium, size:S
---
**Why.** The no-OOM suite ({{#m3-10-no-oom-ci}}, `crates/everygraph-memtest`) found three gaps in the build of {{#m2-08-csr-build}}, the first two in its first pass, the identifier dictionary's read of the text:

1. Each text block is parsed into a batch of rows (`TextReader::read_block`, about 24 bytes per row, `BLOCK_BYTES_PER_ROW`) that no reservation holds. The later passes reserve these batches as "text blocks in flight" (`build/rows.rs`, `Flight`); the first pass does not, because the largest block's row count is not known yet. Untracked heap by the counting allocator, `everygraph ingest` on generated graphs of 2^12, 2^14 and 2^16 nodes (4 threads): 1.0, 1.5 and 6.3 MB for the random family, 9.3 MB at 2^16 for sparse text ids (88 and 131 bytes per node over those sizes; `cargo test -p everygraph-memtest --test no_oom -- --nocapture`, exploratory). Below 4 MiB of text that is one block; above it, up to one block per thread at once, so the bound is threads × (2 × 4 MiB + rows per block × 24 B), about 33 MB per thread for the shortest rows: outside the budget. The control group sees it too: for sparse text ids at 2^20 nodes, a budget of 151,947,264 bytes and 4 threads, the anonymous peak was 35,974,826 bytes above the ledger's peak (record `ledger_peak=151945558 anon_before=77660160 anon_peak=265580544`, `cargo test --release -p everygraph-memtest --test cgroup -- --ignored`, one exploratory run on the lab machine, PR #251).
2. `build::estimate` says that a budget below its `smallest_budget` "is refused before any work". The build does not check it: at half of it, the build reads the whole input for the dictionary and then refuses at "text blocks in flight" (random family, 2^12 nodes: estimate 39.9 MB, true smallest budget 38.5 MB, the refusal after 1.9 MB of heap; `every_budget_completes_or_refuses_cleanly`).
3. Reading spilled rounds back holds untracked heap in the phase after the reservation "spill reader" (`build/fill.rs`, `load_round`), at a budget tight enough to spill: 9,756, 265,920 bytes at 2^16 and 2^18 nodes, random family, 1.05 × the estimate (release build, `untracked_heap_does_not_grow_at_nightly_sizes`, exploratory, PR #251). Two sizes do not say whether it grows with the nodes or with the rounds; find what holds it.

**Scope**
- [ ] The first pass reserves its blocks in flight before it reads: from the sample's rows per block, grown (or refused) when a block holds more rows than reserved, before its batch is kept
- [ ] The build compares the budget with `estimate(…).smallest_budget` before its first read, and refuses below it with `InsufficientBudget`, as the estimate's documentation says
- [ ] The rounds read back from spill files hold no heap that no reservation covers
- [ ] Remove `Op::Ingest` from `KNOWN` in `crates/everygraph-memtest/src/ops.rs`; the slope test, the refusal check and the cgroup gap then cover the build like every other operation

**Done when.** `cargo test -p everygraph-memtest --test no_oom` passes with `KNOWN` empty.

**Depends on** {{#m3-10-no-oom-ci}}
