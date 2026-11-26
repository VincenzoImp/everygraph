---
title: Resource efficiency beyond peak memory: memory over time, CPU time, energy, cost
milestone: M9 · After v0.1
labels: type:infra, area:bench, P2, risk:low, size:S
---
**Why.** 16 GB for ten hours can cost more than 64 GB for five minutes. The harness records wall time, CPU time, peak memory, bytes read and written, and page faults. It does not record memory integrated over time or energy, and the capacity curve of m3-12 (S9) shows time against memory only.

**Scope**
- [ ] Memory integrated over time from the cgroup samples; CPU-seconds per run
- [ ] Energy where the host exposes it reliably (RAPL), labelled with its source; none otherwise
- [ ] A Pareto view of time, memory and quality per kernel and per map build
