---
title: Harness: an exact match with a reference of 0 counts as within
milestone: M3 · Engine and kernels
labels: type:infra, area:bench, P1, risk:low, size:S
---
**Why.** A reference value of 0 against a measured 0 is reported as `NaN%, OFF`. The relative difference divides by the reference, so 0 / 0 gives NaN, and NaN fails the tolerance test. Seen in the official run `bench/results/references/20261127T022357Z-2510b9c16c0a.jsonl` (`validate/on=orkut`: 0 invalid, 0 not run, both expected 0). The run was correct; the report called it a failure.

**Scope**
- [ ] `reference_checks` (`crates/everygraph-bench/src/harness/report.rs`): an exact match counts as within, whatever the value. A reference of 0 with a measured value that is not 0 is off, and the report prints the absolute difference, not a percentage
- [ ] A test for each case: 0 against 0, 0 against a non-zero value, a non-zero reference as today

**Done when.** The report of the run above shows both checks as within, and the tests pass.
