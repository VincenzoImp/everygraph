# m3-27: is the frontier driver the common cause of the speed gaps? (#266)

**Question.** `Plan::frontier_with` cuts a frontier into runs of 256 nodes and never
splits a list. A frontier of 256 nodes or fewer runs on the calling thread. Is this the
common cause of the k-core, SSSP, BFS and Leiden gaps (`docs/research/sota-review-2026-11.md`
S3, S10, F3)? Should S3 (runs cut by entries, hub lists split for commuting push bodies)
and S10 (prefix decode, chunked hub records) be built?

**Answer.** No common cause. The driver is a large cause for **k-core** on all three
graphs, and for **SSSP on twitter-sym** only. It is not a cause for SSSP on orkut and
Friendster, for BFS, or for Leiden on orkut. Compressed **BFS** decodes 8.5–20.6× the
entries it examines, so S10 passes its 2× bar. Compressed **WCC** decodes 1.9–2.0× the
prefix its sampling needs, which is just under the bar. But WCC uses only 1 or 2 entries
per list, and on Friendster its finish decodes 1.5 G entries for 26 K entries handed out.

All numbers here are **exploratory counts of work**, not times. They do not depend on
load. They depend on the schedule only through the local queues (k-core) and racing
relaxations (SSSP). Runs a and b on the same graph differ by at most 1.7% in a call
count and 1.0% in entries or span. The span columns are a **model** (an estimate, not a measurement), defined below.

## Conditions

- Product kernels at `main` 50c15cd. The kernels crate is built with the feature
  `frontier-counters` (off by default). The feature adds counters only. The result hashes
  are the same as `edge_map_bench` without the feature on orkut: k-core
  `ebea627d802a1caf`, SSSP `10447d27a528faff`, compressed BFS `6fae1d393d667d29`.
- 16 threads (Rayon), unless the run says t1. Release build, the workspace's flags.
- Graphs from `/home/tux/graphlab/data/csr`: orkut (3.07 M nodes, 234 M entries),
  twitter-sym (41.7 M, 2.41 G), Friendster (65.6 M, 3.61 G). All symmetric.
- k-core, SSSP and Leiden: raw CSR, targets memory-mapped. BFS and WCC: the production
  codec, data part memory-mapped (`fc encode`). Warm page cache. Loading not counted.
- Memory: `systemd-run --user --scope -p MemoryMax=16G -p MemorySwapMax=0` (orkut,
  twitter-sym); 24G for Friendster.
- SSSP: source is the node of largest degree. Costs are integers 1..255 hashed from the
  entry index (`edge_map_bench`'s `uniform`), in f64. Δ from `sssp::delta`.
- SSSP on twitter-sym and Friendster runs a copy of the product kernel (`src/sssp.rs`)
  that computes `cost(e)` instead of reading a column. The f64 column alone is 19.2 GB
  (twitter-sym) and 28.9 GB (Friendster), above the limits. Only the cost lookup and the
  weight check differ. On orkut at one thread, the copy and the product kernel write
  identical step logs and the same distance hash.
- BFS: from the node of largest degree, without parents. WCC: default options
  (symmetric, sampled). Leiden: default options (deterministic mode).

## What is counted

Per call of `frontier_with` (a k-core sub-round or exact count, an SSSP step, a BFS
top-down step, a Leiden sub-round):

- **entries**: entries handed to the body, side lists (`Lists::get`, the local queues of
  k-core and SSSP) included;
- **runs** of 256 nodes; a **single-run** call is one done by one thread;
- the entries of the largest run, the longest list, and the longest **side chain** (side
  entries read by one call of the body: work that one task must do);
- **draws**: Philox draws of k-core's sampling; **restarts** of k-core without sampling.

For BFS and WCC, per step: **decoded** (entries the codec decoded), **handed** (entries
handed to bodies) and **examined** (the prefix of each list that the body needs: up to
the first frontier hit in bottom-up BFS; up to the drawn neighbour in WCC's sampling;
the whole list elsewhere).

**Span model** at P = 16, in entries, summed over calls:

- now: E for a single-run call; else max(E / P, largest run);
- S3: max(E / P, min(E, 1024), longest side chain);
- ideal: E / P.

k-core's exact counts split each hub list themselves (`par::map_ranges`, 16,384 entries
per part), so their span is about E / P in both columns. The model counts entries only.
It ignores the fixed cost per call, memory bandwidth and contention, so a real gain is
smaller than the model's ratio.

