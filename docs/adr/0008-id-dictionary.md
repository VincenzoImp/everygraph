# ADR 0008 — ID dictionary: typed keys and a minimal perfect hash

- **Status:** accepted, partly validated
- **Decision:** D8 in [DESIGN.md](../DESIGN.md#4-decisions)
- **Date:** 2026-11-25

## Context

Real inputs name nodes with integers, hex strings (blockchain addresses) or arbitrary
strings (URLs, user names). Ingest must map billions of endpoint occurrences to dense ids
(ADR 0003) and results must come back with the original ids.

## Decision

Keys are typed. Integers map through a rank bitmap; hex keys are stored as fixed-width
bytes; strings use a minimal perfect hash (`ptr_hash`) plus an id-ordered key store; an
optional front-coded index gives prefix search. No FST by default (ADR 0000).

## Evidence

From the identifier experiment (FINDINGS § F1; shared machine):
- **MPHF at 500 M keys** (20-byte binary keys, 32 threads;
  `lab/f1-query/results/dict_mphf_ethbin_500000000_t32.json`): MPHF built in 10.8 s; the
  whole dictionary with its key store in 19.4 s (DESIGN's earlier "built in 11 s" was the MPHF
  alone; corrected on 2026-11-25); 2.61 bits/key (163 MB); warm forward lookup p50 0.56 µs; batch mapping in the
  ingest test 132 M lookups/s. FINDINGS marks the 132 M/s as unverified; DESIGN dropped that
  qualifier until on 2026-11-25.
- **FST at 65 M keys** (single-threaded build): 1.91× the raw key bytes on random binary
  keys (113 s), 1.03× on hex text (201 s), 0.28× on URL-like keys (53 s)
  (`lab/f1-query/results/dict_fst_ethbin_65000000_t4.json`,
  `lab/f1-query/results/dict_fst_eth_65000000_t4.json`,
  `lab/f1-query/results/dict_fst_user_65000000_t4.json`). The FST is the smallest form on
  URL-like keys; it loses on random keys and on build time.
- **Front-coded sorted blocks**: prefix search in about 3 µs at 18 bytes/key on URL-like
  keys (FINDINGS § F1; `lab/f1-query/results/dict_fc_user_65000000_t4.json`).
- **First dedupe pass**: about 15 M keys/s, which is about 4 minutes for 3.6 B endpoints —
  an extrapolation, not a measurement (FINDINGS § F1;
  `lab/f1-query/results/dict_dedupe_eth_65000000_t32.json`). Spilling was not tested.
- **Integer keys through a rank bitmap**: not measured in the lab.

## Consequences

- #34 builds the dictionary with a spilling dedupe; its acceptance (65 M hex keys in
  < 15 s at 16 threads, 3.6 B mappings in < 60 s, a 500 M-key build under 4 GiB) tests
  the parts the lab did not.
- The ingest bottleneck for string-id inputs is the dedupe pass, not the lookup; #25
  measures it on Friendster-shaped inputs.

## What would change our mind

- #25 or #34 show that the MPHF path cannot map 3.6 B endpoints in < 60 s, or cannot be
  built under a 4 GiB budget.
- String-id inputs with long shared prefixes turn out to dominate, and an FST built in
  parallel beats the MPHF plus key store on size by more than 2× at comparable build time.

## Closed by

#34 (dictionary with spilling dedupe), #25 (real-world inputs: string ids, Parquet,
compressed files).
