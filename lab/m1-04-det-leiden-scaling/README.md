# m1-04 — deterministic Leiden: the sub-round rule (issue #18, first A/B)

Question: does a small constant number of sub-rounds per sweep (Mt-KaHyPar 16, DSLM 4) give
deterministic Leiden the modularity of today's rule (256–1,024 sub-rounds at every level) at a much
lower cost, so that D14's 2.4–3.0× (det/async, 16 threads) falls toward ≤ 1.5×?

**Every timing here is exploratory**, taken on a shared, heavily loaded machine (load average
15–37; other processes kept 3–23 cores busy during the runs, recorded per row as
`others_busy_cores`). Ratios are taken inside a block (same round, graph, threads, iterations; the
variants of a block run back to back in shuffled order) and are the decision evidence; absolute
seconds are not quotable. Modularity, partition hashes, sub-round counts, iterations and scanned
work do not depend on load and are quotable as lab results.

## Setup

- Program: `lab/b2-leiden` (`b2`, f64 weights) built from this branch; three new knobs for the
  sub-round rule, defaults unchanged: `clamp(next_pow2(n_level / --sub-div), --sub-min, --sub-max)`
  (defaults 16384, 256, 1024; `--sub-rounds S` still forces a constant S), and `--seq-below`
  (default 1024: sub-rounds with fewer active vertices run on one thread). The default rule's hash
  is unchanged (lj 1 iteration `44ae14c7774277eb`, 3 iterations `f2ac2926a639c837`, orkut
  `ca792d246e7f461f`, `68211360786489b2`, as in `lab/b2-leiden/results/scaling` and `quality`).
  Every rule's count is a function of the current level's vertex count only, never of threads or
  budget.
- Graphs: lj, orkut (raw CSR glab-csr-v0, mapped and pre-faulted, warm page cache).
  Every run under `systemd-run --user --scope -p MemoryMax=8G -p MemorySwapMax=0`, ≤ 16 threads,
  not pinned. Time = `time_s`, b2's in-process kernel time: excludes loading and the final
  modularity evaluation; no `fsync`.
- Rules: `auto` (today; 256 at every level of lj and orkut), constants `s4` … `s64`, and three
  level-size rules: `L16k` (div 16384, min 4: ~16k vertices per sub-round), `L64k` (div 65536,
  min 4), `L64k-max16` (div 65536, min 4, max 16 — the issue's "16 on large levels, fewer on
  coarse ones"). Plus `async` in every block.
- `ab.py`: 5 rounds at 4 and 16 threads, 1 round at 1 thread (hash check and 1-thread baseline);
  1 and 3 iterations; 440 runs, `results/ab-lj-orkut.jsonl`.
- `counts.py`: one run per rule with the binary instrumented to count sub-rounds run by all threads
  (3 barriers each) and by one thread, `results/counts-lj-orkut.jsonl`. The counters (and
  `hash_levels_low53` for the harness) were added after the A/B had started, so the A/B rows come
  from the same source without them; every counts row's hash equals the A/B's for its rule.
- `seeds.sh`: modularity of each rule over 6 more schedule seeds (`--bucket-shift` 0–3, 5, 6, which
  changes which vertices share a level-0 sub-round), `results/seeds-lj-orkut.jsonl`.
- Friendster: `ab.py --graphs friendster` runs `lab/m1-02-ooc-leiden`'s `m102` (same local moving
  and sub-round code as b2; the three knobs ported, defaults unchanged) out of core: compressed
  level 0 (`store-m113/friendster`) mapped lazily, sequential aggregation, coarse levels spilled
  to scratch, `MemoryMax=24G`. b2 cannot run it in RAM under 24 GiB (23.6 GB anonymous plus the
  15 GB mapped graph in `lab/b2-leiden/results/big`). One interleaved block at 16 threads
  (`results/ab-friendster.jsonl`) and auto, L64k at 8 threads (`results/ab-friendster-t8.jsonl`).
- `summarize.py` → `results/summary-lj-orkut.txt` / `.json`, `results/summary-friendster.txt` /
  `.json`; `tables.py` renders the tables below from the JSON.

    python3 ab.py --out results/ab-lj-orkut.jsonl --rounds 1                # round 0, 1/4/16 threads
    python3 ab.py --out results/ab-lj-orkut.jsonl --threads 4 16 --rounds 5 # rounds 1-4
    python3 counts.py --out results/counts-lj-orkut.jsonl
    ./seeds.sh
    python3 ab.py --graphs friendster --threads 16 --iterations 1 --rounds 1 \
        --variants auto,L16k,L64k,async --mem 24G --timeout 2400 --out results/ab-friendster.jsonl
    python3 ab.py --graphs friendster --threads 8 --iterations 1 --rounds 1 \
        --variants L64k,auto --mem 24G --timeout 2400 --out results/ab-friendster-t8.jsonl
    python3 summarize.py results/ab-lj-orkut.jsonl --counts results/counts-lj-orkut.jsonl \
        --seeds results/seeds-lj-orkut.jsonl --json results/summary-lj-orkut.json