## Frontier calls (run a; run b in `results/summary.json`)

| graph | kernel | calls | rounds | entries | single-run calls | their entries | share | draws | longest list | span now / S3 / ideal (M) | now / S3 |
|---|---|---|---|---|---|---|---|---|---|---|---|
| orkut | k-core sub-rounds | 420 | 253 | 234,370,166 | 249 | 36,829,682 | 15.7% | 128,581 | 23,934 | 69.0 / 18.6 / 14.6 | 3.70 |
| orkut | k-core exact | 16 | | 526,999 | (nested) | | | | 33,313 | 0.3 / 0.3 / 0.0 | 1.00 |
| twitter-sym | k-core sub-rounds | 2,189 | 2,153 | 2,405,026,092 | 1,584 | 670,230,944 | 27.9% | 292,489,368 | 2,696,902 | 952.9 / 350.5 / 150.3 | 2.72 |
| twitter-sym | k-core exact | 1,173 | | 3,525,227,305 | (nested) | | | | 2,997,487 | 223.9 / 223.9 / 220.3 | 1.00 |
| Friendster | k-core sub-rounds | 597 | 304 | 3,612,134,270 | 296 | 108,396,599 | 3.0% | 0 | 5,214 | 449.4 / 232.3 / 225.8 | 1.93 |
| orkut | SSSP steps | 267 | | 234,773,777 | 96 | 279,943 | 0.1% | | 33,313 | 17.0 / 14.8 / 14.7 | 1.15 |
| twitter-sym | SSSP steps | 487 | | 2,432,665,760 | 276 | 33,275,302 | 1.4% | | 2,997,487 | 360.7 / 175.5 / 152.0 | 2.05 |
| Friendster | SSSP steps | 1,050 | | 3,640,587,393 | 643 | 383,167 | 0.0% | | 5,214 | 228.4 / 227.8 / 227.5 | 1.00 |
| orkut | BFS top-down | 4 | | 9,173,368 | 2 | 33,446 | 0.4% | | 33,313 | 0.9 / 0.6 / 0.6 | 1.56 |
| twitter-sym | BFS top-down | 12 | | 3,589,886 | 9 | 2,997,664 | 83.5% | | 2,997,487 | 3.0 / 0.2 / 0.2 | 13.5 |
| Friendster | BFS top-down | 19 | | 15,364,713 | 8 | 5,502 | 0.0% | | 5,214 | 1.0 / 1.0 / 1.0 | 1.00 |
| orkut | Leiden | 1,053 | | 2,512,432,332 | 305 | 56,021,462 | 2.2% | | 278,364 | 233.1 / 157.1 / 157.0 | 1.48 |

k-core restarts: 0 on every graph and run. Friendster has no node in sample mode (largest
degree 5,214, below 20,000), so no draws and no exact counts.

**k-core, all calls together**, run a:

| graph | entries read | of which exact counts | single-run share of all entries | span now / S3 / ideal (M) | now / S3 |
|---|---|---|---|---|---|
| orkut | 234,897,165 | 0.2% | 15.7% | 69.2 / 18.9 / 14.7 | 3.67 |
| twitter-sym | 5,930,253,397 | 59.4% (1.47× the graph) | 11.3% | 1,176.8 / 574.5 / 370.6 | 2.05 |
| Friendster | 3,612,134,270 | 0% | 3.0% | 449.4 / 232.3 / 225.8 | 1.93 |

**Where the k-core span is** (sub-rounds, run a; calls by their entries):

