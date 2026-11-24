---
title: Make the first lab reproducible with one command
milestone: M0 · Foundations and decisions
labels: type:infra, area:bench, P1, risk:low, size:M, lab-evidence
---
**Why.** The evidence behind the design lives in eight lab prototypes whose `bench.sh` scripts were written under time pressure: B2's takes the lab lock itself (it deadlocked a wrapped re-run), C's and F2's were never run end to end, B1's Friendster half was only run piecewise, and several agents could not write their `REPORT.md`.

**Scope**
- [ ] Port each lab `bench.sh` to an `everygraph-bench` spec ({{#m0-03-bench-harness}}); remove all per-script locking
- [ ] Replace absolute `/home/tux/graphlab` paths with a configurable data root
- [ ] Re-run everything on an idle machine and replace the "shared machine" numbers in `docs/FINDINGS.md` with idle numbers, keeping the old ones in an appendix
- [ ] Record what could not be reproduced

**Acceptance**
- `everygraph-bench run lab-v1` completes on the lab machine and regenerates the FINDINGS tables
- Every number in DESIGN.md § 2 and § 6 is traceable to a result file

**Depends on** {{#m0-03-bench-harness}}, {{#m0-04-datasets}}
