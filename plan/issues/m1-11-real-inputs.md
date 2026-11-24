---
title: Real-world inputs at billion scale: string ids, Parquet, compressed files
milestone: M1 · De-risking lab, round 2
labels: type:experiment, area:ingest, P1, risk:medium, size:M, lab-evidence
---
**Question.** The ingest prototype handles integer ids below 2^32 from plain text. Real inputs are Parquet or compressed CSV with string or hex identifiers. What is the end-to-end rate, and where is the bottleneck?

**What the lab found.** Parse-only reaches 7–10 GB/s; gzip inflate is the bottleneck by 10–20× (0.3–0.6 GB/s); the first distinct-key pass runs at ~15 M keys/s (≈ 4 minutes extrapolated for 3.6 B endpoints) and was not tested with spilling; the MPHF maps 132 M keys/s afterwards.

**Method**
- [ ] Generate Friendster-shaped inputs with (a) 42-byte hex addresses, (b) URL-like strings; as plain CSV, gzip, zstd and Parquet (dictionary-encoded and plain)
- [ ] Prototype the dedupe pass with hash partitioning and disk spill under 8 and 16 GiB budgets
- [ ] Exploit Parquet dictionary pages to avoid re-hashing repeated keys
- [ ] Measure total time, peak memory, temp disk; compare with DuckDB doing the same relabeling

**Success.** Friendster-shaped input with hex ids → `.eg` in < 4 minutes under a 16 GiB budget; otherwise a clear statement of the achievable rate.

**Feeds.** {{#m2-07-id-mapping}}, {{#m2-06-parquet-reader}}.
