# ADR 0000 — Assumptions the first lab overturned

- **Status:** accepted
- **Decision:** none of D1–D25 alone; it protects D4, D8 and D14 in [DESIGN.md](../DESIGN.md#4-decisions)
- **Date:** 2026-11-25

## Context

The feasibility lab (`lab/LAB.md`) started from working assumptions "to be validated or
refuted". Three of them were refuted by measurement. They are natural defaults that a later
session, a contributor or a reviewer could propose again without knowing they were tested.

## Decision

The three assumptions below are not reintroduced as defaults. Proposing one again needs a
new measurement that answers the evidence recorded here, and an ADR.

1. **An FST as the identifier dictionary.** Replaced by typed keys and a minimal perfect
   hash (ADR 0008).
2. **Raw CSR by default, compressed only when the graph does not fit.** Replaced by
   compressed adjacency as the default storage (ADR 0004).
3. **"Deterministic within 30% of the asynchronous time."** Determinism stays the default
   (ADR 0014), but its price is stated as measured, not as 30%.

## Evidence

**1. FST dictionary** (FINDINGS § F1; shared machine; 65 M keys; FST built with one thread
after a 4-thread sort):
- 20-byte binary random keys (Ethereum addresses as bytes): FST 2.48 GB = 1.91× the raw key
  bytes, built in 113 s (`lab/f1-query/results/dict_fst_ethbin_65000000_t4.json`).
- 42-byte hex text keys: 1.03× raw, 201 s (`lab/f1-query/results/dict_fst_eth_65000000_t4.json`).
- Against it, the minimal perfect hash on the same 65 M hex keys: 2.61 bits/key, built in
  2.1 s at 32 threads, 3.5 s with the key store
  (`lab/f1-query/results/dict_mphf_eth_65000000_t32.json`).
- Caveat kept on purpose: on URL-like string keys the FST was the smallest form measured,
  0.28× raw (`lab/f1-query/results/dict_fst_user_65000000_t4.json`). The FST is rejected as
  the *default* (random keys, single-threaded build), not as a possible prefix index.

**2. Raw by default** (FINDINGS § C, which says this result "overturns the 'raw by default,
compressed when it does not fit' plan"; shared machine):
- Delta + Stream VByte stores social graphs in 16.1–16.7 bits/edge against 32.8–33.1 raw
  (`lab/c-compression/results/bakeoff.jsonl`, orkut and twitter-sym, 32 threads).
- At 32 threads it scans twitter-sym faster than raw: 21,688 vs 13,623 M edges/s (same file).
- Under an 8 GB memory cap, twitter-sym, 6 threads: the compressed form (4.84 GB) stays
  resident and re-scans at 6,437 M edges/s; raw (9.95 GB) is re-read from disk at 1,232 M
  edges/s — 5.2× (`lab/c-compression/results/dev6_cold8g.jsonl`).
- Not settled by this: raw PageRank was 2× faster on orkut (0.026 vs 0.051 s per
  iteration, 32 threads, `bakeoff.jsonl`), and no other kernel was run on the compressed
  form. #118 measures that; if it reverses the result, the change goes through ADR 0004.
  *Settled 2026-11-25 by #118 (ADR 0004, "Result of #118"):* not reproduced — with batched
  decode orkut PageRank on the compressed form is 0.73× of raw at 16 threads and 1.00× at
  1 thread, idle; raw by default stays overturned.

**3. "Within 30%"** (FINDINGS § B2 and "Idle-machine re-runs"; idle; deterministic ÷
asynchronous wall time at the same thread count; `lab/b2-leiden/results/summary.txt`):
- Leiden at 8 threads: 2.51× (lj), 1.96× (orkut); at 16 threads 3.02× and 2.43×.
- Louvain at 8 threads: 1.77× (lj), 1.10× (orkut). The claim "Louvain within 30% up to 8
  cores" holds on orkut only; it never held for Leiden.

## Consequences

- ADR 0008, ADR 0004 and ADR 0014 cite this record; reviewers reject a pull request that
  brings back one of the three defaults without an ADR.
- The cost of determinism is quoted with the measured ratios and the graphs they come from.

## What would change our mind

- FST: a key population where an FST is both smaller than the MPHF plus key store and
  buildable in parallel within the 65 M-key build time of #34.
- Raw by default: #118 showing that, on graphs that fit in RAM, the default configuration
  is slower than raw by more than ADR 0004 allows.
- 30%: #18 bringing deterministic Leiden to ≤ 1.3× asynchronous at 8 and 16 threads on lj,
  orkut and Friendster, idle machine.
