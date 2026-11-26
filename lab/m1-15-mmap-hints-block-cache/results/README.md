# m1-15: mmap hints and a budgeted block cache (#172)

**Status: exploratory.** All runs are on a shared machine. Other agents were compiling.
The 1-minute load was 14 to 34. No absolute time here may be quoted. The decisions come
from interleaved A/B runs: the variants alternate within one harness run (A, B, A, B, ...).
Bytes read do not depend on load. They are the process's own `read_bytes` from
`/proc/self/io`, and page-fault reads of mapped files are included. The cgroup's
`io.stat` is not available here: systemd does not delegate the io controller to user
scopes (`bench/README.md`). The official commands are at the end.

## Files

| File | What |
|---|---|
| `layout_16g.jsonl`, `layout_32g.jsonl` | harness rows: gmap layout, no madvise against MADV_RANDOM |
| `leiden_ooc.jsonl` | harness rows: M1-02 out-of-core Leiden, no madvise against MADV_RANDOM for the whole run |
| `leiden_cache.jsonl`, `leiden_inram.jsonl` | harness rows: step 2, mmap against per-phase MADV_RANDOM against the block cache; the in-RAM reference |
| `pq_cold.jsonl` | harness rows: cold point queries, MADV_NORMAL against MADV_RANDOM |
| `hier.json`, `hier_b2.json` | the Friendster hierarchy the layout read (b2 Louvain, map-shaped) |
| `../cgprobe.py` | runs inside the scope; records `memory.stat` (kernel terms, refaults, faults) and the command's bytes read |
| `../summarize.py` | prints the rows and the per-pass B/A ratios |
| `bench/specs/mmap-hints.toml` | the spec of every row |

Code used: `lab/e-map/gmap` (`GMAP_MADV`), `lab/f1-query` `pq` (bytes per query),
`lab/b2-leiden` (`--no-quot`), `lab/m1-02-ooc-leiden` (`--advise random-lm`,
`--cache-gb`, `src/bcache.rs`, per-pass `lm_ref_read_self_bytes`).

## Machine and common conditions

- Ryzen 9 7950X, 61.9 GiB, Samsung 990 PRO, ext4, Linux 6.12.116, MGLRU on
  (`lru_gen/enabled` 0x0007), `read_ahead_kb` 128, THP `always`.
- 16 threads. Every run is under `systemd-run --user --scope -p MemoryMax=<limit>
  -p MemorySwapMax=0`. The harness reads a cold private copy of the dataset, evicted and
  checked before each run.
- Exploratory rows. `git.dirty` is true in them: the branch was being written while they ran.

## Step 1: MADV_RANDOM on the layout (gmap, Friendster, 16G)

Conditions: gmap layout as in the tracer (FINDINGS § M1-01), CSR (glab-csr-v0,
uncompressed, 15.5 GB) memory-mapped, `--phi 0.72`, f32 positions, no fsync. The time
includes loading. The hierarchy is b2 Louvain, async, cap 64, min ratio 3, 16 threads
(levels 10,515,294 / 1,332,781 / 241,773 / 69,315 / 14,375 / 61, Q 0.5792). It is close
to the tracer's hierarchy but not the same one.

| pass | advice | seconds | bytes read | refault_file | major faults | load at start |
|---|---|---|---|---|---|---|
| 1 | none | 718.5 | 651.6 GB | 155.3 M | 21.6 M | 13.8 |
| 1 | random | 321.6 | 103.7 GB | 21.6 M | 25.2 M | 23.2 |
| 2 | none | 671.9 | 612.6 GB | 145.8 M | 20.6 M | 31.3 |
| 2 | random | 217.8 | 100.0 GB | 20.7 M | 24.4 M | 22.3 |
| 3 | none | 376.5 | 578.4 GB | 137.5 M | 19.3 M | 17.9 |
| 3 | random | 245.5 | 105.5 GB | 22.0 M | 25.7 M | 21.6 |

