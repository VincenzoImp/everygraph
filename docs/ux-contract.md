# The UX contract: preflight, progress, resume and messages

Status: specification for v0.1 (issue #13). Implemented by #40 (ingest preflight and
resume), #45 (planner and `explain`), #76 (Jupyter widget), #81 (Python API) and #98
(command line); every other issue that runs for more than a moment obeys it.

This is the contract behind two of the product promises in `docs/DESIGN.md` § 2: the tool
says what it is about to do before it does it, and it survives being stopped. It covers
four things, the same on every surface: the **plan** shown before work starts, the
**progress** shown while it runs, what **stopping and running again** do, and the shape
of every **message**. Section 8 says how each rule is tested.

**Authority.** What is decided is in `design/DECISIONS.md`; the approved wording is in
`design/preview/cli.md` and `design/preview/api.md`; the approved layout is the typeset
form in `design/preview/style/terminal.html` and `notebook.html`, under the rules of
`design/preview/style/SPEC.md`. This document turns them into rules. Where the previews
were silent or disagreed, the choice is listed in § 9 "Decided here" and nowhere else.

**Every figure in a transcript is invented.** The sizes, counts, durations and rates
below are carried over from the approved previews to show shape and arithmetic. None is
a measurement, an estimate of real performance or a target. The handful of thresholds
this document introduces are marked *placeholder*: the implementing issue sets the value,
the rule stays.

**Rule identifiers.** Rules are numbered (`S-1`, `P-3`, …) so that tests and reviews can
cite them. "Must" is a requirement on the implementation.

---

## 1. Terms

| Term | Meaning |
|---|---|
| surface | Where a person meets the engine: `cli` (the `everygraph` command), `python` (calls from a prompt or a script; text on standard error), `notebook` (Jupyter: cards and the map widget). |
| operation | One command or one API call, from start to its final message. |
| phase | A named, ordered part of an operation with its own progress line. An operation has one or more. |
| plan | What the engine decided before starting: what it detected, how it will run, what that costs. A value (`Plan` in Python, JSON on the command line), not only text. |
| long operation | One whose estimated time is at least `T_PLAN` = 5 s (*placeholder*). |
| memory budget | The limit on memory the engine may own, set by `memory=` / `--memory`. "Budget" never means anything else (SPEC, last section). |
| scratch | Temporary files of a running operation, in the scratch directory (`temp_dir=` / `--temp-dir`, default the system temporary directory). |
| refusal | A plan that cannot be carried out inside the limits, reported before any work. |
| rendering | The form text takes on a stream: colour and Unicode, no colour, ASCII, or piped (§ 2.2). |

---

## 2. Streams, renderings and typography

These rules apply to everything in §§ 3–6.

### 2.1 What goes where

| | `cli` | `python` | `notebook` |
|---|---|---|---|
| The result as data (a table, a path, a file's description) | standard output | the returned object; its `repr` | the returned object; its card |
| Summary, plan, progress, result line, hints | standard error | `sys.stderr` | one card in the cell's output, updated in place |
| Warnings | standard error | `sys.stderr` | a notice card |
| Errors and refusals | standard error, and the exit code (§ 6.4) | an exception (§ 6.1) | the error card, traceback folded |
| A plan asked for on its own (`--dry-run`, `dry_run=True`, `explain`) | standard output | the returned `Plan`; nothing is printed | the returned `Plan`; its card |

- **S-1** Standard output of the command carries only what a program would consume: the
  result, and nothing else. `everygraph info`, `doctor`, `--help` and `--version` are
  results. Everything about *how the work is going* is on standard error.
- **S-2** With `--json`, standard output is exactly one JSON document: the result, or the
  error document of § 6.1 when the command fails, or the plan with `--dry-run`. Standard
  error keeps the plan, progress and warnings; it does not repeat the final message.
- **S-3** A library call prints only through the progress channel, and only when
  `progress=True` (the default). `progress=False` prints nothing; `progress=f` sends
  the events of § 4.6 to `f` and prints nothing. Results never print themselves.
- **S-4** Nothing ever waits for an answer. There is no confirmation prompt and no
  `--yes`; standard input is not read except as data.
- **S-5** Text is English only in 0.1.

### 2.2 The four renderings

One source of text, rendered per stream. Colour and symbols are an enhancement; the other
forms are complete on their own.

| Rendering | When | What changes |
|---|---|---|
| Colour and Unicode | The stream is a terminal, its encoding is UTF-8, and colour is not turned off. | The full form. |
| No colour | `NO_COLOR` is set to a non-empty value, or `TERM=dumb`. | Colour, bold and dim are removed. Symbols and words carry the status. |
| ASCII only | The terminal's encoding is not UTF-8. | Every symbol is replaced as in § 2.3. Colour is decided separately. |
| Piped | The stream is not a terminal (a pipe, a file, a job log). | ASCII, no colour, nothing redrawn, no bars: one line per finished phase (§ 4.5). Tables on standard output become tab-separated (§ 2.6). |

- **S-6** The rendering is chosen per stream: standard output and standard error are
  examined separately.
- **S-7** Colour uses six instructions and nothing else: reset, bold, faint, green,
  yellow, red. No background is set, no exact colour value is sent, and "bright black"
  is never used for dim.
- **S-8** Colour goes on the symbol, never on the sentence: green on `✓`, red on `✗`,
  yellow on `!`. Bold marks the headline of a block (a file name, the sentence of an
  error or warning) and anything meant to be copied and run: the suggested next command,
  an option in "Ways forward", a URL. Dim marks labels, table headers, the prompt, the
  finished bar and secondary remarks.

### 2.3 Symbols and fallbacks

| Unicode | ASCII | Meaning | Width change |
|---|---|---|---|
| `✓` | `ok` | Done; a check that passed. | +1 |
| `✗` | `error:` | Refused or failed. Always followed by one sentence. | +5 |
| `!` | `warning:` | It will go on, but there is something to know. | +7 |
| `█` | `#` | A filled cell of a progress bar. | 0 |
| `░` | `.` | A cell not yet filled. | 0 |
| `·` | `\|` | Separates facts on one line. | 0 |
| `—` | `-` | Joins a statement to its reason. | 0 |
| `→` | `->` | Becomes; leads to. | +1 |
| `…` | `...` | Something was shortened; a range. | +2 |
| `×` | `x` | Times, in arithmetic. | 0 |

The command prints no box-drawing characters: its tables are aligned with spaces. The
Python text forms of tables and paths (`docs/api-sketch.md` § 3) do use them, with these
fallbacks: `─` → `-`, `│` → `|`, every corner and junction → `+`, `▶` → `>`.

- **S-9** No other character outside ASCII is printed by the engine. Data (identifiers,
  column names, file names) is printed as it is.

### 2.4 Width

- **W-1** Nothing printed for a person is wider than 80 columns, in any rendering. A
  wider terminal changes nothing, so a transcript pasted into an issue looks the way it
  did on screen.
- **W-2** A line that is redrawn in place stops at column 78: a progress line that wraps
  cannot be redrawn.
- **W-3** Because some fallbacks are wider (§ 2.3), the Unicode form of a line may use at
  most the width that leaves room for them. Where a line cannot (a path of several
  identifiers), the ASCII form wraps before a `->`, never inside an identifier.
- **W-4** An identifier too long for its place is shortened in the middle on a terminal
  — the first six and the last four characters around `…`, as in `0x28c6…1d60` — and
  never in piped or machine output, nor in what a command accepts.
- **W-5** Machine output is exempt: tab-separated rows and JSON are as long as the data.
- **W-6** In a terminal narrower than 80 columns a progress line gives up its bar first,
  then its detail; other lines are left to the terminal to wrap.

### 2.5 Alignment

- Two spaces of indent under a headline, four under a sub-heading. No tabs.
- In label-and-value blocks the label is lowercase and dim; the summary, plan and stop
  blocks use a label field of 10 columns, so values start at column 13.
- Text is left-aligned; numbers are right-aligned so digits of the same weight sit under
  each other; a column of decimals shows the same number of decimals on every row.
- A table header aligns like its column, is dim, and is the column's real name, so it can
  be typed back. At least two spaces between columns; no rules, borders or stripes.

### 2.6 Numbers, sizes, times

| Kind | Rule | Examples |
|---|---|---|
| Count in a result | Exact, with thousands separators. | `1,830,397,917` |
| Count that is an estimate, in a label-and-value line | Rounded to at most four significant digits with a suffix `k`, `M`, `B` (10³, 10⁶, 10⁹), preceded by `~`. | `~212.4 M distinct` |
| Any estimate, in a sentence | The word "about". | `about 24 min` |
| Number meant to be copied | No separators. In Python code, `_` separators are allowed. | `--memory 6GB`, `transfers.csv:48211907`, `limit=100_000` |
| Size | `kB`, `MB`, `GB`, `TB` are powers of 10 (`GB` = 10⁹ bytes). Exact sizes and budgets: one decimal below 100. Estimates and what the machine reports: two significant digits. | `38.2 GB`, `8.0 GB`; `61 GB scratch`, `44 GB free` |
| Duration | Below 1 s: `ms`. Below 1 min: whole seconds. Below 1 h: `M min S s`. From 1 h: `H h M min`. No zero padding; zero parts are left out. | `1.3 ms`, `38 s`, `1 min 41 s`, `2 h 5 min` |
| Estimated duration | Rounded to 5 s below 1 min, 10 s below 10 min, 1 min below 1 h, 5 min from 1 h. | `3 min 40 s`, `about 2 min` |
| Rate | The unit per second, sizes and counts as above. | `1.9 GB/s`, `5.6 M/s` |
| Percentage of progress | A whole number. | `61%` |
| Share in a result | Two decimals. | `8.57%` |
| Date and time | ISO 8601; a range uses ` … `. | `2024-01-01 … 2024-12-31` |
| Age | The largest unit only. | `6 min ago`, `3 days ago` |

In a terminal the space between a number and its unit is an ordinary space.

- **N-1** A number is never printed with more digits than are known. Before the input
  has been read in full, a count is either exact from the file's own metadata (the row
  count of a Parquet file) or an estimate marked with `~`.