| graph | entries per call | calls | entries | single-run calls | their entries | span now (M) |
|---|---|---|---|---|---|---|
| orkut | 64 Ki – 1 Mi | 267 | 63,366,391 | 164 | 26,734,003 | 42.9 |
| orkut | ≥ 1 Mi | 70 | 168,839,296 | 2 | 7,931,200 | 23.9 |
| twitter-sym | 64 Ki – 1 Mi | 1,162 | 590,920,909 | 930 | 389,027,285 | 507.0 |
| twitter-sym | ≥ 1 Mi | 510 | 1,807,256,999 | 137 | 274,355,475 | 439.0 |
| Friendster | 64 Ki – 1 Mi | 188 | 76,788,592 | 154 | 52,203,466 | 73.5 |
| Friendster | ≥ 1 Mi | 290 | 3,532,286,014 | 23 | 53,133,469 | 372.9 |

Calls below 64 Ki entries hold under 4% of the span on every graph (`results/summary.json`).

**Spread.** k-core runs a and b: sub-rounds 2,189 / 2,190 and single-run entries
670.2 M / 663.7 M on twitter-sym; 597 / 607 and 108.4 M / 108.0 M on Friendster;
420 / 420 and 36.8 M / 36.7 M on orkut, 36.5 M at one thread. SSSP runs a and b: 487 / 485 steps on twitter-sym, identical step counts on the
other graphs.

## Decode surplus (compressed source, 16 threads)

| graph | kernel | step | steps | decoded | handed | examined | decoded / examined | decoded / handed | handed / examined |
|---|---|---|---|---|---|---|---|---|---|
| orkut | BFS | bottom-up | 3 | 485,565,512 | 147,692,659 | 49,266,494 | 9.86 | 3.29 | 3.00 |
| orkut | BFS | all | 7 | 494,738,880 | 156,866,027 | 58,439,862 | **8.47** | 3.15 | 2.68 |
| twitter-sym | BFS | bottom-up | 3 | 5,154,855,670 | 2,030,583,067 | 313,907,262 | 16.42 | 2.54 | 6.47 |
| twitter-sym | BFS | all | 15 | 5,158,445,556 | 2,034,172,953 | 317,497,148 | **16.25** | 2.54 | 6.41 |
| Friendster | BFS | bottom-up | 3 | 10,836,402,810 | 3,051,393,956 | 511,396,313 | 21.19 | 3.55 | 5.97 |
| Friendster | BFS | all | 22 | 10,851,767,523 | 3,066,758,669 | 526,761,026 | **20.60** | 3.54 | 5.82 |
| orkut | WCC | sample | 1 | 234,370,166 | 234,370,166 | 120,238,312 | 1.95 | 1.00 | 1.95 |
| orkut | WCC | finish | 1 | 0 | 0 | 0 | – | – | – |
| twitter-sym | WCC | sample | 1 | 2,405,026,092 | 2,405,026,092 | 1,243,340,528 | 1.93 | 1.00 | 1.93 |
| twitter-sym | WCC | finish | 1 | 57,293,684 | 7,080 | 7,080 | 8,092 | 8,092 | 1.00 |
| twitter-sym | WCC | all | 2 | 2,462,319,776 | 2,405,033,172 | 1,243,347,608 | **1.98** | 1.02 | 1.93 |
| Friendster | WCC | sample | 1 | 3,612,134,270 | 3,612,134,270 | 1,864,640,743 | 1.94 | 1.00 | 1.94 |
| Friendster | WCC | finish | 1 | 1,506,841,674 | 26,112 | 26,112 | 57,707 | 57,707 | 1.00 |
| Friendster | WCC | all | 2 | 5,118,975,944 | 3,612,160,382 | 1,864,666,855 | **2.75** | 1.42 | 1.94 |

The decode surplus has two parts:

- **decoded / handed**: `Compressed::scan` decodes every list of a reduction block when
  one node of the block passes the filter. In bottom-up BFS that is 2.5–3.5×. In WCC's
  finish, which skips the giant component's nodes, it is the whole surplus.
- **handed / examined**: the part of each list after the entry the body last needs. This
  is what a prefix decode saves: 2.7–6.4× in BFS, 1.9× in WCC's sampling.

WCC's sampling reads one or two entries per list: 6.1 M (orkut), 81.4 M (twitter-sym)
and 117.6 M (Friendster) entries in all, against 234 M, 2.41 G and 3.61 G decoded
(30–39×). The prefix it needs is long only because the drawn neighbour is uniform over
the whole list.