## Answers

1. **A small constant (4, 8, 16) does not give today's quality at lower cost.** Every one of them
   raises the total work 1.4–2.3× (lj, 1 iteration: 10.4–16.6 level-0 edge passes against auto's
   7.5; orkut 11.5–15.5 against 6.6), because level 0 needs more sweeps and coarse levels stop
   converging: 1–9 levels end at the 20-iteration cap, the sign of synchronous oscillation (DSLM's
   and Mt-KaHyPar's warning). At 4 threads they are mostly no faster than auto (0.97–1.40× its
   time in 10 of the 12 cells). At 16 threads they save 5–40% because they run far fewer
   sub-rounds, but in every cell either L64k or L64k-max16 (a tie on lj, 3 iterations) saves
   more. Mt-KaHyPar's "16 is enough" does not transfer to this local-moving rule on these graphs.
   Their modularity was measured at one schedule seed only. Each value lies inside the
   deterministic seed spread of the other rules (e.g. orkut, 1 iteration: s4 0.6627 against
   0.654–0.686), so quality is not what rules them out; work and non-convergence are.
2. **A rule that follows the current level's size does: `L64k`** = clamp(next_pow2(n_level / 65536),
   4, 1024). Same modularity as auto within the schedule-seed spread (column "Q over seeds" in
   the tables: means differ by ≤ 0.002). Same partition hash
   at 1, 4 and 16 threads. It runs 3.3–3.8× fewer parallel sub-rounds than auto and 12–17× fewer
   in total (lj, 1 iteration: 905 parallel + 239 on one thread, against 3,087 + 15,809). At 16
   threads it takes 0.59–0.73× auto's time, the best of the nine rules in every cell (tied on lj,
   3 iterations). At 4 threads it takes 0.74–1.03×. `s64` and `L16k` sit between auto and L64k.
3. **The ≤ 1.5× target is not reached on lj; orkut is at the border** (exploratory, loaded machine;
   in-block medians, 16 threads): det/async for L64k is lj 2.04 (1 iteration) and 1.86 (3), orkut
   1.53 and 1.43, against auto's 3.39, 3.15, 2.55, 2.09 in the same blocks. Auto's 3.39 on lj is
   above D14's idle 2.4–3.0×: barrier-heavy code suffers more from the 3–23 busy cores of other
   processes than async does. So the absolute ratios here overstate the idle cost, and only the
   official run can say where L64k lands on an idle machine.
4. **Determinism holds for every rule.** For each (graph, iterations, rule), one hash across all
   thread counts (1, 4, 16) and rounds: 36 of 36 combinations identical
   (`results/summary-lj-orkut.txt`, section "Determinism"). Hashes differ between rules, as
   expected. 0 disconnected communities in every run.
5. **Where the remaining cost is** (1 iteration, 16 threads, medians of 5, seconds, async / auto /
   L64k). lj: level-0 local moving 0.33 / 0.72 / 0.69; coarse-level local moving 0.17 / 1.90 /
   0.76; refinement 0.10 / 0.39 / 0.30; aggregation 0.44 / 0.45 / 0.49. orkut: 0.81 / 1.44 / 1.22;
   0.16 / 2.40 / 0.65; 0.30 / 0.74 / 0.55; 0.99 / 0.93 / 1.03. L64k removes most of the coarse-level
   cost of auto. What is left there is non-convergence, not barriers: with fewer sub-rounds,
   coarse levels end at the iteration cap (lj 6 levels, orkut 2; async 0). So the next lever on
   coarse levels is oscillation control. Level 0 (1.5–2.1× async) and refinement (1.8–3×) are not
   touched by any sub-round rule.
6. **At 1 thread** deterministic Leiden (auto) costs 0.95× async on lj and 1.24× on orkut (16.5 s
   vs 17.3 s, 42.9 vs 34.6; one round, loaded): most of the price is in parallel execution.
