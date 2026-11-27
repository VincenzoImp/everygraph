# ADR 0004 — Compressed adjacency is the default storage

- **Status:** accepted, partly validated
- **Decision:** D4 in [DESIGN.md](../DESIGN.md#4-decisions)
- **Date:** 2026-11-25
- **Amended:** 2026-11-25, with the result of #118 (conditions added to the decision);
  2026-11-25, by #30 (no degree before each record: the degree comes from the entry-offset
  index of condition 4)
- **Validated:** four social graphs in RAM (lj, orkut, twitter-sym, Friendster; idle
  machine). Not validated: web graphs, weighted or directed kernels (#24); ingest writing
  the format under a budget (#35); out-of-core streaming on the format (#46).

## Context

The lab started from "raw CSR by default, compressed when it does not fit" (ADR 0000). The
codec bake-off reversed that for storage. But every kernel, community and point-query
number the product quoted was taken on uncompressed arrays, so the default format had not
been measured end to end until #118 (below).

## Decision

Adjacency is stored compressed by default: per-node records (the first neighbour relative
to the node, then the gaps, coded with Stream VByte) in node-aligned blocks of about 64K
entries, 4 KiB-aligned sections, with our own codec (about 400 lines). *Amended 2026-11-25
by #30:* the records carry no degree; the degree is read from the Elias–Fano index of entry
offsets (condition 4), which would otherwise store it twice — 0.47 bits per entry saved on
lj, 0.12 on orkut (byte layout: `docs/spec/eg-adjacency.md`). The decision holds under five conditions, each
measured by #118:

1. **Per-node Elias–Fano byte offsets.** A block directory alone is not enough.
2. **Sequential access decodes batches of lists** before the kernel body runs, in the
   edge-map framework (#44); never list by list interleaved with the kernel.
3. **Budgeted queries decode list prefixes**, not whole lists (#90).
4. **The format carries an Elias–Fano index of entry offsets**, so that degree is a lookup
   that does not touch the data (#30).
5. **A raw CSR is an in-RAM decode cache, a planner tier (#45)**, built only by the rule
   in "Result of #118", with its break-even.

Per-list framing costs short-list graphs 1.24–1.40× on single-thread PageRank (lj); a
block-framed stream would decode without a per-list branch but complicate random access.
*Settled by #30:* an interleaved single-thread A/B ran block framing 2–5% faster for
PageRank on lj, no faster on orkut, and 12–30% slower for scans and random lists, so the
records stay per list (`docs/spec/eg-adjacency.md` § 8).

## Evidence

What the first lab measured on the compressed form — all in the codec bench of
experiment C, on a shared machine (FINDINGS § C: "32T rows are official runs under the
lock", which serialised official runs while other work kept running):
- **Size**: 16.66 bits/edge on orkut and 16.11 on twitter-sym against 32.84 and 33.11 raw
  (index included); 11.43 against 34.70 on uk-2005 (`lab/c-compression/results/bakeoff.jsonl`,
  `lab/c-compression/results/summary.txt`).
- **Sequential scan, 32 threads**: twitter-sym 21,688 vs 13,623 M edges/s raw; uk-2005
  21,744 vs 12,347; orkut 9,773 vs 12,734. At 1 thread raw is faster on all three
  (orkut 4,715 vs 1,757). Same files.
- **One pull-PageRank iteration, 32 threads, f32**: orkut 0.051 s vs 0.026 s raw (raw 2×
  faster); twitter-sym 0.854 vs 0.833 s (equal); uk-2005 0.084 vs 0.128 s. Same files.
  *Not reproduced by #118* (2026-11-25): with batched decode, orkut PageRank on the
  compressed form is 0.73× of raw at 16 threads and 1.00× at 1 thread (below).
- **Random access to one list**: 300–369 ns vs 93–113 ns raw (orkut, twitter-sym).
- **Under an 8 GB cap**, twitter-sym, 6 threads, repeated scan: compressed (4.84 GB, stays
  resident) 6,437 M edges/s vs raw (9.95 GB, re-read from disk) 1,232 M edges/s — 5.2×
  (`lab/c-compression/results/dev6_cold8g.jsonl`).

The end-to-end measurement is in the next section and in FINDINGS § M1-13.

## Result of #118

**Done (2026-11-25, idle machine, `lab/m1-13-compressed/results/`, pull request 133).**
The store was measured as specified — per-node records of delta + Stream VByte in
node-aligned ~64K-entry blocks, Elias–Fano byte offsets — on lj, orkut, twitter-sym and
Friendster, against raw arrays in the same binary, at 1 and 16 threads, graphs in RAM
(anonymous memory, warm, kernel time only, best of 3 unless FINDINGS § M1-13 notes fewer
repetitions). Before every row, foreign CPU use was at most 0.48 CPUs and free memory at
least 57.3 GiB (`busy_cpus_before`, `mem_available_gb_before`; stage log
`lab/m1-13-compressed/results/stages.jsonl`). Outputs are identical in every configuration
(`lab/m1-13-compressed/results/verify.jsonl`, summary §§ 2–4).

**Size** (`encode.jsonl`): 18.11, 16.66, 16.11 and 23.19 bits/edge on lj, orkut,
twitter-sym and Friendster (raw 35.7, 32.8, 33.1, 33.2). Friendster's ids have no
locality; "16–17 bits/edge on social graphs" does not hold for it, nor for lj.

**Time, compressed decoded on the fly ÷ raw** (`kernels.jsonl`, `kernels_big.jsonl`,
`leiden.jsonl`, `leiden_big.jsonl`; ranges over the four graphs unless a graph is named;
16 threads unless stated):
- PageRank, f32 and f64, 20 iterations: 0.73–0.97; at 1 thread 0.95–1.11 on orkut and
  twitter-sym, 1.24–1.40 on lj.
- WCC (Afforest) 0.88–1.43; at 1 thread 0.96–1.57 (Friendster 1.57).
- BFS (direction-optimizing) 1.20–1.52; at 1 thread 1.78–2.04.
- k-core 0.88–1.09 (orkut at 16 threads not trusted: 0.22–0.36 s in every configuration);
  triangles 0.89–1.03 (twitter-sym and Friendster one repetition).
- Leiden, one iteration: async 0.995–1.04, deterministic 0.99–1.02; at 1 thread 1.06–1.09
  (lj, orkut). Deterministic hashes identical on raw and compressed.
- Point queries (`pq.jsonl`; 1 thread, in-process): neighbours +0.02–0.11 µs per call on
  timer-free batch means (2.5–4.2×); bidirectional BFS p50 1.13–1.41 (graph in heap;
  1.09–1.50 mmapped); 2-hop count p50 0.99–1.35; 3-hop with a 10k budget p50 1.06–1.30,
  except twitter-sym at 2.1 (p95 2.7) because the query decodes whole lists.
- A raw copy decoded at open equals raw arrays (kernels 0.97–1.03 except lj WCC at
  1 thread, 0.88, and k-core; Leiden 0.98–1.03; point queries on lj and orkut). The decode
  takes 0.060–0.061 s (orkut) to 0.92–0.99 s (Friendster) at 16 threads warm
  (`open.jsonl`) and costs 4 bytes per entry plus 8 per node of anonymous memory (0.90 GiB
  on orkut, 13.95 GiB on Friendster).

**Two implementation facts carry these numbers** (`ablation.jsonl`, `scan.jsonl`,
`perf.jsonl`): sequential access decodes batches of lists before the kernel body runs
(list-by-list decoding costs orkut PageRank f32 1.35× at 1 thread and 1.38× at 16), and
random access uses per-node offsets (with block offsets only, one list costs 10–33 µs
instead of 87–217 ns, and orkut BFS at 16 threads takes 2.24 s instead of 0.028 s). The
first lab's "raw PageRank 2× faster on orkut" is not reproduced. At 16 threads a raw scan
tops out at 12.4–13.7 G entries/s on the four graphs while the compressed scan reaches
14.9–27.5; orkut PageRank runs at IPC 0.49 raw and 1.97 compressed — consistent with raw
being memory-bandwidth-bound there.

**Under a hard memory limit** that the raw arrays exceed and the compressed store fits
(`capped.jsonl`; 16 threads, plain mmap; twitter-sym at 8 GB, Friendster at 12 GB),
compressed is 1.2–1.9× faster for PageRank and 5.4–8.8× for WCC and BFS on Friendster;
BFS on twitter-sym is 1.5× slower compressed (its entries stay resident even for raw);
decoding to raw is killed. Under 2 GB on Friendster, through plain mmap, compressed and
raw are equal (4.31 vs 4.42 s/iteration, one repetition, 5 iterations including the
initial degree pass), which is 1.35× the in-RAM 3.19 s/iteration (arithmetic); the format
has no streaming reader yet (#46).

**Planner rule** (for #45). Run on the compressed store by default for every kernel.
Build the raw decode cache only when it fits the budget together with the kernel's state
and the session repeats traversals or point lookups, enough to repay the decode. Break-even,
arithmetic on measured values (decode time ÷ per-operation difference; not measured
sessions): on Friendster about 6 BFS runs at 16 threads, 900–970 shortest-path queries
(from mean latencies) or 8.2–8.7 M neighbour lookups; on orkut about 7 BFS runs, about 280
shortest-path queries or 0.85 M neighbour lookups. Never build it for PageRank, WCC,
k-core, triangles or Leiden. When the raw arrays do not fit and the compressed store does,
stay compressed; when neither fits, stream (#46).

**Against "What would change our mind".** PageRank and Leiden: not triggered — at 16
threads the default is at most 1.04× slower than raw arrays, and PageRank is faster
(0.73–0.97). Point queries: not settled. At 16 threads they were not measured. At 1 thread,
with the decode tier they equal raw (lj, orkut); without it neighbour lookups are 2.5–4.2×
slower on batch means — above the 1.5× threshold, but by 0.02–0.11 µs per call — while
bidirectional BFS stays within 1.13–1.41× (heap) and budgeted 3-hop reaches 2.1× on
twitter-sym until prefix decode exists. The criterion therefore holds only with conditions
3 and 5 of the decision.

**Not measured** (FINDINGS § M1-13, "Not measured, not trusted"): point-query throughput at
16 threads; a streaming or semi-external reader on the format; ingest writing the format,
with or without `fsync`, and an encoder under a memory budget; weighted or directed
kernels; web graphs; prefix decode for budgeted queries; batched prefix decode for WCC
(the Friendster WCC slowdown of 1.36–1.57× is not explained).

## Consequences

- The codec (#30) gains the per-node Elias–Fano offsets as a requirement, the Elias–Fano
  index of entry offsets (0.11–0.35 bits/edge measured), list-prefix decode and the
  per-list framing question; its "≤ 17 bits/edge on social graphs" target does not hold for
  lj (18.1) or Friendster (23.2) and is restated there.
- The edge-map framework (#44) decodes batches of lists for sequential access.
- The planner (#45) gets the decode tier with the rule above.
- Point queries (#90) use prefix decode for budgeted traversals and measure throughput at
  16 threads on the default storage.
- The streaming regime (#46) needs a reader for the compressed format; without one the
  "within 1.35× under 2 GB" target of DESIGN § 6 has no margin (1.35× measured through plain
  mmap).
- Ingest (#35) needs an encoder that runs under the memory budget; the one measured holds
  input and output in RAM.
- Public performance claims (#100) may quote the default-storage numbers of FINDINGS
  § M1-13 with their conditions; raw-array numbers are quoted as such.
- **Not to implement** (the review of 2026-11-26, [sota-review-2026-11.md](../research/sota-review-2026-11.md) (index in [docs/research/README.md](../research/README.md))):
  - a chunk layout of the whole graph (ChunkGraph). It fixes raw 4-byte lists read through
    4 KiB pages; our records are compressed and an average Friendster record (about
    157 B, arithmetic) crosses a page boundary about 4% of the time. Restart points inside
    long hub records are a separate question (m1-17, #174);
  - KedaGraph's codec (XOR deltas in Simple-8b words). For a sorted list the XOR of two
    neighbours is never smaller than their gap, and Simple-8b wastes about half a 64-bit
    word per list under per-list random access (arithmetic);
  - FrontOrder as a storage order. It groups nodes that are active in the same BFS
    frontiers and spreads each group over partitions, which works against gap compression
    (inference from its code). BFS order is already measured in m1-09 (#23).

## What would change our mind

If, on graphs that fit in RAM, the default configuration (compressed on disk, with whatever
decode tier the planner picks within the memory budget) is more than 1.5× slower than raw
arrays for PageRank, Leiden or point queries at 16 threads on an idle machine, then either
the in-RAM default becomes raw (compressed stays the on-disk and out-of-core form) or the
public claims are restated — by a new ADR. The 1.5× threshold is set by this ADR, not
measured. #118 did not trigger it for PageRank or Leiden; for point queries the 16-thread
measurement is exploratory only (the pull request of #90: neighbours 2.1–2.5× slower on the
compressed lists than on raw arrays on orkut, 3-hop 1.0–1.2×, on a loaded machine, with no
decode tier); the idle-machine run is #337's, and the same test applies to it.

Also: web graphs (#24) showing the byte codec more than 1.5× slower than raw for the same
kernels, or a streaming reader (#46) that cannot bring Friendster PageRank under 2 GB
within 1.35× of in-RAM on this format.

## Closed by

#46 (streaming reader on the compressed format), #240 (the speed of ingest writing the format
under a budget), #24 (web, weighted and directed graphs); #337 for
point-query throughput at 16 threads. The decode-tier evidence on twitter-sym is in #240.
