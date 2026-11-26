---
title: Index and .events sections in segments, for graphs past 1 B nodes
milestone: M2 · Format and ingest
labels: type:feature, area:format, P2, risk:medium, size:M
---
**Why.** The build of {{#m2-08-csr-build}} streams the records of an adjacency, but three things are still one contiguous allocation each (review of PR #215, S3):

- the index: `IndexBuilder::finish` builds both Elias–Fano sequences (the `low` and `high` vectors of `EliasFano::from_values`) and writes `AdjacencyIndex::write()` into one vector;
- the `.events` section: `encode_events` returns one vector;
- `build::check`, which reads the whole index and the whole `.events` section into memory.

By arithmetic, not measured: the index passes 2 GiB from about 1.2 B nodes (about 15 bits per node over both sequences), and `.events` from about 5.7 B entries at `l = 1`. Friendster (65.6 M nodes, 3.6 B entries) is far below both. DESIGN D2 allows no single allocation above `MAX_ALLOCATION_BYTES` (1 GiB), and wasm32 refuses one above 2 GiB.

**Scope**
- [ ] Build the Elias–Fano sequences of the index and of `.events` into segmented arrays (`SegVec<u64>`), and write them to their section piece by piece: the low part, then the high part, with no whole copy in memory.
- [ ] `check` verifies the index and `.events` through the section's `ByteSource`, in bounded pieces.
- [ ] A test with a small segment size that crosses segment ends in both sequences, byte-identical to the in-memory writer.

**Acceptance**
- No allocation of the build or of `check` is larger than `MAX_ALLOCATION_BYTES`, for any graph the format can hold (2^32 − 1 nodes). A test with a counting accountant and small segments shows it.

**Depends on** {{#m2-08-csr-build}}
