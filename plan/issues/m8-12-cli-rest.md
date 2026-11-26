---
title: "Command line, the rest: map, open, query, doctor, stopping, completions"
milestone: M8 · CLI, docs and v0.1 launch
labels: type:feature, area:cli, P0, risk:low, size:M
---
**Why.** {{#m8-01-cli}} built `run`, `info`, `export` and `compact` on the graph file, with `clap`, the result store and the three forms of a result. What is left of the command line waits on features that are not merged yet; this issue takes each as it lands, in the same code (`crates/everygraph-cli`) and to the same contract (`docs/ux-contract.md`, `docs/api-sketch.md` § 5).

**Scope**
- [ ] `everygraph map GRAPH [VIEW] [--out FILE --reveal full|summary]`: commits its `communities`, `hierarchy` and `layout` phases as stored results as each finishes, so a stopped `map` keeps them and a later run shows them as `kept` (`docs/ux-contract.md` § 5.1, C-2); `--out` without `--reveal` is a usage error with the two ways forward (§ 5.2); the result block and its two hints (§ 4.4)
- [ ] `everygraph open GRAPH`: serves the map on `127.0.0.1` only until `Ctrl-C`, with the notice of § 6.3
- [ ] `everygraph query neighbors|khop|path GRAPH NODE [NODE]` with `--limit`, `--timeout`, `--direction`, `--max-hops`, `--avoid-hubs`; a result cut short exits with 0 and carries `truncated` (M-10, M-11); `UnknownNode` exits with 6
- [ ] `everygraph run communities` with `--use-direction`, `--deterministic/--fast` and `--seed`; `run triangles`
- [ ] `everygraph doctor`: cores, memory, storage and its bandwidth, cgroup limits, a suggested budget; its JSON schema (`urn:everygraph:schema:doctor:1`, `docs/api-sketch.md` § 5.3)
- [ ] Stopping: SIGINT and SIGTERM during a long command print the stop block (§ 5.4) and, with `--json`, the stop document, and exit with 130 or 143; the work between two cancellation checks stays within C-4 (the C-4 test of § 8)
- [ ] The interruption and `kill -9` tests of `map` (§ 8, "Interruption tests", "Hard stops")
- [ ] `run --dry-run` and the plan block of `run` once the planner gives estimates ({{#m3-02-planner}}); `--temp-dir` on `run`, `map` and `export`
- [ ] Shell completions (bash, zsh, fish, PowerShell) and a man page, generated from the `clap` definitions
- [ ] The single static binary per platform, also installed by the Python wheel

**Acceptance**
- `everygraph ingest edges.csv && everygraph map edges.eg && everygraph open edges.eg` produces a browsable map of a 1 M-edge file in under 10 s on a laptop (measured by the harness on an idle machine)
- Every command's `--json` document validates against its closed schema in the tests, the stop document included, with exit statuses 130 and 143
- The golden transcripts of `tests/golden/` cover `map`, `query` and `doctor`, and pass `docs/tools/check_transcripts.py`

**Depends on** {{#m8-01-cli}}, {{#m7-01-point-queries}}, {{#m7-02-shortest-paths}}, {{#m4-07-map-hierarchy}}, {{#m5-03-tile-builder}}, {{#m5-05-viewer-core}}, {{#m3-02-planner}}

**Design reference.** As {{#m8-01-cli}}: `design/preview/cli.md` and `design/preview/style/terminal.html`; the rows "Reading the command line, with `clap`", "Short options" and "`everygraph run` before the planner" of `design/DECISIONS.md`.
