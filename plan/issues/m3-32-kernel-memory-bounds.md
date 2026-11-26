---
title: Kernel memory inside the budget, and bounds the planner can hold tight
milestone: M3 · Engine and kernels
labels: type:feature, area:engine, P1, risk:medium, size:M
---
**Why.** The planner ({{#m3-02-planner}}, PR #249) bounds what a run reserves, term by term from the code, and the oracle corpus checks the bound. Two gaps remain.
- *Allocations outside the budget.* BFS's `levels` (8 B a level: O(n) on a path), the per-chunk vectors of the roots in `components::number` (which may reach about twice the 8 bytes a component they reserve), `decode_all`'s pieces, the edge-map task lists and the stack of every thread `par::with_threads` starts. They are small on the corpus, but the planner's peak does not bound resident memory until they are counted.
- *Loose bounds.* On lj and orkut at 16 threads the runs reserve 0.65–0.66 of the bound for components (it holds a root for every node) and 0.59–0.67 for k-core (every bucket and frontier at once). A budget between what runs need and the bound is refused.

**Scope**
- [ ] Reserve, or remove, each allocation above; the stack of a pool's threads counted once per pool
- [ ] `components::number` in memory that does not grow with the component count (4 bytes a root, or a pass that reuses the sizes array)
- [ ] k-core's buckets and frontiers bounded by what they can hold together, not each by every node
- [ ] A counting-allocator check, as `tests/heap.rs` does, that the heap a run holds stays within its plan's peak plus a fixed slack, on the corpus; the resident check under a cgroup is {{#m3-10-no-oom-ci}}'s

**Acceptance**
- Every run of the corpus holds at most its plan's peak plus the fixed slack, counted by the allocator
- Reserved ÷ planned at least 0.85 for components and k-core on lj and orkut

**Depends on** {{#m3-02-planner}}
