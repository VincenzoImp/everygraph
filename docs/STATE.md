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

As of 2026-11-26, 07:45 (rules for speed: CLAUDE.md §4, §6; quality gates §6).

**Merged on the night of 25–26 November (23 PRs):** ADR 0028 pairs + `.events` (#184); research wave 2 plan (#185); `.eg` container (#170, closes #28); dictionary memory (#203; #34 closed with the official build-500m, 76.4 s under 5 GiB); docs consistency (#206); safe parallel scatter (#201, closes #189); edge-map framework (#202, #44 open: compressed official rows and WCC overhead); Leiden sub-round lab (#208); core numerics (#210, closes #187, #188); sweep 1 (#211); verify + fault injection + fuzz CI (#212, #41/#190/#191 open for items that need later issues); property columns (#213, closes #37); MSRV 1.100 (#214); build text → `.eg` and `everygraph ingest` (#215, #35 open for speed); results in the graph file, range maps, one writer (#216, closes #39, #204, #205); LDBC oracle (#217, closes #177); mmap hints lab (#221); ADR 0029 proposed (#222); edge-map spec fix (#224); hierarchy shaping (#225, closes #19); PageRank family + BFS/SSSP (#220, #47/#49 open for official numbers); CI Miri limit (#227); sweep 2 (#228).

**Open PRs:**
- #218 components + k-core + degrees: second review running (k-core must-fix fixed; ported to #220's kernels).
- #229 Python slice (#81), draft: waits for #218.

**Agents running:** production Leiden (#57), Louvain collapse (#17), ordering bake-off (#23), layout baseline (#175), CLI (#98, moving to clap, will reuse `everygraph_ingest::native` from #229).

**Next when they land:** in-adjacency (#38) then SCC (m3-20); #229 out of draft after #218; m2-31 (#200) code-size half with per-crate targets (sweep 2: format 2,706 → 9,453, ingest 10,584 → 14,171, core 3,640 → 6,079 src lines); the second measurement window.

**Decided by the coordinator tonight (records in DECISIONS, ADRs or issue files):** S1 lost-generation warning in `verify` (exit 0, `--strict` 5); dictionary dense form chosen from the data only, never the budget; `find_result` reads key and provenance only; MSRV 1.100 for a safe page backend; arrow-rs in the py crate with three deny exceptions; clap for the CLI; window splitting; merge order #220 before #218.

## Measurement queue (official runs, batched into one window)

- #34 id dictionary: closed 2026-11-26; build-500m official 76.4 s under MemoryMax 5G (bench/results/id-dictionary/20261126T042659Z-c0d7f248bb47.jsonl). (~20 min, ~45 GB disk).
- #19 hierarchy shaping (PR #162, after its fixes): prepare, then `everygraph-bench run hierarchy-shaping --official --skip-prepare …` (~50 min).
- #44 edge-map: official 05:30 run — lab and raw/stream rows ok (raw ÷ lab at 16 threads: PageRank f32 1.02, f64 1.02, BFS 1.03, WCC 1.13 with the proof WCC; at 1 thread 0.97–1.00), compressed/mmap cases failed on a store path shared with codec.toml (fixed in PR #224); re-run only those: `everygraph-bench run edge-map --official --data /home/tux/graphlab/data --only source=compressed --only source=mmap`. Was: `everygraph-bench run edge-map --official --data /home/tux/graphlab/data` (~15 min, estimate); closes #44 when the ≤ 5% overhead and compressed ≤ 1.0× raw criteria hold.
- #35 build (PR #215, after merge): `everygraph-bench run build --official --data /home/tux/graphlab/data` (~30 min, estimate; Friendster needs a 40 GB limit).
- #172 mmap hints / block cache (PR #221, merged): six `everygraph-bench run mmap-hints --official --only …` cases, ~3.5 h total, 44 GiB available needed — too long for one night window; run as its own window. Exploratory: MADV_RANDOM cuts bytes read 0.16× (layout 16G) and 0.08–0.15× (Leiden local moving 4–8G); harmful when the graph fits (32G: 8.7× more bytes). ADR 0029 (proposed, PR #222 merged): accepted after this official run.
- #47/#49 PageRank family and traversals (PR #220, after review): `everygraph-bench run kernels --official --data /home/tux/graphlab/data` (~25 min + the Friendster store once; GAP rows need the network once).
- #48/#50 components and k-core (PR #218, after review): `everygraph-bench run components-kcore --official --data /home/tux/graphlab/data` (~50 min, estimate; Friendster/twitter-sym rows at 40G).
- #167 page backend (PR #219, merged): `everygraph-bench run id-dictionary --official --only build-500m` and `--only hex-65m` (without the GLIBC_TUNABLES stopgap; compare with 20261126T042659Z-c0d7f248bb47.jsonl).
- #18 deterministic Leiden (PR #208, merged): `everygraph-bench run det-leiden --official` (lj+orkut ~40 min `--only scaling --only quality`; Friendster ~45 min `--only friendster`, estimates). Exploratory result: per-level rule clamp(next_pow2(n_level/65536), 4, 1024) cuts det/async at 16 threads to 2.04 (lj) / 1.53 (orkut) at 1 iteration from 3.39 / 2.55; small constants (4–16) oscillate and do more work.
- Follow-ups from #118: WCC with batched prefix decode (~10 min), Friendster encode at 1 thread (~3 min).

## Needs the owner's hands (nothing here blocks development)

Tell the owner when each becomes due, with everything prepared.

- At publication: enable GitHub private vulnerability reporting; authorize `Signed-off-by` on
  commits made by agents under the owner's account, then set the repository variable
  `DCO_ALL_PULL_REQUESTS=true`.
- Accounts with 2FA on PyPI, crates.io and npm — at the first release.
- **Published 2026-11-26 ~00:40:** the owner made the repository public; private vulnerability reporting enabled by Claude. CI minutes are free from now on. Sign-offs authorized by the owner 2026-11-26 (agents sign `Signed-off-by` with his identity); `DCO_ALL_PULL_REQUESTS=true` set; names reserved 2026-11-26 by the owner from his Mac, verified on the registries: crates `everygraph` and `everygraph-cli` 0.0.0, PyPI `everygraph` 0.0.0, npm `everygraph` (listed as 0.0.0-stage) (placeholders, no code); local copies deleted.
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

- 2026-11-26 02:54 self-check: the main slowdown tonight is serial review rounds on product PRs (author waits for review, reviewer waits for CI, author waits for review again; #170 took three rounds). Mitigation: send both reviews' findings to the author in one message, and skip a further round when no must-fix touches correctness. Second cause: a commit without Signed-off-by after the DCO check was turned on; agents now get the sign-off rule in their brief.
- 2026-11-26 03:54 self-check: load average 45 on 32 hardware threads with about nine code agents (kernels ×2, allocator, results cache, columns fixes, verify fixes, mmap experiment, LDBC, one review). CPU, not the network, is now the limit: more compiling agents would slow all of them. Rule of thumb for this machine: at most about 8 agents that compile at once (with `CARGO_BUILD_JOBS=6`), research and reviews on top.
- 2026-11-26 05:15: a 2.5-hour window idles every code agent (seven had their fixes written and waited to compile). Split long windows: run the short official cases first, let agents build and push, then run the long ones (det-leiden, mmap-hints) in a second window when few agents compile.
- 2026-11-26 05:54 self-check: after the window the machine fell to load 1.6 with four authors waiting on CI and two new agents starting — idle capacity again. Cause: every stream had reached a CI or review wait at once. Mitigation: keep a queue of ready experiments (M1 issues that need no merge) to start whenever load drops; started #23 and #175.
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
