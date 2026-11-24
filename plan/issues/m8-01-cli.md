---
title: Command-line interface
milestone: M8 · CLI, docs and v0.1 launch
labels: type:feature, area:cli, P0, risk:low, size:M
---
**Why.** The analyst persona and every automation path start at the CLI. The old prototype had twelve subcommands and a defensive tone; this one should have a small verb set and say what it will do before doing it.

**Scope**
- [ ] Verbs: `everygraph ingest`, `everygraph info`, `everygraph run <algorithm>`, `everygraph query`, `everygraph map`, `everygraph open`, `everygraph export`, `everygraph verify`, `everygraph doctor`
- [ ] Single static binary per platform, no runtime dependencies; also installed by the Python wheel
- [ ] Global options: `--threads`, `--memory`, `--temp-dir`, `--deterministic/--fast`, `--seed`, `--json`
- [ ] Preflight printed before long operations with `--dry-run` and `--yes` ({{#m0-12-ux-contract}})
- [ ] Progress with ETA on TTYs; quiet structured logs otherwise; meaningful exit codes
- [ ] Error messages that say what to do next (smallest sufficient budget, how to convert gzip, …)
- [ ] `everygraph doctor`: hardware probe, storage bandwidth probe, cgroup limits, recommended settings
- [ ] Shell completions; man page

**Acceptance**
- `everygraph ingest edges.csv && everygraph map && everygraph open` produces a browsable map of a 1 M-edge file in < 10 s on a laptop
- Every command supports `--json` output validated against a schema

**Depends on** M2–M5 features
