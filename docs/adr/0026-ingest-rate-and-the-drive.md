# ADR 0026 — The ingest rate of promise 1, and the drive

- **Status:** accepted, partly validated
- **Decision:** promise 1 ("open anything fast") in [DESIGN.md § 2](../DESIGN.md#2-the-product)
- **Date:** 2026-11-25
- **Validated:** one machine and one drive (Samsung 990 PRO 2 TB, ext4), the lab prototype
  `glab` writing uncompressed arrays without `fsync`, three social graphs. Not validated:
  the product's build pipeline writing the default compressed format (#35), other drives,
  ingest with `fsync`.

## Context

Promise 1 said that a file becomes an analysis-ready graph "at about 100 M edges/s", backed
by the lab's runs of 2026-11-24 (one run each; lj and orkut the best of 3): lj 0.42 s,
orkut 0.89 s, Friendster 21.1 s with a 32 GB budget (86 M edges/s) and 52.4 s with 8 GB
(34 M edges/s; `lab/b1-core/results/bench/ingest.jsonl`). The benchmark harness (#4)
repeated these runs on an idle machine on 2026-11-25. lj and orkut came out faster.
Friendster at 32 GB came out about 40% slower than the lab's run on every repetition but
the first (29.06 s against 21.1 s), and at 8 GB faster (36.97 s against 52.4 s). The
investigation (PR 136) traced the 32 GB difference to the drive: the first ingest after a
light period meets a rested drive, the following ones its sustained write rate. A promise
quoted as one rate has to say which of the two it means.

## Decision

Promise 1 keeps **about 100 M edges/s** as the ingest rate while what the ingest writes fits
in memory or in the drive's fast-write burst, and states plainly that a graph whose ingest
writes more is **bounded by the drive's sustained write rate**: about 60 M edges/s for
Friendster (1.8 B edges) with a 32 GB budget on the lab machine, 49 M edges/s with 8 GB.
Every quoted ingest time names which case it is (sustained, after a warm-up, or rested
drive), and the sustained figure is the one targets are checked against. The harness's
write-heavy cases run one untimed warm-up repetition so that every measured repetition
meets the sustained rate.

## Evidence

All from the benchmark harness, idle machine, 2026-11-25, official runs (disturbance check:
others ≤ 0.31 cores and ≤ 0.5 MB/s in every window): `glab ingest`, text edge list →
validated symmetric uncompressed CSR, 32 threads, no `fsync`, output memory-mapped, each
repetition a separate process. Files: run 1
`bench/results/headline/20261125T101402Z-79634d76b95b.jsonl`, run 2
`…/20261125T104322Z-79634d76b95b.jsonl`, re-take with a warm-up
`…/20261125T114658Z-0ffe756ac9ac.jsonl` (FINDINGS "Idle-machine re-runs" and § B1).

| graph, budget | case | seconds | M edges/s | rows |
|---|---|---|---|---|
| lj (34.7 M edges), 8 GB | text in page cache, output small | 0.386 (0.370–0.395), median of 6 | 90 | runs 1 and 2, rows 1–3 |
| orkut (117.2 M), 8 GB | same | 0.781 (0.754–0.806), median of 6 | 150 | runs 1 and 2, rows 4–6 |
| Friendster (1.806 B, 31 GiB of text, `O_DIRECT` input), 32 GB | rested drive | 17.7 and 19.5 | 102 and 92 | runs 1 and 2, row 37 |
| Friendster, 32 GB | sustained (after a warm-up) | 29.06 (27.52–29.56), median of 3 | 62 | re-take, rows 1–3 |
| Friendster, 8 GB | sustained (after a warm-up) | 36.97 (35.06–37.14), median of 3 | 49 | re-take, rows 4–6 |

- **The CPU is not the bound on Friendster.** Every row records the cgroup's bytes written:
  45.1 GB per ingest at 32 GB, 55.4 GB at 8 GB. The rested ingests used 14–16 cores on
  average; the sustained ones 7.8–9.3 of 32 (user + system time over wall time, same rows).
- **The drive's two rates** (exploratory, not in a committed result file; from the
  investigation of PR 136): a 64 GB `O_DIRECT` write probe wrote the first ~10 GB after 5
  idle minutes at 5.3 GB/s, then 1.4–1.6 GB/s, at 49–55 °C with no thermal throttling.
  `/proc/pressure/io` stall time tracked the slowdown: 9.7 s of stall in a 22.5 s ingest,
  18.3 s in a 33.6 s one. Ruled out in the same investigation: deleting the previous output,
  write-back left over from the previous run, ext4 discard, temperature, and other load.
  The cause is an inference with strong support, not a measurement of the drive's firmware.
- **What "about 100 M" covers.** lj 90, orkut 150 and Friendster on a rested drive 92–102 M
  edges/s: "about 100 M" holds where the writes fit. Sustained Friendster, 49–62 M, does not;
  the promise no longer claims it.
- **Not yet the default storage.** These rates end at uncompressed arrays. Encoding to the
  default compressed storage took a further 0.064, 0.168 and 3.41 s at 16 threads (lj,
  orkut, Friendster; #118, FINDINGS § M1-13), measured separately: by addition, lj 0.45 s
  (77 M edges/s), orkut 0.95 s (123 M edges/s), sustained Friendster at 32 GB 32.5 s
  (56 M edges/s). The encoder measured holds input and output in RAM, so it cannot run under
  an 8 GB budget (ADR 0004).
- **Not measured:** another drive or file system; ingest with `fsync` (one shared-machine
  lj run took 0.68 s instead of 0.37 s, DESIGN § 5.6); the product's pipeline, which writes
  fewer temporary bytes by design (#35: no spill pass, 6-byte bucket entries). The lab's
  52.4 s at 8 GB, slower than the harness's 36.97 s, is not explained; it was a single run.

## Consequences

- DESIGN § 2 and the README state the promise with both rates and their conditions;
  DESIGN § 6 checks the Friendster 8 GB target against the sustained 36.97 s, not the lab's
  52.4 s.
- Bytes written are part of ingest speed on large graphs. The build pipeline (#35) reports
  them next to its time, and its temp-traffic reductions are speed work.
- Ingest targets on large graphs are measured under the harness's warm-up protocol; a
  rested-drive figure may be reported next to the sustained one, never instead of it.
- The plan's targets derived from the lab's 21 s (Friendster ≤ 25 s at 32 GB in #35 and in
  the M2 epic) now sit below the sustained baseline (29.06 s, before encoding); the plan
  records that against the new baseline.

## What would change our mind

A build pipeline that, under the warm-up protocol, ingests Friendster at ≥ 90 M edges/s
(≤ 20 s) on this drive would let the promise drop its second sentence for graphs of that
size. A measurement on another NVMe drive whose sustained rate differs from 1.4–1.6 GB/s
would replace "about 60 M edges/s for Friendster" with a rule in bytes written per second.
Repeated idle measurements of lj or orkut below about 80 M edges/s, on the product's pipeline
including its encoding, would mean the first sentence no longer holds either.

## Closed by

#35 (the build pipeline, measured under the warm-up protocol, writing the default format
and reporting bytes written).
