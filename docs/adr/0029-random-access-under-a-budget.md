# ADR 0029 — Random access under a budget: an explicit cache, not plain mmap

- **Status:** accepted
- **Accepted:** 2026-11-26, on the load-independent byte counts of m1-15; times stay exploratory until the M3 milestone window (CLAUDE.md § 6).
- **Decision:** D11 in [DESIGN.md](../DESIGN.md#4-decisions) (amends ADR 0011: regime 2, "mmap under pressure", for random access)
- **Date:** 2026-11-26

## Context

D11 has three regimes: mmap warm, mmap under memory pressure, and semi-external streams
(ADR 0011). Regime 2 lets the kernel's page cache decide what stays in memory. For scans
this works: Friendster PageRank runs within about 10% of in RAM (ADR 0011). For random
access it does not. The tracer's layout re-read 733 GB of a 15.5 GB graph under 16 GiB
(FINDINGS § M1-01), and level-0 Leiden re-reads the store 6.0–12.8× per sweep below
16 GiB (FINDINGS § M1-02).

The kernel source explains why (`docs/research/sota-io-memory.md` § 3, D11, read by a
helper). A miss in a `MADV_NORMAL` mapping reads 128 KiB around the page. The heuristic
that stops this needs more than 100 net misses, and every hit cancels one, so with most
of the file cached it never stops. Under `memory.max` the extra pages push out pages that
are still needed.

The research proposed two fixes: a budgeted block cache over `ByteSource` (C1) and better
mmap hints now (C5). Experiment m1-15 (#172, its lab pull request merged on 2026-11-26) tested both on Friendster.
This ADR records what it found and what we do now. Every run of m1-15 was on a shared
machine. The decision rests on the bytes read, which do not depend on the load; the times
are exploratory. The official `mmap-hints` run (the commands at the end of
`lab/m1-15-mmap-hints-block-cache/results/README.md`) runs in the M3 milestone window
(CLAUDE.md § 6) and gives the times that documents may quote.

## Decision

**Regime 2 for random access whose working set exceeds the budget is an explicit,
budgeted cache over `ByteSource`, not plain mmap.** The cache holds fixed-size frames in
anonymous memory reserved from `Budget`, evicts with CLOCK, and reads misses with
`pread` (`O_DIRECT` where the backend has it). Its memory is in the ledger, and global
reclaim cannot take it. mmap stays for warm data (regime 1) and for scans, including
scans under pressure.

**Until the cache exists (m2-23, #178), a random-access phase whose working set exceeds
its budget uses `MADV_RANDOM`.** The advice is set per phase, not per run, and is reset to
`MADV_NORMAL` before a scan phase. It is **never** set when the working set fits the
budget. `MADV_SEQUENTIAL` was not tested and is not part of this decision.

The choice between the two needs a working-set estimate per phase. That is the size-based
planning that D11 already requires.

## Evidence

All from `lab/m1-15-mmap-hints-block-cache/results/README.md` and its row files
(`layout_16g.jsonl`, `layout_32g.jsonl`, `leiden_ooc.jsonl`, `leiden_cache.jsonl`,
`leiden_cache_pf.jsonl`, `leiden_inram.jsonl`, `pq_cold.jsonl`). Friendster, 16 threads,
each run under `systemd-run --user --scope -p MemoryMax=<limit> -p MemorySwapMax=0`, on a
cold private copy of the data. Variants alternate within one harness run (A, B, A, B).

How far each kind of number can be trusted:

- **Bytes read are exploratory but independent of load.** They are the process's own
  `read_bytes` from `/proc/self/io`, which includes page-fault reads of mapped files. In 7
  of 8 Leiden rows they agree with the machine-wide `/proc/diskstats` figure to within
  0.1 GB. Within a variant and a limit, they vary by less than 3% from pass to pass.
- **Times are exploratory and are not to be quoted as absolute numbers.** The machine was
  shared: other agents were compiling, and the 1-minute load was 14 to 34. Only ratios
  within one interleaved run are used here, and even those vary widely (one from 0.16 to
  1.05). The official run in the milestone window replaces them.

**1. Plain mmap reads far more than needed under a budget.** Bytes, exploratory:

- Layout (lab `gmap`, uncompressed CSR of 15.5 GB, f32 positions) under 16G, default
  advice: 578–652 GB per run, 37–42× the CSR (`layout_16g.jsonl`). The tracer read 733 GB
  at 32 threads.
- Leiden (M1-02 lab code, async, one iteration, f64 weights, compressed store of
  10.4 GB) level-0 local moving and refinement, default advice: 375 GB at 8G and
  799–933 GB at 4G, 36–90× the store (`leiden_ooc.jsonl`, `leiden_cache.jsonl`).

**2. `MADV_RANDOM` cuts the bytes when the working set exceeds the budget.**

- Layout under 16G: bytes 0.163× (median of three per-pass ratios; 100.0–105.5 GB against
  578.4–651.6 GB). Time 0.448× (0.324–0.652), exploratory. The positions and tile nodes
  have the same SHA-256 in all six runs (`layout_16g.jsonl`).
- Leiden level-0 local moving and refinement, advice on that phase only (`random-lm`):
  bytes 0.152–0.154× at 8G (55.8–58.5 GB) and 0.073–0.088× at 4G (62.9–73.4 GB)
  (`leiden_cache.jsonl`, `leiden_cache_pf.jsonl`). Q 0.598–0.607 for every variant.
- Cold point queries (lab `f1-query` `pq`, single thread, 8G): about 30× fewer bytes per
  degree, neighbours or common-neighbours query, 12× for a bidirectional BFS path
  (`pq_cold.jsonl`).

**3. `MADV_RANDOM` must be per phase.** With the advice on the whole Leiden run, the
level-0 aggregation, which is a scan, took 1.58× (8G) and 1.44× (4G) as long, because
the advice also turns off its readahead. The whole run was 1.06× and 0.90× as long: no
fix (`leiden_ooc.jsonl`; times exploratory).

**4. `MADV_RANDOM` hurts when the working set fits.** The same layout under 32G, where the
CSR fits, one pass each (`layout_32g.jsonl`):

| advice | seconds (exploratory) | bytes read | refault_file | pages taken by global reclaim |
|---|---|---|---|---|
| none | 96.9 | 15.3 GB | 0 | 0 |
| random | 281.4 | 133.6 GB | 28.9 M | 29.9 M (kswapd 20.0 M, direct 9.9 M) |

With the advice the layout was 2.9× slower and read 8.7× more, though its cgroup never
reached its limit (0 `memory.max` events). The whole machine's page cache was full, and
global reclaim took the advised mapping's pages, which have no protection from their
accessed bits. That this comes from a reclaim change in Linux 6.3 is from the issue and
the kernel history; the kernel source was not read. One run, not repeated, not idle. On
an idle machine with free memory it may not happen. The rule "never when it fits" holds
either way: when the working set fits, the advice can only lose the read-around.

**5. The prototype cache meets the byte target, not the time target.** The prototype is
`lab/m1-02-ooc-leiden/src/bcache.rs` (282 lines with its test, no `unsafe`): 4 or 16 KiB
frames, CLOCK per shard, `O_DIRECT` misses, 3.0 GiB at 8G and 0.75 GiB at 4G. With
`--cache-prefetch` the missing frames of each 64-node block are read with one `pread` per
run of neighbouring frames.

- Bytes, level-0 local moving and refinement: 62.5 GB at 8G and 87.3 GB at 4G with 4 KiB
  frames, against 375 GB and 799–933 GB for plain mmap; close to per-phase `MADV_RANDOM`.
  16 KiB frames read 161.5–162.6 GB.
- Cache size matters little: with 16 KiB frames the 0.75 GiB cache missed as often as the
  3 GiB one (10.56 M against 10.49 M misses). Local moving visits blocks in a new order
  each sweep, so one sweep reuses little of the last.
- Identity: deterministic Leiden on lj under 2G gives the same `hash_levels`
  (44ae14c7774277eb) and Q 0.747972 with no cache, a 4 KiB `O_DIRECT` cache and a 16 KiB
  buffered cache. Not checked on Friendster.
- Kernel memory charged to the cgroup: at most 186 MiB at 8G (2.3% of the limit).
  Neither the cache nor per-phase advice raises it.
- **Time, exploratory:** level-0 local moving at 8G with the best variant (coalesced 4 KiB
  cache, `leiden_cache_pf.jsonl`) took 80.9–91.3 s against 42.0 s in RAM in the same
  session (`leiden_inram.jsonl`, 48G, nothing spilled): **1.9–2.2×, not the ≤ 1.5× that
  #172 set.** Against plain mmap in the same run it was 0.53–0.85× (median 0.689); against
  per-phase `MADV_RANDOM`, 0.76–0.88×.

**Inference, not tested: synchronous reads, not bytes, set the time.** Every miss makes a
thread wait for its read and only then compute. Local moving times tend to follow the
number of synchronous reads (17.8 M misses of 4 KiB, 10.5 M of 16 KiB, 5.5 M coalesced
reads) more than the bytes. This comes from two runs at different loads, so it is not a
clean test. When the machine was quieter, the kernel's read-around (375 GB at about
4 GB/s) was no slower than the uncoalesced cache (run A, pass 2). Overlapping the next
block's reads with the current block's compute is the untested missing piece.

**Why a cache and not `MADV_RANDOM` alone,** from the measurements above: `MADV_RANDOM`
reads about the same bytes as the cache but loses its pages to any global reclaim, and
its pages are counted as file cache, not in the ledger. The cache's memory is exact, small
enough to matter little, and immune to global reclaim (anonymous, no swap). It also works
over sources that cannot be mapped (OPFS, HTTP ranges; D2).

Not measured: one graph, one layout hierarchy, async Leiden only. No official run. The
refinement loop and the deterministic mode do not announce blocks to the cache. No
asynchronous read-ahead, no batching across blocks, no io_uring.

## Consequences

- **m2-23 (#178), the block cache,** is built, with asynchronous read-ahead of the next
  blocks (a reader pool over `pread`, or io_uring) from the start, not as a later step.
  Its acceptance includes: **level-0 local moving on Friendster at 8 GiB within 1.5× of
  the same algorithm in RAM**, measured as in m1-15 (16 threads, same session).
- **Advice is chosen per phase.** The callers of `advise()` that m2-02 (#29) provided —
  the edge-map framework, out-of-core Leiden, the layout, the tile builder, point queries
  — take the advice from the plan for each phase. The planner (m3-02, #45) sets
  `MADV_RANDOM` only for a random-access phase whose working set exceeds its budget,
  resets `MADV_NORMAL` for scans, and never advises random when the working set fits.
  Once the cache exists, such a phase gets the cache instead.
- **Access order is the bigger lever.** Every variant still reads the whole store once per
  sweep: about 62 GB, six reads of the 10.4 GB store for five sweeps and refinement. Only
  an order that reuses data across sweeps reads less: storage-order visits on relabelled
  ids (m1-14, #146) for Leiden, and a store renumbered by the hierarchy for the layout
  (`docs/research/memory-bounded-algorithms.md`, addendum § E). The cache does not replace
  that work.
- The documentation must not say "mmap under pressure" for random access. Until m2-23
  lands it says: random access under a budget smaller than its working set uses
  `MADV_RANDOM`, and costs a few reads of the data per pass.
- A comment in the M1-02 lab code (`leiden.rs`) says `/proc/self/io` does not count page
  faults on a mapped file. The m1-15 rows show it does. The lab is frozen; the product's
  measurement code must not repeat the claim.

## What would change our mind

- The official `mmap-hints` run of the milestone window shows per-phase `MADV_RANDOM` does not cut the bytes read
  under 16G (layout) or 8G (Leiden local moving) by more than half, or makes local moving
  slower than the default: the interim rule is dropped, and regime 2 for random access
  waits for m2-23.
- m2-23 with asynchronous read-ahead cannot bring level-0 local moving at 8 GiB within
  1.5× of in RAM, and is not faster than per-phase `MADV_RANDOM` on the same run: the
  cache is kept only for sources that cannot be mapped, and regime 2 on local files is
  mmap with per-phase advice, by a new ADR.
- An idle-machine run of the 32G layout with `MADV_RANDOM` shows no slowdown: this does
  not change the rule (the advice has nothing to gain when the data fits), but the 2.9×
  is then reported as a case of global memory pressure only.

## Closed by

Accepted on 2026-11-26 on the bytes read of m1-15. #178 builds the block cache; its acceptance is
level-0 local moving at 8 GiB within 1.5× of in RAM. When it lands, D11's text changes
from per-phase `MADV_RANDOM` to the cache.
