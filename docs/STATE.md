# State of the project

Read this first. It says where the project is now; it is not a journal. At the end of a
session, replace what is no longer true.

## Where we are (2026-11-26)

- **The repository** is public (`VincenzoImp/everygraph`). The board is
  `users/VincenzoImp/projects/7`. Every commit is signed off; the DCO check runs on every
  pull request.
- **The plan** is `plan/issues/*.md`: 169 issue files in 10 milestones, 43 closed. The
  order of work and the critical path are in `docs/ROADMAP.md`.
- **The product** so far: the `.eg` container, the compressed adjacency and the property
  columns; the I/O layer and the memory accountant; the text reader, the identifier
  dictionary and the graph build; the edge-map framework with PageRank, BFS, SSSP,
  components, core numbers and degrees; `everygraph ingest`, `verify`, `run`, `info`,
  `export` and `compact`; a first slice of the Python package. Nothing is released.
- **Milestones.**
  - M0: done, but for two issues to close by hand (#8 dropped, #10 merged into #122).
  - M1: 11 experiments closed, 11 open.
  - M2: 16 closed. Open and P0: the build carrying columns (#246), the allocator policy
    (#167), the Parquet reader (#33).
  - M3: the kernels are merged. Open and P0: the planner (#45), the semi-external source
    (#46), the no-OOM and determinism checks in CI (#53, #54).
  - M4: production Leiden is in review (#57).
  - M5–M8: not started, except the design system (#121) and the first command-line slice.
- **Decisions in force** are in `docs/DESIGN.md` and `docs/adr/`; user-facing choices in
  `design/DECISIONS.md`.

## In flight

Open pull requests:

| PR | Issue | What |
|---|---|---|
| #232 | #57 | Production Leiden on the edge-map framework |
| #236 | #200 | M2 cleanup: product code size |
| #239 | #38 | In-adjacency for directed graphs |
| #244 | #121 | Design system as shared assets |
| #245 | #193 | cargo-mutants on pull-request diffs |
| #249 | #45 | Planner |

Agents with a scratch directory and no pull request yet: edge rendering (#20), layout area
(#21), the semi-external source (#46), the reference runs (#55).

To close on GitHub by hand, because the plan merged or dropped them and no pull request
closes them: #8 (dropped), #10 (into #122), #59 (into the M4 performance pass, after
#232), #98 (into #247), then the M0 epic #1.

Next, when these merge: strong components (#223) after #239; the build's columns (#246);
out-of-core Leiden (#60) after #232 and #46; the M3 close (`m3-23-milestone-close`).

## Measurement queue

Official runs happen once per milestone and before a number is published (CLAUDE.md § 6).
Issues close on tests plus labelled exploratory evidence. Missed speed targets go to the
milestone's performance pass (M2 and M3: #240). The official commands of each issue are in
its pull request and its issue file. The M3 window runs the smallest set that answers the
M3 exit criteria.

## Needs the owner's hands

Nothing here blocks development today. Tell the owner when each becomes due, with
everything prepared.

- **Accounts with 2FA** on PyPI, crates.io and npm, at the first release. The names are
  reserved as 0.0.0 placeholders (crates `everygraph` and `everygraph-cli`, PyPI
  `everygraph`, npm `everygraph`).
- **Domains, a GitHub organization and the npm scope** `@everygraph` (#122).
- **A trademark and patent professional** (#122): advisable before the first release, not
  a blocker (`design/DECISIONS.md`, "When the repository becomes public").
- **Optional after 0.1, never a gate** (owner, 2026-11-26; `design/DECISIONS.md`, "Usability, accessibility and hardware checks"): a session with real users and screen-reader users, a run on an integrated GPU and on Safari, outside reviewers. Before 0.1 these checks are agent-driven sessions (Playwright) and rendering bounds (software renderers, the RTX 4090).

## Decided on 2026-11-26

- CI runs on GitHub-hosted runners only; no self-hosted runner is registered on this public host. Nightly jobs are GitHub-hosted scheduled workflows; tier-2 graphs and GPU checks run in the coordinator's official window on this host.
- Graphalytics eligibility (#209) is a goal beside 0.1, not a gate.

**Decision changes recorded:** ADR 0021 and DESIGN D21 amended on 2026-11-26 (PR #272): the query language comes by choosing an embedded engine (LadybugDB on Arrow CSR or Grafeo over a read-only `.eg` store) and bridging to it, not by our own parser; review in `docs/research/sota-review-2026-11.md`.

## Lessons that still change behaviour

Lessons that became rules are in CLAUDE.md and are not repeated here.

- Screen trademark registers (USPTO through TSDR, TMview) before proposing any name. The
  name was chosen from package registries only, and an application for it had been filed a
  week earlier.
- Measure palettes and contrast; a palette chosen by eye failed for colour-blind readers.
  Typeset user-facing text before calling it done: text that looked fine in Markdown broke
  when typeset.
- Measure the configuration that ships. The first headline numbers were taken on
  uncompressed arrays while the default storage is compressed.
- Experiment reports overstate in predictable places: a check that cannot fail counted as
  evidence, a memory peak given to the wrong phase. Keep the independent review for lab
  pull requests whose numbers reach FINDINGS.
- Review rounds in series are the main slowdown on product pull requests. Send the
  findings of both reviews to the author in one message, and skip a further round when no
  must-fix touches correctness.
- Each agent builds cold in its own worktree (24 worktrees held 149 GB, 10–28 GB of
  `target/` each) with no shared compiler cache; load reached 77 with eleven agents that
  ran `cargo`. Build efficiency is being measured.
- Load-sensitive tests (heap and retention bounds, file locks; #230) failed on four pull
  requests in one day, and each cost a CI rerun.
- When every stream waits on CI or review at once, the machine is idle. Keep a queue of
  ready experiments (M1 issues that need no merge) to start when the load drops.
- Runs on `main` shared one CI concurrency group, so GitHub cancelled every pending run when a newer merge queued: most merged commits were never tested on `main` (fixed in PR #262). After the dev profile changed (#256), caches were cold and an advisory mutants shard hit its job limit and showed red.- About twelve Opus and Fable agents at once (eight research agents, four implementers, reviewers and a synthesis) used up the account's 5-hour usage window at about 12:00, and every agent stopped for 20 minutes. Usage, not only CPU, limits parallelism. Resume in priority order and stagger: the critical path and reviews first, experiments and long research later.
- 12:42 sweep skipped on purpose: the Fable coherence pass (PRs #252, #253) went over the plan, rules and documents two hours earlier, and the usage window is the scarce resource. A sweep repeats only after a stretch of merges large enough to drift.
- Closing #57 and #21 made `check_adrs.py` fail on every open pull request, because ADRs and DESIGN still named them as holding open work. Rule: after each merge that closes an issue, the coordinator runs `board.py sync` and `docs/tools/check_adrs.py` and re-points the citations in the same push.
- Parallel authors picked the same next issue number (m3-25 to m3-28 three times, m8-12 twice). Rule: a subagent names a proposed issue file `mN-xx-<slug>.md`, and the coordinator gives it its number before the merge.
- Two pull requests green alone broke `main` together: #270 removed `FileSink::into_file` and #239 added an example that called it (fixed in #276). CI on each push to `main` (#262) caught it within the hour. Before merging a PR that removes or renames a public item, grep the other open PRs for it.
- 26 polling loops (`until grep …; do sleep …; done`) outlived the agents that started them, some for two days, waiting on files that would never change. Each self-check now lists our `sleep` loops with their parent's age and working directory (`ps -u tux`, `/proc/PID/cwd`) and stops, by PID, those whose agent has finished. Agents bound every wait loop with a count or a timeout.
- The 5-hour usage window ran out again at about 17:00, with 10–12 agents active since 12:20. The machine stood idle for 30 minutes (load 0). Keep about 6 agents at once and use medium effort where the table allows; resume in two waves (critical path and reviews first, then the rest).

