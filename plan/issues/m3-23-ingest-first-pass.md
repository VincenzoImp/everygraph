---
title: "Ingest: reserve the first pass's text blocks, and refuse below the estimate before any work"
milestone: M3 · Engine and kernels
labels: type:feature, area:ingest, P1, risk:medium, size:S
---
**Why.** The no-OOM suite ({{#m3-10-no-oom-ci}}, `crates/everygraph-memtest`) found two gaps in the build of {{#m2-08-csr-build}}, both in its first pass, the identifier dictionary's read of the text:

1. Each text block is parsed into a batch of rows (`TextReader::read_block`, about 24 bytes per row, `BLOCK_BYTES_PER_ROW`) that no reservation holds. The later passes reserve these batches as "text blocks in flight" (`build/rows.rs`, `Flight`); the first pass does not, because the largest block's row count is not known yet. Untracked heap by the counting allocator, `everygraph ingest` on generated graphs of 2^12, 2^14 and 2^16 nodes (4 threads): 1.0, 1.5 and 6.3 MB for the random family, 9.3 MB at 2^16 for sparse text ids (88 and 131 bytes per node over those sizes; `cargo test -p everygraph-memtest --test no_oom -- --nocapture`, exploratory). Below 4 MiB of text that is one block; above it, up to one block per thread at once, so the bound is threads × (2 × 4 MiB + rows per block × 24 B), about 33 MB per thread for the shortest rows: outside the budget.
2. `build::estimate` says that a budget below its `smallest_budget` "is refused before any work". The build does not check it: at half of it, the build reads the whole input for the dictionary and then refuses at "text blocks in flight" (random family, 2^12 nodes: estimate 39.9 MB, true smallest budget 38.5 MB, the refusal after 1.9 MB of heap; `every_budget_completes_or_refuses_cleanly`).

**Scope**
- [ ] The first pass reserves its blocks in flight before it reads: from the sample's rows per block, grown (or refused) when a block holds more rows than reserved, before its batch is kept
- [ ] The build compares the budget with `estimate(…).smallest_budget` before its first read, and refuses below it with `InsufficientBudget`, as the estimate's documentation says
- [ ] Remove `Op::Ingest` from `KNOWN` in `crates/everygraph-memtest/tests/no_oom.rs`; the slope test and the refusal check then cover the build like every other operation

**Done when.** `cargo test -p everygraph-memtest --test no_oom` passes with `KNOWN` empty.

**Depends on** {{#m3-10-no-oom-ci}}
