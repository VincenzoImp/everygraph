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

As of 2026-11-25 evening — **M2 started in parallel** (rules for speed: CLAUDE.md §4, §6):

- **#29 I/O layer** (`everygraph-io`), **#30 adjacency codec** (`everygraph-format`, Tier 2),
  **#31 arrays and memory accountant** (core, Tier 2), **#32 text reader** and **#34 id
  dictionary** (`everygraph-ingest`, #34 Tier 2), **#36 graph-semantics spec**
  (`docs/spec/`). Each hands back official commands for a batched measurement window.


- **Ingest and the drive** — merged (PR #137, ADR 0026). Was: doc agent (Opus medium), branch `ingest-drive-finding`: records
  the harness's finding that Friendster ingest is bounded by the SSD's sustained write rate
  (29 s sustained vs 18–20 s on a rested drive; the lab's 21 s was one rested run) and an ADR
  on the wording of promise 1 ("about 100 M edges/s"). Report it prominently to the owner.
- **#16 out-of-core Leiden** — official run done (PR #143): Friendster completes under 16 GiB at ≤ 1.13× any in-RAM reference, deterministic hash identical in RAM and out of core, map-shaped Leiden under 16 GiB in 164 s. The review (comment on the PR) found the in-RAM reference not like for like (different aggregation) and several numbers without files; the agent is adding a same-algorithm in-RAM reference (short window) and the fixes. The ADR 0015 amendment (seeded relabelling) is **not** adopted: experiment `m1-14-relabel-at-ingest` decides it.
- **Measurement windows are serialized by the coordinator**: one official run at a time,
  nothing else compiling meanwhile.

Done today: **#142 harness defects** fixed (PR #144: a failed warm-up keeps its measurement; a timed-out run counts its orphaned children's I/O through a child subreaper; I/O marked unknown when it cannot be established). Open small points: a test that checks "not disturbed" can still be tripped by noise starting mid-run; a run whose descendant escaped its scope is still `ok` if it exits 0 (candidate follow-up). **#15 tracer** closed (PRs #138, #140, #141): the chain does **not** fit 16 GiB on Friendster — blockers: community detection with a map-shaped hierarchy (pass-0 aggregation; every level kept in memory), layout must not load the graph into the heap, tile builder (all tiles plus per-zoom state in RAM, one graph pass per zoom). Unlimited it completes in 455.5 s of stage time. The 16 GiB criterion is annotated, not changed, until #16 reports. Literature survey in `docs/research/memory-bounded-algorithms.md` (PR #139). **#4 benchmark harness** merged (PR #136) after a review and three official windows: 408 + 6 official rows, none disturbed; criteria amended with the measured causes (ingest references were single lab runs; the SSD write cache; k-core at 16 threads is high-variance). **M0: 10 of 14 closed**; left #8 (lab reproducibility) and #10 (trademark watch). **#6 correctness oracle** merged (PR #131) after an independent review and a fix round: three witnesses (Rust, NetworkX, igraph/scipy), 27 invariants each shown able to fail, seeded-bug matrix enforced by the command, sparse-id families, golden files for all 17 tier-1 datasets, CI job about 3 billed minutes; it found that the lab PageRank's bits follow the scan chunking (now a requirement in #44/#47). **#118** closed (PRs #133, #134): **D4 amended** — compressed adjacency stays the default, with five conditions (ADR 0004, DESIGN D4, FINDINGS § M1-13); public numbers in README and DESIGN restated on the default storage; the 2 GB PageRank target (1.35×) and the 8 GB ingest target (needs a budgeted encoder) have no margin and are annotated in DESIGN § 6. Follow-ups to schedule in an idle window: WCC with batched prefix decode (~10 min), Friendster encode at 1 thread (~3 min). **#7 ADRs** merged (PR #132): 25 ADRs under `docs/adr/` plus 0000 on overturned assumptions; DESIGN, FINDINGS and README corrected where they said more than the result files (13 statements, list in the PR); **D10 is open** (mask vs materialized view: the "~25%" rule was never measured, #93 derives it); **D14 is widened** (identical results across budgets and regimes; unmeasured so far); D4's ADR has a pending section for #118. To watch: with the default of one Leiden iteration the idle result is 2.7–3.4% below the best reference, while the §6 criterion (within 1%) is written for three iterations — #57 settles the default. Promise 1 restated by ADR 0026 (PR #137): about 100 M edges/s where the ingest's writes fit in memory or the drive's fast-write burst (harness: lj 90 M, orkut 150 M, Friendster rested 92–102 M edges/s), bounded by the drive's sustained write rate otherwise (Friendster 62 M at 32 GB, 49 M at 8 GB). **#12 API sketch and #13 UX contract** merged (PRs #125, #124) after two rounds of independent review and a joint alignment pass; their obligations are written into the issues that build them (commits "Plan: …" of 2026-11-25) and the choices are in `design/DECISIONS.md`. **#5 datasets** merged (PR #129) after an independent review and a fix round (17 tier-1 datasets, 75 MB, all usable by anyone; follow-up for real bipartite and co-occurrence graphs in `m3-13-dataset-gaps`). **#3 CI** merged (PR #127; all six Linux jobs green on `main`, run 45530945188; macOS, Windows and Linux arm64 run weekly; the threaded wasm job moved to #108). **#9 licence and governance** merged (PR #126; its CI box is ticked when #3
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

## Measurement queue (official runs, batched into one window)

- #29 I/O layer: `everygraph-bench run io --official --only friendster` (~4 min), after the review fixes.
- #31 arrays and accountant: `everygraph-bench run segvec --official` (~12 min, 40 GiB free), after review.
- Follow-ups from #118: WCC with batched prefix decode (~10 min), Friendster encode at 1 thread (~3 min).

## Needs the owner's hands (nothing here blocks development)

Tell the owner when each becomes due, with everything prepared.

- At publication: enable GitHub private vulnerability reporting; authorize `Signed-off-by` on
  commits made by agents under the owner's account, then set the repository variable
  `DCO_ALL_PULL_REQUESTS=true`.
- Accounts with 2FA on PyPI, crates.io and npm — at the first release.
- **Publication (owner's decision, 2026-11-25):** the owner runs `gh repo edit VincenzoImp/everygraph --visibility public --accept-visibility-change-consequences`. Then Claude enables private vulnerability reporting, sets `DCO_ALL_PULL_REQUESTS=true` after the owner authorizes sign-offs, and keeps CI on GitHub-hosted runners (free once public). GitHub Actions: no spending.
- Package names: placeholders ready in `/home/tux/graphlab/name-reservation` (crates `everygraph`, `everygraph-cli`; PyPI `everygraph`; npm `everygraph`); publishing needs the owner's accounts with 2FA — all four names were free on 2026-11-25.
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
- 2026-11-25 — An agent that owned the machine died on a server error after its
  benchmark had finished; nothing else was allowed to run and 2.5 hours were lost. When one
  agent holds the machine, the coordinator schedules a fallback wake-up to check that the
  holder is still alive, and long runs write a `done` line the coordinator can see.
- 2026-11-25 — The coordinator merged PR #138 with five CI jobs still pending (two green). Merge only on a green CI, read with `gh pr checks` immediately before `gh pr merge`; `gh pr merge --auto` can wait for it.
- 2026-11-25 — Experiment reports overstate in predictable places (a check that cannot fail counted as evidence; a memory peak attributed to the wrong phase). An independent review of lab PRs catches these before they reach FINDINGS; keep it even for lab code.

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