7. **Something not asked but found: deterministic modularity on orkut is about 1 point below
   async at 1 iteration**, whatever the rule. Over 7 schedule seeds, every det rule averages
   0.6702–0.6725 (single seeds 0.654–0.686, sd 0.007–0.009), while async averages 0.6816 over 11
   runs (0.676–0.686). At 3 iterations: 0.6851–0.6889 against 0.6939. On lj deterministic is
   slightly higher: 0.7498–0.7511 against 0.7494 at 1 iteration, 0.7665–0.7681 against 0.7649
   at 3. The earlier single-run comparison (orkut det 0.6760 vs async 0.6689,
   `docs/research/sota-communities.md` D14 row) was one draw on each side; with more draws it
   reverses on orkut. Deterministic modularity also varies 2–3× more from seed to seed than
   async varies from run to run.
8. **Friendster** (first det-vs-async comparison with the new rules; m102 out of core under
   `MemoryMax=24G`, 16 threads, 1 iteration, one run each, loaded: others 11–15 cores busy).
   det/async: auto 1.44, L64k 1.31, L16k 1.49. L64k takes 0.91× auto's time. Its level 0 is
   identical to auto's (65.6 M vertices → 1,024 sub-rounds under both rules); the gain is level 1
   (local moving 15.3 s against 28.9 s). Modularity 0.6055 (auto), 0.6057 (L64k, L16k, async).
   Hashes identical at 8 and 16 threads for auto (`2a12e5882874c38d`, which is m1-02's earlier
   deterministic Friendster hash) and L64k (`fe95529b65bab316`). These ratios are diluted by the
   sequential out-of-core aggregation (≈ 36 s of level-0 aggregation in every variant) and come
   from a single run. They are not a "Friendster meets 1.5×" claim.

## Tables (lj, orkut; from results/summary-lj-orkut.json via tables.py)