## Verdicts

**The driver is not the common cause.**

- **k-core: yes, a main cause.** 15.7% (orkut), 27.9% (twitter-sym) and 3.0%
  (Friendster) of the sub-rounds' entries are in sub-rounds done by one thread. The
  model puts the sub-rounds' span at 3.7×, 2.7× and 1.9× the span with S3. On Friendster
  the cause is unequal runs, not single-run calls: 290 calls of over 1 Mi entries hold
  most of the span. On twitter-sym a second cost is not the driver: the exact counts of
  the sampled hubs read 3.53 G entries, 1.47× the graph, in 1,173 calls. They are
  already split per list.
- **SSSP: only on twitter-sym.** The model gives 2.05× on twitter-sym, from runs that hold
  lists of up to 3.0 M entries. It gives 1.15× on orkut and 1.00× on Friendster. The SSSP
  gap against GAP on lj and orkut (m3-22) is therefore elsewhere: the passes per step and
  the representation (G2, S7).
- **BFS: no.** Top-down steps carry 0.2–5.9% of the entries handed out. The one large
  single-run step is twitter-sym's first: the source is the 3.0 M-entry hub. On a
  compressed source the cost is the bottom-up decode surplus (S10).
- **Leiden (orkut only): no.** 2.2% of its entries are in single-run calls; the model
  gives 1.48× from entry cuts alone. Its bodies gather a whole list into a table, so they
  are not commuting pushes, and hub splitting does not apply to them.

**S3: build it, for k-core sub-rounds and SSSP relaxations.** Cut runs by entries
everywhere; split hub lists in `peel_one` (`fetch_sub`) and in SSSP's relax
(`fetch_min`). Model: k-core span 0.27× (orkut), 0.49× (twitter-sym), 0.52× (Friendster)
of today; SSSP 0.49× on twitter-sym, no change on orkut and Friendster. These are
estimates. The S3 success bar (k-core on twitter-sym ≤ 0.7× of today, no case slower
than 1.05×) needs an interleaved A/B. Look at the exact counts of k-core's sampling too:
on twitter-sym they read more entries than the peel.

**S10: build it, for BFS.** Decoded / examined is 8.5× (orkut), 16.3× (twitter-sym) and
20.6× (Friendster), above the 2× bar. The prefix part alone (handed / examined) is
2.7–6.4×. For WCC, a prefix decode alone gives 1.9–2.0×, under the bar. Two cheaper
changes cover more than S10 there:

1. Decode only the records of the nodes that pass the filter (the index has a byte
   offset per node). This removes the decoded / handed part: 2.5–3.5× in bottom-up BFS,
   and in WCC's finish on Friendster 1.5 G decoded entries for 26 K handed.
2. Draw WCC's sampled neighbour from a short prefix of the list. ConnectIt's k-out
   sampling does not need a uniform neighbour. The prefix then falls to 1 or 2 entries
   per list (30–39× fewer entries than today).

## Not measured, and what failed

- Times. Nothing here says how much of a kernel's time the driver takes. The model counts
  entries on the critical path only.
- Leiden on twitter-sym: killed at the 16 GB limit (exit 137). Leiden on Friendster was
  not tried.
- The fixed cost per call: k-core runs 436 (orkut), 3,362 (twitter-sym) and 597
  (Friendster) frontier calls. S3 does not change that count.
- lj and roadnet-ca (where m3-22 measures the SSSP gap to GAP) were not run. An lj smoke
  test is not in the results.

## Reproduce

```sh
lab/m3-27-frontier-counters/run.sh          # about 10 minutes; results/ is rewritten
python3 -I lab/m3-27-frontier-counters/summarize.py lab/m3-27-frontier-counters/results
```

`run.sh` builds `fc`, encodes the three stores under `/home/tux/graphlab/scratch/m3-27`,
runs every case under its memory limit, and writes `results/runs.jsonl` (one JSON line
per run, with BFS and WCC marks) and `results/calls/*.jsonl.gz` (one line per frontier
call). `summarize.py` writes `results/summary.json` and prints the tables.
