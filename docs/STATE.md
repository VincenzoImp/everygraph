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

As of 2026-11-25, 03:45 (policy for models in CLAUDE.md § 6):

- **#118 compressed storage end to end** (M1) — experimenter (Fable high, started before the
  policy), branch `lab-m1-13-compressed-end-to-end`. **It owns the machine**: about 3.5 hours
  of measurements; even one busy core skewed its timings by 10–40%, so nothing that compiles
  or computes for more than a moment runs until it reports. Encode and verify stages are
  done (bits per edge reported: lj 18.1, orkut 16.7, twitter-sym 16.1, Friendster 23.2 —
  to be checked against its result files).
- **#6 oracle** — draft PR #131 (`m0-05-oracle`), agent paused. To do when the machine is
  free (resume the same agent): rebase on main; golden files for the tier-1 datasets through
  the registry of #5; a CI job that runs `oracle/run-ci.sh` (with `uv`); time `run-ci.sh`;
  final seeded-bug matrix; toy and lab adapter on the nightly tier; `run-nightly.sh`. The
  semantics questions Q1–Q13 are answered in `design/DECISIONS.md` ("Graph semantics").
  Then independent review (Opus high). Corpora kept in scratch `m0-05-oracle` (110 MB).
- **#4 benchmark harness** — ready (its dependency #5 is merged) but not started: it would
  compile during the measurement window. Start it (Opus high) when #118 reports.

Done today: **#7 ADRs** merged (PR #132): 25 ADRs under `docs/adr/` plus 0000 on overturned assumptions; DESIGN, FINDINGS and README corrected where they said more than the result files (13 statements, list in the PR); **D10 is open** (mask vs materialized view: the "~25%" rule was never measured, #93 derives it); **D14 is widened** (identical results across budgets and regimes; unmeasured so far); D4's ADR has a pending section for #118. To watch: with the default of one Leiden iteration the idle result is 2.7–3.4% below the best reference, while the §6 criterion (within 1%) is written for three iterations — #57 settles the default. Promise 1 ("about 100 M edges/s") holds with ample memory (83–132 M/s) and not under a tight budget (34 M/s for Friendster at 8 GB): say both wherever the number is quoted. **#12 API sketch and #13 UX contract** merged (PRs #125, #124) after two rounds of independent review and a joint alignment pass; their obligations are written into the issues that build them (commits "Plan: …" of 2026-11-25) and the choices are in `design/DECISIONS.md`. **#5 datasets** merged (PR #129) after an independent review and a fix round (17 tier-1 datasets, 75 MB, all usable by anyone; follow-up for real bipartite and co-occurrence graphs in `m3-13-dataset-gaps`). **#3 CI** merged (PR #127; all six Linux jobs green on `main`, run 45530945188; macOS, Windows and Linux arm64 run weekly; the threaded wasm job moved to #108). **#9 licence and governance** merged (PR #126; its CI box is ticked when #3
lands). **#2 workspace** merged (PR #123) after an independent review found the
portability rule narrower than claimed; fixed (lint lists, `forbid` at the crate root,
`tools/check_core_portability.py`). **#11 patent** closed. **#10 name**: the owner keeps
"Everygraph" knowing a US application for the identical mark was filed 2026-11-17; watch
serial 50223142 at every milestone.

## Next actions

1. Each returning PR gets an independent review (`reviewer`, Opus high) before merge; the
   coordinator re-runs the checks it relies on.
2. After #5 merges: #4 benchmark harness (Opus high), then #8 lab reproducibility.
3. After #118 reports: record it in FINDINGS, update D4 and the public numbers, write the
   ADRs (#7), then run the end-to-end tracer on Friendster under 16 GiB (#15).
4. When #12 is merged, M2 can start: container spec (#29), I/O layer (#30), codec (#31 area —
   check numbers with `plan/next.py`), arrays and accountant.

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

## Needs the owner's hands (nothing here blocks development)

Tell the owner when each becomes due, with everything prepared.

- At publication: enable GitHub private vulnerability reporting; authorize `Signed-off-by` on
  commits made by agents under the owner's account, then set the repository variable
  `DCO_ALL_PULL_REQUESTS=true`.
- Accounts with 2FA on PyPI, crates.io and npm — at the first release.
- A trademark and patent professional (#122) — before anything is public.

## Lessons (append, never delete)

- 2026-11-25 — The name was chosen after checking package registries only; a trademark
  application for the identical name had been filed a week earlier. Screen trademark
  registers (USPTO via TSDR, TMview through a headless browser) before proposing any name.

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
- 2026-11-25 — An oracle reference run (`--jobs 6`, no limit) reached 61.5 GB resident,
  the host swapped 25 GB (other people's services included) and experiment #118 had to
  discard its rows and restart. Cause of the damage: the brief said "no benchmarks" but set
  no memory limit. Now a rule (CLAUDE.md § 7): every data-loading run under `MemoryMax`,
  and the limit is stated in each brief.

## Journal

- **2026-11-25 (evening)** — First code on `main` (#2). Trademark finding on the name; owner
  keeps it. Model policy: Opus by default, Fable by escalation. Wave 2 launched (#3, #5, #6, #9).
- **2026-11-25 (later)** — Owner approved the previews. #119 merged; M0 started (#2, #10, #11)
  together with the M1 experiment #118.
- **2026-11-25** — Detailed style guide built (nine pages, three subagents). Measurements
  overturned Claude's hand-picked data palette and five contrast values; tokens corrected.
- **2026-11-24 (later)** — Owner delegated every decision and asked for previews of all
  matters of taste. Added #119; produced the previews with three subagents.
- **2026-11-24** — Project conceived as a rethink of `largegraph`. Feasibility lab (8
  experiments), design baseline, 117-issue plan published. Full review of the plan found
  ten errors; fixed, one issue added (#118), three issues moved between milestones.
  Owner delegated execution. Machine cleaned (239 GB → 54 GB), toolchain installed,
  operating rules written.
