---
title: Leiden's reserved peak independent of the thread count
milestone: M4 · Communities and hierarchy
labels: type:feature, area:community, P1, risk:medium, size:S
---
**Why.** D14 promises the same result at any thread count and any memory budget, and the accountant refuses before it allocates. But production Leiden ({{#m4-01-leiden-production}}) keeps one table per worker, so its reserved peak grows with the thread count. A budget that a run on one thread fits can then refuse the same run on eight. Measured in PR #285 (exploratory): on a 20,000-node nested planted graph, the reserved peak of a 1-thread run refuses an 8-thread run at "quotient weights" (895,608 bytes needed, 344,576 available). The map builder ({{#m4-07-map-hierarchy}}) adds no thread-dependent memory of its own: when the budget refuses regions solved together, it solves them again one at a time. Leiden itself has no such fallback.

**Scope**
- [ ] Either a bound on the per-worker tables that does not depend on the thread count, or, when a reservation is refused, Leiden runs again with fewer threads before it refuses, as the planner does when it chooses the threads for a budget ({{#m3-02-planner}}). The result does not change either way (D14).
- [ ] The refusal names the smallest budget that fits on one thread.

**Done when.** A test builds Leiden and the map tree at 1 thread under an unlimited budget, takes the reserved peak, and runs both again at 8 and 16 threads under a budget of that peak: the same bits, no refusal.

**Depends on** {{#m4-01-leiden-production}}
