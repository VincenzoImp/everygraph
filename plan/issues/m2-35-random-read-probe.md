---
title: Experiment: random-read rate of the drive by size and queue depth
milestone: M2 · Format and ingest
labels: type:experiment, area:format, P1, risk:low, size:S
---
**Why.** Reads are synchronous today, so the queue depth is the thread count. The block cache ({{#m2-23-block-cache}}), frontiers and Leiden's level 0 read 4–64 KiB pieces at random. The state-of-the-art review of 2026-11-26 (`/home/tux/graphlab/sota-2026-11-26/synthesis.md` S5, experiment F5; `ooc.md`, `gpu-resources.md`) proposes a `pread` pool first, with io_uring only on evidence. io_uring is blocked by Docker's default seccomp profile and on several platforms.

**Method**
- [ ] On the Friendster store, with a quiet disk: random reads of 4, 16 and 64 KiB at queue depths 1–256. Compare a `pread` thread pool with io_uring (the raw `io-uring` crate), both buffered and `O_DIRECT`.
- [ ] Record IOPS and bytes per second, and their spread over three repetitions.

**Done when.** The queue depth the block cache needs is fixed. io_uring is kept as an optional Linux backend only if the `pread` pool stays below 90% of its IOPS.
