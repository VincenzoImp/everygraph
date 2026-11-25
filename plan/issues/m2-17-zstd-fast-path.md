---
title: Faster zstd input: parallel decode of single-frame files
milestone: M2 · Format and ingest
labels: type:feature, area:ingest, P2, risk:low, size:S
---
**Why.** The text reader ({{#m2-05-text-reader}}, PR #152) decodes zstd with the pure-Rust `ruzstd`. Multi-frame (pzstd) input decodes in parallel (lj 0.65 s), but a single-frame file — what the `zstd` command writes by default — decodes on one core at about 0.3 GB/s (lj 1.57 s), slower than gzip. Exploratory, loaded machine.

**Scope**
- [ ] Measure the options with interleaved A/B runs: overlapping single-frame decode with parsing; an optional native `libzstd` behind a feature (license, `cargo deny`, wasm32 unaffected); a hint in the reading summary suggesting `pzstd` or `zstd -T0 --format=…` for large inputs
- [ ] Adopt the best within the no-C-by-default rule of the workspace, or document the gap

**Acceptance**
- Single-frame zstd at least as fast as gzip on lj at 16 threads, or the summary names the faster input format
