# m1-03 — why asynchronous Louvain collapsed on Friendster (issue #17)

Question: why did asynchronous Louvain give Q 0.4647 on Friendster (largest community 33.4 M
nodes) when Leiden gives 0.6033 and a later run of the same Louvain gave 0.6038? Should the
asynchronous `fast` mode ship in production Leiden (#57), and with which guard?

**Every timing here is exploratory.** The machine was shared (load average 1 to 79, recorded
per row as `load_before`). Modularity values are lab results. In asynchronous mode they depend
on thread timing, so they are given as distributions over many runs, never as one number.

## Answer

**The cause is concurrent moves of a few heavy vertices on the coarse levels.** After pass 0 on
Friendster, three level-1 vertices hold 24.3%, 22.8% and 18.8% of the total volume 2m. Eight
vertices hold more than 1% each, and 61 hold more than 0.1% (the `L1-t16` dump). Their
neighbour scans are the longest of the level, so different threads decide them at the same
time. Two of them then join the same community. Each one sees a community total that does not
yet contain the other. Each move looks good alone. Together they lose 2·k_u·k_v/(2m)² of
modularity: 0.02–0.3 in these runs. In the next iteration one of them leaves again,
with a real gain, and the cycle repeats. The same races also make heavy vertices chase each
other's labels: large claimed gains, no real change. The claimed gain never falls below the
tolerance, so the pass runs to the 20-iteration cap. **The final Q depends on where in the
cycle the cap stops.** The FINDINGS row (0.4647) and the tracer row (0.6038) are two draws from
one distribution. The memory limit, `--min-ratio` and f32 weights are not the cause.

`results/heavy-moves-replay.log` shows the cycle: in replay pass 0 (original pass 1), vertices
47665 (26.6% of 2m) and 126570 (2.3%) move into community 41887 in the same iteration
(Q 0.6033 → 0.5811), leave it in the next (→ 0.6034), and repeat until the cap.

**Guard (recommended for #57):** a vertex whose volume is above θ·2m, with θ = 0.001, is
visited by one thread after each parallel sweep. At most 1/θ = 1,000 vertices qualify. No two
heavy vertices then move at the same time. A race between two light vertices still misjudges a
gain, but by at most 2θ² = 2·10⁻⁶ per pair. With this guard the replayed Friendster coarse
levels give Q 0.6034 in 120 of 120 runs at 4, 8 and 16 threads, with no pass at the cap. Full
Friendster runs give 0.5996–0.6089 (6 of 6). Without the guard, 25 of 140 replays at 16 threads
end below 0.55, and 4 of 20 full runs do.

## Results

### 1. Reproduction on the full graph (the issue's first step)

Friendster, async Louvain, 1 iteration, w32 build (as both originals), 16 threads (not 32:
shared machine), `--trace` added (coarse passes only; it does not change decisions). Order:
FINDINGS and tracer flags interleaved, 16G and 24G interleaved, 5 rounds. Warm page cache.
`results/full.jsonl`, `results/full-heavy.jsonl`.

| flags | limit | runs | Q mean [min, max] | Q < 0.55 | largest community (M nodes) | passes at the cap (mean) |
|---|---|---|---|---|---|---|
| FINDINGS | 16G | 5 | 0.5918 [0.5664, 0.6073] | 0 | 21.0–24.1 | 2.4 |
| FINDINGS | 24G | 5 | 0.5883 [0.5390, 0.6069] | 1 | 20.1–28.5 | 1.6 |
| tracer | 16G | 5 | 0.5756 [0.4791, 0.6071] | 1 | 20.3–23.8 | 2.2 |
| tracer | 24G | 5 | 0.5284 [0.2913, 0.6075] | 2 | 19.2–41.0 | 1.2 |
| FINDINGS + `--heavy 0.001` | 16G | 3 | 0.6034 [0.6012, 0.6060] | 0 | 20.8–21.6 | 0 |
| tracer + `--heavy 0.001` | 16G | 3 | 0.6033 [0.5996, 0.6089] | 0 | 20.2–21.8 | 0 |

The collapse reproduces with both flag sets and under both limits. The worst run, Q 0.2913,
ended with one community of 41.0 M nodes (75% of 2m). The 16G vs 24G and FINDINGS vs tracer
differences are inside the run-to-run spread (5 runs each). In 17 of the 20 unguarded runs, two
giants merge at some iteration of a coarse pass: the top community's share of 2m rises from
0.27 to 0.46–0.80, and the lowest Q seen inside a pass is 0.24–0.60 (per-iteration `trace` in
each row). Whether the run ends low depends on the iteration at which the pass stops.

### 2. Minimal reproduction: replaying the coarse levels

One full run (f64 build, 16 threads, 24G; `results/dump.jsonl`, Q 0.6020) wrote its level-1
graph: 1,163,162 vertices, 11,254,644 entries, f64 weights (`results/L1-t16-meta.json`). The
graph itself (145 MB) was deleted with the scratch directory; `run.py --plan dump` writes a new
one, from a new pass-0 result. A replay runs passes 1… of the same schedule on it in
under 2 s:

    b2 run --graph L1-t16 --algo louvain --threads 16 --tol 0.001 --max-passes 9 \
        --trace --trace-q0 --node-weights L1-t16/nodes.u32 [--order-seed S]

Q of the replay equals Q of the composed partition on Friendster. `--order-seed` S = 0…39
changes the visiting order; at more than one thread the run is not repeatable anyway.
Limit 8G. `results/replay-base.jsonl`, `replay-t16.jsonl` (with the trace of replay pass 0 off),
`replay-t16-guards-q0.jsonl` (on).

| build | threads | runs | Q mean [min, max] | Q < 0.59 | Q < 0.55 | passes at the cap (mean) |
|---|---|---|---|---|---|---|
| f64 / w32 | 1 | 10 / 10 | 0.6032 [0.6030, 0.6036], identical in both builds | 0 | 0 | 0 |
| f64 | 2 | 10 | 0.6032 [0.6029, 0.6036] | 0 | 0 | 0.6 |
| w32 | 2 | 10 | 0.6032 [0.6029, 0.6035] | 0 | 0 | 0.3 |
| f64 | 4 | 10 | 0.6032 [0.6029, 0.6036] | 0 | 0 | 0.9 |
| w32 | 4 | 10 | 0.6031 [0.6015, 0.6036] | 0 | 0 | 1.3 |
| f64 | 8 | 10 | 0.6014 [0.5918, 0.6036] | 0 | 0 | 1.8 |
| w32 | 8 | 10 | 0.5887 [0.4873, 0.6036] | 2 | 1 | 1.9 |
| f64 | 16 | 50 | 0.5787 [0.4357, 0.6037] | 13 | 9 | 2.1 |
| w32 | 16 | 50 | 0.5755 [0.4456, 0.6037] | 19 | 12 | 2.2 |
| w32, trace of replay pass 0 on | 16 | 40 | 0.5862 [0.2794, 0.6037] | 5 | 4 | 1.6 |

At one thread the claimed gain equals the true change (race loss 0) and no pass reaches the
cap. From 2 threads on, passes reach the cap; from 8 threads on, Q collapses. f32 and f64
give the same distribution at 16 threads, and identical results at one thread: **f32 weights
are ruled out.**

### 3. Guards, on the same replay at 16 threads (w32, 40 runs each)

`results/replay-t16-guards.jsonl`, `replay-t16-guards-q0.jsonl` (rollback and true-stop need
the trace on every pass, so they come from the second file), `replay-heavy-threads.jsonl`.

| guard | Q mean [min, max] | passes at the cap (mean) |
|---|---|---|
| none (50 runs) | 0.5755 [0.4456, 0.6037] | 2.2 |
| `--max-iters 5` (iteration cap) | 0.5643 [0.4134, 0.6036] | 3.6 |
| `--true-stop` (stop on the true gain) | 0.5325 [0.2289, 0.6016] | 0 |
| `--rollback` (undo an iteration that lowers Q, end the pass) | 0.6011 [0.5923, 0.6036] | 0 |
| `--heavy 0.01` | 0.6034 [0.6033, 0.6036] | 0 |
| `--heavy 0.001` | 0.6034 [0.6034, 0.6034] | 0 |
| `--heavy 0.001`, 4 and 8 threads, f64 and w32 (80 runs) | 0.6034 [0.6034, 0.6034] | 0 |
| `--heavy 0.01`, 4 and 8 threads, f64 and w32 (80 runs) | 0.6034 [0.6033, 0.6036] | 0.03 |
| `--det` (deterministic, auto sub-rounds) | 0.6036, one result | 0 |

- A lower iteration cap does not help: it only picks another point of the cycle.
- Stopping on the true gain is worse: it stops right after a merge, inside the bad state.
- Rollback avoids the collapse but ends passes early and loses up to 0.011 Q.
- The heavy rule removes the cause. The passes converge (no cap), so coarse local moving gets
  faster, not slower. On level 0 of Friendster no vertex qualifies (largest degree 5,214, a
  share of 1.4·10⁻⁶), so pass 0 is unchanged.
- Exploratory time of the replay (coarse levels only, under 2 s of a 70–230 s run; trace on,
  so one modularity evaluation per iteration is included; separate `run.py` calls): median
  0.17 s with `--heavy 0.001` against 0.51 s without. This is not a measurement of the guard's
  cost.
- Deterministic mode with few sub-rounds (1 run each, it is deterministic): 2, 4, 16, 64
  sub-rounds give Q 0.6036, 0.6036, 0.6036, 0.6036 (0.603582–0.603596); 2 sub-rounds hit the
  cap in 3 of 4 passes with 1.1 M back moves in the first, 4 sub-rounds in 1 of 4. #18's
  L64k rule (`--sub-div 65536 --sub-min 4`): Q 0.603609, first replay pass at the cap. So
  synchronous sub-rounds also oscillate on these levels, but the frozen decisions did not
  collapse in these runs. Not recorded as JSON lines (printed by hand, command in "Commands").

### 4. Thread-count drift on lj and orkut

Async, 1 iteration, f64 build, `--lowmem`, 8G, warm cache. Six order seeds per cell (at one
thread the order seed is the only source of variation). `results/small.jsonl`,
`small-heavy.jsonl`.

| graph | algorithm | 1 thread | 4 threads | 16 threads | 16 threads, `--heavy 0.001` |
|---|---|---|---|---|---|
| lj | Louvain | 0.7491 [0.7476, 0.7514] | 0.7491 [0.7465, 0.7511] | 0.7481 [0.7469, 0.7491] | 0.7486 [0.7471, 0.7511] |
| lj | Leiden | 0.7504 [0.7486, 0.7534] | 0.7502 [0.7483, 0.7537] | 0.7503 [0.7490, 0.7516] | 0.7497 [0.7485, 0.7509] |
| orkut | Louvain | 0.6702 [0.6634, 0.6761] | 0.6697 [0.6641, 0.6769] | 0.6675 [0.6597, 0.6710] | 0.6674 [0.6617, 0.6755] |
| orkut | Leiden | 0.6752 [0.6631, 0.6836] | 0.6732 [0.6675, 0.6834] | 0.6749 [0.6660, 0.6832] | 0.6733 [0.6637, 0.6872] |

**The orkut drift (Leiden 0.6836 at 1 thread → 0.6743 at 32) is visiting-order variance, not a
thread effect.** At one thread the order seed alone moves orkut Leiden over 0.6631–0.6836
(0.6836 is seed 0, today's order, the top of that range). The 16-thread mean is 0.0003 below the
1-thread mean. On these graphs no pass reaches the cap. The largest race loss is on lj Louvain
at 16 threads: a mean of 0.021 (claimed minus true gain, summed over a run). The heavy rule makes
it 0 and does not change Q beyond the seed spread. 32 threads were not run (shared machine).

### 5. Leiden on Friendster

Async Leiden, 1 iteration, 16 threads, `m102` (f64, compressed level 0 mapped lazily, coarse
levels spilled to scratch, the m1-04 command), `MemoryMax=24G`, variants interleaved, 3 rounds.
`results/leiden-ooc.jsonl`.

| variant | Q per run | passes at the cap |
|---|---|---|
| none | 0.6066, 0.6044, 0.6062 | 0 |
| `--heavy 0.001` | 0.6056, 0.6047, 0.6045 | 0 |

**Leiden did not show the collapse in these 6 runs**, and no pass reached the cap. A likely
reason, not tested: Leiden aggregates the refined partition and starts each pass from the
parent communities, so a giant community is not one vertex that has to move; its refined pieces
are already in it. The guard changes nothing measurable here (3 runs each; the spread is 0.002).
Six runs cannot exclude a rare collapse. The earlier in-RAM run at 32 threads
(`lab/b2-leiden/results/big/friendster_leiden_inram34g.json`) has its last pass at the cap with
6,652 moves, the same symptom on a level of 38,852 vertices, with Q 0.6033.

## Hypotheses of the issue

| hypothesis | result |
|---|---|
| Herding on stale community totals at many threads | **Confirmed, on the coarse levels, by a few heavy vertices** (section 2: none at 1 thread, from 8 threads on; the per-move log). Not in pass 0: no heavy vertex there. Deterministic mode (frozen decisions) gives 0.6036 every time. |
| Oscillation on coarse levels | **Confirmed**: back moves (a vertex returning to the community it left within two iterations) reach 10⁴–10⁶ per run without the guard and 10²–10³ with it; Q swings by up to 0.36 inside a pass. |
| f32 quotient weights | **Ruled out**: same distribution as f64 at 16 threads; identical results at 1 thread. |
| Iteration cap, tolerance schedule and graph size | **Part of the mechanism, not the cause**: the claimed gain stays above the tolerance, so passes run to the cap; a lower cap makes it worse (section 3). |
| Page-cache thrash under 16 GiB | **Ruled out**: collapses under 24G as under 16G; replays have the graph in RAM (11 M entries) and collapse too. Unlimited runs were not made (the brief caps limits at 24G). |
| `--min-ratio 3` | **Ruled out by the code**: level compaction runs after `run()` and composes levels; the flat partition is the same. |
| Machine state | **Not a separate cause**: collapses happen in replays at load average ~2 and in full runs at 15–50. |

## Recommendation for #57

1. **Ship the asynchronous `fast` mode, but only with the heavy-vertex rule.** Leiden showed no
   collapse here, but its local moving is the same code as Louvain's, so the rule belongs in it.
   In every asynchronous move sweep, vertices with k_u > θ·2m (θ = 0.001) are skipped by the
   parallel sweep and visited after it by one thread, in id order. It is a few lines and a list
   of at most 1,000 ids per level. It changes nothing on a level whose largest vertex volume is below
   0.1% of 2m, such as level 0 of Friendster.
2. Keep the 20-iteration cap only as a bound on work. Do not use it, a true-gain stop or a
   rollback as the guard: none of them removes the cycle (section 3).
3. Add a test that would catch a regression: on a graph with two or more vertices of large
   volume joined by few edges (a coarse level is enough: two stars, or the level-1 dump of a
   real graph), asynchronous mode at 16 threads must give, over 20 runs, a minimum Q within
   0.001 of the 1-thread result. Without the rule this test fails on the Friendster dump: 25 of
   140 unguarded replays at 16 threads ended below 0.55. A small synthetic graph that triggers
   the race was not built here.
4. The deterministic mode is not affected in these runs, but #18's sub-round oscillation on
   coarse levels is the same family (vertices of one sub-round do not see each other). The
   same rule — heavy vertices in their own sequential step — is deterministic and is the
   natural fix to try there; it was not implemented for the deterministic branch here.

## Method

- Program: `lab/b2-leiden` (`b2`), built twice: `w32` (f32 quotient weights, as both original
  runs) and the default f64 build. New flags, defaults unchanged (lj, 1 thread, 1 iteration:
  hash `4614ee3ca1c6b9b1`, Q 0.747635, as in
  `lab/b2-leiden/results/scaling/lj_louvain_async_t1.json`):
  - `--trace`: per local-moving iteration of passes ≥ 1 (`--trace-q0` adds pass 0): moves,
    claimed gain (the sum of the gains each move saw when it was decided), true modularity
    before and after (recomputed on the level graph), the top community's share of 2m, and
    back moves. Race loss = claimed gain − true change; it is 0 up to rounding at one thread.
    With `--verbose`, every move of a vertex above 1% of 2m is printed.
  - `--dump-level1 DIR`: writes the level-1 quotient graph (glab-csr-v0 with `weights.f64`,
    and `nodes.u32`, the input-vertex count of each vertex) after pass 0. `Graph::load` now
    reads `weights.f64` when present. `--node-weights FILE` reports the largest community in
    input vertices (`largest_nodes`).
  - `--order-seed S`: rotates the block permutation of the visiting order (0 = today's order).
  - Guards: `--heavy θ`, `--rollback`, `--true-stop`, and the existing `--max-iters`.
- `lab/m1-02-ooc-leiden` (`m102`) got the same `--heavy` for the Leiden check on Friendster:
  b2's Leiden needs 23.6 GB anonymous memory in RAM, over this run's 24 GiB limit.
- Every run under `systemd-run --user --scope -p MemoryMax=… -p MemorySwapMax=0`: Friendster
  16G or 24G, replays and lj/orkut 8G. At most 16 threads. Graphs: raw CSR glab-csr-v0
  (`$EVERYGRAPH_DATA/csr`), compressed store for m102 (`store-m113/friendster`).
- `run.py` writes one JSON line per run: argv, limit, build, load, wall time, and b2's JSON
  (which has the per-pass, per-iteration trace). `summarize.py` prints the tables.
- "Back moves" and "race loss" cover the traced passes only: passes ≥ 1 of full runs; replay
  passes ≥ 1 in `replay-base`, `replay-t16`, `replay-t16-guards` and `replay-heavy-threads`;
  every replay pass in `replay-t16-guards-q0`. Q and the cap counts cover every pass.

## The two original runs, exactly

| | FINDINGS § B2 row | tracer row (m1-01) |
|---|---|---|
| result file | `lab/b2-leiden/results/big/friendster_louvain_semiext16g.json` | `lab/m1-01-tracer/results/b2_louvain_plain_16G.json` |
| command | `lab/b2-leiden/run_friendster.sh 16G louvain semiext16g`: `target-w32/release/b2 run --graph csr/friendster --algo louvain --threads 32 --lowmem --no-populate --verbose --json …` | `tracer.py` (`B2_FLAGS` + `HIER_FLAGS["plain"]`): `b2 run --graph csr/friendster --name friendster --algo louvain --threads 32 --lowmem --no-populate --min-ratio 3 --verbose --out-hier … --json … --notes …` |
| limit | `MemoryMax=16G MemorySwapMax=0`, `timeout 540`, inside `flock bigmem.lock` and `bench.lock` | `MemoryMax=16G MemorySwapMax=0`; input evicted from the page cache before the run |
| build, machine | w32, shared | w32, idle |
| Q, largest | 0.4647, 33,434,257 | 0.6038, 19,829,005 |
| iterations per pass | 5, 20, 20, 20, 20 | 5, 6, 3, 7, 20 |

`--min-ratio 3` only merges levels after the run (`compact_levels` is called after `run()`; the
top level is always kept), so it cannot change Q. `--out-hier` keeps the quotient graphs in
memory and writes them; local moving is the same code. Here the tracer flags are run without
`--out-hier` and without eviction.

## Commands

    cd lab/m1-03-louvain-collapse
    # builds (scratch targets; run.py reads B2_F64, B2_W32, M102 to override)
    S=/home/tux/graphlab/scratch/m1-03-louvain-collapse
    (cd ../b2-leiden && cargo build --release --target-dir $S/target \
        && cargo build --release --features w32 --target-dir $S/target-w32)
    (cd ../m1-02-ooc-leiden && cargo build --release --target-dir $S/target-m102)
    python3 run.py --plan dump --threads 16 --mems 24G --out results/dump.jsonl
    python3 run.py --plan replay --graph-dir $S/L1-t16 --threads 1 2 4 8 16 --seeds 10 --out results/replay-base.jsonl
    python3 run.py --plan replay --graph-dir $S/L1-t16 --threads 16 --seeds 40 --out results/replay-t16.jsonl
    for f in "--heavy 0.01" "--heavy 0.001" "--max-iters 5" "--det"; do
      python3 run.py --plan replay --graph-dir $S/L1-t16 --threads 16 --seeds 40 --builds w32 --flags="$f" \
        --out results/replay-t16-guards.jsonl; done
    for f in "--rollback" "--true-stop" ""; do
      python3 run.py --plan replay --graph-dir $S/L1-t16 --threads 16 --seeds 40 --builds w32 --flags="$f" \
        --out results/replay-t16-guards-q0.jsonl; done
    for f in "--heavy 0.001" "--heavy 0.01"; do
      python3 run.py --plan replay --graph-dir $S/L1-t16 --threads 4 8 --seeds 20 --flags="$f" \
        --out results/replay-heavy-threads.jsonl; done
    python3 run.py --plan full --reps 5 --threads 16 --mems 16G 24G --out results/full.jsonl
    python3 run.py --plan full --reps 3 --threads 16 --mems 16G --flags="--heavy 0.001" --out results/full-heavy.jsonl
    python3 run.py --plan small --threads 1 4 16 --seeds 6 --builds f64 --out results/small.jsonl
    python3 run.py --plan small --threads 4 16 --seeds 6 --builds f64 --flags="--heavy 0.001" --out results/small-heavy.jsonl
    python3 run.py --plan leiden-ooc --reps 3 --threads 16 --mems 24G --out results/leiden-ooc.jsonl
    # deterministic sub-round sweep (section 3), by hand:
    for sr in 2 4 16 64; do $S/target/release/b2 run --graph $S/L1-t16 --algo louvain --threads 16 --det \
        --sub-rounds $sr --tol 0.001 --max-passes 9 --trace --trace-q0 --node-weights $S/L1-t16/nodes.u32; done
    python3 summarize.py results/replay-t16.jsonl results/replay-t16-guards.jsonl --group build,threads,flags

`results/replay-t16-guards.jsonl` first also held `--rollback`, `--true-stop` and `--heavy 0.001
--rollback` rows run without `--trace-q0`. Those guards were then off in replay pass 0, so the
rows were removed; the q0 file replaces them.

## What was not done

- No run at 32 threads (the brief allows 16 on tonight's shared machine), no unlimited-memory
  run, no idle-machine run of the full graph. Replays at load ~2 cover the idle case for the
  coarse levels only.
- No timing for the record. The guard's cost was not measured; the only argument is that it
  touches at most 1,000 vertices per level and none on level 0 here.
- The heavy rule was not implemented for the deterministic branch or for Leiden's refinement.
- θ was tried at 0.01 and 0.001 only. Other graph families (web, road, bipartite) were not run.
- One level-1 dump only (one pass-0 result). The full runs vary pass 0 and agree with it.
