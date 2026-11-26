---
title: Real-world inputs at billion scale: string ids, Parquet, compressed files
milestone: M1 · De-risking lab, round 2
labels: type:experiment, area:ingest, P1, risk:medium, size:M, lab-evidence
---
**Question.** Real inputs are Parquet or compressed CSV with string or hex identifiers. What is the end-to-end rate of the product on them at billion scale, and where is the bottleneck?

**What is known.** First lab (shared machine): parse-only reaches 7–10 GB/s; gzip inflate is the bottleneck by 10–20× (0.3–0.6 GB/s). The production dictionary ({{#m2-07-id-mapping}}) builds 500 M 20-byte keys under a 4 GiB budget in 76.4 s (official, `bench/results/id-dictionary/20261126T042659Z-c0d7f248bb47.jsonl`), with spilling. The lab questions about the dedupe pass are answered by that code; what is left is the whole path on real input shapes.

**Method.** Product code only (`everygraph ingest`), through the harness.
- [ ] Generate Friendster-shaped inputs with (a) 42-byte hex addresses, (b) URL-like strings; as plain CSV, gzip, zstd and, once {{#m2-06-parquet-reader}} reads it, Parquet (dictionary-encoded and plain)
- [ ] Total time, peak memory and scratch under 8 and 16 GiB budgets; the same relabelling in DuckDB as the reference
- [ ] Parquet dictionary pages: does mapping each distinct key once pay?

**Success.** Friendster-shaped input with hex ids → `.eg` in < 4 minutes under a 16 GiB budget (the M2 exit criterion); otherwise a clear statement of the achievable rate and its bottleneck.

**Feeds.** ADR 0008, {{#m2-25-id-mapping-staged}}, {{#m3-22-performance-pass}}.

**Depends on** {{#m2-08-csr-build}}

The Parquet rows wait for {{#m2-06-parquet-reader}}; the others do not.
