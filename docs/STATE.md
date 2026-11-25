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

As of 2026-11-25, six agents (Opus unless noted; policy in CLAUDE.md § 6):

- **#3 CI** — `m0-02-ci` (Opus medium). Includes the portability canary and cost discipline
  for Actions minutes on a private repository.
- **#5 datasets** — `m0-04-datasets` (Opus medium). Registry and fetcher in `everygraph-bench`.
- **#6 oracle** — `m0-05-oracle` (Opus high). Generators, reference runners, invariants,
  plug-in trait; proven against the lab kernels and seeded bugs.
- **#12 API sketch, #13 UX contract** — one specification agent, two PRs (Fable high, started
  before the model policy).
- **#118 compressed storage end to end** (M1) — experimenter (Fable high, started before the
  policy). It has the machine for measurements; nothing else may benchmark until it reports.

Done today: **#9 licence and governance** merged (PR #126; its CI box is ticked when #3
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

- Domains `everygraph.dev` and `everygraph.org` (appeared unregistered on 2026-11-25) — any time.
- A contact address for `CODE_OF_CONDUCT.md` (placeholder in the file) — before the repository is public.
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