Median of the three per-pass ratios (random / none): bytes read **0.163**, time **0.448**
(0.324–0.652), `workingset_refault_file` 0.142, `workingset_activate_file` 0.176. In all six
runs, `pos.f32` and `tnodes.bin` have the same SHA-256 (`c01342d5c5d2…`, `bd103a60dec6…`).

- The default reproduces the tracer: 578–652 GB here, 733 GB in the tracer at 32 threads.
- With no advice, each major fault reads about 30 KB on average (read-around). With
  MADV_RANDOM, each fault reads 4 KiB: bytes / faults = 4.1 KB.
- MADV_RANDOM still reads 6.5–6.8× the CSR. The first pass over the graph (depth 0) is
  slower with it (24.5 s against 15.9 s in pass 1): there is no read-around on first touch.
- **The issue's step-1 success criterion is met:** the bytes read under 16 GiB fall by
  more than half (to 0.16×).

**The same layout under 32G (the graph fits), one pass:**

| advice | seconds | bytes read | refault_file | kswapd / direct scans | memory.max events |
|---|---|---|---|---|---|
| none | 96.9 | 15.3 GB | 0 | 0 / 0 | 0 |
| random | 281.4 | 133.6 GB | 28.9 M | 20.0 M / 9.9 M | 0 |

With MADV_RANDOM, the layout is 2.9× slower even though its cgroup never reaches its limit.
The page cache of the whole machine was full (60 GB cached: other agents and the harness's
copies). Global reclaim (kswapd, and direct reclaim from global pressure) took 29.9 M pages
from this cgroup. With no advice it took none. This is the caveat in the issue, measured:
the kernel gives a MADV_RANDOM mapping's referenced pages no protection in reclaim. The
statement that this comes from the 6.3+ reclaim change is from the issue and the kernel
history; the kernel source was not read here. The run was not repeated, and it was not
run on an idle machine.

So MADV_RANDOM helps when the stage's working set is larger than its limit. It hurts when
the working set fits but the machine is under memory pressure from others.

## Step 1: MADV_RANDOM on out-of-core Leiden (M1-02, Friendster, 8G and 4G)

Conditions: as in FINDINGS § M1-02. Async Leiden, one iteration, f64 weights (integer),
compressed level 0 (D4 store) memory-mapped from a cold copy, sequential aggregation,
spilled quotient graphs. The time is the kernel only. "LM+ref bytes" is the level-0 local
moving and refinement (`passes[0].lm_ref_read_self_bytes`). MADV_RANDOM is on the level-0
map for the whole run.

| limit | pass | advice | seconds | level-0 LM | level-0 aggregation | LM+ref bytes | all bytes | Q |
|---|---|---|---|---|---|---|---|---|
| 8G | 1 | none | 202.9 | 112.1 | 40.3 | 375.2 GB | 459.3 GB | 0.6078 |
| 8G | 1 | random | 197.9 | 81.5 | 65.7 | 56.5 GB | 136.6 GB | 0.6055 |
| 8G | 2 | none | 201.8 | 104.0 | 45.2 | 375.2 GB | 460.1 GB | 0.6063 |
| 8G | 2 | random | 231.8 | 102.4 | 68.8 | 57.5 GB | 139.8 GB | 0.6007 |
| 4G | 1 | none | 307.2 | 169.1 | 46.4 | 799.4 GB | 1009.3 GB | 0.6057 |
| 4G | 1 | random | 281.7 | 106.6 | 70.7 | 63.0 GB | 364.2 GB | 0.6064 |
| 4G | 2 | none | 340.3 | 175.0 | 60.1 | 799.5 GB | 1056.2 GB | 0.6055 |
| 4G | 2 | random | 300.9 | 108.1 | 81.2 | 63.0 GB | 321.7 GB | 0.6048 |

