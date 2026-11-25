# State of the project

Updated at the end of every session. Read this first.

## Where we are (2026-11-24)

- **Phase:** preparation complete; no production code yet. Next milestone: M0.
- **Plan:** 118 issues in 10 milestones on GitHub (`VincenzoImp/everygraph`, private),
  board `users/VincenzoImp/projects/7`. Source of truth: `plan/issues/*.md`.
- **Evidence:** first feasibility lab done; results in `docs/FINDINGS.md`, raw result
  files in `lab/*/results`, prototypes in `lab/`.
- **Owner's standing instructions:** every decision is delegated (2026-11-24); the owner
  helps where hands are needed (2FA, credentials, payments). Keep the machine clean;
  coordinate subagents. Safeguards Claude keeps: CLAUDE.md § 8. The owner wants to see
  previews of everything that is a matter of taste before it is built (`design/preview/`).

## In flight

- **#119 design previews** — branch `m0-13-design-previews`, draft PR #120. Published at
  `design/preview/`: viewer mockup, identity rounds 1 and 2, API, CLI, choices, and the
  detailed guide of the chosen style (`design/preview/style/`, entry `README.md` on GitHub
  or `index.html` locally). Owner decided: identity = B's system with A's mark. Claude chose
  the "Vernier" system and the spelling of the name (both provisional in
  `design/DECISIONS.md`). **Waiting for the owner's reaction to the detailed style and to
  the choice lists.** Afterwards: record answers, align names in the older mockups with
  `api.md`, prune superseded screenshots (`design/preview/screens` is ~77 MB), merge.
  The Playwright helper scripts live in `$EVERYGRAPH_SCRATCH/pw` (`shot.mjs`, `sections.mjs`).

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
  tool), node 26, uv. `perf` 6.12 with `kernel.perf_event_paranoid=2` (persistent): user-space hardware counters work without privileges.

## Open questions for the owner

- Answers to the design previews (`design/preview/README.md`, issue #119).

## Lessons (append, never delete)

- 2026-11-25 — A colour palette chosen by eye failed measurement (two pairs invisible to
  colour-blind readers, one colour equal to the accent). Measure palettes and contrast.
- 2026-11-25 — Text that looked fine in Markdown broke when typeset (lines over 80 columns,
  one word with two meanings). Typeset user-facing text before calling it done.

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

- **2026-11-25** — Detailed style guide built (nine pages, three subagents). Measurements
  overturned Claude's hand-picked data palette and five contrast values; tokens corrected.
- **2026-11-24 (later)** — Owner delegated every decision and asked for previews of all
  matters of taste. Added #119; produced the previews with three subagents.
- **2026-11-24** — Project conceived as a rethink of `largegraph`. Feasibility lab (8
  experiments), design baseline, 117-issue plan published. Full review of the plan found
  ten errors; fixed, one issue added (#118), three issues moved between milestones.
  Owner delegated execution. Machine cleaned (239 GB → 54 GB), toolchain installed,
  operating rules written.
