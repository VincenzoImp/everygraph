---
title: Benchmark harness and measurement protocol
milestone: M0 · Foundations and decisions
labels: type:infra, area:bench, P0, risk:medium, size:L
---
**Why.** The first lab ran eight experiments at once; every timing carried ±30% noise and one re-run deadlocked on a nested lock. Performance is a headline claim of this project, so measurement needs to be a product-quality tool, not shell scripts.

**Scope**
- [ ] `eg-bench` binary: declarative benchmark specs (dataset, command, threads, memory limit, repetitions), JSON-lines output with machine fingerprint, git revision, load average at start/end
- [ ] Idle check: refuse to record an "official" result when the 1-minute load average exceeds a threshold or another official run is active (single lock, taken in exactly one place)
- [ ] Memory-limited runs through `systemd-run --user --scope -p MemoryMax=… -p MemorySwapMax=0`, recording cgroup peak, anonymous peak and OOM events separately from RSS (mmap inflates RSS — see FINDINGS § B1)
- [ ] Cold-cache protocol: evict with `posix_fadvise(DONTNEED)` on a private copy, verify with `mincore`, never re-read the same LBA twice in a row (SSD cache trap found in F1)
- [ ] Peak-memory, wall, user/sys time, bytes read/written per run
- [ ] Comparison runners for NetworKit, igraph, GVE-Louvain/Leiden, DuckDB, and the engines measured in F2
- [ ] Report generator: markdown tables + chart-ready JSON

**Acceptance**
- `eg-bench run headline` reproduces the ingest, kernel and semi-external tables of FINDINGS § B1 within 10% on the lab machine
- Two consecutive idle runs of the same spec differ by < 5% for in-RAM kernels
- Starting a second official run while one is active fails fast instead of queueing

**Pointers.** `lab/tools/timev.py`, `lab/*/bench.sh`, LAB.md rules.

**Depends on** {{#m0-01-workspace}}, {{#m0-04-datasets}}
