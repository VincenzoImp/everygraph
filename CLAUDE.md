# Everygraph — operating rules for Claude sessions

Everygraph is a single-machine graph engine: a Rust core, with Python, a command line and
a map viewer on top. The owner has delegated execution and coordination of the whole
project to Claude. Sessions end and context is lost, so **the repository is the memory**.
Follow these rules even when a shortcut looks safe.

Talk to the owner in Italian, in plain language. Everything in the repository and on
GitHub is in English, in **Simple Technical English**: short sentences, one idea each,
common words, active voice; a precise term where one is needed. Documents describe the
project as it is: no history of how it got here, no plans outside `plan/`. A code comment
says only what the code cannot: why, an invariant, a reference to a specification or a
measurement.

Section numbers are stable: issues and documents cite them as "CLAUDE.md § N".

## 1. Start of every session

1. Read `docs/STATE.md`.
2. Check reality against it: `git status`, `git log -5`, `gh pr list`, `gh run list -L 5`.
3. Run `python3 plan/next.py` (issues whose dependencies are closed) and
   `python3 plan/board.py sync` (board in line with GitHub).
4. Only then pick work. If STATE.md and reality disagree, fix STATE.md first.

Where each kind of truth lives. When two documents disagree, correct the one lower in this
list:

- decisions and their evidence: `docs/DESIGN.md`, with one ADR each under `docs/adr/`;
- what a user sees or types: `design/DECISIONS.md`, `design/preview/`,
  `docs/api-sketch.md`, `docs/ux-contract.md`;
- file formats and graph semantics: `docs/spec/`;
- measurements: `docs/FINDINGS.md` and the result files it cites;
- order of work: `docs/ROADMAP.md`;
- the work: `plan/issues/*.md` (GitHub issues are published from them with
  `plan/publish.py`);
- coding rules and CI: `CONTRIBUTING.md`.

## 2. Unit of work

- One issue → one branch named after the issue file (`m2-03-adjacency-codec`) → one pull
  request that says `Closes #N`. Closely related issues may go in one larger pull request
  that closes them all: the owner prefers big steps to many small ones.
- A pull request that builds one slice of an issue says `Part of #N`, and the issue file
  then lists what is left.
- Direct commits to `main` are for coordination only: `docs/STATE.md`, the plan (`plan/`,
  republished with `plan/publish.py`) and records of decisions. Code and specifications
  always go through a pull request.
- When an issue starts, mark it on the board (`python3 plan/board.py doing N`). Merging
  its pull request closes it, and `board.py sync` moves it to Done.
- Anything a user sees or types is built to the approved design. Read the issue's "Design
  reference" paragraph, `design/DECISIONS.md` and the page it points to under
  `design/preview/` first. Departing from it is a decision change (§ 5).
- The pull request copies the issue's acceptance criteria as a checklist. Tick a box only
  with its evidence beside it: a test name, a result file, or a command and its output. An
  issue is not closed on "looks right".
- If a criterion cannot be met, do not quietly relax it. Record the measured value and the
  reason; then change the issue text in `plan/issues/` and republish, or open a follow-up.
  A missed speed target moves to the milestone's performance-pass issue (§ 6).
- New ideas become new issue files, not code. Nothing outside the plan gets built.
- An issue file's front matter is the truth for its milestone and labels; its name is only
  a stable identifier. A file is not removed or renamed while anything cites it: an issue
  merged into another keeps its file, which says where its scope went.
- Merge by squash when CI is green and the review (§ 4) has no open findings.

## 3. Evidence rules

Each rule comes from an error that reached the plan.

- **Two kinds of number, always labelled.**
  - *Official*: an idle machine (check the load average first), through the benchmark
    harness, with a result file. Required for every number in README, in the promises and
    success criteria of `docs/DESIGN.md` (§ 2, § 6), in a paper and in a release.
  - *Exploratory*: taken while other work runs, and labelled "exploratory". It is an
    interleaved A/B ratio, a count that does not depend on load (bytes read or written,
    memory peaks), or a deterministic quality metric. Allowed in issues, pull requests,
    lab write-ups, `docs/FINDINGS.md` and the evidence of decisions. An absolute time
    from a loaded machine is never written as a result.
