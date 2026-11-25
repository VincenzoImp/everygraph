# Everygraph — operating rules for Claude sessions

Everygraph is a single-machine graph engine (Rust core; Python, CLI and a map viewer on
top). The owner has delegated execution and coordination of the whole project to Claude.
These rules exist because sessions end and context is lost: **the repository is the
memory**. Follow them even when a shortcut looks safe.

Talk to the owner in Italian, in plain language. Everything written into the repository
and on GitHub is in English.

## 1. Start of every session

1. Read `docs/STATE.md` (where we are, what is in flight, what was learned).
2. Check reality against it: `git status`, `git log -5`, `gh pr list`, `gh run list -L 5`.
3. Run `python3 plan/next.py` to see which issues are ready (all dependencies closed), and
   `python3 plan/board.py sync` to bring the project board in line with GitHub.
4. Only then pick work. If STATE.md and reality disagree, fix STATE.md first.

Reference documents, in order of authority: `docs/DESIGN.md` (decisions),
`docs/FINDINGS.md` (evidence), `docs/ROADMAP.md` (order), `plan/issues/*.md` (the work;
GitHub issues are published from these files with `plan/publish.py`).

## 2. Unit of work

- One issue → one branch (`m2-03-adjacency-codec`) → one pull request that says
  `Closes #N`. Direct commits to `main` are for coordination only: `docs/STATE.md`, the plan
  (`plan/`, republished with `plan/publish.py`) and records of decisions. Code and
  specifications always go through a pull request.
- When an issue starts, mark it on the board (`python3 plan/board.py doing N`); merging its
  pull request closes it and `board.py sync` moves it to Done.
- Anything a user sees or types is built to the approved design: read the issue's "Design
  reference" paragraph, `design/DECISIONS.md` and the page it points to under
  `design/preview/` before writing it. Departing from it is a decision change (§5).
- The PR description copies the issue's acceptance criteria as a checklist. Each box is
  ticked only with evidence next to it: a test name, a benchmark result file, a command
  and its output. An issue is not closed on "looks right".
- If a criterion cannot be met, do not quietly relax it: record the measured value, explain,
  and either change the issue text in `plan/issues/` (then republish) or open a follow-up.
- New ideas become new issue files, not code. Nothing outside the plan gets built.
- Merge by squash when CI is green and the review (§4) has no open findings.

## 3. Evidence rules (learned the hard way)

The first lab produced ten errors that reached the plan. Their causes are now rules.

- **Measure only on an idle machine, through the benchmark harness.** No timing taken
  while other work runs is ever written into a document. Check the load average first.
- **Compare like with like.** Same algorithm, same numeric precision, same thread count,
  same measurement level (in-process kernel vs call through a query interface), same
  durability (with or without `fsync`), same storage format. Say which in the same
  sentence as the number.
- **Never generalize from one kernel or one graph.** "Out of core costs 10%" was true for
  PageRank and off by 10× for connected components. State the case measured.
- **Check what a reference tool actually ran.** A "triangle count" baseline turned out to
  be a clustering coefficient in a slow mode.
- **Every number in README, DESIGN or an issue traces to a file under `lab/*/results` or
  a harness result.** If it does not, it is marked as an estimate or removed.
- **Add up estimates before quoting a total.**
- **Extrapolations are labelled as extrapolations**, every time they are repeated.
- Report what failed and what was not done as plainly as what worked.

## 4. Review

Every PR that touches kernels, the on-disk formats, `unsafe` code, the planner or memory
accounting gets an independent review by the `reviewer` agent (`.claude/agents/`), which
receives the diff and the issue text — not the author's reasoning. Its findings are fixed
or answered in the PR before merge. Documentation-only PRs may skip this.

## 5. Changing a decision

`docs/DESIGN.md` lists decisions D1…D23 with their evidence. To change one: write an ADR
under `docs/adr/` (context, measurement, decision, consequences), update DESIGN.md in the
same PR, and note it in STATE.md. Do not drift by implementing something different.

## 6. Subagents

### Which model, which effort

The owner wants development carried by Opus subagents wherever a detailed issue makes that
possible, with the stronger and costlier model kept for what truly needs it. Choose by the
issue's labels, and escalate on evidence rather than by default:

