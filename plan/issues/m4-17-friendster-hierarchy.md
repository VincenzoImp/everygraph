---
title: Friendster map hierarchy: memory and size on disk of the stored levels
milestone: M4 · Communities and hierarchy
labels: type:experiment, area:community, P1, risk:low, size:S
---
**Why.** The last acceptance criterion of {{#m4-06-hierarchy-artifact}}: the Friendster map hierarchy is written with memory that does not grow with the number of levels, and its size on disk is reported against the tracer's 42.5 GB (`bench/results/tracer/20261125T122609Z-3e764d115d86.jsonl` row 14). It needs more memory than the 8 GB of an implementation agent, so it runs in the coordinator's window. On lj and orkut (exploratory, PR #268) the writer's reserved peak is below Leiden's, and the files are 3.8× and 4.8× smaller than the lab's layout of the same levels.

**Scope**
- [ ] `leiden_bench encode` and `leiden_bench run --source mmap --hier FILE` on Friendster (the commands of PR #268), under `MemoryMax=48G`: Leiden's and the writer's reserved peaks, the anonymous peak of the process, the file's bytes, and the time of each table read, cold and warm.
- [ ] The map tree of {{#m4-07-map-hierarchy}} (PR #285) on Friendster under `MemoryMax=16G`: `map_bench --mmap STORE --threads 16 --store FILE --export DIR` (the commands of PR #285): the build's reserved peak and the anonymous peak of the process, the stored map's bytes against the tracer's map-shaped 42.5 GB, `everygraph verify --deep` intact, and the lab layout (`lab/e-map/gmap layout --hier DIR`) placing every parent under 16 GiB. This closes the last criteria of {{#m4-07-map-hierarchy}}.

**Done when.** The numbers are in FINDINGS with their result file, and `everygraph verify --deep` finds the file intact.

**Depends on** {{#m4-06-hierarchy-artifact}}