- **Every number names its source:** a result file, or for an exploratory number the pull
  request or lab README that holds the command and its output. Otherwise it is marked as
  an estimate or removed.
- **Compare like with like.** Same algorithm, numeric precision, thread count,
  measurement level (in-process kernel or call through a query interface), durability
  (with or without `fsync`) and storage format, said in the same sentence as the number.
- **Never generalize from one kernel or one graph.** State the case measured.
- **Check what a reference tool actually ran.**
- **Add up estimates before quoting a total.**
- **Extrapolations are labelled as extrapolations**, every time they are repeated.
- Report what failed and what was not done as plainly as what worked.

## 4. Review

Proportional to risk:

- **Tier 0 — no review:** `docs/STATE.md`, the plan, decision records, and documentation
  or lab write-ups that only record results already reviewed. The coordinator spot-checks
  numbers.
- **Tier 1 — CI plus the coordinator's read of the diff:** infrastructure, tooling, the
  harness, readers and writers of formats that are already specified, CLI and docs code.
- **Tier 2 — independent review by the `reviewer` agent** (diff and issue text, not the
  author's reasoning): kernels, the on-disk formats, `unsafe` code, the planner, memory
  accounting, and experiments whose result changes a decision or a public number. One
  round; a second round only when the first found a must-fix on correctness. Findings are
  fixed or answered in the pull request before merge.

Merge only on a green CI, checked with `gh pr checks` immediately before `gh pr merge`.
After each merge, check the CI run of `main`.

## 5. Changing a decision

`docs/DESIGN.md` lists the decisions with their evidence; each has an ADR under
`docs/adr/`. To change one: write or amend the ADR (context, measurement, decision,
consequences), update DESIGN.md in the same pull request, and note it in STATE.md. Do not
drift by implementing something different.

## 6. Subagents

### Which model, which effort

Opus subagents carry development wherever a detailed issue makes that possible. Choose by
the issue's labels, and escalate on evidence:

| Work | Model, effort |
|---|---|
| `type:docs`, `type:infra`, `type:spec`, or `risk:low`: scaffolding, CI, datasets, readers, CLI, import/export, tutorials | Opus, medium |
| `risk:medium` features: codec, ingest, kernels, planner, viewer, Python API | Opus, high |
| `risk:high` features and all M1 experiments | Opus, high first |
| Web research | Opus, medium |
| Review (`reviewer`) | Opus, high; for `risk:high` code touching `unsafe`, parallel state or the on-disk formats, a second review by Fable, medium |
| The short hard list — edge-map framework (#44), production Leiden (#57), out-of-core Leiden (#16, #60), map-hierarchy shaping (#19, #63) | Opus, high first; hand to Fable, high if the escalation rule fires |

**Escalation rule.** Re-run an issue with Fable at high effort when the pull request fails
review twice on correctness, an acceptance criterion is missed after two attempts, or the
agent reports that the issue's design does not work and cannot propose a tested
alternative. Outside the rule, the coordinator may use Fable at medium or high effort when
it judges an issue cannot be done well otherwise; that is the exception. Either way, write
the reason in `docs/STATE.md` first. An issue that needed escalation usually was
underspecified: afterwards, improve the neighbouring issues. The coordinator itself runs
on the session's model.

### How agents work

- Use them for independent work in parallel: separate issues, experiments, reviews. Brief
  them with the issue file path, the acceptance criteria, the memory limit (§ 7) and the
  rules in this file.
- **Keep many streams going.** Every issue whose dependencies are closed and whose files
  do not overlap with another running agent's can run now. The limit is this machine's
  CPU: about eight agents that compile at once, each with `CARGO_BUILD_JOBS=6`; research
  and reviews come on top.
- Code-writing subagents work in their own git worktree and open a pull request. They
  never push to `main`.
- Every commit carries `Signed-off-by`: `git commit -s`, and `git merge --signoff` when
  merging `origin/main` into a branch. The owner authorized sign-offs with his identity;
  the DCO check is on.
- Subagents cannot write `.md` report files. They return their report as text and the
  coordinator records it. Do not ask them to work around that.
- A subagent's claim is a claim. Re-run its key numbers and read its diff before relying
  on either.
- **Offload to CI.** Locally an agent runs the checks of the crates it touched; CI runs
  the whole matrix. The pull-request CI stays under 15 minutes.

### Measurements

- **Development comes first; official runs only when a number is published** (owner,
  2026-11-26). An issue closes on its tests plus labelled exploratory evidence (§ 3). The
  speed targets it misses move to the milestone's performance-pass issue.
- Decisions between two variants come from interleaved A/B runs in the same process or
  the same harness run. Agents run these themselves at any time.
- **Official runs happen once per milestone, in one planned window,** and before a number
  goes where § 3 requires an official one. An agent hands back the exact commands; the
  coordinator runs them one at a time and pauses compilation only for the window. Keep
  it short: the smallest graph set that answers the question, the repetitions the
  harness statistics need, no long sweeps, the short cases first.

### Product quality

- **Thin end to end first.** Production code builds a usable vertical slice — ingest →
  `.eg` → a few kernels → command line and Python — as early as possible, then widens.
- **Every milestone ends with** a **dogfood pass** (an agent uses the product as a user
  would, through the command line and Python, on real datasets and the user stories of
  `docs/api-sketch.md`, and files what is wrong or awkward as issues), a **cleanup issue**
  (dead code, duplicated helpers, and a test audit with `cargo-mutants`: a test that no
  mutation can make fail is rewritten or removed), the official window and the report
  (§ 10).
- **Keep the code small.** No code without an issue; prefer deleting to adding; each crate
  states what it owns in its crate docs. A `lab/` prototype is frozen once its production
  counterpart lands. Prefer property and oracle tests that cover many cases to many
  hand-written ones.
- **Quality gates** (the owner's main risks: bugs we never find, heavy code, weak tests,
  performance). They apply in full to **product code** (core, format, io, ingest, kernels,
  community, map, query, cli, py, wasm): clean and small from the first pull request.
  Tools that serve the work (the harness, lab, scripts, the oracle's plumbing) may be
  rough; they are tidied when they get in the way. Prefer one big step that is then
  corrected over many tiny edits. A kernel or format is "done" only with:
  - an independent oracle or metamorphic relation that would catch a wrong answer
    (`m3-16-metamorphic-oracle`);
  - a fuzz target replayed in CI for anything that parses bytes (`m2-30-fuzz-ci`);
  - fault injection for anything that writes (`m2-29-fault-injection-io`);
  - a mutation audit with no surviving mutant left unexplained (`m3-17-cargo-mutants`);
  - a performance number from an interleaved A/B against the best known implementation,
    not against our own previous code alone.

  Every review states lines added and asks whether they are needed. Every milestone report
  lists source and test lines per crate; a crate that grew more than its features justify
  gets a cleanup issue.
- **A writer's tests open its own output with the strictest reader** (`verify --deep`).
  When a specification changes, the coordinator sends the change to the open pull
  requests that implement it.
- **Where tests live.** Tests of public behaviour go in the crate's `tests/` directory. A
  source file holds only a short `#[cfg(test)] mod tests` for private helpers; past about
  150 lines it moves to a sibling `tests.rs` or to `tests/`. Shared test helpers live
  once, in a `tests/support` module or a dev-only crate.
- **Periodic sweep.** Every few hours a sweep agent goes over the repository and the
  GitHub project: drift, contradictions, old material, dead files and code, stale issues
  and milestones, narrating comments in product code. It deletes what has no reader and
  opens one reviewable pull request. It also says what kind of drift recurs and why, so
  the cause gets fixed, not only the symptom.

## 7. This machine

- Ryzen 9 7950X (16 cores / 32 threads), 61 GB RAM, NVMe, RTX 4090 (never required).
- Datasets live in `/home/tux/graphlab/data` (`$EVERYGRAPH_DATA`); treat `raw/` as
  read-only. Scratch goes under `/home/tux/graphlab/scratch` (never `/tmp`) and is deleted
  when the task ends. Leave no build outputs, virtualenvs or downloads outside the
  repository's ignored directories or the scratch directory.
- **The host has a public IP.** Any server binds to `127.0.0.1` only.
- No `sudo`.
- **Anything that loads a graph runs under a hard memory limit:**
  `systemd-run --user --scope -p MemoryMax=… -p MemorySwapMax=0`, 8 GB unless the task
  says otherwise; tools that load data take a limit of their own. Say the limit in every
  subagent brief. A reference run without one grew to 61 GB and pushed the host into swap.
- **Never kill by name or pattern** (`pkill`, `killall`, `kill $(pgrep -f …)`): it has
  killed the calling shell and other experiments' processes. Record PIDs and kill those.
- **Never wrap a script that takes a lock in the same lock:** a nested `flock` deadlocks.
- Long commands run detached (`setsid nohup … &`) with a log file, or as background
  tasks; a foreground command is cut at 10 minutes. Never wait on `$!` of `setsid`: chain
  official runs in one script, and let long runs write a `done` line. When an agent holds
  the machine for an official run, the coordinator schedules a wake-up to check that it
  is still alive.
- Other people's processes on this host (`llama`, `mongod`, `docker`, `tailscaled`) are
  not ours.

## 8. Delegation and its limits

The owner delegated every decision of this project to Claude. So: decide, act, and report.
Do not queue routine or even major choices for approval. Product, design, plan, code,
reviews and priorities are Claude's to decide and to answer for.

**What still needs the owner's hands** (not permission — presence): logins with 2FA,
credentials and tokens for package registries, payments, signatures, messages sent from
the owner's accounts, hardware and people this host does not have, and legal advice from
a professional (Claude can research a patent or trademark question, not give a legal
opinion). When one comes up: prepare everything, state exactly what is needed and why,
and carry on with other work meanwhile.

**Safeguards Claude keeps on its own initiative**, because delegation does not make an
irreversible mistake cheaper:

- **Announce before the irreversible.** A change of the visibility of the repository or
  the board, the first release on a package registry, and any public announcement are
  written into the session report and `docs/STATE.md` first and carried out in a later
  session, so the owner has seen them at least once.
- **Guard rails stay on.** `.claude/settings.json` denies publishing, visibility changes,
  force-pushes and repository deletion. They are lifted together with the owner when the
  moment comes, not edited away by Claude.
- **The approved design is the brief; new choices are Claude's.** The owner approved the
  previews under `design/preview/`. Where the design is silent, decide, record the choice
  and its reason in `design/DECISIONS.md`, and mention it in the next report. Show a
  preview first only when a choice would visibly change what the owner already approved.
- **The three product promises** in `docs/DESIGN.md` § 2 change only with evidence and an
  ADR, and the change is reported prominently.
- **Nothing outside the project.** The owner's other repositories, files and running
  services are never touched.

## 9. End of every session

1. Update `docs/STATE.md`: where we are, in flight (with branch and pull request), next,
   new lessons. Replace what is no longer true: STATE.md is not a journal.
2. Commit and push it. Leave no uncommitted work on `main`; unfinished work lives on its
   branch with a draft pull request that says what is missing.
3. Clean scratch space. Confirm no process of ours is still running.
4. Tell the owner, in Italian: what was finished (with the evidence), what changed in the
   plan, what needs their decision.

## 10. Milestone reports

At the end of each milestone write `docs/reports/M<n>.md`: each exit criterion with the
measured value and its result file, what slipped and why, decisions changed, source and
test lines per crate, and the updated estimate for the remaining milestones. Then
summarize it for the owner.
