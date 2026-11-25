---
title: Compressed adjacency codec: Stream VByte gap blocks with Elias–Fano offsets
milestone: M2 · Format and ingest
labels: type:feature, area:format, P0, risk:medium, size:L, lab-evidence
---
**Why.** Compressed adjacency is the default storage (DESIGN D4). The lab bake-off: Stream VByte gaps cost 16.1–18.1 bits/edge on lj, orkut, twitter-sym and 23.2 on Friendster (raw 32.8–35.7; #118, idle), decode 5.0–21.9× faster than BVGraph (9.1–21.9× on social graphs), scan faster than raw at 32 threads on twitter-sym (21.7 vs 13.6 G edges/s), and under an 8 GB cap run 5.2× faster than raw because they stay resident.

**Format (proposed in the lab as "glab-adj-v1")**
- Node-aligned blocks of ~64K edges; a list is never split; a huge list gets its own block
- Per list: `[degree varint][zigzag(first − v) varint][gaps − 1 in Stream VByte]`
- Elias–Fano per-node byte offsets are required (0.12–0.41 bits/edge measured): with a block directory only, one list costs 10–33 µs and orkut BFS at 16 threads 2.24 s instead of 0.028 s (#118)
- An Elias–Fano index of entry offsets (cumulative degrees), 0.11–0.35 bits/edge: degree without touching the data, needed by edge-indexed property columns; BFS at 16 threads 0.99–1.22× of raw with it (#118)
- Per-block flag for an archival encoding (varint + zstd: 2.4 bits/edge on web graphs) with smaller blocks when random access matters

**Scope**
- [ ] Own encoder/decoder with no C dependencies: scalar, SSSE3/AVX2, NEON and wasm `simd128` paths with runtime dispatch (the `stream-vbyte` crate's SIMD path needs nightly)
- [ ] SIMD prefix sum for gap decoding (untested in the lab; estimated 1.5–2× headroom single-threaded)
- [ ] Elias–Fano offsets (own or a small dependency that builds for wasm32)
- [ ] Decode APIs: iterate a list, decode a block into a caller buffer, and "decode whole graph to raw CSR" for the hot tier
- [ ] Decode APIs also offer list-prefix decode (first k targets) and batch decode of consecutive lists into a caller buffer
- [ ] Decide per-list vs block framing before freezing the spec: per-list framing costs lj 1.24–1.40× on single-thread PageRank (#118); block framing removes the per-list branch but complicates random access. Not tested
- [ ] Id-width agnostic gaps (lab: identical bits/edge with ids shifted by 2^33)
- [ ] Fuzzing (cargo-fuzz) of the decoder against truncated and corrupted input; spec in `docs/spec/eg-adjacency.md`

**Acceptance (lab machine, idle)**
- ≤ 17 bits/edge on orkut and twitter-sym, ≤ 18.5 on lj, ≤ 24 on Friendster, both indexes included (measured in #118: 16.66, 16.11, 18.11, 23.19 before the entry-offset index, which adds 0.11, 0.14, 0.35, 0.14 — by addition 16.77, 16.25, 18.46, 23.33; Friendster's ids have no locality — a reordering ({{#m1-09-hierarchy-order}}) may lower it); web graph ≤ 12 with the byte tier and ≤ 3 with the archival tier
- Warm scan ≥ 1.7 G edges/s single-threaded and ≥ 10 G edges/s at 16 threads on twitter-sym
- Random list decode ≤ 0.5 µs warm on orkut
- Decoder never reads out of bounds under fuzzing

**Pointers.** `lab/c-compression/bench` (tool `glc`), FINDINGS § C. Ordering interaction: {{#m1-09-hierarchy-order}}.
