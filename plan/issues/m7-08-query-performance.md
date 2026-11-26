---
title: M7 performance pass: point-query speed gaps and the runs on the large graphs
milestone: M7 · Queries and views
labels: type:feature, area:query, P1, risk:medium, size:M
---
**Why.** {{#m7-01-point-queries}} closed on correctness and exploratory evidence (CLAUDE.md § 6). Its missed speed targets, and the runs its 8 GiB memory limit did not allow, come here. All numbers below are exploratory: `pq_bench` (`crates/everygraph-query/examples/pq_bench.rs`) on a loaded machine (1-minute load 3–6), raw CSR and compressed lists both in the heap, interleaved rounds in one process; the pull request of #90 holds the command and its output.

**Gaps to close**
- [ ] 3-hop with a limit of 10,000, p50, 1 thread: orkut 163–175 µs on the compressed lists, 151–166 µs on raw arrays, against < 150 µs (and #118's 51 µs, whose budget may count something else: check what the lab's `pq` counted first). lj: 137–153 µs compressed, 123–129 µs raw.
- [ ] Common neighbours p50, 1 thread: orkut 1.61–1.77 µs compressed (raw 0.87–0.90), against < 1.3 µs. The candidates: fewer Elias–Fano look-ups per list (today five: the degree for the query's bound, two degrees and the entry offset in `Compressed::list_prefix`, and the record's byte range), and the check of each decoded list for targets out of range.
- [ ] Neighbour throughput at 16 threads, compressed against raw: 2.1–2.5× on orkut (52–55 M against 113–136 M queries per second), above ADR 0004's 1.5×. A raw copy read by the queries would hold it at 1 where the copy fits; the planner does not choose a tier for queries yet. Without one, the same candidates as above. 3-hop throughput: 1.0–1.2× (within).
- [ ] The acceptance runs not made: twitter-2010 and Friendster (raw arrays above the 8 GiB limit), for the single-thread targets, the 16-thread ratios, and `khop` on their hubs (time against `limit`). On lj and orkut a 2-hop from the largest hub with a 1 ms deadline returned in 1.00–1.10 ms, cut by its timeout, and the time of a cut 2-hop grew with the limit: 1.2–1.9, 10–15, 107–122 µs, 1.6–2.0 ms at limits 10^2, 10^3, 10^4, 10^5.
- [ ] Bidirectional BFS throughput at 16 threads (an acceptance item of #90 that needs `path`): with {{#m7-02-shortest-paths}}.
- [ ] Official runs of all of the above in the M7 window, through a harness spec for `pq_bench`. **The idle run of the 16-thread ratio is ADR 0004's decision test** ("What would change our mind"): a ratio above 1.5× on the default configuration means a new ADR (CLAUDE.md § 5), not a quiet change.
- [ ] The planner chooses how queries read: the tier (a raw copy when it fits and repays), with both adjacencies of a directed graph counted.

**Done when.** Each gap is closed (interleaved A/B, ratio stated) or its remaining cause is written down with a follow-up; the milestone-end official run records the final numbers.

**Depends on** {{#m7-01-point-queries}}