- The level-0 LM+ref bytes reproduce M1-02: 375 GB here against 406 GB at 8G; 799 GB
  against 799 GB at 4G. MADV_RANDOM cuts them to **0.152× at 8G and 0.079× at 4G**.
- Level-0 local moving is 0.86× (8G) and 0.62× (4G) as long. The level-0 aggregation, a
  sequential scan, is 1.58× and 1.44× as long, because MADV_RANDOM also turns off its
  readahead. The total is 1.06× and 0.90×. So whole-run MADV_RANDOM is no fix for Leiden.
  Step 2 therefore compares per-phase advice (`random-lm`).
- The process's own bytes and the machine-wide `/proc/diskstats` figure that M1-02 used
  agree to within 0.1 GB in 7 of 8 rows. In one row (8G, pass 2, random) the machine
  figure is 17.7 GB higher: other processes' traffic. The comment in `leiden.rs` that
  `/proc/self/io` does not count page faults on a mapped file is wrong. The rows show that
  it does.

## Step 1: cold point queries (lab/f1-query `pq`, Friendster, 8G)

Every query runs on a just-evicted mapping, single thread. The throughput batch runs at
16 threads on an initially cold cache. Bytes are storage bytes read per query. Two passes
gave identical bytes.

| query | no prefetch: normal | no prefetch: random | prefetch: normal | prefetch: random |
|---|---|---|---|---|
| degree | 128.0 KiB | 4.0 KiB | 128.0 KiB | 4.0 KiB |
| neighbors | 256.0 KiB | 8.2 KiB | 256.0 KiB | 8.2 KiB |
| common neighbors | 499 KiB | 16.9 KiB | 499 KiB | 16.9 KiB |
| 2-hop size | 22.9 MiB | 1.10 MiB | 1.38 MiB | 1.10 MiB |
| 3-hop, budget 10k | 9.0 MiB | 0.34 MiB | 9.4 MiB | 9.1 MiB |
| bidirectional BFS path | 388 MiB | 33.5 MiB | 293 MiB | 33.5 MiB |
| neighbors p50 | 3.3–3.4 ms | 161–163 µs | 370–381 µs | 169–192 µs |
| neighbors, cold throughput (16 threads) | 16–21 k q/s | 59–98 k q/s | 19–21 k q/s | 59–69 k q/s |

Without prefetch, MADV_RANDOM cuts bytes per query about 30× for degree, neighbors and
common neighbors, 21× for 2-hop, 26× for 3-hop and 12× for the BFS path. The frontier prefetch
(`MADV_WILLNEED` on exact ranges) already avoids most of the read-around for 2-hop and
3-hop. FINDINGS § F1's cold "neighbors 0.1–0.25 ms" is the MADV_RANDOM figure.

## Step 2: a budgeted block cache (lab/m1-02-ooc-leiden/src/bcache.rs)

The prototype is `bcache.rs` (282 lines with its test) plus about 70 lines in `store.rs`,
`lutil.rs` and `leiden.rs`. `bcache.rs` has no `unsafe`. The one new raw-pointer use is the
existing decoder, called on the cache's copy of a record.
- Fixed frames of 4 or 16 KiB in anonymous memory, reserved in the accountant. CLOCK
  eviction per shard (up to 4,096 shards; 16 neighbouring frames share a shard).
- Misses are `pread` with `O_DIRECT`, so there is no page cache and no double buffering.
- `Comp::deg` and `Comp::nbrs` read records through the cache. The cache is released after
  level-0 refinement, and the aggregation scan reads the map.
- With `--cache-prefetch`, local moving announces each 64-node block. The block's missing
  frames are then read with one `pread` per run of neighbouring frames (the coalesced
  request).
- Size: 3.0 GiB at 8G and 0.75 GiB at 4G, about what the page cache holds during local
  moving in the mmap runs.

Conditions are those of the Leiden table above. Run A and run B are two harness runs; within
each, all variants of a pass alternate. Run R is the same algorithm in RAM (48G, warm,
nothing spilled), in the same session.

