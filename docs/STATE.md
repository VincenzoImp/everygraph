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

As of 2026-11-25, 22:10 (rules for speed: CLAUDE.md §4, §6):

- **M2 closed with official numbers:** #29 I/O (direct 7.10 GB/s ≈ device), #30 codec (sizes 17.99 / 16.64 / 16.09 / 23.17 bits; twitter-sym scan 1.96 G/s at 1 thread, 25.5 at 16; random list 214 ns), #31 arrays and accountant (SegVec ≤ 1.02× Vec; no OOM in 8/8), #32 text reader (7.3–8.2 GB/s at 16 threads). Results under `bench/results/`.
- **#34 id dictionary**: code merged (PR #158). Official run `bench/results/id-dictionary/20261125T220117Z-c855fdd77cf9.jsonl` (not yet committed): hex-65m build 9.3–10.0 s at 16 threads (target < 15 s), 3.6 B mappings 34–37 s (target < 60 s), cold forward p50 93.8 µs split / 53.8 µs slot — but **build-500m was OOM-killed at MemoryMax 5 GiB with a 4 GiB budget** (anon 4.9 GiB at 47 s), where the exploratory run before the second-review fixes completed. The no-OOM promise fails here: investigate after the window (ledger vs allocator retention vs the new sample spill). Issue stays open.
- **State-of-the-art review** (owner's request, 2026-11-25): six researchers read the literature of each area in full text (storage and ordering, I/O and memory, kernels and determinism, communities and hierarchy, maps, ingest/format/query) plus one verifying a list of out-of-core systems; PDFs kept in `/home/tux/graphlab/papers/`, reports to `docs/research/sota-*.md`. Then the coordinator cross-checks every decision and turns what survives an A/B test into ADRs and issues. Until then #35 (CSR build), layout and tiles stay unstarted, so the review can still change them.
- **#19 hierarchy shaping** (PR #162, head 4efe4bc): review fixes done, exploratory groups committed (36 rows, metrics deterministic), stopping-defect figures corrected (lj: 6 passes instead of 7, Q 0.7450 vs 0.7477, cover-90 82 vs 49). Friendster official run queued after the dictionary run (log `/home/tux/graphlab/scratch/window-19.log`). Follow-ups written in the PR: per-region jobs for independence by construction; labelled binning for unsplittable leaves (ASK-GraphView).
- **#44 edge-map framework** (hard list, Opus high): code committed locally on `m3-01-edge-map` (224c856), last refactor (branded node ids `Node<'g>`, `unsafe trait Source`, unchecked indexing in `edge_map/node.rs`) not yet compiled — resumes after the measurement window, then PR and Tier 2 review (+ Fable second review: `unsafe` and parallel state). Exploratory A/B before the refactor (orkut, load 5–16): PageRank f32 0.95–0.98× lab; WCC/BFS 0.99–1.04× lab without bounds checks. Open point for #93/m7-04: reductions keyed by node-id blocks make a masked view and the same view materialized with new ids sum floats differently — D14 promises identity across view representations, so a rule is needed (e.g. materialized views keep the original ids' block boundaries).
- **#28 .eg container** (PR #170): spec v1.0, crash-injection and real `kill -9` tests, stdlib Python reader, ADR 0027 (one file in every state); Tier 2 review running. Follow-ups proposed: range maps in `MmapSource` (commit to a mapped graph), single-writer lock (needs Rust 1.89), parallel hashing.

- **Research waves** (records in `docs/research/`): wave 1 (six areas + coverage) done, decisions in `/docs/research/README.md` via the plan PR in progress; wave 2 done for human factors, approximation/Rust, dynamic graphs, measurement; numerics and testing still running. Found by wave 2 and acted on: **a real spec defect** — graph-semantics § 2.5 stores one entry per event but eg-adjacency § 1 only codes strictly increasing lists, so the default `duplicates="events"` cannot be written; decided: pairs in the adjacency, events in a `<name>.events` section (container kind 5; kind 6 = segment manifest), spec PR + ADR 0028 in progress; segment reservations sent to PR #170. Measurement review: ADR 0026's "~10 GB burst" is probably this host's drive state (weekly trim, ~0.6 TB/day written), not the drive; restate promise 1 as bytes per edge × drive rate after a probe right after the next fstrim (Mon 30 Nov 01:08) and 48 h later; harness needs intervals with their level, 6 official reps, randomized interleaving.

## Measurement queue (official runs, batched into one window)

- #34 id dictionary (merged; script waiting for idle): `everygraph-bench run id-dictionary --official` (~20 min, ~45 GB disk).
- #19 hierarchy shaping (PR #162, after its fixes): prepare, then `everygraph-bench run hierarchy-shaping --official --skip-prepare …` (~50 min).
- Follow-ups from #118: WCC with batched prefix decode (~10 min), Friendster encode at 1 thread (~3 min).

## Needs the owner's hands (nothing here blocks development)

Tell the owner when each becomes due, with everything prepared.

- At publication: enable GitHub private vulnerability reporting; authorize `Signed-off-by` on
  commits made by agents under the owner's account, then set the repository variable
  `DCO_ALL_PULL_REQUESTS=true`.
- Accounts with 2FA on PyPI, crates.io and npm — at the first release.
- **Publication (owner's decision, 2026-11-25; within hours, commit email exposure accepted; secrets scan of the whole history clean; the November private Actions quota is nearly spent, so CI may pause until then):** the owner runs `gh repo edit VincenzoImp/everygraph --visibility public --accept-visibility-change-consequences`. Then Claude enables private vulnerability reporting, sets `DCO_ALL_PULL_REQUESTS=true` after the owner authorizes sign-offs, and keeps CI on GitHub-hosted runners (free once public). GitHub Actions: no spending.
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
- 2026-11-25: `setsid nohup … & echo $!` can print the PID of a `setsid` that forked and exited at once, so a script waiting on that PID went ahead and two official runs started together; the harness lock refused the second. Chain official runs in one script, or wait on the PID of the real process (`pgrep -f <script path>`), never on `$!` of `setsid`.

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
