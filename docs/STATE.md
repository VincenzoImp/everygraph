# State of the project

Read this first. It says where the project is now; it is not a journal. At the end of a
session, replace what is no longer true.

## Where we are (2026-11-27)

- **The repository** is public (`VincenzoImp/everygraph`). The board is
  `users/VincenzoImp/projects/7`. Every commit is signed off; the DCO check runs on every
  pull request.
- **The plan** is `plan/issues/*.md`: 206 issue files in 10 milestones (`python3
  plan/next.py` lists what is ready). The order of work and the critical path are in
  `docs/ROADMAP.md`.
- **The product** so far: the `.eg` container, the compressed adjacency and the property
  columns; the I/O layer, the memory accountant and the block cache; the text reader, the
  identifier dictionary and the graph build with columns; the edge-map framework with
  PageRank, BFS, SSSP, weak and strong components, core numbers and degrees; the planner;
  production Leiden and the map hierarchy; the `.egmap` file; point queries and filtered
  views; `everygraph ingest`, `verify`, `run`, `info`, `export` and `compact` (the command
  line does not use the planner yet, #247); a first slice of the Python package. Nothing is
  released.
- **Milestones.** M0 done. M1: lab round 2, experiments open. M2: open and P0 the allocator
  policy (#167) and the Parquet reader (#33). M3: kernels, planner and the official window
  done; open the semi-external source (#46), the no-OOM and determinism checks (#53, #54)
  and the performance pass (#240). M4: Leiden merged; next the out-of-core Leiden (#60), the
  hierarchy artifact and the map hierarchy (#62, #63). M5: layout (#67) and tile builder
  (#69) not started. M7: point queries and views merged.
- **CI** was redesigned on 2026-11-27 (#323–#330): one lint gate, a build with four test
  partitions, one job per concern, platforms in ci.yml, `ci-ok` at the end; rules in
  CONTRIBUTING.md.
- **Decisions in force** are in `docs/DESIGN.md` and `docs/adr/`; user-facing choices in
  `design/DECISIONS.md`.

## In flight

**Pace from 2026-11-27 (owner):** product development is paused until the owner says
otherwise. Only repository care: stale documents, and a review of the plan with the owner. No official window is due before the
M3 close or a number to publish; between them, interleaved A/B runs.

## Measurement queue

Official runs happen once per milestone and before a number is published (CLAUDE.md § 6).
Issues close on tests plus labelled exploratory evidence. Missed speed targets go to the
milestone's performance pass (M2 and M3: #240). The official commands of each issue are in
its pull request and its issue file. The M3 window runs the smallest set that answers the
M3 exit criteria.

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

**Open decision trigger:** ADR 0004 (compressed adjacency by default). Point queries read neighbours 2.1–2.5× slower compressed than raw on orkut at 16 threads (PR #307, exploratory, loaded machine), above the ADR's 1.5× threshold. The idle-machine run of m7-08 decides it; a result above 1.5× means a new ADR.

**Official window, 2026-11-27 07:21 (3b3befb5).** edge-map after #339: PageRank raw 0.92× of the previous run at 16 threads, 1.09–1.10× the lab (target ≤ 1.05, m3-22); engine-cc-compressed lj and orkut measured (`bench/results/edge-map/20261127T062155Z-3b3befb5cf63.jsonl`, `bench/results/components-kcore/20261127T064455Z-3b3befb5cf63.jsonl`). Watch: cc on the raw copy at 1 thread 1.09× (3 repetitions).

**Official window, 2026-11-26/27.** Done. The headline is in `bench/results/headline/20261126T223356Z-fd29f95d486e.jsonl`; the other suites (edge-map, semi-external, scc, transpose, components-kcore, references on orkut) are the result files of commit 588b76ab, all on revision 2510b9c1.

## Lessons that still change behaviour
- Read-only agents disturb an official window: the sweep's sub-agents ran greps during the window of 2026-11-27, and the harness marked 7 components-kcore rows disturbed. During a window, run only work bound by the network (2026-11-27).
- Never commit in the main checkout during a window: the next official run records the new revision (2026-11-27).
- A CI agent force-pushed its own branch once, before its pull request existed. The rule is no force-push at all, on any branch (2026-11-27).
- A window script that runs official suites back to back is refused from the second suite on: the first suite's own load trips the idle gate. Wait for the 1-minute load to fall between suites, and take `rc=$?` before any `$(…)` in the log line (2026-11-27).
- A nested `Cargo.lock` that builds workspace crates goes stale when a workspace crate gains a dependency (#288 → the reference driver); CI now checks every nested lock outside `lab/` (#318).

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
- Runs on `main` shared one CI concurrency group, so GitHub cancelled every pending run when a newer merge queued: most merged commits were never tested on `main` (fixed in PR #262). After the dev profile changed (#256), caches were cold and an advisory mutants shard hit its job limit and showed red.
- About twelve Opus and Fable agents at once (eight research agents, four implementers, reviewers and a synthesis) used up the account's 5-hour usage window at about 12:00, and every agent stopped for 20 minutes. Usage, not only CPU, limits parallelism. Resume in priority order and stagger: the critical path and reviews first, experiments and long research later.
- 12:42 sweep skipped on purpose: the Fable coherence pass (PRs #252, #253) went over the plan, rules and documents two hours earlier, and the usage window is the scarce resource. A sweep repeats only after a stretch of merges large enough to drift.
- Closing #57 and #21 made `check_adrs.py` fail on every open pull request, because ADRs and DESIGN still named them as holding open work. Rule: after each merge that closes an issue, the coordinator runs `board.py sync` and `docs/tools/check_adrs.py` and re-points the citations in the same push.
- Parallel authors picked the same next issue number (m3-25 to m3-28 three times, m8-12 twice). Rule: a subagent names a proposed issue file `mN-xx-<slug>.md`, and the coordinator gives it its number before the merge.
- Two pull requests green alone broke `main` together: #270 removed `FileSink::into_file` and #239 added an example that called it (fixed in #276). CI on each push to `main` (#262) caught it within the hour. Before merging a PR that removes or renames a public item, grep the other open PRs for it.
- 26 polling loops (`until grep …; do sleep …; done`) outlived the agents that started them, some for two days, waiting on files that would never change. Each self-check now lists our `sleep` loops with their parent's age and working directory (`ps -u tux`, `/proc/PID/cwd`) and stops, by PID, those whose agent has finished. Agents bound every wait loop with a count or a timeout.
- The 5-hour usage window ran out again at about 17:00, with 10–12 agents active since 12:20. The machine stood idle for 30 minutes (load 0). Keep about 6 agents at once and use medium effort where the table allows; resume in two waves (critical path and reviews first, then the rest).
- Wait loops of the form `until ! pgrep -f "<pattern>"` never end: the loop's own command line contains the pattern. Nine such loops of one agent ran for 2–3 hours. Agents wait on a recorded PID or a `done` line, with a bound (in the implementer brief).
- #46 and #45 closed on merge although their PRs said `Part of`: an earlier commit message said `Closes #N`, and the squash merge kept it. After merging a `Part of` PR, the coordinator checks the issue is still open and reopens it if not.
- Main's test job was red for about 90 minutes after #288: the map reader check found a partial `node_modules` in `target/`, which the Rust cache prunes (fixed in #306). The merges after #288 did not check main's run. Tools that install into `target/` check the installed version, not only that a directory exists.
- Four pull requests changed the edge-map `Source` trait at once (#291 prefix decode, #303 cache, #307 `list_prefix`, #315 views), and two designed prefix decode twice. A shared core interface now has one owning stream at a time (CLAUDE.md § 6).
- #46 closed a second time from PR #303's squash message: a squash lists every commit of the branch, including main's commits merged into it, so #254's `Closes #46` came along. The coordinator now merges with an explicit body (`gh pr merge --squash --body …`) that names only the PR's own closing keyword.

