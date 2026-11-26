---
title: Chunked records for hub lists: restart points inside long adjacency lists
milestone: M1 · De-risking lab, round 2
labels: type:experiment, area:format, P1, risk:medium, size:M, lab-evidence
---
**Question.** Our format never splits a list: a list longer than 65,536 entries gets a block of its own (`docs/spec/eg-adjacency.md` § 4). One hub's list is therefore decoded whole, by one task. Ligra+ and GBBS store lists above T ≈ 1,000 entries as chunks: each chunk's first word is relative to the source, and chunk offsets live in the record. The chunks are decoded in parallel, and intersections can skip by chunk start. Should lists above about 1,024 entries be chunked?

**What the literature and our lab say** (`docs/research/sota-storage.md` § 3 D4 "Hub lists", § 4 challenge 3; `docs/research/sota-kernels.md` § 4 challenge 4; paper numbers, their machines):
- Ligra+ (DCC 2015, 40 cores) is "up to 2 times slower" on some application/graph pairs without chunking, with the same performance for T from 100 to 10,000. GBBS builds map, filter, pack and intersection on these chunks for its 225 B-edge runs.
- Ours (FINDINGS § M1-13, idle): BFS is 1.20–1.52× raw at 16 threads, budgeted 3-hop 2.1× on twitter-sym, Friendster WCC 1.36–1.57×. Part of each is whole-hub decode (not separated yet).
- The cost, by arithmetic: about 0.03 bits/entry at T = 1,024 (one 4-byte offset per 1,024 entries).

**Method.** Lab prototype, interleaved A/B chunked against unchunked in one binary, idle machine, 16 threads (and 1 thread for decode cost), under `systemd-run --user --scope -p MemoryMax=… -p MemorySwapMax=0`.
- [ ] Graphs: twitter-sym and Friendster
- [ ] Kernels: BFS; 3-hop with a 10k budget; common neighbours; triangle forward lists; PageRank as a control (expected unchanged)
- [ ] T ∈ {256, 1,024, 4,096}; bits/entry with the chunk offsets; cut points from a degree prefix sum only, so task boundaries are data-defined and results identical at any thread count (D14)
- [ ] Format note for the decision: a codec value or a version 2 of `eg-adjacency`, since § 4 says "no list is ever split"; whether {{#m2-01-container-spec}} or {{#m2-03-adjacency-codec}} must reserve anything now

**Done when.** A table kernel × graph × T with time and bits/entry against unchunked, and a go/no-go on chunked hub records (with the format change it implies) recorded in FINDINGS.

**Feeds.** `docs/spec/eg-adjacency.md` (a version 2 or a new codec value; version 1, from the closed {{#m2-03-adjacency-codec}}, never splits a list), {{#m3-01-edge-map}} (edge-balanced tasks that split one hub list), {{#m3-08-triangles}}.

**Where it plugs in (edge-map, #44).** Splitting one hub list across tasks needs restart points in long compressed lists; in the framework it plugs into the task cut (`Plan::with`) plus an entry range on `Source::scan`.

**Pointers.** `docs/research/sota-storage.md` § 4 challenge 3; `docs/research/sota-kernels.md` § 3 primitives table ("Edge-balanced tasks"), § 4 challenge 4; `lab/m1-13-compressed`.

**Depends on** {{#m1-13-compressed-end-to-end}}
