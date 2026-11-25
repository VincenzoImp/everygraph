---
title: No-OOM guarantee: budget fuzzing under cgroups
milestone: M3 · Engine and kernels
labels: type:infra, area:engine, P0, risk:medium, size:M, lab-evidence
---
**Why.** "Never run out of memory" must be a tested property, not a hope. The lab showed file-backed pages never cause an OOM kill; anonymous O(n) state does.

**Scope**
- [ ] Test matrix: kernel × graph × budget ∈ {0.5, 1, 2, 4, 8 GiB, unlimited} × regime, each run inside a cgroup scope with `MemoryMax = budget + fixed slack` and `MemorySwapMax=0`
- [ ] Expected outcomes are exactly two: success within the limit, or a clean `InsufficientBudget` refusal before work starts
- [ ] Record accountant peak vs cgroup anonymous peak; the gap is the "untracked memory" metric and must stay below the slack
- [ ] Also record, per run, the kernel terms of `memory.stat` (`pagetables`, `kernel`, `shmem`) and `memory.events` `oom_kill`. The counting `GlobalAlloc` below is approximate, because the compiler may elide allocations (GlobalAlloc docs); the cgroup record is the ground truth (`docs/research/sota-io-memory.md` § 3, m3-10 row)
- [ ] The cgroup monitor of {{#m2-19-allocator-policy}} runs in every case of the matrix. A thrashing run must end as an early, explicit refusal that names a regime that would work, not as a kill. The tracer's Leiden-16G case (killed at 460.5 s while thrashing) is one of the cases
- [ ] Nightly job on a self-hosted runner for tier-2 graphs; a reduced matrix on tier-1 in regular CI
- [ ] Same harness reused for ingest ({{#m2-08-csr-build}}) and communities ({{#m4-04-ooc-leiden}})
- [ ] A counting `#[global_allocator]` in kernel tests asserts that heap bytes stay within `budget.used()` plus a fixed slack: it catches the growth a lint cannot (`push`, `extend`, `collect`, `clone`) and needs no cgroup (review of PR #154)

**Acceptance**
- Zero OOM kills and zero "untracked > slack" results across the matrix for two consecutive weeks of nightly runs

**Depends on** {{#m2-04-arrays-accountant}}, {{#m0-03-bench-harness}}
