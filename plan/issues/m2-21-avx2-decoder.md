---
title: AVX2 adjacency decoder, only if the twitter-sym single-thread scan misses its target
milestone: M2 · Format and ingest
labels: type:feature, area:format, P2, risk:medium, size:S
---
**Trigger.** The official `codec` run's `scan-twitter-sym/t1` below 1.7 G entries/s ({{#m2-03-adjacency-codec}}). The per-record check added after review costs 9–14% at 1 thread (exploratory, `lab/m2-03-codec/results/record-check.jsonl`), so the margin is smaller than before.

**Scope**
- [ ] 8 words per iteration (two 128-bit shuffles), prefix sum across 8 lanes, run-time detection, Miri, agreement with scalar in tests and fuzzing
- [ ] Interleaved A/B against SSSE3 on lj, orkut, twitter-sym; keep only with ≥ 10% gain on twitter-sym at 1 thread, otherwise record the number and close
