# State of the project

Updated at the end of every session. Read this first.

## Where we are (2026-11-24)

- **Phase:** preparation complete; no production code yet. Next milestone: M0.
- **Plan:** 118 issues in 10 milestones on GitHub (`VincenzoImp/everygraph`, private),
  board `users/VincenzoImp/projects/7`. Source of truth: `plan/issues/*.md`.
- **Evidence:** first feasibility lab done; results in `docs/FINDINGS.md`, raw result
  files in `lab/*/results`, prototypes in `lab/`.
- **Owner's standing instructions:** full autonomy on this repository and this machine;
  keep the machine clean; coordinate subagents; the items in CLAUDE.md § 8 stay with the
  owner.

## In flight

Nothing.

## Next actions

1. M0, in dependency order (`python3 plan/next.py`): workspace (#2) → CI (#3) → benchmark
   harness (#4), datasets (#5), oracle (#6) in parallel; ADRs (#7); API sketch (#12).
2. Start the legal look-ups early, they have lead time: name/trademark (#10), patent (#11).
   The research is ours; the conclusions are the owner's.
3. In parallel with M0, the two M1 experiments that decide the most: the end-to-end tracer
   on Friendster under 16 GiB (#15) and kernels on compressed storage (#118).

## Machine

- Datasets: `/home/tux/graphlab/data` — `raw/` (lj, orkut, friendster as .gz; twitter-2010
  as BVGraph), `csr/` (lj, orkut, friendster, twitter-dir, twitter-dir-in, twitter-sym in
  the lab's `glab-csr-v0` format), `hier/` (lab hierarchies for lj and orkut). 54 GB.
- Lab working directories were deleted on 2026-11-24 after archiving sources and results
  into `lab/`. The uk-2005 web graph and decompressed text inputs must be re-fetched or
  regenerated when needed.
- Toolchain: Rust stable 1.100 + nightly (miri, rust-src), targets wasm32-unknown-unknown and
  i686-unknown-linux-gnu, cargo-deny, cargo-nextest, cargo-fuzz, hyperfine, maturin (uv
  tool), node 26, uv. `perf` 6.12 is installed, but `kernel.perf_event_paranoid` is 3, so unprivileged counters are denied until the owner lowers it to 2 (needed for #22).

## Open questions for the owner

- None blocking. Reserved decisions are listed in CLAUDE.md § 8.

## Lessons (append, never delete)

- 2026-11-24 — Timings taken while eight experiments shared the machine were off by up to
  2–3× in comparisons (GVE) and hid real effects (SMT). Idle machine only.
- 2026-11-24 — A baseline labelled "triangles" was NetworKit's local clustering
  coefficient in non-turbo mode. Read what the reference actually runs.
- 2026-11-24 — "Out of core costs 10–35%" generalized one kernel; components were 10×.
- 2026-11-24 — Headline numbers were measured on uncompressed arrays while the design
  makes compressed storage the default. Measure the configuration you will ship (#118).
- 2026-11-24 — `pkill -f`/`kill $(pgrep -f …)` killed the calling shell twice and another
  experiment's processes once. PIDs only.
- 2026-11-24 — Wrapping a script that takes `flock` in the same lock deadlocked silently.
- 2026-11-24 — Subagents cannot write `.md` report files; one bypassed the block through
  the shell. Reports come back as text and the coordinator records them.
- 2026-11-24 — The short name `eg` is taken on PyPI, crates.io and npm.
- 2026-11-24 — Effort estimates quoted without adding them up were 30% too low.

## Journal

- **2026-11-24** — Project conceived as a rethink of `largegraph`. Feasibility lab (8
  experiments), design baseline, 117-issue plan published. Full review of the plan found
  ten errors; fixed, one issue added (#118), three issues moved between milestones.
  Owner delegated execution. Machine cleaned (239 GB → 54 GB), toolchain installed,
  operating rules written.
