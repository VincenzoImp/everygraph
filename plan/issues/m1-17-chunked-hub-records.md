---
title: Chunked records for hub lists: restart points inside long adjacency lists
milestone: M1 · De-risking lab, round 2
labels: type:experiment, area:format, P0, risk:medium, size:M, lab-evidence
---
**Question.** Our format never splits a list: a list longer than 65,536 entries gets a block of its own (`docs/spec/eg-adjacency.md` § 4). One hub's list is therefore decoded whole, by one task. Ligra+ and GBBS store lists above T ≈ 1,000 entries as chunks: each chunk's first word is relative to the source, and chunk offsets live in the record. The chunks are decoded in parallel, and intersections can skip by chunk start. Should lists above about 1,024 entries be chunked?

**What the literature and our lab say** (`docs/research/sota-storage.md` § 3 D4 "Hub lists", § 4 challenge 3; `docs/research/sota-kernels.md` § 4 challenge 4; paper numbers, their machines):
- Ligra+ (DCC 2015, 40 cores) is "up to 2 times slower" on some application/graph pairs without chunking, with the same performance for T from 100 to 10,000. GBBS builds map, filter, pack and intersection on these chunks for its 225 B-edge runs.
- Ours (FINDINGS § M1-13, idle): BFS is 1.20–1.52× raw at 16 threads, budgeted 3-hop 2.1× on twitter-sym, Friendster WCC 1.36–1.57×. Part of each is whole-hub decode (not separated yet).
- The cost, by arithmetic: about 0.03 bits/entry at T = 1,024 (one 4-byte offset per 1,024 entries).

**Before the format freezes (decided 2026-11-26, `docs/research/sota-review-2026-11.md` S10).** Chunked records change the adjacency format, so this experiment reports before the `.eg` format is frozen for 0.1 ({{#m2-36-format-freeze}} depends on it). It is now P0. The reasons, from the review:
- Splitting hub lists in the frontier driver (S3, {{#m3-22-performance-pass}}) works on raw lists. On a compressed source, one hub's list still decodes serially without restart points in the record.
- On twitter-sym, 41.6% of the entries are in lists of more than 1,024 entries, and 11.2% in lists of more than 65,536 (review's count from the offsets, `docs/research/sota-review-2026-11.md` S10).
- The counts of {{#m3-27-frontier-counters}} (entries decoded against entries examined for compressed BFS and WCC; the share of entries in single-thread runs) give the size of the gain before any timing. Read them first. If the decode surplus is below 2× and hub entries are a small share of single-thread runs, the go/no-go can be "no" without the timed A/B.

**The counts of {{#m3-27-frontier-counters}}** (exploratory counts of work, not times; compressed source, 16 threads; `lab/m3-27-frontier-counters/README.md`, PR #278, "Decode surplus"):
- BFS, entries decoded against entries examined: 8.47× (orkut), 16.25× (twitter-sym), 20.60× (Friendster). Both parts are above 1: whole blocks decoded for one passing node (decoded / handed, 2.54–3.54×) and the tail of each list after the last entry the body needs (handed / examined, 2.68–6.41×).
- WCC: 1.95× (orkut), 1.98× (twitter-sym), 2.75× (Friendster). The prefix part is 1.93–1.95×.
- So the surplus is above the 2× bar for BFS on all three graphs. For WCC it is below the bar on orkut and twitter-sym; on Friendster it is above it because of block decode in WCC's finish (decoded / handed 1.42×), not because of the list tails.

Prefix decode (S10) is being built under {{#m3-22-performance-pass}}. It removes the handed / examined part without a format change. The go/no-go here reads these counts and what is left of them after prefix decode: chunked records are needed only if the surplus that remains is in long hub lists that one task decodes serially.

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
