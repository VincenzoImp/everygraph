# m1-09: an ordering bake-off (#23, widened by research decision B2)

**Status: exploratory.** Every run here was on a shared machine: other agents were
compiling, and the 1-minute load was 25 to 57 on 32 hardware threads. No time in this
directory may be quoted as an absolute number. Sizes (bits, BV bits, log-gap cost) and
modularity do not depend on load. Kernel times are interleaved A/B ratios in one process
(A = the graph in its input ids, B = the same graph renumbered), and even those are noisy
at this load: see "How noisy". The official command is at the end.

## Files

| File | What |
|---|---|
| `orders.jsonl` | one line per computed order: the time to compute it (`order_s`), and for BP the per-level iterations, swaps and seconds |
| `eval.jsonl` | one line per graph and order: permute and encode times, codec 0 bits per entry (index included), log-gap cost, locality ratio, codec-word size classes, and the kernel A/B rows with their checks. Per (graph, order) the last line with kernel rows is summarized, else the last line |
| `bv.jsonl` | BV bits per arc of each order (webgraph-cli 0.5.0, default flags) |
| `leiden/*.json` | the Leiden runs (lab b2) that built each hierarchy: time, modularity, levels |
| `leiden_orders.jsonl` | Leiden on the graph stored in a given order (#146's fifth arm) |
| `wcc_work.jsonl` | WCC alone, with the list entries it handed to its bodies in A and B |
| `summary.txt`, `summary.json` | the tables below, from `../summarize.py` |

Code: `../src` (`m109`: orders, BP, permute, encode, kernels A/B), `../run.sh`,
`../leiden.sh`, `../webgraph.sh`, `../leiden_orders.sh`; `lab/b2-leiden` for Leiden;
webgraph-cli 0.5.0 (Apache-2.0 OR MIT) for LLP and BV. Spec: `bench/specs/hierarchy-order.toml`.

## Conditions (every number below)

- Ryzen 9 7950X, 61.9 GiB, NVMe, Linux 6.12. 16 threads for everything (orders, Leiden,
  LLP, kernels). Each command under `systemd-run --user --scope -p MemoryMax=<limit>
  -p MemorySwapMax=0`: 8G for lj and orkut, 24G for twitter-sym orders, 28G for its
  evaluation, 30G for its Leiden; Friendster as stated in its section.
- Graphs: the lab CSRs in `$EVERYGRAPH_DATA/csr` (symmetric, sorted, no self-loops).
  `uk-2005` is not in the data directory or the dataset registry: **not measured**.
- Bits per entry: our codec (`everygraph-format` encoder, codec 0, Stream VByte 1234 of
  zigzag first neighbour and gaps − 1), index included, all entries of the symmetric CSR.
- Log-gap cost: the mean of log2 g over all entries, where g = |x0 − v| for the first
  neighbour of v and x_j − x_{j−1} after it (the BP paper's LogGap per entry).
- Locality ratio: bits per entry ÷ (2 + log2(n/d)), d = entries / n (challenge 7).
- "0124 est": the data bits per entry that Stream VByte 0124 (block codec 2, #m1-16) would
  need, from the measured word-size classes (2 control bits per word; no index). An
  estimate from counts, not an encoder.
- Kernels: the edge-map proof kernels (`everygraph_kernels::proof`), because the
  production kernels (#218, #220) are not merged. PageRank f32, 20 iterations, damping
  0.85; direction-optimizing BFS from the largest-degree node (the same node in B);
  Afforest WCC. Sources: raw CSR in the heap, and codec 0 in the heap ("compressed").
  Each kernel: one untimed run of A and of B, then R repetitions alternating which goes
  first (A B, B A, …); the ratio is median(B) / median(A). Kernel time only: no loading,
  no fsync. Each B answer is checked against A's, relabelled: BFS depths and the component
  partition exactly, PageRank to 1e-3 of the largest rank. Every check passed.
- Orders (all deterministic; seed 1 where seeded): `random` (seeded hash; the
  literature's worst case), `dbg` (degree-based grouping, 8 groups), `bfs` (from the
  largest-degree node), `flat` (level-0 Leiden community, then id), `hier` (the whole
  Leiden path, top first, then id: a DFS of the dendrogram), `hier-deg` / `hier-bfs` (the
  same path, ties by degree or by BFS position), `hier-cap64` (the DFS of the cap-64
  "map-shaped" hierarchy), `llp` (webgraph-cli `run llp`, default gammas, seed 1), `bp`
  (recursive graph bisection, `../src/bp.rs`: seeded random start, depth floor(log2 n) − 5,
  at most 20 iterations per split, cooling). Hierarchies: lab b2 Leiden, gamma 1, async,
  levels kept if they shrink 3×; "natural" without a cap, "cap64" with fan-out cap 64.

The BP code is a lab implementation (`../src/bp.rs`, about 230 lines), not the published
one: Mackenzie et al.'s `rgbisect` (MIT, crates.io) allocates two arrays of n counters per
subproblem, which is quadratic at depth log n − 5 on graphs, so it was read, not linked.
It is deterministic at any thread count: the permutation files at 16 and 3 threads are
byte-identical (web-google), and two different parallel schedules gave the same lj file.

## Results: sizes (load-independent)

Codec 0 bits per entry, index included, and the change against the input ids.
Full tables, with the permute and encode times, are in `summary.txt`.

| order | lj | orkut | twitter-sym | Friendster |
|---|---|---|---|---|
| input ids | 17.99 | 16.64 | 16.09 | 23.17 |
| random | 22.62 (+25.8%) | 19.30 (+16.0%) | 21.13 (+31.3%) | 24.39 (+5.3%) |
| dbg | 18.44 (+2.5%) | 16.70 (+0.4%) | **15.59 (−3.1%)** | 21.17 (−8.6%) |
| bfs | 18.34 (+1.9%) | 16.52 (−0.8%) | 17.13 (+6.4%) | 20.81 (−10.2%) |
| flat (level-0 community) | 17.59 (−2.2%) | 15.63 (−6.1%) | 17.24 (+7.1%) | not run |
| hier (Leiden DFS) | 15.43 (−14.2%) | 14.61 (−12.2%) | 17.32 (+7.6%) | 19.65 (−15.2%) |
| hier-deg / hier-bfs | 15.43 / 15.44 | 14.55 / 14.60 | 17.50 / 17.58 | not run |
| hier-cap64 | **14.98 (−16.7%)** | 14.37 (−13.6%) | 17.94 (+11.5%) | not run |
| llp | 15.32 (−14.8%) | 14.28 (−14.2%) | not run | not run |
| bp | 15.73 (−12.5%) | **13.92 (−16.4%)** | not finished (see below) | not run |

BV bits per arc (webgraph-cli defaults), lj and orkut only:

| order | lj | orkut |
|---|---|---|
| input ids | 14.33 | 15.25 |
| random | 23.08 (+61%) | 20.00 (+31%) |
| bfs | 14.71 | 14.98 |
| hier | 11.56 | 12.10 |
| hier-cap64 | **10.87 (−24%)** | 11.61 |
| llp | 10.95 (−24%) | 11.32 (−26%) |
| bp | 11.91 (−17%) | **10.82 (−29%)** |

Checks against earlier and published numbers: orkut input 15.25 and LLP 11.32 BV bits are
the lab's earlier values (FINDINGS § C); lj random +61% BV is close to the LLP paper's
+64% on LiveJournal (Table 6; their graph and machine). Our BP is weaker than the
paper's on LiveJournal (11.91 here against their 10.73 from 14.61; their LiveJournal is not
our SNAP com-lj, so this is not like for like). Four variants on lj stay within
11.50–11.91 BV bits: input start instead of seeded random (11.87), no cooling (11.70), each
node also its own term (11.78), bisection down to single nodes (11.50). So the gap is not
one of these choices. BP's own objective agrees: log-gap cost 7.70 against 6.92 for LLP.

**Our codec gains less than BV from the same order.** lj: −16.7% (codec 0) against −24%
(BV) for the best order; orkut −16.4% against −29%. Every gap costs at least one byte and
two control bits, so a gap of 1 costs 10 bits however good the order. After the best
orders, 19–24% of the codec words are zero (gap 1) on lj (15.7% on the input ids) and
12–15% on orkut (3.5%).

**twitter-sym is the exception: its input ids are already a good order.** Every order
measured except DBG makes it larger: BFS +6.4%, the Leiden DFS +7.6% (natural) and +11.5%
(cap 64), DBG −3.1%. Its Leiden modularity is low (0.476 natural, 0.356 cap 64, against
0.61–0.75 on the other graphs), so a community order has little to group. BP on
twitter-sym did not finish in this session (more than 2.5 hours at 16 threads on the
loaded machine; it is in the official command).

**The log-gap cost predicts our bits.** Within each graph, across its orders, the Pearson
correlation of log-gap cost and codec 0 bits per entry is 0.998 (lj, 15 orders), 0.999
(orkut, 11), 1.000 (twitter-sym, 9) and 0.998 (Friendster, 5); with BV bits 0.992 (lj)
and 0.999 (orkut). So the encoder can rank candidate orders from one pass over the
permuted lists, without encoding them.

**Stream VByte 0124 (block codec 2, #m1-16), estimated from the word sizes** (data bits
per entry, no index; the 1234 estimate from the same counts is within 0.21 of the real
codec 0 data part on every row). It saves 3–4% after the best orders (lj hier-cap64 14.02
→ 13.54, orkut bp 13.65 → 13.15). It costs 5–20% on the input ids (lj 17.02 → 17.93,
Friendster 22.79 → 27.41), because 3-byte words become 4. A per-block choice would take
the smaller.

**Reorder cost** (exploratory times, loaded machine, 16 threads; `orders.jsonl`). Seconds
to compute the order; permuting and re-encoding add 0.2–1.3 s (lj, orkut), 5–11 s
(twitter-sym) and 11–22 s (Friendster) in total.

| | lj | orkut | twitter-sym | Friendster |
|---|---|---|---|---|
| random, dbg | 0.1 | 0.1 | 0.3–0.6 | 1.1–1.7 |
| bfs (sequential) | 0.9 | 1.5 | 25 | 218 (targets memory-mapped) |
| Leiden natural, then hier | 1.0 + 0.6 | 2.2 + 0.9 | 127 + 7 | 150 + 12 |
| Leiden cap 64, then hier | 1.3 + 0.8 | 3.2 + 0.4 | 173 + 8 | not run |
| llp (webgraph-cli) | 64 | 361 | not run | not run |
| bp (lab) | 74 | 282 | > 9,000, unfinished | not run |

## Results: the locality score (challenge 7)

| graph | ratio on the input ids | log-gap cost | best order measured | its change | random ids |
|---|---|---|---|---|---|
| twitter-sym | 0.750 | 9.14 | dbg | −3.1% | +31.3% |
| lj | 0.908 | 10.08 | hier-cap64 | −16.7% | +25.8% |
| orkut | 0.962 | 10.35 | bp | −16.4% | +16.0% |
| Friendster | 1.044 | 16.60 | hier (BP not run) | −15.2% | +5.3% |

The review's arithmetic (0.86, 0.95, 0.74, 1.04) agrees for orkut, twitter-sym and
Friendster; lj is 0.908 here because these bits include the index (0.76 bits per entry
on lj, 0.23 on orkut, 0.31 on Friendster).

What the ratio ranks:

- **The locality the input ids carry**, which a random relabelling destroys. Its order
  (twitter-sym < lj < orkut < Friendster) is exactly the order of random's cost (+31%,
  +26%, +16%, +5%).
- **Whether an order is worth trying, only at the low end.** twitter-sym, the lowest
  ratio, gains nothing (−3% at best). The other three gain 15–17% with their best order,
  although their ratios span 0.91–1.04. So the ratio does not predict the size of the gain
  among them.

For the encoder's stats and the planner's `explain()`, the useful pair is the ratio
(how much locality the ids have) plus the log-gap cost of one cheap candidate (the
hierarchy DFS, seconds after Leiden), against the input's. The correlation above says the
second number decides the size question without encoding.

## Results: kernels (interleaved A/B, exploratory)

B/A ratios, wall / CPU time (CPU = user + system time of the process, which does not grow
while a worker waits for a core another process holds). Below 1: the reordered graph is
faster. lj 11 repetitions, orkut 7, twitter-sym 2, Friendster 3 (after one untimed run of
each). Sources: lj and orkut raw (compressed rows in `summary.txt`, same direction);
twitter-sym raw, A memory-mapped and warm, B in the heap (the A/A row shows no bias);
Friendster compressed.

| graph | order | PageRank | BFS | WCC |
|---|---|---|---|---|
| lj | input (A/A) | 1.05 / 0.96 | 1.03 / 0.99 | 0.89 / 0.99 |
| lj | random | 1.18 / 1.32 | 1.56 / 1.61 | 1.82 / 2.02 |
| lj | bfs | 0.98 / 0.99 | 0.61 / 0.64 | 0.89 / 0.86 |
| lj | hier-cap64 | 1.13 / 0.90 | 1.21 / 1.40 | 1.49 / 1.24 |
| lj | llp | 0.81 / 0.87 | 0.89 / 1.17 | 1.27 / 1.42 |
| lj | bp | 0.97 / 0.96 | 0.94 / 1.07 | 1.72 / 1.61 |
| orkut | input (A/A) | 1.05 / 1.07 | 0.64 / 1.08 | 1.07 / 1.06 |
| orkut | random | 1.28 / 1.24 | 1.52 / 1.46 | 1.72 / 1.66 |
| orkut | bfs | 0.88 / 1.02 | 0.61 / 0.58 | 1.56 / 0.96 |
| orkut | hier | 0.66 / 0.69 | 1.28 / 1.47 | 1.53 / 1.66 |
| orkut | hier-cap64 | 0.92 / 0.87 | 0.87 / 1.01 | 1.19 / 1.23 |
| orkut | llp | 0.81 / 0.75 | 1.02 / 1.01 | 1.49 / 1.33 |
| orkut | bp | 0.81 / 0.75 | 1.01 / 1.01 | 1.89 / 2.01 |
| twitter-sym | input (A/A) | 1.00 / 1.01 | 1.01 / 1.01 | 1.00 / 1.00 |
| twitter-sym | random | 1.21 / 2.08 | 1.10 / 1.14 | 1.13 / 1.09 |
| twitter-sym | dbg | 1.33 / 0.63 | 0.78 / 0.76 | 0.66 / 0.66 |
| twitter-sym | bfs | 0.78 / 1.07 | 0.49 / 0.53 | 0.59 / 0.59 |
| twitter-sym | hier | 0.74 / 1.18 | 0.92 / 0.94 | 0.88 / 0.90 |
| Friendster | input (A/A) | 0.97 / 1.01 | 1.04 / 1.01 | 0.95 / 1.00 |
| Friendster | random | 1.05 / 1.07 | 1.07 / 1.06 | 1.36 / 1.36 |
| Friendster | dbg | 0.54 / 0.55 | 0.75 / 0.70 | 0.82 / 0.81 |
| Friendster | bfs | 0.59 / 0.56 | 0.31 / 0.31 | 0.60 / 0.60 |
| Friendster | hier | 0.69 / 0.60 | 0.92 / 0.92 | 0.89 / 0.89 |

**How noisy.** The A/A rows are within ±7% on CPU time but reach 0.64 on wall time
(orkut BFS). A's own median varied 2–5× between runs of the same case as the load
changed (`summary.txt` lists the ranges). twitter-sym PageRank, with 2 repetitions, has
wall and CPU ratios on opposite sides of 1 for three orders: no conclusion there. Only
effects well above ±10% on both wall and CPU time are read as results:

- **Random ids are slower for every kernel on every graph**: PageRank 1.05–2.08, BFS
  1.07–1.61, WCC 1.09–2.08 (wall and CPU both above 1 in every row).
- **A locality order speeds PageRank where the input ids are poor**: Friendster 0.54–0.69
  (dbg, bfs, hier); orkut 0.66–0.81 (hier, llp, bp). On lj it is within noise except LLP
  (0.81 / 0.87).
- **BFS order speeds BFS from the hub** (0.31–0.64 on all four graphs).
- **Afforest WCC is slower on the community-style orders of lj and orkut** (hier, llp,
  bp: 1.2–2.0), and faster on bfs and dbg, and on the non-random orders of Friendster and
  twitter-sym. It hands the same number of list entries to its bodies in A and B (within
  1%, `wcc_work.jsonl`), so the extra time is in its union-find links, not in more lists.
  A likely cause, not measured: its two sampling rounds link each node to its first two
  neighbours, which in a community order are in the same community, so the giant
  component forms later and more links contend on the same roots. The production WCC
  (#218) should be checked on a reordered graph.

## #146 (m1-14): random relabelling is the worst case

Confirmed on our numbers, for storage and for the kernels:

- Bits: +25.8% (lj), +16.0% (orkut), +31.3% (twitter-sym), +5.3% (Friendster) with
  codec 0; +61% and +31% BV on lj and orkut.
- Kernels: slower on every kernel and graph (above).
- Leiden quality is not the problem (lab b2, in RAM, 16 threads, 3 runs each;
  `leiden_orders.jsonl`). The fifth arm, a seeded locality order (BP) with Leiden visiting
  in storage order:

| graph | stored order, visit | Q, 3 async runs |
|---|---|---|
| lj | input ids, block order (ADR 0015) | 0.7505, 0.7491, 0.7494 |
| lj | input ids, storage order | 0.7269, 0.7286, 0.7351 |
| lj | random, storage order | 0.7549, 0.7476, 0.7481 |
| lj | bp, storage order | 0.7548, 0.7566, 0.7558 |
| lj | hier-cap64, storage order | 0.7458, 0.7489, 0.7477 |
| orkut | input ids, block order | 0.6829, 0.6799, 0.6756 |
| orkut | input ids, storage order | 0.6596, 0.6506, 0.6472 |
| orkut | random, storage order | 0.6838, 0.6663, 0.6664 |
| orkut | bp, storage order | 0.6724, 0.6731, 0.6631 |
| orkut | hier-cap64, storage order | 0.6715, 0.6854, 0.6789 |

On lj, BP visited in storage order matches block order on the input ids (0.755 against
0.750). On orkut both locality orders stay 0.5–1 point below block order and above
input-id order. Deterministic mode gave one Q per order, the same for both visits (its
sub-rounds do not follow the visiting order): lj 0.7480 (input), 0.7481 (random), 0.7514
(bp), 0.7484 (hier-cap64); orkut 0.6760, 0.6773, 0.6774, 0.6765. In RAM only: the
out-of-core arm (16, 8, 4 GiB) is #146's to run. Leiden times in that file are not usable
(the deterministic runs on lj took 5–23 s at load 40–75).

## Answers

- **Renumber by the Leiden hierarchy by default? No, not unconditionally.** The DFS order
  of the hierarchy gains 12–17% on lj, orkut and Friendster and costs seconds once Leiden
  has run, but it loses 7.6–11.5% on twitter-sym, whose input ids are already ordered. A
  default must not make a graph worse. Recommended: offer the hierarchy DFS as the
  compaction order, and apply it only when its log-gap cost is lower than the input's
  (one pass; correlation 0.998 with our bits), never inside the ingest (promise 1).
- **cap 64 or natural hierarchy?** cap 64 is better on lj (−16.7% against −14.2%) and orkut
  (−13.6% / −12.2%), worse on twitter-sym (+11.5% / +7.6%). Ties inside a community (by id,
  degree or BFS position) change at most 0.4% on lj and orkut, and make twitter-sym up to
  1.5% worse: order by id. The map's hierarchy (cap 64, m1-05) is
  the one Everygraph computes anyway, so the layout order and the storage order can be the
  same permutation; the layout I/O half of this issue was not measured here.
- **BP (#179):** best on orkut (−16.4% codec 0, −29% BV), third on lj (−12.5%, behind the
  hierarchy DFS and LLP), not finished on twitter-sym in 2.5 hours; 74 s (lj) and 282 s
  (orkut) at 16 threads on a loaded machine. Deterministic at any thread count, as the
  issue required. Against the cap-64 hierarchy DFS it is 3 points better on orkut and 5
  points worse on lj, for 35–80× the time (Leiden included).
  Recommendation for m2-24: keep it opt-in and lower its priority below a hierarchy-DFS
  compaction step; build it only if the official run shows BP clearly ahead of the
  hierarchy DFS on a graph family where Leiden finds little (twitter-sym is that case),
  and measure its acceptance against the hierarchy DFS, not against the input order.
- **Default storage order:** the input order, as now (first appearance, m2-07), with the
  hierarchy-DFS compaction as the offered rewrite. Never a random relabelling.
- **m1-16 (codec 2, 0124):** worth it only after an order: estimated −3 to −4% of data
  bits after the best orders, +5% to +20% on unordered ids. Its baseline order should be
  the hierarchy DFS (or BP), chosen per block.
- **m2-20 (archival):** after the best order BV is 10.87 (lj) and 10.82 (orkut) bits per
  arc, against 14.98 and 13.92 for codec 0 with its index: still 27% and 22% smaller.
- **#146:** a random relabelling of the stored graph costs 5–31% of storage and slows
  every kernel; Leiden's quality does not need it (block order on the input ids, or a
  locality order in storage order, are as good). This supports the Leiden-only working
  copy, or keeping ADR 0015's block order.

## Not done

- uk-2005: not in the data directory or the registry.
- LLP on twitter-sym and Friendster (an estimate of 40 minutes or more each from the orkut
  time; the BV graphs would need a 2.4–3.6 billion-line text pipe); BV sizes on those two.
- BP on twitter-sym (unfinished after 2.5 hours) and Friendster; flat, hier-deg, hier-bfs
  and the cap-64 hierarchy on Friendster.
- The layout criterion (Friendster under 16G, the lab layout on hierarchy-ordered ids):
  not run.
- The production kernels (#218, #220): not merged; the proof kernels were used.
- Any timing on an idle machine: the official command below.

## Official command

```
everygraph-bench run hierarchy-order --official --data /home/tux/graphlab/data
```

The groups must run in order (leiden, order, eval, bv); each can be its own window with
`--only <group>`. Wall time: see the spec's header.
