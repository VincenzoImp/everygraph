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
3. Run `python3 plan/next.py` to see which issues are ready (all dependencies closed).
4. Only then pick work. If STATE.md and reality disagree, fix STATE.md first.

Reference documents, in order of authority: `docs/DESIGN.md` (decisions),
`docs/FINDINGS.md` (evidence), `docs/ROADMAP.md` (order), `plan/issues/*.md` (the work;
GitHub issues are published from these files with `plan/publish.py`).

## 2. Unit of work

- One issue → one branch (`m2-03-adjacency-codec`) → one pull request that says
  `Closes #N`. No direct commits to `main` except documentation of state (`docs/STATE.md`).
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

## 8. Reserved for the owner — stop and ask

These are irreversible or commit the owner to others. Several are also blocked in
`.claude/settings.json`.

- Making the repository or the project board public; any public announcement.
- Publishing packages (crates.io, PyPI, npm) or releases.
- The name, trademark and patent conclusions (research them; the decision is the owner's).
- Spending money (cloud machines, domains, services).
- Contacting people or organizations on the owner's behalf.
- The go/no-go at the end of M1, and any change to the three product promises.
- Anything touching the owner's other repositories or files outside this project.

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
