---
title: Harness: keep a failed warm-up's measurement, count a timed-out run's own I/O
milestone: M3 · Engine and kernels
labels: type:infra, area:bench, P1, risk:low, size:S
---
**Why.** The tracer bullet ({{#m1-01-e2e-tracer}}, official run `bench/results/tracer/20261125T122609Z-3e764d115d86.jsonl`, 2026-11-25) hit two defects of the benchmark harness ({{#m0-03-bench-harness}}). Both lose or misreport measurements of exactly the runs that matter most — the ones that fail. Both were confirmed from the code by the independent review of the tracer's lab PR.

1. **A failed warm-up writes no measurement.** In `crates/everygraph-bench/src/harness/run.rs` (the warm-up loop, around lines 1215–1240) a warm-up's `exec.measurement` is used only to decide whether it succeeded; on failure the harness writes `base_row(case, 0, reps)` with status `failed`, no memory, OOM, exit or I/O fields, and only the tail of standard error. Row 10 of the tracer file (`communities-leiden`, the map-shaped Leiden under 56 GiB) is such a row: it says "failed" although the run was OOM-killed at 117.3 s with a 55.8 GiB anonymous peak, figures that survive only in the helper's report copied to `lab/m1-01-tracer/results/logs/communities-leiden-56G.warmup1.measure.json`.
2. **On a timeout the run's own I/O is not counted.** `crates/everygraph-bench/src/harness/measure.rs` takes `read_bytes`/`write_bytes` from the helper's `/proc/self/io` (line 333), which includes only descendants the helper has reaped. On timeout `signal_scope` (line 129) sends SIGKILL to every process of the scope at once; the measured command's own children (here `tracer.py` → `gmap`) die orphaned, are reaped by init, and their counters are lost. Row 27 (`tiles-16g`, graph memory-mapped, cut at 3,600 s) records `read_bytes` 86,016, so `others_io` (`gate::others_io`, line 337) attributes the run's own page-fault reads to other processes (3,210.66 MB/s) and marks the row disturbed although nothing else was running. OOM rows are not affected: only the victim is killed and its parent reaps it (rows 9 and 11–12 record 1.80 TB and 2.72–2.74 TB read).

**Scope**
- [ ] A failed warm-up writes a row with its full measurement — status `oom`, `timeout` or `failed` as for a measured repetition, memory and I/O fields, OOM events, exit — marked as a warm-up (repetition 0) so it is never counted in a median; the following repetitions are still skipped
- [ ] On timeout or stop, the run's own I/O is counted: make the helper a child subreaper (`PR_SET_CHILD_SUBREAPER`) and reap every orphaned descendant before reading `/proc/self/io`, or read the scope's cgroup `io.stat` if the io controller is delegated to user scopes; whichever is chosen also covers grandchildren on the normal path
- [ ] If the run's own I/O cannot be established, the row says so (I/O unknown) and the disturbance check does not use the I/O criterion for it, instead of attributing all host traffic to others
- [ ] Tests: a spec whose warm-up is OOM-killed under a small `MemoryMax` writes a row with `oom` and a non-zero anonymous peak; a command that forks a child doing known reads and is cut by a timeout records at least those bytes and is not marked disturbed on an idle machine
- [ ] Protect the measuring helper from the kernel OOM killer (raise the measured command's `oom_score_adj`, or read the scope's cgroup counters from outside the scope), so an OOM of the measured command can no longer kill the helper and lose the row; then drop the nextest retries for `a_failed_warm_up_writes_its_measurement` and `an_exploratory_run_records_every_field` (`.config/nextest.toml`; they failed in 2 of 4 `cargo test` jobs on GitHub runners, PR #156)

**Acceptance**
- Re-running the two cases above (synthetic, small) gives rows with the measurement and the right status; neither is marked disturbed on an idle machine
- The harness's existing tests pass; the row schema change is documented where the schema is described

**Depends on** {{#m0-03-bench-harness}}
