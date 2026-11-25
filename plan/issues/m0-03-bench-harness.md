---
title: Benchmark harness and measurement protocol
milestone: M0 · Foundations and decisions
labels: type:infra, area:bench, P0, risk:medium, size:L
---
**Why.** The first lab ran eight experiments at once; every timing carried ±30% noise and one re-run deadlocked on a nested lock. Performance is a headline claim of this project, so measurement needs to be a product-quality tool, not shell scripts.

**Scope**
- [ ] `everygraph-bench` binary: declarative benchmark specs (dataset, command, threads, memory limit, repetitions), JSON-lines output with machine fingerprint, git revision, load average at start/end
- [ ] Idle check: refuse to record an "official" result when the 1-minute load average exceeds a threshold or another official run is active (single lock, taken in exactly one place)
- [ ] Memory-limited runs through `systemd-run --user --scope -p MemoryMax=… -p MemorySwapMax=0`, recording cgroup peak, anonymous peak and OOM events separately from RSS (mmap inflates RSS — see FINDINGS § B1)
- [ ] Cold-cache protocol: evict with `posix_fadvise(DONTNEED)` on a private copy, verify with `mincore`, never re-read the same LBA twice in a row (SSD cache trap found in F1)
- [ ] Peak-memory, wall, user/sys time, bytes read/written per run
- [ ] Comparison runners for NetworKit, igraph, GVE-Louvain/Leiden, DuckDB, and the engines measured in F2
- [ ] Report generator: markdown tables + chart-ready JSON
- [ ] Record the estimates and actuals of `docs/ux-contract.md` P-9 for every run, report per quantity the share of runs whose actual lies in `[low, high]`, and measure the 2 s stop bound of C-4 on the tier-1 datasets (both apply from the first issue that produces a plan; the harness provides the fields and the report now)
- [ ] Every run under a hard memory limit (`systemd-run --user --scope -p MemoryMax=…`), stated in the spec file and recorded in the result (CLAUDE.md § 7)
- [ ] Command line through a parser crate (`clap` or `lexopt`) now that the tool gains subcommands beyond the dataset ones (review of PR #129)

**Acceptance** (amended 2026-11-25 after the first official runs; the misses and their causes are in PR #136)
- `everygraph-bench run headline` reproduces the idle re-run (FINDINGS § "Idle-machine re-runs", files under `lab/b1-core/results/bench/`) within 10% on the lab machine for kernels, probes and semi-external cases (naive mmap under 8/16 GiB gated against the idle rows of `semiext.jsonl`; semi-external at 16 GiB reported, not gated). Ingest references are the harness's own official idle measurements: the lab's were single runs, and its Friendster ingest figures were taken on a rested SSD whose first ~10 GB are written at 5.3 GB/s and the rest at ~1.5 GB/s, so write-heavy cases run one untimed warm-up repetition
- Two consecutive idle runs of the same spec differ by < 5% for in-RAM kernels, except cases whose within-run spread exceeds 5%, which are listed with their spread (k-core at 16 threads on orkut: 0.266–0.364 s)
- Starting a second official run while one is active fails fast instead of queueing

**Pointers.** `lab/tools/timev.py`, `lab/*/bench.sh`, LAB.md rules.

**Depends on** {{#m0-01-workspace}}, {{#m0-04-datasets}}
