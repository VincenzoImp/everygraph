---
title: The plan and the actual figures in each result's provenance
milestone: M3 · Engine and kernels
labels: type:feature, area:engine, P1, risk:medium, size:S
---
**Why.** `docs/ux-contract.md` P-9: the actual figures are recorded next to the estimates in the provenance of every result, and the relative error of each of the four quantities is the input the harness calibrates the cost model with. #45 makes the plan ({{#m3-02-planner}}, PR #249), but a kept result cannot hold it: `Writer::add_result` keeps only the parameters of the result's key, so anything else in its provenance is refused.

**Scope**
- [ ] A provenance field of a result for what is not in its key: the regime, the tier, the threads, and the estimate and the actual value of time and peak memory. A specification change in `docs/spec/eg-container.md` § 6, with the reader and `verify` updated in the same pull request
- [ ] `run::store` writes it from the plan and what the run measured (time, `Budget::peak`)
- [ ] `everygraph info` and the result document show it

**Acceptance**
- A result written by `everygraph run` carries its plan's estimate and its actual time and peak, and `verify --deep` accepts the file
- The relative error of P-9 is computed from them, `null` where the actual is 0

**Depends on** {{#m3-02-planner}}