- **N-2** In piped tables and JSON, numbers are plain: no separators, no units; sizes
  are bytes and durations are seconds, and the key says so (`needs_bytes`,
  `elapsed_seconds`).

---

## 3. Preflight

### 3.1 When

- **P-1** Every operation is planned before it does any work. No scratch and no output
  byte is written until a plan exists and fits.
- **P-2** The plan is computed when the command or call starts, from four sources, none
  of which reads the whole input:

  | Source | Gives | Owner |
  |---|---|---|
  | The file's size and metadata, and a sample of its rows | format, columns, identifier type, estimated rows and distinct identifiers | #40 |
  | The graph file's header | nodes, edges, direction, columns, what is already computed | #28, #39 |
  | The operation's cost model | per-node state, passes, scratch, output size | #45 |
  | A probe of the machine | cores, memory installed, free and limited by a container, storage type, free disk in the scratch and output directories | #45 |

  Free memory and free disk are read at that moment, not cached. The time spent planning
  is bounded: at most 1 s or 1 % of the estimated run, whichever is larger (*placeholder*).
- **P-3** The plan is printed, without being asked, exactly when the operation is long
  (§ 1). It is always printed for `--dry-run`, and returned for `dry_run=True` and
  `explain`. It is always part of a refusal. A short operation prints no plan and no
  progress: only its result.
- **P-4** After the plan is printed the work starts at once (S-4).
- **P-5** Before starting, the engine checks, in this order, and refuses on the first
  failure (§ 3.5):
  1. the per-node state of every phase fits the memory budget;
  2. the scratch directory has room for the upper bound of the scratch estimate;
  3. the output directory has room for the upper bound of the output estimate, and the
     output can be created there.

### 3.2 Fields of a plan

| Field | What it holds | In text |
|---|---|---|
| `operation`, `parameters`, `call` | What was asked, with every default resolved; the call and the command that reproduce it. | — |
| `input` | For an operation that reads a file: path, size, format, compression, rows. | headline |
| `graph` | For an operation on a graph: path, size, nodes, edges, direction. | headline |
| `detected` | What was understood from the input: identifier type and distinct count; direction and where it came from (the caller, the column names, the default); how repeated edges and self-loops are treated; each column and its role. | `ids`, `edges`, `columns` |
| `machine` | Physical cores and hardware threads; memory installed and free, and a container limit if there is one; storage type and free space. | `machine` |
| `budget` | The memory budget and where it came from (the argument, the environment, the default); threads. | `budget` |
| `regime` | How the operation will run, as one of four names, and the reason with its arithmetic. | `plan` |
| `steps` | For `explain`: each step, where its data is, the memory it holds, the work it does. | the step table |
| `phases` | The phases in order, each with its own estimate and whether it will be reused from an earlier run. | `phases` (dry run); the progress lines |
| `estimate` | Wall time, peak memory, scratch disk, output size. Each is a value with a range and a basis (§ 3.3). | `estimate` |
| `scratch`, `output` | Where scratch goes and how much is free there; what will be written and where. | `estimate`, `result` |
| `fits`, `refusal` | Whether the plan can run inside the limits; if not, why and the ways forward (§ 3.5). | the refusal |

