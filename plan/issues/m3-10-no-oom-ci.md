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
- [ ] The cgroup matrix runs nightly on a self-hosted runner, tier-2 graphs included, for what only the kernel sees (allocator retention, {{#m2-19-allocator-policy}}; page cache against anonymous memory; thread stacks); a reduced matrix on tier-1 in regular CI
- [ ] Same harness reused for ingest ({{#m2-08-csr-build}}) and communities ({{#m4-04-ooc-leiden}})
- [ ] A counting `#[global_allocator]` in kernel tests asserts that heap bytes stay within `budget.used()` plus a fixed slack: it catches the growth a lint cannot (`push`, `extend`, `collect`, `clone`) and needs no cgroup (review of PR #154)

**Budget fuzzing: search, not only a matrix** (added 2026-11-25, `docs/research/README.md` T2; `docs/research/sota-testing.md` challenge 3). The lab's only OOM was untracked anonymous O(n) state (WCC under 1 GB, FINDINGS), and #34's `build-500m` case was OOM-killed under `MemoryMax=5G` with a 4 GiB budget in the official run `bench/results/id-dictionary/20261125T220117Z-c855fdd77cf9.jsonl`. Untracked heap that grows with n has a signature visible at any size:
- [ ] **Slope test.** Per kernel and per generator family, in one process under the counting allocator, n = 2^10 … 2^16: record U(n) = peak(heap) − peak(ledger), counted as *requested* bytes (allocator-independent; RSS stays with the cgroup job), and fit it against n. U must be bounded by a constant, or by threads × per-thread buffers; any bytes-per-node slope fails. A slope of b bytes/node predicts b × 65.6 M bytes untracked on Friendster (an extrapolation, labelled as such). The sizes straddle the spill, chunk (2^16 entries) and segment thresholds, and the generators include hubs, long chains, duplicates, sparse id ranges, isolates and many small components. Cheapest check: the slope test on the lab WCC that OOM-ed must show a slope of about 4–8 bytes/node before its fix (the report's estimate: one u32/u64 per node) and ≈ 0 after
- [ ] **Search.** Fitness = U(n)/n, the fitted slope over three sizes, or heap peak / budget — byte counts from the counting allocator, never RSS or wall time. The space: generator parameters (Singularity-style components: add node, add edge, grow edge, self-loop, bridge, merge, update weight) plus hub size, number of components, path length, id sparsity, key length and type, duplicate multiplicity, weight columns, chunk size, threads and budget. A configuration that raises the fitness is kept even with no new outcome (MemLock). In CI: targeted-PBT hill climbing, fixed seed, 1–3 minutes on ≤ 10^4 edges; nightly: a longer evolutionary search whose findings are committed as patterns that CI replays at three sizes
- [ ] **Per-phase maxima.** The accountant sets a thread-local phase tag at each reservation; the counting allocator keeps a per-tag maximum of untracked bytes, and the search keeps an input that raises any tag's maximum (PerfFuzz). Findings are deduplicated by phase, never by input
- [ ] **Budget boundary.** Per (kernel, graph), binary-search the smallest budget that succeeds and compare it with the planner's declared minimum ({{#m3-02-planner}}): declared < true is an OOM in waiting, declared ≫ true a needless refusal (the ratio is reported). Below the declared minimum the refusal itself allocates O(1)
- [ ] **Fail the k-th reservation** (DESIGN D2's fallible allocation): for k = 1 … K, `try_reserve` fails at the k-th call; every run ends in a typed error, with no panic, no leaked scratch file, and the accountant back at zero

**Acceptance**
- Zero OOM kills and zero "untracked > slack" results across the matrix for two consecutive weeks of nightly runs
- The slope test passes for every kernel and generator family, and the reintroduced lab WCC state (a canary behind a test-only flag) fails it
- Every k of the k-th-reservation sweep ends in a typed error with the accountant at zero

**Depends on** {{#m2-04-arrays-accountant}}, {{#m0-03-bench-harness}}
