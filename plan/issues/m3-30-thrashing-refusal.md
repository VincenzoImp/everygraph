---
title: Thrashing is a refusal, not a slow death
milestone: M3 · Engine and kernels
labels: type:feature, area:engine, P1, risk:medium, size:M, lab-evidence
---
**Why.** A plan in the mapped regime can still read the graph many times over when the page cache keeps too little of it. The tracer ran 460–648 s before it was killed (`bench/results/tracer/20261125T122609Z-3e764d115d86.jsonl` rows 9, 11–12). {{#m3-02-planner}} chooses the regime from sizes only; nothing watches a run (rule in `plan/issues/m3-02-planner.md`, `docs/research/sota-io-memory.md` C3).

**Scope**
- [ ] A monitor of the run's file refaults per pass (`memory.stat` `workingset_refault_file` of the process's control group, or `/proc/self/io` read bytes against the bytes a pass needs), sampled between the phases and passes of a kernel
- [ ] When the refaults of one pass exceed about one file's size, the stage stops with a refusal that names the regime that would work: a larger budget, or the streamed regime ({{#m3-03-semi-external}}, PR #254) for a kernel that has one
- [ ] The rule's threshold from {{#m2-19-allocator-policy}}'s monitor experiment

**Acceptance**
- A run that would thrash (Friendster PageRank mapped under a budget far below its working set) stops within two passes with the refusal, and `Nothing was changed.`
- A run that does not thrash is never stopped: the corpus and the no-OOM matrix ({{#m3-10-no-oom-ci}}) pass unchanged

**Depends on** {{#m3-02-planner}}