det/async time at 16 threads (in brackets: time / auto's), median of in-block ratios over rounds

| rule | lj, 1 it. | lj, 3 it. | orkut, 1 it. | orkut, 3 it. |
|---|---|---|---|---|
| auto | 3.39 (1.00) | 3.15 (1.00) | 2.55 (1.00) | 2.09 (1.00) |
| s4 | 2.85 (0.86) | 2.09 (0.64) | 2.22 (0.95) | 1.91 (0.89) |
| s8 | 2.25 (0.67) | 1.88 (0.60) | 1.91 (0.81) | 1.76 (0.85) |
| s16 | 2.80 (0.83) | 2.12 (0.67) | 2.07 (0.81) | 1.61 (0.75) |
| s32 | 2.40 (0.77) | 2.02 (0.62) | 1.86 (0.74) | 1.54 (0.82) |
| s64 | 2.51 (0.67) | 2.10 (0.68) | 1.78 (0.78) | 1.65 (0.78) |
| L16k | 2.56 (0.76) | 2.03 (0.68) | 1.65 (0.76) | 1.81 (0.80) |
| L64k | 2.04 (0.62) | 1.86 (0.59) | 1.53 (0.63) | 1.43 (0.73) |
| L64k-max16 | 2.19 (0.66) | 1.89 (0.59) | 1.87 (0.77) | 1.64 (0.77) |

det/async time at 4 threads (in brackets: time / auto's), median of in-block ratios over rounds

| rule | lj, 1 it. | lj, 3 it. | orkut, 1 it. | orkut, 3 it. |
|---|---|---|---|---|
| auto | 1.98 (1.00) | 2.16 (1.00) | 1.81 (1.00) | 1.64 (1.00) |
| s4 | 2.55 (1.29) | 1.97 (0.97) | 2.31 (1.40) | 2.17 (1.23) |
| s8 | 2.01 (1.02) | 1.49 (0.69) | 1.95 (1.12) | 1.78 (1.12) |
| s16 | 2.06 (1.00) | 1.84 (0.89) | 1.90 (1.05) | 1.64 (1.06) |
| s32 | 2.00 (1.01) | 1.78 (0.82) | 1.87 (1.12) | 1.69 (1.03) |
| s64 | 1.88 (1.00) | 1.70 (0.76) | 1.43 (0.86) | 1.65 (1.05) |
| L16k | 1.70 (0.83) | 1.85 (0.93) | 1.49 (0.85) | 1.37 (0.96) |
| L64k | 1.81 (0.90) | 1.84 (0.86) | 1.40 (0.74) | 1.56 (1.03) |
| L64k-max16 | 2.04 (0.99) | 2.10 (0.93) | 2.16 (1.18) | 1.66 (1.01) |

load-independent: work (scanned entries / level-0 entries), iterations, levels at the 20-iteration cap, sub-rounds run by all threads / by one thread, modularity at the default seed and over seeds


lj, 1 iteration(s)

| rule | work | iterations | capped levels | sub-rounds all / one thread | Q (seed 4) | Q over seeds: mean [min, max] (n) | communities | disconnected |
|---|---|---|---|---|---|---|---|---|
| auto | 7.5 | 70 | 1 | 3087 / 15809 | 0.7480 | 0.7506 [0.7475, 0.7560] (7) | 1363 | 0 |
| s4 | 16.6 | 159 | 7 | 367 / 305 | 0.7513 |  | 806 | 0 |
| s8 | 11.1 | 101 | 3 | 440 / 422 | 0.7504 |  | 652 | 0 |
| s16 | 10.4 | 107 | 4 | 542 / 1289 | 0.7500 |  | 564 | 0 |
| s32 | 9.8 | 68 | 0 | 873 / 1519 | 0.7566 | 0.7506 [0.7471, 0.7566] (7) | 1023 | 0 |
| s64 | 7.9 | 54 | 0 | 1194 / 2640 | 0.7513 | 0.7498 [0.7465, 0.7519] (7) | 1051 | 0 |
| L16k | 8.1 | 119 | 5 | 2484 / 400 | 0.7488 | 0.7511 [0.7480, 0.7562] (7) | 1359 | 0 |
| L64k | 9.8 | 132 | 6 | 905 / 239 | 0.7528 | 0.7506 [0.7463, 0.7528] (7) | 1036 | 0 |
| L64k-max16 | 12.1 | 152 | 7 | 493 / 319 | 0.7509 |  | 585 | 0 |
| async | 6.0 | 46 | 0 | — | 0.7483–0.7495 | 0.7494 [0.7483, 0.7500] (11) | 1691 | 0 |

lj, 3 iteration(s)

| rule | work | iterations | capped levels | sub-rounds all / one thread | Q (seed 4) | Q over seeds: mean [min, max] (n) | communities | disconnected |
|---|---|---|---|---|---|---|---|---|
| auto | 13.1 | 122 | 1 | 5628 / 28669 | 0.7651 | 0.7665 [0.7646, 0.7699] (7) | 1227 | 0 |
| s4 | 23.6 | 261 | 9 | 558 / 588 | 0.7700 |  | 861 | 0 |
| s8 | 17.6 | 178 | 4 | 738 / 843 | 0.7691 |  | 685 | 0 |
| s16 | 16.9 | 178 | 4 | 1118 / 2054 | 0.7707 |  | 629 | 0 |
| s32 | 15.4 | 114 | 0 | 1631 / 2653 | 0.7714 | 0.7679 [0.7639, 0.7714] (7) | 1029 | 0 |
| s64 | 13.6 | 104 | 0 | 2385 / 5371 | 0.7700 | 0.7675 [0.7643, 0.7700] (7) | 994 | 0 |
| L16k | 13.9 | 198 | 6 | 4793 / 2133 | 0.7658 | 0.7671 [0.7655, 0.7701] (7) | 1228 | 0 |
| L64k | 15.7 | 240 | 9 | 1715 / 740 | 0.7698 | 0.7681 [0.7647, 0.7715] (7) | 1021 | 0 |
| L64k-max16 | 18.7 | 220 | 7 | 893 / 476 | 0.7711 |  | 658 | 0 |
| async | 11.0 | 97 | 0 | — | 0.7641–0.7653 | 0.7649 [0.7641, 0.7654] (11) | 1417 | 0 |

orkut, 1 iteration(s)

| rule | work | iterations | capped levels | sub-rounds all / one thread | Q (seed 4) | Q over seeds: mean [min, max] (n) | communities | disconnected |
|---|---|---|---|---|---|---|---|---|
| auto | 6.6 | 42 | 0 | 2462 / 8216 | 0.6760 | 0.6702 [0.6539, 0.6818] (7) | 29 | 0 |
| s4 | 15.5 | 108 | 4 | 285 / 182 | 0.6627 |  | 28 | 0 |
| s8 | 11.5 | 67 | 1 | 323 / 272 | 0.6725 |  | 27 | 0 |
| s16 | 11.7 | 55 | 1 | 411 / 567 | 0.6680 |  | 30 | 0 |
| s32 | 9.3 | 46 | 0 | 943 / 689 | 0.6813 | 0.6725 [0.6614, 0.6813] (7) | 31 | 0 |
| s64 | 6.7 | 41 | 0 | 960 / 1906 | 0.6694 | 0.6719 [0.6612, 0.6863] (7) | 29 | 0 |
| L16k | 7.1 | 72 | 2 | 2216 / 152 | 0.6759 | 0.6703 [0.6540, 0.6822] (7) | 30 | 0 |
| L64k | 7.3 | 66 | 2 | 656 / 118 | 0.6699 | 0.6721 [0.6618, 0.6864] (7) | 30 | 0 |
| L64k-max16 | 12.5 | 121 | 5 | 450 / 309 | 0.6682 |  | 26 | 0 |
| async | 8.1 | 33 | 0 | — | 0.6783–0.6821 | 0.6816 [0.6761, 0.6858] (11) | 24 | 0 |

orkut, 3 iteration(s)

| rule | work | iterations | capped levels | sub-rounds all / one thread | Q (seed 4) | Q over seeds: mean [min, max] (n) | communities | disconnected |
|---|---|---|---|---|---|---|---|---|
| auto | 10.9 | 69 | 0 | 5302 / 13945 | 0.6905 | 0.6856 [0.6723, 0.6939] (7) | 27 | 0 |
| s4 | 21.5 | 139 | 4 | 415 / 222 | 0.6744 |  | 28 | 0 |
| s8 | 18.4 | 110 | 1 | 599 / 421 | 0.6931 |  | 26 | 0 |
| s16 | 16.4 | 92 | 1 | 722 / 1036 | 0.6855 |  | 27 | 0 |
| s32 | 14.0 | 77 | 0 | 1498 / 1440 | 0.6938 | 0.6889 [0.6820, 0.6938] (7) | 29 | 0 |
| s64 | 11.9 | 76 | 0 | 1920 / 3830 | 0.6866 | 0.6865 [0.6746, 0.6957] (7) | 28 | 0 |
| L16k | 12.0 | 108 | 2 | 4497 / 1702 | 0.6931 | 0.6851 [0.6715, 0.6942] (7) | 29 | 0 |
| L64k | 11.9 | 98 | 2 | 1412 / 209 | 0.6837 | 0.6869 [0.6761, 0.6960] (7) | 30 | 0 |
| L64k-max16 | 17.1 | 147 | 5 | 743 / 353 | 0.6760 |  | 25 | 0 |
| async | 11.7 | 60 | 0 | — | 0.6925–0.6948 | 0.6939 [0.6922, 0.6954] (11) | 23 | 0 |

## Recommendation for production Leiden (#57)

- Sub-round count per level = clamp(next_pow2(n_level / 65536), 4, 1024): a function of the current
  level's vertex count only. Not a small constant.
- Keep single-thread execution of sub-rounds under 1,024 vertices. A different `--seq-below` was
  not tested: it changes only speed, not the result, and this machine's load made that unmeasurable.
- Before claiming ≤ 1.5× on lj: (a) the official run below; (b) oscillation control on coarse
  levels (the afterburner filter of challenge 11, or rejecting a move whose target vertex moves the
  other way, generalizing the singleton smaller-label rule); (c) level-0 local moving and
  refinement, which stay 1.5–3× async with any sub-round rule (profiling not done: the issue's
  "barrier wait vs decide vs commit" bullet is still open).
- D14's documented price must not change from these numbers. It changes after the official run.

## What needs an official run before it can be quoted

Every time and every det/async ratio above, including Friendster. The modularity values, hashes,
sub-round counts, iterations and work are load-independent lab results and can be cited from
`results/`. The official run is the harness spec `bench/specs/det-leiden.toml`. It covers auto,
s64 and L64k against async at 1, 2, 4, 8 and 16 threads, 1 iteration, plus 3 iterations at 16
threads, on lj and orkut; Friendster under 24 GiB at 8 and 16 threads. On an idle machine:

    everygraph-bench run det-leiden --official --data /home/tux/graphlab/data
    # lj + orkut only (~40 min, estimate):  ... --only scaling --only quality
    # Friendster only (~45 min, estimate): ... --only friendster

## What was not done

- No idle-machine measurement; no profile per phase (barrier wait vs decide vs commit).
- `--seq-below` not varied (see above); the afterburner, colouring and spanning-tree refinement
  candidates not tried (the issue gates them on this A/B failing; it partly did, on lj).
- Seed spread measured only for auto, s32, s64, L16k and L64k (not s4–s16, L64k-max16).
- 1-thread and 4-thread rows: 1 round at 1 thread, 5 at 4 and 16.