PF_TABLE

Medians of the per-pass ratios against `none` in the same run:

PF_RATIOS

What this shows:
- **Bytes.** Every non-default variant reads about the bytes that are needed. The 4 KiB
  cache reads 62.5 GB at both limits, against 375 GB (8G) and 799–933 GB (4G) for mmap.
  Per-phase MADV_RANDOM reads 56–63 GB. 16 KiB frames read 2.6× more than 4 KiB frames.
- **Cache size hardly matters.** The 0.75 GiB cache at 4G misses as often as the 3 GiB
  cache at 8G (10.56 M against 10.49 M misses with 16 KiB frames). Local moving visits
  the blocks in a new permuted order each sweep, so a sweep reuses almost nothing from the
  last one. The hit rate (98%) comes from neighbouring lists within a frame.
- **Time follows the number of synchronous reads, not the bytes.** 17.8 M misses of 4 KiB,
  10.5 M of 16 KiB and 5.5 M coalesced reads give local moving times in that order. The
  kernel's read-around reads 375 GB at about 4 GB/s and is not slower than the
  uncoalesced cache when the machine is quieter (run A, pass 2).
- **The issue's step-2 criterion is not met.** Level-0 local moving at 8G with the best
  variant (coalesced cache) takes 80.9–91.3 s against 42.0 s in RAM in the same session:
  1.9–2.2×, not ≤ 1.5×. Each block's read is still synchronous. Overlapping it with the
  compute of the previous block (read-ahead of the next block, or io_uring) is the missing
  piece. This is an inference from the miss counts and the times; it was not tested.
- In both runs and at both limits, Q is 0.598–0.607 for every variant, as in M1-02's async
  range.

## Kernel memory charged to the cgroup

Peaks are sampled every 100 ms from the scope's `memory.stat`.

| run | pagetables | kernel | shmem |
|---|---|---|---|
| layout 16G, either advice | 36 MiB | 125 MiB | 0 |
| layout 32G, either advice | 36 MiB | 125 MiB | 0 |
| Leiden 8G, none / random | 45 MiB | 141–143 / 176–186 MiB | 0 |
| Leiden 4G, none / random | 39–40 MiB | 126–129 / 144 MiB | 0 |
| Leiden 8G, block cache | 42–45 MiB | 140–150 MiB | 0 |
| Leiden 4G, block cache | 40 MiB | 125–126 MiB | 0 |

The kernel terms are small against the limits (at most 186 MiB of `kernel` at 8G, 2.3%).
MADV_RANDOM raises `kernel` by 10–30% in Leiden. The block cache does not change it.

## Not done, and limits

- No official run. All times are from a loaded machine and only their ratios are used.
  The time ratios vary from pass to pass by up to ±30%. The byte ratios vary by less than 3%.
- One graph (Friendster), one layout hierarchy, async Leiden only. The cache's identity is
  checked in deterministic mode on lj only (below), not on Friendster.
- The 32G MADV_RANDOM result is one run under global memory pressure. On an idle machine
  with free memory it may not happen.
- The cache's reads are synchronous: a thread waits for its block, then computes. There is
  no read-ahead of the next block, no batching across blocks and no io_uring. The
  refinement loop and deterministic mode do not announce blocks (no coalescing there).
- The kernel source behind "MADV_RANDOM pages lose reclaim protection" was not read.
- Run A's 8G rows used the m102 binary before the coalescing code. Its 4G rows and run B
  used the binary with it. Without `--cache-prefetch` the new code does nothing.

**Identity of the cache path** (deterministic Leiden on lj under 2G, `--det`): no cache,
a 22 MB O_DIRECT cache with 4 KiB frames, and a 55 MB buffered cache with 16 KiB frames
give the same `hash_levels` 44ae14c7774277eb (the M1-02 value) and Q 0.747972. The
per-phase advice gives the same hash too. Unit test `bcache::tests::reads_equal_the_file_under_eviction`.