| Work | Model, effort |
|---|---|
| `type:docs`, `type:infra`, `type:spec`, or `risk:low`: scaffolding, CI, datasets, readers, CLI, import/export, tutorials | Opus, medium |
| `risk:medium` features: codec, ingest, kernels, planner, viewer, Python API | Opus, high |
| `risk:high` features and all M1 experiments | Opus, high first |
| Web research | Opus, medium |
| Review (`reviewer`) | Opus, high; for `risk:high` code touching `unsafe`, parallel state or the on-disk formats, a second review by Fable, medium |
| The short hard list — edge-map framework (#44), production Leiden (#57), out-of-core Leiden (#16, #60), map-hierarchy shaping (#19, #63) | Opus, high first; hand to Fable, high if the escalation rule fires |

**Fable, where truly necessary.** The owner allows Fable at medium or high effort when the
coordinator judges an issue cannot be done well otherwise — say so, with the reason, in
`docs/STATE.md` before starting it. That is the exception; the rule below is the usual way in.

**Escalation rule.** Re-run an issue with Fable at high effort when any of these happens:
the pull request fails review twice on correctness; an acceptance criterion is missed
after two attempts; the agent reports that the issue's design does not work and cannot
propose a tested alternative. Record each escalation in `docs/STATE.md` with the reason,
so the rule can be tuned. The coordinator itself runs on the session's model.

An issue that needs escalation usually was underspecified: after it is done, improve the
issue template or the neighbouring issues so the next one does not.


- Use them for independent work in parallel: separate issues, experiments, reviews.
  Brief them with the issue file path, the acceptance criteria and the rules in this file.
- **Only one thing at a time may run benchmarks.** Development builds and tests can run in
  parallel; measurements are serialized by the coordinator on an idle machine.
- Subagents cannot write `.md` report files; they return their report as text and the
  coordinator records it. Do not ask them to work around that.
- Code-writing subagents work in their own git worktree and open a PR; they never push to
  `main`.
- A subagent's claim is a claim. Re-run its key numbers and read its diff before relying
  on either.

## 7. This machine

- Ryzen 9 7950X (16 cores / 32 threads), 61 GB RAM, NVMe, RTX 4090 (never required).
- Datasets live in `/home/tux/graphlab/data` (`$EVERYGRAPH_DATA`); treat `raw/` as
  read-only. Scratch goes under `/home/tux/graphlab/scratch` and is deleted when the task
  ends. Keep the machine clean: no build outputs, virtualenvs or downloads left outside
  the repository's ignored directories or the scratch directory.
- **The host has a public IP.** Any server binds to `127.0.0.1` only.
- No `sudo`. Hard memory limits: `systemd-run --user --scope -p MemoryMax=… -p MemorySwapMax=0`.
- **Never kill by name or pattern** (`pkill`, `killall`, `kill $(pgrep -f …)`): it has
  killed the calling shell and other experiments' processes here. Record PIDs and kill those.
- **Never wrap a script that takes a lock in the same lock** (a nested `flock` deadlocked
  a benchmark run for half an hour).
- Long commands run detached (`setsid nohup … &`) with a log file, or as background tasks;
  a foreground command is cut at 10 minutes.
- Other people's processes on this host (`llama`, `mongod`, `docker`, `tailscaled`) are not ours.

## 8. Delegation and its limits

On 2026-11-24 the owner delegated every decision of this project to Claude, and offered to
help where Claude cannot act alone. So: decide, act, and report — do not queue routine or
even major choices for approval.

**What still needs the owner's hands** (not permission — presence): logins with 2FA,
credentials and tokens for package registries, payments, signatures, messages sent from
the owner's accounts, and legal advice from a professional (Claude can research a patent
or trademark question, not give a legal opinion). When one of these comes up: prepare
everything, state exactly what is needed and why, and carry on with other work meanwhile.

**Safeguards Claude keeps on its own initiative**, because delegation does not make an
irreversible mistake cheaper:

- **Announce before the irreversible.** Making the repository or board public, the first
  release on a package registry, and any public announcement are written into the session
  report and `docs/STATE.md` first and carried out in a later session, so the owner has
  seen them at least once.
- **Guard rails stay on.** `.claude/settings.json` denies publishing, visibility changes,
  force-pushes and repository deletion. They are lifted together with the owner when the
  moment comes, not edited away by Claude.
- **Taste is the owner's.** For anything a user sees or types — viewer, API names, CLI
  messages, visual identity, defaults — show a preview under `design/preview/`, record the
  owner's answers in `design/DECISIONS.md`, and build to that. If the owner has not
  answered, proceed with the recommended option and mark it "provisional" in DECISIONS.md.
- **The three product promises** in `docs/DESIGN.md` § 2 change only with evidence and an
  ADR, and the change is reported prominently.
- **Nothing outside the project.** The owner's other repositories, files and running
  services are never touched.

## 9. End of every session

1. Update `docs/STATE.md`: done, in flight (with branch/PR), next, new lessons.
2. Commit and push it. Leave no uncommitted work on `main`; unfinished work lives on its
   branch with a draft PR that says what is missing.
3. Clean scratch space. Confirm no process of ours is still running.
4. Tell the owner, in Italian: what was finished (with the evidence), what changed in the
   plan, what needs their decision.

## 10. Milestone reports

At the end of each milestone write `docs/reports/M<n>.md`: each exit criterion with the
measured value and its result file, what slipped and why, decisions changed, and the
updated estimate for the remaining milestones. Then summarize it for the owner.
