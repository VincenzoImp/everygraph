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
- [ ] Preflight printed before long operations, and with `--dry-run`; no `--yes`, since nothing asks ({{#m0-12-ux-contract}} S-4)
- [ ] Progress with ETA on TTYs; quiet structured logs otherwise; meaningful exit codes. Progress takes the piped form (nothing redrawn, no bars, one line per finished phase) also under `TERM=dumb` and with `--progress=plain`, so a screen reader is not fed redrawn bars (`docs/ux-contract.md` S-10, added 2026-11-25, `docs/research/README.md` H6)
- [ ] Error messages that say what to do next (smallest sufficient budget, how to convert gzip, …)
- [ ] `everygraph doctor`: hardware probe, storage bandwidth probe, cgroup limits, recommended settings
- [ ] Shell completions; man page
- [ ] `--json` gives exactly one document on standard output in every outcome, a stop included (stop document, exit 130/143); exit statuses 0–6, 130, 143 as `docs/ux-contract.md` § 6.4; the tests § 8 assigns to this issue: interruption and `kill -9` of `everygraph map`, the C-4 work-between-checks test, the golden transcripts
- [ ] Exit status 6 for `UnknownNode`; `--overwrite`; `ingest --self-loops`, `--duplicates`; `run communities --use-direction`; `map --out FILE` requires `--reveal`; a result's summary goes to standard output, the input summary and progress to standard error; a dry run is wholly on standard output (`docs/ux-contract.md` § 2.1)

**Acceptance**
- `everygraph ingest edges.csv && everygraph map && everygraph open` produces a browsable map of a 1 M-edge file in < 10 s on a laptop
- Every command supports `--json` output validated against a schema
- The result document is validated against `urn:everygraph:schema:result:1` (`docs/api-sketch.md` § 5.3) and the document of `ingest`, `map` and `export` against `urn:everygraph:schema:file:1` (`docs/ux-contract.md` § 4.7); errors of the operating system are reported as `IoError`, an existing output that is not ours as `OutputExists`, both with exit status 1; `info`, `doctor` and `verify` have schemas of their own, written by this issue

**Depends on** M2–M5 features

**Design reference (decided 2026-11-25, `design/DECISIONS.md`).** Approved in `design/preview/cli.md` and typeset in `design/preview/style/``terminal.html`: the command is `everygraph` with no short alias installed; verb-then-thing shape (`everygraph run pagerank FILE`, `everygraph query path FILE A B`); starts without asking; readable tables on a terminal and plain output when piped or with `--json`; restrained colour with automatic no-colour and ASCII fallbacks; the suggested next command in bold; English only for 0.1; `GB` means 10⁹ bytes.
