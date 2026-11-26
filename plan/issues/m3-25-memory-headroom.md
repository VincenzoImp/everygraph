---
title: Memory promise holds for what the user types: headroom, non-Linux probe, true advice
milestone: M3 · Engine and kernels
labels: type:feature, area:engine, P0, risk:medium, size:M
---
**Why.** The state-of-the-art review of 2026-11-26 (`docs/research/sota-review-2026-11.md` § 1, rows 1–4 and 11) found three gaps in promise 2 ("never run out of memory", `docs/DESIGN.md` § 2):
- The command line gives `Budget` the whole `--memory` and accepts a budget equal to the cgroup limit (`crates/everygraph-cli/src/machine.rs` l. 59–66). The no-OOM suite tests `MemoryMax = budget + 96 MiB` instead (PR #251). Memory outside the ledger was measured at up to 9 MB, and 34 MB for ingest (PR #251). Page tables, thread stacks and the mapped regime's page cache also need room.
- Without a memory probe (every platform but Linux), the default budget is a fixed 8 GB, which is all the RAM of an 8 GB laptop.
- One `MADV_WILLNEED` call reads at most `max(io_pages, ra_pages)`, 128 KiB on this NVMe (Linux 6.12 `mm/readahead.c`). On a cold 64 MiB file, one call left 131,072 bytes resident; 128 KiB pieces made all 64 MiB resident (`/home/tux/graphlab/sota-2026-11-26/synthesis-check/willneed.py`). The planner still advises `WillNeed` on whole ranges and prints "warmed ahead".

**Scope**
- [ ] A stated headroom inside the budget: the accountant gets `budget − headroom`. The headroom is written in `docs/ux-contract.md`, with its parts measured.
- [ ] The no-OOM suite runs at `MemoryMax = budget`, with no slack (this is the review's experiment F1). Zero `oom_kill` on the nightly matrix.
- [ ] A memory probe on macOS (`host_statistics64`) and Windows (`GlobalMemoryStatusEx`). Without one, the default budget is a stated fraction of total RAM, not a fixed number.
- [ ] Advice that does what the plan says: `WillNeed` in pieces no larger than the device's read-ahead limit, or `MADV_POPULATE_READ` where the kernel has it, chosen by an interleaved A/B of the cold first pass on orkut and Friendster. The plan text says "warmed ahead" only when it is true.
- [ ] The planner's storage constant is the measured device read rate (7.07 GB/s, `bench/results/io/20261125T205022Z-21498df39590.jsonl`), not a PageRank throughput labelled as one.

**Done when.** The suite passes at the shipped configuration on Linux, and a cold first pass under advice reads ahead what the plan says it does.

**Depends on** {{#m3-10-no-oom-ci}}, {{#m3-02-planner}}
