---
title: Experiment: is the frontier driver the common cause of the speed gaps?
milestone: M3 · Engine and kernels
labels: type:experiment, area:engine, P1, risk:low, size:S
---
**Why.** The state-of-the-art review of 2026-11-26 (`/home/tux/graphlab/sota-2026-11-26/synthesis.md` § 0 item 3, S3, experiment F3; `kernels.md` finding 1) read in the code that `Plan::frontier_with` cuts frontiers by 256 nodes, never splits a list, and pays a fixed cost per run. A frontier of 256 nodes or fewer runs on one thread. Parallel-KCore splits lists of 128 entries or more; PASGAL splits at 1,024 (BFS) and 10,240 (SSSP). This may explain the k-core, SSSP, BFS and Leiden gaps in {{#m3-22-performance-pass}} and m4-12. How much it weighs is a hypothesis.

**Method** (counts that do not depend on load)
- [ ] An instrumented run of k-core and SSSP on twitter-sym, orkut and Friendster. Per sub-round, record the entries, the runs, how many ran on one thread, the restarts and the draws.
- [ ] For compressed BFS and WCC: entries decoded against entries examined.
- [ ] From these counts, the share of entries that sit in single-thread runs, and the decode surplus.

**Done when.** The report says whether S3 (runs cut by entries from a degree prefix sum, hub lists split for commuting push bodies) and S10 (prefix decode, chunked hub records) are worth building: S10 if the decode surplus is at least 2×.
