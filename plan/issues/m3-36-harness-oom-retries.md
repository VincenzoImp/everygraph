---
title: Remove the nextest retries of the two harness OOM tests
milestone: M3 · Engine and kernels
labels: type:infra, area:bench, P2, risk:low, size:S
---
**Why.** `.config/nextest.toml` gives two retries to two `everygraph-bench` harness tests that drive the OOM killer in a 32 MiB scope: `an_exploratory_run_records_every_field` and `a_failed_warm_up_writes_its_measurement`. PR #299 fixed the cause (the OOM killer ended the measuring helper after the command). CONTRIBUTING.md "CI", rule 8: a flaky test is quarantined with an issue, never retried silently.

**Scope**
- [ ] The override with `retries = 2` and its comment removed from `.config/nextest.toml`
- [ ] If either test fails again on CI without a code change: the test goes into the quarantine filter (rule 8) with an open issue that names the run, and runs nightly

**Done when.** The retries are gone and 20 consecutive CI runs of the test job have no failure of the two tests, or the failing test is quarantined with its issue.