## Recommendation

1. **Use MADV_RANDOM now, per phase, in m2-02's `advise()` callers and the m3-02
   planner.** Use it only for a random-access phase whose working set is larger than its
   budget, and go back to MADV_NORMAL or MADV_SEQUENTIAL for scans. Measured here:
   - layout under 16G: bytes 0.16×, time 0.45×;
   - Leiden level-0 local moving and refinement: bytes 0.15× (8G) and 0.07–0.08× (4G);
   - cold point queries: about 30× fewer bytes per single-list query.

   For the whole Leiden run it is wrong: the aggregation scan becomes 1.4–1.6× slower.
2. **Do not set MADV_RANDOM when the working set fits.** At 32G the layout became 2.9×
   slower and read 8.7× more, because global reclaim took the unprotected pages. So the
   planner needs the working-set estimate it already plans for (D11: the regime is chosen
   from sizes).
3. **D11 regime 2 needs an ADR: "an explicit, budgeted cache for random access", with
   mmap kept for warm data and scans.** The reasons are measured, not taken from the papers:
   - Neither kernel policy is fit for a budget. With the default advice, the layout
     reads 37–42× its CSR (578–652 GB for 15.5 GB), and Leiden's level-0 local moving and
     refinement read 36–90× the store (375–933 GB for 10.4 GB). MADV_RANDOM loses its
     pages to any global reclaim.
   - The explicit cache gets the exact bytes (62.5 GB, like MADV_RANDOM). Its memory is in
     the accountant, global reclaim cannot take it (anonymous, no swap), and its size can
     be small (0.75 GiB did as well as 3 GiB here).
   - The ADR must also say what the prototype did not reach. On time it is at best 0.53×
     the mmap default and 0.76–0.88× per-phase MADV_RANDOM in local moving (8G, run B).
     It is 1.9–2.2× the in-RAM time, so the issue's 1.5× criterion is not met.
     m2-23-block-cache should therefore be built with asynchronous read-ahead of the next
     blocks (or io_uring) from the start. Its acceptance test should be level-0 local
     moving at 8 GiB within 1.5× of in RAM, measured as here.
4. **The larger lever is the access order, not the pager.** Every variant still reads the
   whole store once per sweep: about 62 GB, six reads of the 10.4 GB store (five sweeps
   and the refinement). Only an order
   that reuses data across sweeps reads less: storage-order visits (M1-02's L1, under study
   in m1-14) or, for the layout, a store renumbered by hierarchy
   (`docs/research/memory-bounded-algorithms.md` addendum § E.2).

## Official commands

On an idle machine, from a clean checkout of this branch:

```sh
export EVERYGRAPH_DATA=/home/tux/graphlab/data
cargo build --release -p everygraph-bench
./target/release/everygraph-bench run mmap-hints --official --only layout-friendster-16g
./target/release/everygraph-bench run mmap-hints --official --only layout-friendster-32g
./target/release/everygraph-bench run mmap-hints --official --only leiden-friendster-ooc
./target/release/everygraph-bench run mmap-hints --official --only leiden-friendster-cache
./target/release/everygraph-bench run mmap-hints --official --only leiden-friendster-inram
./target/release/everygraph-bench run mmap-hints --official --only pq-friendster-cold
python3 -I lab/m1-15-mmap-hints-block-cache/summarize.py bench/results/mmap-hints/*.jsonl
```

The first run also builds the hierarchy (prepare step, about 2.5 minutes under 40G).
Wall time (estimate from these runs, idle machine faster): layout 16G about 40 min,
32G about 7 min, Leiden ooc about 40 min, Leiden cache about 60 min, in RAM about 4 min,
point queries about 5 min. Disk: about 52 GB of copies, plus up to 40 GB of spill per
running case. The gate needs 44 GiB available.