The four regimes. The planner (#45) chooses; this contract fixes the names and the words.

| `regime.name` | Printed as | Chosen when |
|---|---|---|
| `in_memory` | `in memory` (ingest: `build in memory`) | Everything the operation touches fits the budget. |
| `mapped` | `edges on disk, read as needed` | Per-node state fits, the graph does not, and access is random or storage is fast and local. |
| `streamed` | `edges on disk, read in passes` | Per-node state fits, the graph does not, and a hard bound was asked for or storage is not fast and local. |
| `on_disk` | `build on disk` | A build (ingest, tiles) whose working data does not fit. |

- **P-6** The reason always carries its arithmetic: `the graph is 4.6× the memory
  budget`, `the graph fits the memory budget`. A choice of regime is never printed
  without one.

### 3.3 Estimates and their uncertainty

- **P-7** Every estimate is a triple — `value`, `low`, `high` — with a `basis`:
  `exact` (known, `low` = `value` = `high`), `sampled` (from a sample of the input) or
  `model` (from the cost model alone). The engine claims that the actual figure will lie
  in `[low, high]`.
- **P-8** In text, the form follows the range:

  | Range | Form | Example |
  |---|---|---|
  | exact | the number | `1,830,402,118 rows` |
  | `low` and `high` within 30 % of `value` | the number, rounded as in § 2.6 | `3 min 40 s`, `peak 5.9 GB` |
  | wider | "about" and one or two significant digits | `about 2 min` |
  | open-ended (the operation runs until it converges) | the estimate for the expected number of passes, and that number | `about 2 min for 12 passes` |

- **P-9** The actual figures are recorded next to the estimates when the operation ends:
  in the ingest report (#36, #40), in the provenance of every result (#45), in the
  `result` event (§ 4.6) and in the `--json` document. The relative error is
  `(actual − value) / actual` for each of the four quantities. The target of #40 — within
  ±30 % on the tier-2 datasets — applies to all four.

### 3.4 Text form

The plan is at most four blocks, in this order, separated by one empty line.

1. **Headline.** The file's name in bold, its size, its rows or its nodes and edges.
2. **Summary** — only for an operation that reads an input the engine has not seen
   (`ingest`, `eg.read`): `ids`, `edges`, `columns`.
3. **Plan block** — always these four lines: `machine`, `budget`, `plan`, `estimate`.
4. **Then** the progress lines (§ 4) for a run, or the two lines `phases` and `result`
   and a closing sentence for a dry run.

```console
$ everygraph ingest transfers.parquet --source from --target to \
      --weight value --time block_time
transfers.parquet · 38.2 GB · 1,830,402,118 rows

  ids       hex addresses (20 bytes) · ~212.4 M distinct
  edges     directed · duplicates kept as events · self-loops dropped
  columns   value → weight · block_time → time · ignored: gas, nonce, input

  machine   16 cores · 61 GB RAM (16 GB free) · NVMe, 1.4 TB free
  budget    8.0 GB memory (default: half of free RAM)
  plan      build on disk — the graph is 4.6× the memory budget
  estimate  3 min 40 s · peak 5.9 GB · 61 GB scratch in /tmp · result 9.3 GB
```

Rules for the lines:

| Line | Content, in order | Variants |
|---|---|---|
| headline | name · size · rows | `~1.83 B rows` when the count is sampled (a text file); for a graph: name · size · nodes · edges |
| `ids` | kind of identifier (width) · distinct count | kinds: `integers`, `hex addresses (20 bytes)`, `text` |
| `edges` | direction · repeated edges · self-loops | `directed (from the column names)` or `undirected (the column names give no direction)` when the caller did not say; a second line, aligned with the first value, when one does not fit |
| `columns` | each used column `name → role` · `ignored:` the rest | `a → source · b → target` first when they were detected rather than given; more than three ignored columns are counted: `ignored: 12 columns` |
| `machine` | physical cores · memory installed (free) · storage, free space | `(8.0 GB allowed by the container)` in place of the free figure when a limit is lower |
| `budget` | size `memory` (where it came from) | `(default: half of free RAM)`, `(--memory)`, `(memory=)`, `(EVERYGRAPH_MEMORY)` |
| `plan` | regime `—` reason | § 3.2 |
| `estimate` | time · `peak` memory · scratch `in` directory · `result` size | `no scratch` when none; `result 1.7 GB in transfers.eg` when the result is stored inside the graph file |

- **P-10** The summary states rules and estimates, never a count that needs the whole
  file (N-1). Exact counts of what was dropped appear in the result block (§ 4.4).
- **P-11** The default memory budget is half of the memory that is free when the plan is
  computed. "Free" is what the process could still obtain: the system's available
  memory or, inside a container with a lower limit, what remains under that limit. The
  `budget` line always says where the figure came from.

A dry run prints the same blocks, then what would happen instead of doing it:

```console
$ everygraph ingest transfers.parquet --source from --target to \
      --weight value --time block_time --dry-run
transfers.parquet · 38.2 GB · 1,830,402,118 rows

  ids       hex addresses (20 bytes) · ~212.4 M distinct
  edges     directed · duplicates kept as events · self-loops dropped
  columns   value → weight · block_time → time · ignored: gas, nonce, input

  machine   16 cores · 61 GB RAM (16 GB free) · NVMe, 1.4 TB free
  budget    8.0 GB memory (default: half of free RAM)
  plan      build on disk — the graph is 4.6× the memory budget
  estimate  3 min 40 s · peak 5.9 GB · 61 GB scratch in /tmp · result 9.3 GB

  phases    reading → identifiers → building → checking
  result    transfers.eg, beside the input

Dry run: nothing was written.
  to run it: the same command without --dry-run
```

In Python the last line reads `to run it: the same call without dry_run=True`.

An operation on a graph has no summary block:

```console
$ everygraph map transfers.eg
transfers.eg · 9.3 GB · 212,418,907 nodes · 1,830,397,917 edges

  machine   16 cores · 61 GB RAM (16 GB free) · NVMe, 1.4 TB free
  budget    8.0 GB memory (default: half of free RAM)
  plan      edges on disk, read in passes — the graph is 1.2× the memory budget
  estimate  about 4 min · peak 5.6 GB · 12 GB scratch in /tmp · result 4.1 GB
```

The plan of an operation on a graph, **asked for on its own** — `g.explain("communities")`
or `dry_run=True` in Python, `--dry-run` on the command line — is the step-table form
typeset in `notebook.html`. It is the same text on both surfaces, except that the
command line writes the operation without parentheses and ends with the dry-run notice:

```text
Plan for communities() on transfers.eg — Leiden, resolution 1.0, deterministic
  fits     peak 5.6 GB of 8.0 GB budget

  1  keep per-node state         memory            5.1 GB
  2  move nodes between groups   edges from disk   0.5 GB   about 12 passes
  3  merge groups and repeat     memory          < 0.4 GB   4 levels
  4  write the result            disk                   —

  why      212,418,907 nodes × 24 bytes = 5.1 GB of state
           the edges (9.3 GB) stay on disk and are read each pass
  estimate about 2 min
```

- **P-12** When something already computed will be reused, the plan says so in place of
  the work: the phase is listed with `kept` (§ 4.3) and its estimate is zero.

In a notebook the plan is the top of the progress card (`notebook.html`, "While
`eg.read()` runs"): the same facts in the same order — what it understood, the plan, the
estimate — then one bar per phase. A `Plan` displayed on its own is the plan card (cell 3
of the same page). Text and card must agree fact for fact.

### 3.5 Refusal

A refusal is a plan with `fits = false`. It is decided before any work (P-5), so its
last line can always say that nothing happened.

- **P-13** A refusal for memory states: the step; what it needs and for what; the
  arithmetic (count × bytes each); the budget; what stays on disk regardless; the
  smallest budget that would work; and every other way forward the engine knows.
- **P-14** The smallest budget is the per-node state plus the fixed buffers of the plan,
  rounded **up** to a value that can be typed: a multiple of 1 GB from 4 GB, of 0.5 GB
  from 1 GB, of 100 MB below. The exact figure is in the machine-readable form.
- **P-15** A way forward is listed only if it would work: an option the operation does
  not have, or a lower-memory variant that does not exist for it, is not offered.

```console
$ everygraph run communities transfers.eg --memory 4GB
✗ Not enough memory for this step.

  communities needs 5.1 GB for per-node state (212,418,907 nodes × 24 bytes).
  The budget is 4.0 GB. The edges stay on disk either way.

  Ways forward:
    --memory 6GB            the smallest budget that works
    --precision low         needs 3.4 GB; slightly lower quality

Nothing was changed.
```

The same refusal in Python is an exception (§ 6.1 gives the pattern) and in a notebook
the error card of `notebook.html`, cell 7. With `--dry-run`, `dry_run=True` or `explain`
a plan that does not fit is *returned*, not raised: the refusal is its content, and the
command exits with status 4.

### 3.6 Machine-readable form

`Plan.to_dict()` in Python, the `plan` event (§ 4.6), and the standard output of
`--dry-run --json` are the same document. Keys may be added within a schema version;
removing a key or changing its meaning increments `schema`. Readers must ignore keys they
do not know.

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "urn:everygraph:schema:plan:1",
  "title": "Everygraph plan, version 1",
  "type": "object",
  "required": [
    "schema", "operation", "parameters", "machine", "budget", "regime",
    "phases", "estimate", "scratch", "output", "fits", "refusal"
  ],
  "properties": {
    "schema": {"const": 1},
    "operation": {"type": "string"},
    "parameters": {"type": "object"},
    "call": {
      "type": "object",
      "properties": {
        "python": {"type": "string"},
        "cli": {"type": "string"}
      }
    },
    "input": {"oneOf": [{"type": "null"}, {"$ref": "#/$defs/input"}]},
    "graph": {"oneOf": [{"type": "null"}, {"$ref": "#/$defs/graph"}]},
    "detected": {"oneOf": [{"type": "null"}, {"$ref": "#/$defs/detected"}]},
    "machine": {"$ref": "#/$defs/machine"},
    "budget": {"$ref": "#/$defs/budget"},
    "regime": {"$ref": "#/$defs/regime"},
    "steps": {"type": "array", "items": {"$ref": "#/$defs/step"}},
    "phases": {
      "type": "array",
      "minItems": 1,
      "items": {"$ref": "#/$defs/phase"}
    },
    "estimate": {
      "type": "object",
      "required": [
        "seconds", "peak_memory_bytes", "scratch_bytes", "output_bytes"
      ],
      "properties": {
        "seconds": {"$ref": "#/$defs/estimate"},
        "peak_memory_bytes": {"$ref": "#/$defs/estimate"},
        "scratch_bytes": {"$ref": "#/$defs/estimate"},
        "output_bytes": {"$ref": "#/$defs/estimate"}
      }
    },
    "scratch": {
      "type": "object",
      "required": ["path", "free_bytes"],
      "properties": {
        "path": {"type": "string"},
        "free_bytes": {"$ref": "#/$defs/count"}
      }
    },
    "output": {
      "type": "object",
      "required": ["path", "kind"],
      "properties": {
        "path": {"type": ["string", "null"]},
        "kind": {"enum": ["graph", "map", "result", "file", "none"]},
        "free_bytes": {"$ref": "#/$defs/count"}
      }
    },
    "fits": {"type": "boolean"},
    "refusal": {"oneOf": [{"type": "null"}, {"$ref": "#/$defs/refusal"}]}
  },
  "$defs": {
    "count": {"type": "integer", "minimum": 0},
    "origin": {"enum": ["argument", "graph", "environment", "default"]},
    "estimate": {
      "type": "object",
      "required": ["value", "low", "high", "basis"],
      "properties": {
        "value": {"type": "number", "minimum": 0},
        "low": {"type": "number", "minimum": 0},
        "high": {"type": "number", "minimum": 0},
        "basis": {"enum": ["exact", "sampled", "model"]}
      }
    },
    "input": {
      "type": "object",
      "required": ["path", "bytes", "format", "rows"],
      "properties": {
        "path": {"type": "string"},
        "bytes": {"$ref": "#/$defs/count"},
        "format": {"type": "string"},
        "compression": {"enum": ["none", "gzip", "zstd"]},
        "rows": {"$ref": "#/$defs/estimate"}
      }
    },
    "graph": {
      "type": "object",
      "required": ["path", "bytes", "nodes", "edges", "directed"],
      "properties": {
        "path": {"type": ["string", "null"]},
        "bytes": {"$ref": "#/$defs/count"},
        "nodes": {"$ref": "#/$defs/count"},
        "edges": {"$ref": "#/$defs/count"},
        "directed": {"type": "boolean"},
        "view": {"type": ["string", "null"]}
      }
    },
    "detected": {
      "type": "object",
      "required": ["ids", "edges", "columns"],
      "properties": {
        "ids": {
          "type": "object",
          "required": ["kind", "distinct"],
          "properties": {
            "kind": {"enum": ["integer", "hex", "text"]},
            "width_bytes": {"type": ["integer", "null"]},
            "noun": {"type": "string"},
            "distinct": {"$ref": "#/$defs/estimate"}
          }
        },
        "edges": {
          "type": "object",
          "required": ["directed", "directed_from", "duplicates", "self_loops"],
          "properties": {
            "directed": {"type": "boolean"},
            "directed_from": {"enum": ["caller", "column names", "default"]},
            "duplicates": {"enum": ["kept as events"]},
            "self_loops": {"enum": ["dropped"]}
          }
        },
        "columns": {
          "type": "array",
          "items": {
            "type": "object",
            "required": ["name", "role"],
            "properties": {
              "name": {"type": "string"},
              "role": {
                "enum": ["source", "target", "weight", "time", "ignored"]
              },
              "type": {"type": "string"}
            }
          }
        }
      }
    },
    "machine": {
      "type": "object",
      "required": [
        "cores", "hardware_threads", "memory_bytes", "memory_free_bytes",
        "memory_limit_bytes", "storage"
      ],
      "properties": {
        "cores": {"type": "integer", "minimum": 1},
        "hardware_threads": {"type": "integer", "minimum": 1},
        "memory_bytes": {"$ref": "#/$defs/count"},
        "memory_free_bytes": {"$ref": "#/$defs/count"},
        "memory_limit_bytes": {"type": ["integer", "null"]},
        "storage": {
          "type": "object",
          "required": ["kind", "free_bytes"],
          "properties": {
            "kind": {"type": "string"},
            "free_bytes": {"$ref": "#/$defs/count"}
          }
        }
      }
    },
    "budget": {
      "type": "object",
      "required": ["memory_bytes", "memory_from", "threads", "threads_from"],
      "properties": {
        "memory_bytes": {"$ref": "#/$defs/count"},
        "memory_from": {"$ref": "#/$defs/origin"},
        "threads": {"type": "integer", "minimum": 1},
        "threads_from": {"$ref": "#/$defs/origin"}
      }
    },
    "regime": {
      "type": "object",
      "required": ["name", "reason"],
      "properties": {
        "name": {"enum": ["in_memory", "mapped", "streamed", "on_disk"]},
        "reason": {"type": "string"},
        "ratio": {"type": ["number", "null"]}
      }
    },
    "step": {
      "type": "object",
      "required": ["step", "where", "memory_bytes"],
      "properties": {
        "step": {"type": "string"},
        "where": {"enum": ["memory", "edges from disk", "disk"]},
        "memory_bytes": {"type": ["integer", "null"]},
        "work": {"type": ["string", "null"]}
      }
    },
    "phase": {
      "type": "object",
      "required": ["name", "kept", "seconds"],
      "properties": {
        "name": {"type": "string", "pattern": "^[a-z]{1,11}$"},
        "kept": {"type": "boolean"},
        "seconds": {"$ref": "#/$defs/estimate"},
        "regime": {"enum": ["in_memory", "mapped", "streamed", "on_disk"]}
      }
    },
    "refusal": {
      "type": "object",
      "required": [
        "error", "message", "step", "needs_bytes", "available_bytes",
        "ways_forward"
      ],
      "properties": {
        "error": {"enum": ["InsufficientBudget", "InsufficientDisk"]},
        "message": {"type": "string"},
        "step": {"type": "string"},
        "needs_bytes": {"$ref": "#/$defs/count"},
        "available_bytes": {"$ref": "#/$defs/count"},
        "arithmetic": {
          "type": "object",
          "required": ["count", "unit", "bytes_each"],
          "properties": {
            "count": {"$ref": "#/$defs/count"},
            "unit": {"type": "string"},
            "bytes_each": {"$ref": "#/$defs/count"}
          }
        },
        "smallest_budget_bytes": {"type": ["integer", "null"]},
        "ways_forward": {
          "type": "array",
          "items": {"$ref": "#/$defs/way"}
        }
      }
    },
    "way": {
      "type": "object",
      "required": ["option", "python"],
      "properties": {
        "option": {"type": "string"},
        "python": {"type": "string"},
        "note": {"type": "string"},
        "needs_bytes": {"$ref": "#/$defs/count"}
      }
    }
  }
}
```

The plan of the ingest above, as that document (shortened: `call`, `detected.columns`
and three of the four phases are left out here, not in the real output):

```json
{
  "schema": 1,
  "operation": "ingest",
  "parameters": {
    "source": "from", "target": "to", "weight": "value",
    "time": "block_time", "directed": true
  },
  "input": {
    "path": "transfers.parquet", "bytes": 38200000000,
    "format": "parquet", "compression": "none",
    "rows": {
      "value": 1830402118, "low": 1830402118, "high": 1830402118,
      "basis": "exact"
    }
  },
  "graph": null,
  "detected": {
    "ids": {
      "kind": "hex", "width_bytes": 20, "noun": "addresses",
      "distinct": {
        "value": 212400000, "low": 201800000, "high": 223000000,
        "basis": "sampled"
      }
    },
    "edges": {
      "directed": true, "directed_from": "caller",
      "duplicates": "kept as events", "self_loops": "dropped"
    },
    "columns": [
      {"name": "value", "role": "weight", "type": "decimal"}
    ]
  },
  "machine": {
    "cores": 16, "hardware_threads": 32,
    "memory_bytes": 61000000000, "memory_free_bytes": 16000000000,
    "memory_limit_bytes": null,
    "storage": {"kind": "NVMe", "free_bytes": 1400000000000}
  },
  "budget": {
    "memory_bytes": 8000000000, "memory_from": "default",
    "threads": 16, "threads_from": "default"
  },
  "regime": {
    "name": "on_disk",
    "reason": "the graph is 4.6× the memory budget",
    "ratio": 4.6
  },
  "phases": [
    {
      "name": "reading", "kept": false, "regime": "on_disk",
      "seconds": {"value": 105, "low": 75, "high": 135, "basis": "sampled"}
    }
  ],
  "estimate": {
    "seconds": {"value": 220, "low": 160, "high": 280, "basis": "sampled"},
    "peak_memory_bytes": {
      "value": 5900000000, "low": 5300000000, "high": 6500000000,
      "basis": "model"
    },
    "scratch_bytes": {
      "value": 61000000000, "low": 52000000000, "high": 70000000000,
      "basis": "sampled"
    },
    "output_bytes": {
      "value": 9300000000, "low": 7900000000, "high": 10700000000,
      "basis": "sampled"
    }
  },
  "scratch": {"path": "/tmp", "free_bytes": 1400000000000},
  "output": {
    "path": "transfers.eg", "kind": "graph", "free_bytes": 1400000000000
  },
  "fits": true,
  "refusal": null
}
```

The `refusal` of § 3.5, inside a plan whose `fits` is `false`:

```json
{
  "error": "InsufficientBudget",
  "message": "Not enough memory for this step.",
  "step": "communities",
  "needs_bytes": 5098053768,
  "available_bytes": 4000000000,
  "arithmetic": {"count": 212418907, "unit": "nodes", "bytes_each": 24},
  "smallest_budget_bytes": 5598053768,
  "ways_forward": [
    {
      "option": "--memory 6GB",
      "python": "g.communities(memory=\"6GB\")",
      "note": "the smallest budget that works"
    },
    {
      "option": "--precision low",
      "python": "g.communities(precision=\"low\")",
      "note": "slightly lower quality",
      "needs_bytes": 3400000000
    }
  ]
}
```

---

## 4. Progress

### 4.1 Phases

- **G-1** An operation's phases are fixed, named and ordered, and the plan lists them
  before the first one starts, so the whole job is visible from the beginning.
- **G-2** A phase name is one lowercase word of at most 11 letters, a noun or a gerund
  for what is being done or made.

| Operation | Phases, in order | Counts while running | Detail when finished |
|---|---|---|---|
| `ingest`, `eg.read` | `reading` | bytes of input | rows read |
| | `identifiers` | distinct identifiers | identifiers, with the user's noun: `212,418,907 addresses` |
| | `building` | parts | `2,048 parts` |
| | `checking` | sections | the checks that passed: `sorted · symmetric counts` |
| `map`, `g.map()` | `communities` | passes over the edges | `31,960 groups · modularity 0.603` |
| | `hierarchy` | levels | `6 zoom levels` |
| | `layout` | regions | (empty) |
| | `tiles` | tiles | `1,204,118 tiles` |
| `run NAME`, `g.NAME()` | one phase, named after the operation | passes over the edges, or nodes | one or two exact facts about the result |

A whole-graph algorithm is a single phase: `communities`, `pagerank`, `components`,
`kcore`, `triangles`, `degrees`, `bfs`. Point queries (`neighbors`, `khop`, `path`) have
no phases and no progress; they have `limit` and `timeout` instead (§ 6.2, "cut short").

### 4.2 Anatomy of a progress line

```text
  identifiers  █████████░░░░░░░  61% · 5.6 M/s · about 15 s left          23 s
```

| Part | Columns | Rule |
|---|---|---|
| Indent | 1–2 | Phases belong to the command above them. |
| Name | 3–15 | 13 columns, left-aligned: the longest name plus two. |
| Bar | 16–31 | 16 cells. A cell fills when its sixteenth is complete. Whole cells only: no partial blocks, brackets, spinners or colour. |
| Detail | 34–65 | Up to 32 columns, after two spaces. |
| Elapsed | ends at 78 | Right-aligned, at most 11 columns, units written out. At least two spaces after the detail. |

### 4.3 States

```text
  reading      ████████████████  1,830,402,118 rows                 1 min 41 s
  identifiers  █████████░░░░░░░  61% · 5.6 M/s · about 15 s left          23 s
  building     ░░░░░░░░░░░░░░░░
  checking     ░░░░░░░░░░░░░░░░
```

| State | Bar | Detail | Elapsed column | Colour |
|---|---|---|---|---|
| waiting | empty | none | none | the whole line dim |
| running | filled in proportion | § 4.4 | time since the phase began | filled cells in the text colour, empty cells dim |
| done | full | the exact count of what was made (§ 4.1) | the phase's time | the bar dim; the numbers stay |
| kept | full | the same detail as when it was done | the word `kept` | as done |
| stopped | as it was | as it was | as it was | followed by the stop block (§ 5.4) |

- **G-3** `kept` marks a phase whose result already exists and is valid: from the run
  that was stopped (§ 5.3), or from an earlier operation (communities already computed
  when `map` is asked for).
- **G-4** Progress fills are ink, not accent, and never coloured by state.

### 4.4 What a running line may claim

The detail of a running phase is up to three parts joined by ` · `, in this order:

1. the percentage, when the total of the phase's unit is known;
2. the rate (`1.9 GB/s`, `5.6 M/s`), or for a phase made of passes `pass 2 of 3`;
3. the time left: `about 15 s left`.

If the three do not fit 32 columns the second part is dropped, then the third.

- **G-5** The percentage is `floor(100 × done / total)` in the phase's own unit. It never
  goes down, and it does not reach 100 while the phase is running. With no known total
  the detail is the count so far and the rate, with no percentage and no time left.
- **G-6** The time left is for the current phase only; later phases have their estimates
  in the plan. It is always worded `about … left` and rounded as an estimated duration
  (§ 2.6).
- **G-7** A time left is shown only once the phase has run for 3 s and completed 2 % of
  its work (*placeholders*). It is computed from the rate observed in this phase, not
  from the plan.
- **G-8** For a phase that runs until it converges, the total number of passes is not
  known: the detail is `pass 7 · 5.6 M/s` and no time left is claimed.
- **G-9** When the phase finishes, the detail is replaced by exact figures, never
  rounded.
- **G-10** On a terminal the running line is redrawn in place at most 4 times a second
  and at least once a second, and the lines above it never move. Nothing else is
  redrawn.

When the last phase finishes, the result block follows after one empty line:

```console
  reading      ████████████████  1,830,402,118 rows                 1 min 41 s
  identifiers  ████████████████  212,418,907 addresses                    38 s
  building     ████████████████  2,048 parts                              59 s
  checking     ████████████████  sorted · symmetric counts                12 s

✓ transfers.eg · 9.3 GB · 212,418,907 nodes · 1,830,397,917 edges · 3 min 30 s
  dropped: 4,201 self-loops
  next: everygraph map transfers.eg
```

- **G-11** The result line is `✓`, what was made (bold), its size, its exact counts and
  the total time. It is the only place a total time appears.
- **G-12** `dropped:` follows when rows of the input did not become edges, with the
  exact count of each kind. It is left out when nothing was dropped.
- **G-13** At most two hints follow, each a label and something that can be run, the
  runnable part in bold: `next:` on the command line; in Python the hint is a call
  (`reopen later: eg.open("transfers.eg")`). A hint is never a sentence of advice.

```console
$ everygraph map transfers.eg
transfers.eg · 9.3 GB · 212,418,907 nodes · 1,830,397,917 edges

  machine   16 cores · 61 GB RAM (16 GB free) · NVMe, 1.4 TB free
  budget    8.0 GB memory (default: half of free RAM)
  plan      edges on disk, read in passes — the graph is 1.2× the memory budget
  estimate  about 4 min · peak 5.6 GB · 12 GB scratch in /tmp · result 4.1 GB

  communities  ████████████████  31,960 groups · modularity 0.603   2 min 11 s
  hierarchy    ████████████████  6 zoom levels                            18 s
  layout       ████████████████                                           41 s
  tiles        ████████████████  1,204,118 tiles                     1 min 2 s

✓ transfers.egmap · 4.1 GB
  open it:   everygraph open transfers.eg
  share it:  upload transfers.egmap and viewer.html to any static host
```

### 4.5 The other renderings

**ASCII only.** The same lines through the table of § 2.3:

```text
  reading      ################  1,830,402,118 rows                 1 min 41 s
  identifiers  #########.......  61% | 5.6 M/s | about 15 s left          23 s
  building     ................
  checking     ................
```

**Piped.** Nothing is redrawn. The headline, summary and plan block are printed as they
are (in ASCII), then each phase prints one line when it finishes, without a bar, its
time ending at column 60; then the result block.

```text
transfers.parquet | 38.2 GB | 1,830,402,118 rows

  ids       hex addresses (20 bytes) | ~212.4 M distinct
  edges     directed | duplicates kept as events | self-loops dropped
  columns   value -> weight | block_time -> time | ignored: gas, nonce, input

  machine   16 cores | 61 GB RAM (16 GB free) | NVMe, 1.4 TB free
  budget    8.0 GB memory (default: half of free RAM)
  plan      build on disk - the graph is 4.6x the memory budget
  estimate  3 min 40 s | peak 5.9 GB | 61 GB scratch in /tmp | result 9.3 GB

  reading      1,830,402,118 rows                 1 min 41 s
  identifiers  212,418,907 addresses                    38 s
  building     2,048 parts                              59 s
  checking     sorted | symmetric counts                12 s

ok transfers.eg | 9.3 GB | 212,418,907 nodes | 1,830,397,917 edges | 3 min 30 s
  dropped: 4,201 self-loops
  next: everygraph map transfers.eg
```

- **G-14** A log keeps thousands separators in counts: it is read by people.

**Narrow terminal.** At 60 columns the bars are gone and the numbers are not (W-6):

```text
  reading      1,830,402,118 rows                 1 min 41 s
  identifiers  61% · 5.6 M/s · about 15 s left          23 s
  building
  checking
```

**Notebook.** One card for the operation, updated in place (`notebook.html`, "States
and details"): the headline and state (`reading · 48%`), the summary and plan, then one
row per phase with a bar in the text colour, the same detail, and `waiting` for phases
not begun. A finished phase shows a tick, the word `done` and its time; the card may add
the peak memory against the budget (`peak 5.6 GB of 8.0 GB`). The card is driven by the
events of § 4.6 and must not show a fact the text form lacks, except that one.

**Python at a prompt or in a script.** The terminal form, on `sys.stderr`, by the same
renderer: redrawn in place when `sys.stderr` is a terminal, the piped form otherwise.

### 4.6 The event stream

The renderings above are views of one stream of events. Python receives it through
`progress=` (S-3); the notebook widget receives the same objects.

- **G-15** `progress=f` calls `f(event)` with a `dict`, on the thread that made the
  call, never concurrently and never re-entrantly. Events arrive in order.
- **G-16** Every operation emits exactly one `plan` event first — also when it is short
  and prints nothing — and exactly one of `result`, `stopped` or `error` last.
- **G-17** Each phase emits one `phase` event with state `running` when it begins, then
  `running` events at most 4 times a second, then exactly one with a final state:
  `done`, `kept` or `stopped`. A kept phase emits only its `kept` event.
- **G-18** An exception raised by `f` cancels the operation exactly as `Ctrl-C` does
  (§ 5.2) and then propagates to the caller.

Every event has these keys:

| Key | Type | Meaning |
|---|---|---|
| `event` | string | `plan`, `phase`, `warning`, `result`, `stopped` or `error` |
| `schema` | integer | `1` |
| `job` | string | 16 hexadecimal digits, the same for every event of one operation |
| `operation` | string | as in the plan |
| `t` | number | seconds since the operation started |

and, by kind:

| Event | Key | Type | Meaning |
|---|---|---|---|
| `plan` | `plan` | object | the plan document of § 3.6 |
| `phase` | `name` | string | the phase |
| | `index`, `count` | integer | position from 0, and the number of phases |
| | `state` | string | `running`, `done`, `kept` or `stopped` |
| | `unit` | string | `bytes`, `rows`, `nodes`, `edges`, `parts`, `passes`, `tiles`, `items` |
| | `done`, `total` | number; number or null | progress in that unit; the total if known |
| | `fraction` | number or null | `done / total`, from 0 to 1 |
| | `pass` | object or null | `{"index": 2, "of": 3}`; `of` is null when open-ended |
| | `rate` | number or null | units per second |
| | `eta_seconds` | number or null | time left, when G-7 allows a claim |
| | `elapsed_seconds` | number | time in this phase |
| | `detail` | string or null | the finished detail of § 4.1, for final states |
| | `peak_memory_bytes` | integer | peak so far in this phase |
| `warning` | `code` | string | a stable identifier: `slow-gzip`, `cut-short`, … (§ 6.2) |
| | `message` | string | the headline sentence |
| | `why` | string or null | the consequence, with its numbers |
| | `ways_forward` | array | as in § 3.6 |
| | `continuing` | string or null | what happens now |
| `result` | `output` | object | `path`, `kind`, `bytes` |
| | `counts` | object | exact counts: `nodes`, `edges`, `rows`, … |
| | `dropped` | object | exact counts by kind: `self_loops`, … |
| | `elapsed_seconds`, `peak_memory_bytes`, `scratch_bytes` | number | the actual figures |
| | `estimate_error` | object | relative error of each of the four estimates (P-9) |
| | `next` | array | hints: `label`, `python`, `cli` |
| `stopped` | `reason` | string | `interrupt`, `terminate` or `callback` |
| | `during`, `fraction` | string; number or null | the phase that was running and how far it was |
| | `kept`, `dropped` | arrays of strings | phases kept; phases that start again |
| | `scratch` | object or null | `path`, `bytes` of what was kept |
| | `changed` | boolean | always `false` (§ 5.2) |
| | `resume` | object | `python`, `cli`: what continues the work |
| | `discard` | string or null | the command that removes what was kept |
| `error` | (all keys) | | the error document of § 6.1 |

---

## 5. Stopping and continuing

### 5.1 What checkpoints

- **R-1** The unit of resumption is the phase. A finished phase is kept; a phase that was
  interrupted starts again from its beginning. There is no finer checkpoint in 0.1.
- **R-2** This is one behaviour on every surface: the command line, a Python script, a
  notebook cell. Running the same thing again continues from the last finished phase.
  An operation with a single phase therefore begins that phase again — which is said in
  the same words (§ 5.4), not as "start over".

| Operation | Checkpoint after | Kept where | Notes |
|---|---|---|---|
| `ingest`, `eg.read` of a file | `reading`, `identifiers`, `building` | the scratch directory of the job; the unfinished output as `NAME.eg.partial` beside its destination | #40 |
| `map`, `g.map()` | `communities`, `hierarchy`, `layout` | in the graph file, as computed results (#39): a later run finds them like any cached result | `tiles` writes `NAME.egmap.partial` and starts again if interrupted |
| a whole-graph algorithm (one phase) | — | nothing | The result is stored when the phase ends, or not at all. |
| point queries, `info`, `doctor`, `verify` | — | nothing | They write nothing. |

- **R-3** Only work that will be asked for again in the same form is kept. A phase's
  kept output is valid for any memory budget and any number of threads: neither may
  change what a phase writes, because after a refusal or a stop the next run will often
  change them.

### 5.2 What stopping leaves behind

Stopping means `Ctrl-C` (SIGINT), SIGTERM, a `KeyboardInterrupt` in Python (including
"interrupt the kernel" in a notebook), or an exception from a progress callback (G-18).
These are *clean* stops. SIGKILL, a power cut and a kill by the operating system are
*hard* stops: nothing can be printed, and the guarantees must hold anyway.

- **C-1** **A graph file is never left corrupt.** At every instant a file named
  `NAME.eg` either does not exist or is complete and passes `everygraph verify`.
  - A new graph is written as `NAME.eg.partial` in the destination directory, made
    durable, then renamed to `NAME.eg` in one step.
  - A result added to an existing graph is appended; the file's table of contents is
    switched to include it only once the result is durable (the protocol of #28). Until
    that switch the file reads exactly as before.
  - The same holds for `NAME.egmap`.
- **C-2** After a clean stop an existing graph file is byte for byte what it was before
  the operation. After a hard stop it has the same contents as before; bytes appended
  but never referenced may remain at its end until the next write removes them.
- **C-3** After a clean stop the only files left are the ones the stop block names
  (§ 5.4): the job's scratch directory, holding finished phases only, and possibly
  `NAME.eg.partial`. The interrupted phase's files are removed.
- **C-4** A clean stop is prompt: workers check for cancellation at chunk boundaries and
  the stop block is printed within 2 s of the signal (*placeholder*).
- **C-5** A second `Ctrl-C` while the first is being handled ends the process at once,
  with no message. It is a hard stop.
- **C-6** In Python the engine stops its workers, cleans up, prints the stop block and
  then raises a plain `KeyboardInterrupt`, never a wrapped one. The Python process and
  any graph object in it remain usable.
- **C-7** The command exits with status 130 after SIGINT and 143 after SIGTERM.
- **C-8** A failure is not a stop. When an operation fails (§ 6), its scratch is removed
  unless finished phases can still be used by the same command once the cause is fixed
  (the disk filled up during `building`); the error's last line says which.
- **C-9** On success the scratch directory is removed.

### 5.3 The manifest and staleness

The scratch directory of a job is `everygraph-ID` inside the scratch root, where `ID` is
the first four hexadecimal digits of the job's fingerprint (more digits if that name is
taken by a different job). "The same command again" finds it by computing the same
fingerprint: there is no registry.

- **R-4** The fingerprint is a hash of: the operation; every parameter that changes what
  a phase writes (so not the memory budget, not the thread count, not `progress`); the
  identity of the input (below); the output path; and the version of each phase's
  algorithm.
- **R-5** The directory holds `manifest.json`, rewritten atomically at each phase
  boundary, and the kept files it lists.

| Manifest key | Meaning |
|---|---|
| `schema` | `1` |
| `fingerprint` | the full job fingerprint |
| `operation`, `parameters` | as in the plan |
| `engine`, `phase_versions` | the Everygraph version that wrote it; the algorithm version of each phase |
| `input` | `path`, `bytes`, `modified` (nanoseconds), `sample_hash` |
| `output` | the destination path and the `.partial` file, if any |
| `phases` | for each phase: `name`, `state` (`done` or absent), `finished`, `elapsed_seconds`, `detail`, and `files`: name, bytes and content hash of each kept file |
| `started`, `stopped` | timestamps of the first start and of the last stop |
| `owner` | host name and process id of the run that holds the directory, while it runs |

- **R-6** The identity of an input file is its size, its modification time and the hash
  of a fixed sample of its bytes (the first and last 1 MiB — *placeholder*). The whole
  file is not hashed again to decide.
- **R-7** A kept phase is used only if all of these hold; otherwise it and every later
  phase run again:

  | Check | If it fails |
  |---|---|
  | A manifest with this fingerprint exists | A new run. Nothing is said. |
  | No live process owns the directory | Error: another run of this job is in progress (exit 1). Nothing is touched. |
  | The input's identity matches | The kept work is stale: it is removed, a line says so, the run starts from the first phase. |
  | Each phase's algorithm version matches | That phase and the later ones run again; a line says so. |
  | Every kept file exists with the recorded size and hash | That phase and the later ones run again; a line says so. |

- **R-8** A run that continues produces a result byte for byte equal to an uninterrupted
  run with the same parameters (the acceptance test of #40).
- **R-9** Kept scratch is never removed by time. It is removed by a successful run, by a
  run that finds it stale, or by the person, with the command the stop block gives.

### 5.4 The words

**On a clean stop**, after the terminal's own `^C`. Stopping is not an error and gets no
symbol.

```console
  reading      ████████████████  1,830,402,118 rows                 1 min 41 s
  identifiers  █████████░░░░░░░  61% · 5.6 M/s · about 15 s left          23 s
^C
Stopped at your request, during identifiers.

  kept      reading · 31 GB of scratch in /tmp/everygraph-7c1e
  dropped   identifiers at 61% — that phase starts again from its beginning
  written   nothing: there is no transfers.eg yet

  continue:  run the same command again
  discard:   rm -r /tmp/everygraph-7c1e
```

| Line | Rule |
|---|---|
| first | `Stopped at your request, during PHASE.` After SIGTERM: `Stopped by a signal, during PHASE.` |
| `kept` | The finished phases, then the size and place of what is kept. `nothing` when no phase had finished. |
| `dropped` | The interrupted phase and how far it was, and always the words `that phase starts again from its beginning`. |
| `written` | What exists of the output. `nothing: there is no NAME.eg yet`, or for an operation on an existing graph `nothing: NAME.eg is unchanged`. |
| `continue:` | `run the same command again`; in Python `run the same call again`; in a notebook `run the cell again`. |
| `discard:` | The command that removes everything kept, including a `.partial` file. Left out when nothing was kept. |

A single-phase operation uses the same block:

```console
$ everygraph run triangles transfers.eg
  triangles    ███████░░░░░░░░░  48% · pass 2 of 3                  1 min 44 s
^C
Stopped at your request, during triangles.

  kept      nothing
  dropped   triangles at 48% — that phase starts again from its beginning
  written   nothing: transfers.eg is unchanged

  continue:  run the same command again
```

In a notebook the progress card's row reads `stopped at 48%` and the same four facts
follow as text in the card. The sentence in `notebook.html` ("run the cell again to
start over") is replaced by this block (§ 9).

**On continuing.** The headline, then one line that says it is continuing and why it
may; then the phases, with `kept` for the ones not repeated. The summary and plan block
are not printed again. The time on the result line is this run's.

```console
$ everygraph ingest transfers.parquet --source from --target to \
      --weight value --time block_time
transfers.parquet · 38.2 GB · 1,830,402,118 rows
  Continuing the run stopped 6 min ago: the file has not changed since.

  reading      ████████████████  1,830,402,118 rows                       kept
  identifiers  ████████████████  212,418,907 addresses                    38 s
  building     ████████████████  2,048 parts                              59 s
  checking     ████████████████  sorted · symmetric counts                12 s

✓ transfers.eg · 9.3 GB · 212,418,907 nodes · 1,830,397,917 edges · 1 min 49 s
  dropped: 4,201 self-loops
  next: everygraph map transfers.eg
```

When kept work cannot be used, the line says so instead, and a full run follows with its
summary and plan:

| Case | Line |
|---|---|
| The input changed | `The file has changed since the run stopped 6 min ago: starting again.` |
| A phase's algorithm changed | `Everygraph has changed since the run stopped 3 days ago: starting again from identifiers.` |
| A kept file is missing or damaged | `Part of the scratch from the run stopped 6 min ago is missing: starting again from identifiers.` |

---

## 6. Messages

Three patterns, and no others.

| Pattern | Symbol | Use | Ends with |
|---|---|---|---|
| Error | `✗` red | The operation was refused or failed. | What state things were left in. |
| Warning | `!` yellow | The operation goes on, or has finished, and there is something to know. | What happens now, when work continues. |
| Notice | none | A plain statement of fact: stopped, continuing, dry run. | — |

### 6.1 Errors

- **M-1** An error is: one sentence of *what* (bold, after the symbol); an empty line;
  *why*, in at most three lines, with the arithmetic when there is one; an empty line;
  `Ways forward:` and, under it, things that can be copied; an empty line; and a last
  line, flush left, that says what state things were left in — `Nothing was changed.`
  when an existing file was involved, `Nothing was written.` when a new one was to be
  made, or what was kept and how to continue (C-8).
- **M-2** Under `Ways forward:` each line is indented four spaces. An option of the same
  command is shown alone (`--memory 6GB`), with its consequence in a second column
  starting at column 29; a different command is shown in full; a way that is not a
  command is a short phrase.
- **M-3** In Python the same error is an exception. The class name stands where the
  headline stands, the ways forward are calls, and `str()` of the exception is the whole
  text, so a log or a traceback carries everything:

```text
InsufficientBudget: communities() needs 5.1 GB for per-node state
(212,418,907 nodes × 24 bytes); the budget is 4.0 GB.
The edges stay on disk either way.

  Ways forward:
    g.communities(memory="6GB")       the smallest budget that works
    g.communities(precision="low")    needs 3.4 GB; slightly lower quality

Nothing was changed.
```

- **M-4** In a notebook the error is a card with the explanation first and the traceback
  folded beneath it (`notebook.html`, cell 7 and "The error card with the traceback
  unfolded"); each way forward has a copy button.
- **M-5** With `--json` the error is one document on standard output:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "urn:everygraph:schema:error:1",
  "title": "Everygraph error, version 1",
  "type": "object",
  "required": ["schema", "error", "message", "ways_forward", "changed"],
  "properties": {
    "schema": {"const": 1},
    "error": {
      "enum": [
        "InsufficientBudget", "InsufficientDisk", "InvalidInput",
        "InvalidExpression", "UnknownNode", "Unsupported", "CorruptFile",
        "EverygraphError"
      ]
    },
    "message": {"type": "string"},
    "why": {"type": "string"},
    "step": {"type": "string"},
    "needs_bytes": {"type": "integer", "minimum": 0},
    "budget_bytes": {"type": "integer", "minimum": 0},
    "available_bytes": {"type": "integer", "minimum": 0},
    "smallest_budget_bytes": {"type": "integer", "minimum": 0},
    "arithmetic": {"type": "object"},
    "problems": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["file", "line", "problem"],
        "properties": {
          "file": {"type": "string"},
          "line": {"type": "integer", "minimum": 1},
          "problem": {"type": "string"},
          "column": {"type": ["string", "null"]},
          "excerpt": {"type": "string"}
        }
      }
    },
    "invalid_rows": {"type": "integer", "minimum": 0},
    "rows_checked": {"type": "integer", "minimum": 0},
    "ways_forward": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["option"],
        "properties": {
          "option": {"type": "string"},
          "python": {"type": "string"},
          "note": {"type": "string"},
          "needs_bytes": {"type": "integer", "minimum": 0}
        }
      }
    },
    "changed": {"type": "boolean"},
    "kept": {"type": ["object", "null"]}
  }
}
```

```json
{
  "schema": 1,
  "error": "InsufficientBudget",
  "message": "Not enough memory for this step.",
  "why": "communities needs 5.1 GB for per-node state.",
  "step": "communities",
  "needs_bytes": 5098053768,
  "budget_bytes": 4000000000,
  "smallest_budget_bytes": 5598053768,
  "arithmetic": {"count": 212418907, "unit": "nodes", "bytes_each": 24},
  "ways_forward": [
    {"option": "--memory 6GB", "note": "the smallest budget that works"},
    {"option": "--precision low", "needs_bytes": 3400000000}
  ],
  "changed": false
}
```

The headline sentence of each kind of error, and its exception (the taxonomy itself is
in `docs/api-sketch.md` § 2.7):

| Exception | Headline on the command line |
|---|---|
| `InsufficientBudget` | `Not enough memory for this step.` |
| `InsufficientDisk` | `Not enough disk space for this step.` |
| `InvalidInput` | `Cannot read FILE: N lines are not valid rows.` (or the one thing that is wrong) |
| `InvalidExpression` | `Cannot understand the filter.` |
| `UnknownNode` | `ID is not in FILE.` |
| `Unsupported` | `OPERATION cannot be done on FILE.` |
| `CorruptFile` | `FILE is damaged.` |
| `EverygraphError` (no more specific class) | one sentence of what, for example `Another run of this job is in progress.` |

**Worked example: invalid input rows.** Each problem is located as `file:line`, the line
number without separators so it can be pasted into an editor; lines count from 1 and
include the header. For a format without lines the location is `file: row N`. The
excerpt shortens long fields in the middle (W-4) and the carets follow the shortened
text.

```console
$ everygraph ingest transfers.csv --source from --target to --weight value
✗ Cannot read transfers.csv: 2 lines are not valid rows.

  transfers.csv:48211907      5 fields where the header has 7
    0x4f3a…91bc,0x77e0…c1d4,12.400,2024-03-02T11:04:35Z,21000

  transfers.csv:1204877310    value is not a number
    0xb81c…5520,0x02fd…a9e3,n/a,2024-03-04T08:51:12Z,21000,7,0x
                            ^^^

  The whole file was checked: 1,830,402,119 lines, 2 of them invalid.

  Ways forward:
    correct those 2 lines, then run the same command again
    or leave out --weight value: line 1204877310 is then a valid edge

Nothing was written.
```

- **M-6** Input is strict by default: one invalid row refuses the file. The whole file
  is checked before refusing, so every problem is reported in one run and the count is
  exact. At most 10 problems are shown (*placeholder*), then `and 1,204 more`.
- **M-7** A way forward that changes the meaning of the data is offered only with its
  consequence stated (`line 1204877310 is then a valid edge`).

### 6.2 Warnings

- **M-8** A warning is: `!` and one sentence of what (bold); an empty line; the
  consequence, with numbers; an empty line; what would avoid it, as commands; an empty
  line; and what happens now. It never stops the work and never asks.
- **M-9** A warning is printed before the plan it affects, and its numbers agree with
  the plan's.

**Worked example: slow gzip input.** Printed when the input is gzip, which can only be
decompressed on one core, and the estimate for `reading` is at least twice what the
uncompressed file would take (*placeholder*). The faster way is offered only if the disk
has room for it, and the arithmetic is shown.

```console
$ everygraph ingest transfers.csv.gz --source from --target to
! transfers.csv.gz is gzip, which can only be read on one core.

  Reading will take about 24 min instead of about 2 min.

  Faster, if 71.4 GB of disk can be spared (1.4 TB are free):
    gunzip -k transfers.csv.gz
    everygraph ingest transfers.csv --source from --target to

  Continuing with the compressed file. Ctrl-C stops it safely.
```

**Worked example: a result cut short.** A point query stops at its `limit` (nodes) or
its `timeout` (seconds) and returns what it has. The result itself is marked — that is
the rule (`design/preview/choices.md` E1) — and the warning says where it stopped and
how to go further.

```text
>>> near = g.khop("0x28c6…1d60", k=2, limit=10_000)
! cut short: khop() stopped at its limit of 10,000 nodes,
  part of the way through hop 2. The result is partial
  and marked truncated=True.
  to go further: g.khop("0x28c6…1d60", k=2, limit=100_000)
>>> near.truncated
True
```

```console
$ everygraph query khop transfers.eg \
      0x28c6c06298d514db089934071355e5743bf21d60 --k 2 --limit 10000 --top 3
node                                        distance
0x21a31ee1afc51d94c2efccaa2092ad1028285549         1
0xdac17f958d2ee523a2206206994597c13d831ec7         1
0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48         1

3 of 10,000 rows shown
! cut short: khop stopped at its limit of 10,000 nodes,
  part of the way through hop 2. The result is partial.
  to go further: --limit 100000
```

The table is on standard output; the two messages under it are on standard error.

- **M-10** "Cut short" means the answer is partial. It is never used for a table of
  which only some rows are displayed: that is said as `3 of 10,000 rows shown`.
- **M-11** A result that is cut short is still a success: exit status 0, and
  `"truncated"` in the JSON document. An approximate result carries its bound in the
  same way (`docs/api-sketch.md` § 2.5).

Stable codes of the warnings in 0.1: `slow-gzip`, `cut-short`, `direction-guessed`
(the caller did not say whether the graph is directed and no column names decide it;
normally only the `edges` line of the summary says so, and the warning is used when
there is no summary to carry it).

### 6.3 Notices

Plain sentences with no symbol, each fixed:

| When | Text |
|---|---|
| Clean stop | `Stopped at your request, during PHASE.` and its block (§ 5.4) |
| Continuing | `Continuing the run stopped AGE ago: the file has not changed since.` |
| Dry run | `Dry run: nothing was written.` |
| A file was up to date | `transfers.eg is up to date with transfers.parquet: opened, not rebuilt.` |
| A result came from the file | `full result: transfers.eg (cached) · export with --out pagerank.parquet` (dim, under a table on a terminal) |
| Serving the map | `Serving on http://127.0.0.1:7461 (this computer only) — opening your browser.` then `Ctrl-C to stop.` (dim) |

### 6.4 Exit status

| Status | Meaning |
|---|---|
| 0 | Done. Includes a result cut short or approximate, and a dry run whose plan fits. |
| 1 | Failed for a reason not listed below (an input/output error, an internal error, a job already running). |
| 2 | The command line could not be understood: unknown command or option, missing argument, a filter that does not parse (`InvalidExpression`), an identifier that is not in the graph (`UnknownNode`), an operation the graph does not support (`Unsupported`). |
| 3 | The input is not valid (`InvalidInput`). |
| 4 | Refused before starting: `InsufficientBudget`, `InsufficientDisk`. Also a dry run whose plan does not fit. |
| 5 | A graph or map file is damaged (`CorruptFile`), or `everygraph verify` found a problem. |
| 130 | Stopped by `Ctrl-C`. |
| 143 | Stopped by SIGTERM. |

### 6.5 Tone

From `SPEC.md`, "Words", as rules that can be checked:

- **T-1** Plain, exact, unhurried. Say what happened, why — with the arithmetic when
  there is one — and what to do next.
- **T-2** No exclamation marks. (`!` appears only as the warning symbol in the first
  column.)
- **T-3** No "oops", no "simply", and no marketing adjectives. The lint starts from this
  list and grows: "blazing", "powerful", "seamless", "effortless", "amazing",
  "awesome", "lightning". A comparison that carries its number is not marketing
  (`about 24 min instead of about 2 min`).
- **T-4** A sentence ends with a full stop. A label-and-value line and a hint do not.
- **T-5** Every error has its ways forward; an error with none is a defect.
- **T-6** The person is never blamed and never addressed by guesswork: say what the file
  contains, not what the person did.
- **T-7** The user's own nouns where the data reveals them (`addresses`), "nodes" and
  "edges" everywhere else and in every name that is typed.
- **T-8** One word, one meaning: "budget" is memory; "limit" is a count of nodes;
  "timeout" is seconds; "scratch" is temporary disk; "kept" is finished work that will
  not be repeated; "cut short" is a partial answer.

---

## 7. Notebook specifics

The notebook is not a fourth design: each card is the text form with the same facts in
the same order (`notebook.html`, "Rules for outputs" and "Text equivalents"). What this
contract requires of #76:

- one card per operation, driven only by the events of § 4.6;
- cards follow the notebook's theme, never the operating system's;
- the plan card, the progress card and the error card show exactly the fields of § 3.2,
  § 4.3 and § 6.1; the error's traceback is folded, not removed;
- interrupting the kernel is a clean stop (C-6) and the card ends in the stop block;
- a display with no JavaScript (a notebook rendered as a static page, a log) falls back
  to the piped text form.

---

## 8. Test hooks

**H-0, the precondition.** All of this text is produced in one place by a renderer that
is a pure function: `render(events, surface, rendering, width) → text`, with no clock and
no probe of its own. The planner takes the machine and the clock as inputs. Every
message string lives in one catalogue. Without this the tests below are not possible;
with it they need no real graph.

| Rules | Mechanical test | Owner |
|---|---|---|
| S-1, S-2 | Run each command with standard output and standard error captured separately. With `--json`, standard output parses as exactly one JSON document; without it and piped, it is a tab-separated table with a header, or empty. Standard error never contains a result row. | #98 |
| S-3 | A Python test with `progress=False` asserts nothing is written to either stream; with `progress=f`, that `f` received events and nothing was written. | #81 |
| S-4, P-4 | Every command runs to completion with standard input closed. The message catalogue contains no question mark at the end of a line. | #98 |
| S-6 – S-9, W-1 – W-6, G-14 | **Golden transcripts.** Recorded event streams (one JSON object per line) are rendered for each surface in four renderings (`colour`, `plain`, `ascii`, `piped`) at widths 80 and 60 and compared with stored files, colour instructions included. A checker asserts for every golden file: no line over 80 columns; no redrawn line over 78; the `ascii` and `piped` files are pure ASCII; the `plain` file has no escape sequence; the `colour` file uses only the six instructions of S-7. | #98, #81 |
| §§ 2.5, 2.6, N-1, N-2 | Unit tests of the formatters on a table of inputs and expected strings (every example in § 2.6 is a row). A property test: a value with basis `sampled` or `model` never renders without `~` or "about". | #98 |
| P-1, P-5 | With the scratch and output directories on a file system that records writes, a refused operation performs none. Fault injection: each of the three checks of P-5 fails in turn and the matching refusal is produced. | #40, #45 |
| P-2 | Planning an ingest reads no more than the sample: counted through the I/O layer's read counters (#29). | #40 |
| P-3 | With a fake cost model, an operation estimated just under and just over `T_PLAN` prints nothing, respectively the plan. | #45 |
| P-6 – P-8, P-10 – P-12 | Golden transcripts of the plan for: ingest of a Parquet file, of a text file (sampled row count), an operation on a graph, an `explain`, a plan with a kept phase. | #40, #45 |
| § 3.6, P-13 – P-15, M-5, G-15 – G-17 | **Schemas.** The plan and error schemas are kept as files beside the tests, identical to the ones in this document, together with an event schema written from the table of § 4.6. Every plan, error and event produced in the test suite is validated against them; the examples in this document are validated by `docs/tools/check_schemas.py`. A sequence check on recorded streams: `plan` first, exactly one final event, one final `phase` event per phase. | #45, #81, #98 |
| P-9 | **Estimates against actuals.** The ingest report and each result's provenance record `estimate` and `actual` for time, peak memory, scratch and output size. The benchmark harness (#4) asserts each actual lies in the stated `[low, high]`, and for ingest on the tier-2 datasets within ±30 % of `value`; the four relative errors are stored with the run so the model can be tuned. | #40, #45 |
| P-11 | With a fake probe reporting a given available memory, container limit and container usage, the budget is half of what is free by the definition of P-11. | #45 |
| G-1, G-2 | Every phase name in the registry matches `^[a-z]{1,11}$`; every operation's emitted phases equal its plan's. | #45 |
| G-5 – G-9 | Property tests on recorded streams: percentages never decrease and never reach 100 before the final event; no `eta_seconds` before the thresholds of G-7; none when `total` is null; a final event's detail contains no `~`. | #81 |
| G-10 | A fake clock drives 1,000 updates in one second; the renderer emits at most 4 redraws, and one when a second passes with no update. | #98 |
| G-18, C-6 | A callback that raises on its third call: the exception propagates, a `stopped` event with reason `callback` precedes it, and the conditions of C-2 and C-3 hold. | #81 |
| R-1 – R-3, R-7, R-8, C-1 – C-3 | **Interruption tests.** For each operation and each phase: send SIGINT at a random moment inside the phase; assert exit status 130, the stop block (compared with numbers masked), `everygraph verify` on any `.eg` present, and that the directory listing equals what the block named. Run the same command again; assert the `kept` lines and that the output equals, byte for byte, the output of an uninterrupted run. Repeat with a different `--memory` and `--threads` on the second run. | #40 |
| C-1, C-2, R-8 | **Hard stops.** `kill -9` at ten random points of an ingest and of a `map`; every following run completes, the result is byte-identical to an uninterrupted run, and a graph file that existed before is unchanged in content. Crash injection between every write step of the container (#28). | #40, #28 |
| C-4, C-5, C-7 | Time from SIGINT to exit is under the bound on the tier-1 datasets; two SIGINTs 10 ms apart leave the conditions of C-1 true; exit statuses are 130 and 143. | #98 |
| R-4 – R-7 | Unit tests of the fingerprint: changing `--memory`, `--threads` or `progress` does not change it; changing any other parameter, the input's bytes or the output path does. Table-driven tests of R-7, one per row, each asserting the line printed. | #40 |
| M-1 – M-4, M-6 – M-10, § 6.3 | Golden transcripts of every worked example in § 6, on each surface. The corpus of malformed inputs (#32) produces `file:line` locations that are checked against the corpus's own index. | #32, #98, #81 |
| § 6.4 | A table-driven test: each exception class raised inside the command yields its exit status. | #98 |
| T-2 – T-5 | A lint over the message catalogue: forbidden words, exclamation marks, a full stop at the end of each sentence, every error entry has at least one way forward. | #98 |
| this document | `python3 docs/tools/check_transcripts.py docs/ux-contract.md`: every code block within 80 columns; the ASCII rendering of every transcript pure ASCII and within 80; redrawn lines within 78; the JSON blocks parse. `docs/tools/check_schemas.py`: the schemas are valid and the examples validate. | #13 |

---

## 9. Decided here

Choices this document had to make because the previews were silent or disagreed. Each is
one behaviour, with one line of reasoning. None changes an answer recorded in
`design/DECISIONS.md`.

1. **Stopping and running again behaves the same everywhere: finished phases are kept,
   the interrupted phase starts again.** `notebook.html` said "run the cell again to
   start over" while the command line resumed per phase. A single-phase operation does
   begin again, and says so with the same block. *One rule is easier to trust than two,
   and the notebook is where long jobs get interrupted most.*
2. **The default budget is printed as `(default: half of free RAM)`.** `terminal.html`
   was typeset with "a fifth of free RAM, at least 2 GB" before the owner's answer (half
   of free RAM, `cli.md` Choice 4). The example machine now shows 16 GB free so the
   approved figure of 8.0 GB stays true. *The decision wins over the mockup.*
3. **The summary shown before the run never contains a count that needs the whole
   file.** The previews print `4,201 self-loops dropped` before reading and, in `api.md`,
   `212,418,907 distinct (estimated)`. The first becomes `self-loops dropped` in the
   summary and `dropped: 4,201 self-loops` in the result block; the second is
   `~212.4 M distinct`. *A preflight that samples cannot know them, and exact-looking
   estimates are how numbers stop being trusted.*
4. **One text renderer for the command and for Python.** `api.md` and the text
   equivalents in `notebook.html` drew a 20-cell bar in brackets and a different plan
   layout that overflowed 80 columns; the typeset `terminal.html` form (16 cells, bar
   after the name) is used on both. *The contract is "shared by CLI, Python and the
   widget"; two layouts would need two sets of tests.*
5. **Everything `terminal.html` sets in the accent is bold instead.** `SPEC.md` settled
   that the suggested command is bold, not blue, because blue was unreadable on stock
   terminals; the same measurement applies to the URL and the options under "Ways
   forward". *So the command emits six instructions and no blue.*
6. **`map` and the single-phase algorithms print the four-line plan block when they are
   long.** `cli.md` shows `everygraph map` going straight to its phases, but the decision
   is that long operations print their plan. `T_PLAN` = 5 s is a placeholder. *The rule
   has no exception to remember.*
7. **Standard output is for results, standard error for everything else** (§ 2.1), and
   `--json` moves the final message to standard output. The previews did not say.
   *It is what makes `everygraph run … | other-tool` and `> file` safe.*
8. **Exit statuses 0–5, 130, 143** (§ 6.4). #98 asks for "meaningful exit codes" without
   listing them. *Refusal (4) and invalid input (3) are the two a script must tell apart
   from a crash.*
9. **A Python exception's text uses the `Ways forward:` list**, as on the command line
   and in `api.md`, not the bullets of the text equivalent in `notebook.html`. *One
   pattern for errors on every surface.*
10. **"Cut short" is reserved for partial answers.** The notebook cards also tag a table
    of which a few rows are displayed as "cut short"; that case says `N of M rows shown`.
    *T-8: one word, one meaning, and this one is a warning about the data.*
11. **The time left is always worded `about … left` and is per phase.** `api.md` wrote
    `1 min 52 s left`; `terminal.html` wrote `about 15 s left`. *An ETA is an estimate
    and should read like one.*
12. **When piped, the command prints the approved plain log; machine-readable progress
    is the Python event stream.** #98 mentions "structured logs" for non-terminals; the
    approved piped form is the ASCII log, and `--json` gives one final document.
    *The approved form wins; nothing is lost that a script can use.*
13. **Names of what is left on disk**: the scratch directory `everygraph-ID` (as in
    `terminal.html`), `manifest.json` inside it, and `NAME.eg.partial` beside the
    destination. *A person will see these in a listing; they should explain themselves.*
14. **Phase detail says "groups", as typeset.** `terminal.html` shortened
    "31,960 communities" to "31,960 groups" to fit the 32-column detail, although the
    chosen word is "communities" (`choices.md` B1). It is kept as typeset and listed in
    § 10 for the owner. *Following the latest approved page; flagged, not settled.*

## 10. Open points for other issues

Found while writing; none blocks this contract.

| Point | Where it shows | Suggested owner |
|---|---|---|
| "groups" in phase details and in `everygraph info` against "communities" as the chosen word. Either accept "groups" as the short form in width-limited lines or reword (`31,960 · modularity 0.603` fits). | `terminal.html`, § 4.1 | owner, via #121 |
| `terminal.html` still describes seven colour instructions and keeps blue "until that is decided"; `SPEC.md` has decided. Its `budget` line says "a fifth of free RAM". | `design/preview/style/terminal.html` | #121 |
| `notebook.html` uses `budget=10_000` for `khop` (now `limit=`), "start over" on interruption, bullets in the exception text, and "cut short" for display. | `design/preview/style/notebook.html` | #121, #76 |
| The option to skip invalid rows and count them (#32) has no approved name. If it is added (`--skip-invalid` is the obvious one), it becomes a third way forward in the invalid-input error and a `dropped:` kind in the result block. | § 6.1 | #32, then a preview |
| A phase that lasts hours prints nothing in a job log until it ends (the approved piped form has no heartbeat). | § 4.5 | #98 |
| No command lists or removes scratch left by stopped runs; the stop block's `discard:` line is the only pointer. `everygraph doctor` could report it. | § 5.3 | #98 |
| #98 lists `--yes`; nothing ever asks, so the option does not exist (S-4). | `plan/issues/m8-01-cli.md` | coordinator |
| #40's ±30 % criterion does not say which estimates; this contract applies it to all four (P-9). | `plan/issues/m2-13-preflight-resume.md` | coordinator |
| The acceptance of #13 says the contract is "referenced by" #40 and #45; both issue files already point here by number. No edit is needed, but #76, #81 and #98 depend on it as well. | `plan/issues/` | coordinator |
