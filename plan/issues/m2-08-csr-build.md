---
title: Graph build pipeline: one code path in RAM and out of core
milestone: M2 · Format and ingest
labels: type:feature, area:ingest, P0, risk:medium, size:XL, lab-evidence
---
**Why.** This is the "open anything fast" promise. The lab pipeline (parse → rank directory → relabel + partition into ≤ 2,048 buckets → per-bucket sort/dedup → ordered commit) already uses one code path for both regimes and is byte-identical to the reference in every mode.

**Improvements identified by the lab**
- [ ] Partition by original-id range in pass 1 using sampled quantiles and relabel inside the per-bucket sort: removes the spill pass (22 GB of temp traffic on Friendster) and balances skewed buckets (lj's largest bucket was 10× the mean)
- [ ] Temp storage: page-cache-backed files now; large sequential per-thread logs with an extent index if direct I/O is wanted (small appending O_DIRECT writes cost ~0.9 ms each on ext4)
- [ ] Pack bucket entries to 6 bytes (−25% of the dominant temp volume)
- [ ] Output through the aligned-block writer ({{#m2-02-io-layer}}); write directly in the compressed format ({{#m2-03-adjacency-codec}}) with an encoder that runs under the budget — the #118 encoder holds input and output in RAM (Friendster about 14 GiB raw + 9.8 GiB compressed); encode 0.064 / 0.168 / 3.41 s at 16 threads (lj / orkut / Friendster) plus 4.77 s write with fsync on Friendster
- [ ] Budget-driven: bucket count and buffer sizes derived from the memory accountant ({{#m2-04-arrays-accountant}})

**Scope**
- [ ] Undirected (symmetrized) and directed builds
- [ ] Carry property columns through the sort (weights, timestamps) ({{#m2-10-property-columns}})
- [ ] Deterministic output independent of thread count
- [ ] Treatment order per row: read, self-loops (drop or keep), duplicates (`events`; `collapse` keeps the first row in input order; `sum` merges), stored; an identifier whose only rows were dropped loops is still a node, an isolate (`docs/spec/graph-semantics.md` § 2.3)
- [ ] Storage order within each list: (neighbour in graph order, time, input order), nulls of time last, so a pair's events are contiguous; the in-adjacency the same (`docs/spec/graph-semantics.md` § 2.5)
- [ ] Ingest report in the provenance: `rows`, `rejected`, `dropped.self_loops`, `dropped.duplicates`, `merged`, `edges`, kept `self_loops`, `repeated`, `pairs`, `nodes`, `isolates` (`docs/spec/graph-semantics.md` § 2.8)
- [ ] Scaling work: the prototype scales only 3.4–4.9× from 1 to 32 threads on small graphs

**Acceptance (lab machine, idle, plain text)**
- lj ≤ 0.5 s; orkut ≤ 1.0 s; Friendster ≤ 25 s at a 32 GB budget and ≤ 60 s at 8 GB with peak RSS ≤ 4 GB, writing the default format, measured under the harness's warm-up protocol (sustained drive write rate, ADR 0026), fsync reported separately and bytes written reported next to the time (baseline by addition: 36.97 s sustained build at 8 GB, `bench/results/headline/20261125T114658Z-0ffe756ac9ac.jsonl`, + 3.4 s encode at 16 threads from #118 = 40.4 s before the write; at 32 GB 29.06 + 3.4 = 32.5 s, so ≤ 25 s needs the temp-traffic reductions above)
- Output identical for budgets 0.25 GB … 32 GB and thread counts 1 … 32
- Out-of-core NVMe traffic for Friendster at 8 GB reduced from 128 GB to ≤ 90 GB
- On every oracle graph and every `self_loops` × `duplicates` option, a test checks `rows = rejected + dropped.self_loops + dropped.duplicates + merged + edges` and `edges = pairs + repeated` (`docs/spec/graph-semantics.md` § 2.8; shared with #40)

**Pointers.** `lab/b1-core`, FINDINGS § B1 "Where the time goes".

**Depends on** {{#m2-05-text-reader}}, {{#m2-07-id-mapping}}
