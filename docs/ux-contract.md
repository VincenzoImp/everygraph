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
were silent or disagreed, and where this document departs from their wording, the
choice and its reason are listed in § 9 "Decided here" and nowhere else.

**Every figure in a transcript is invented.** The sizes, counts, durations and rates
below are carried over from the approved previews, or made up in the same spirit for
cases the previews do not show (a 2.6 B-node graph, a byte count per edge record), to
show shape and arithmetic. None is
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
| scratch | Temporary files of a running operation, in the job's scratch directory `.everygraph-ID` (§ 5.3). The directory in which that one is created is the **scratch root**: `--temp-dir`, `temp_dir=` or `EVERYGRAPH_TEMP_DIR` when one is given (`design/DECISIONS.md`, "Where scratch goes"). When none is given, the scratch root is the directory of the output file; for an operation that writes no file, the directory of the graph file; for a graph held in memory, the system temporary directory, where scratch on a memory-backed file system counts against the memory budget. P-18 is that rule. The system temporary directory is never the default for an operation on a file. |
| job | The work an operation does, identified by its fingerprint (§ 5.3): running the same command again is the same job. |
| refusal | A plan that cannot be carried out inside the limits, reported before any work. |
| rendering | The form text takes on a stream: colour and Unicode, no colour, ASCII, or piped (§ 2.2). |

---

## 2. Streams, renderings and typography

These rules apply to everything in §§ 3–6.

### 2.1 What goes where

| | `cli` | `python` | `notebook` |
|---|---|---|---|
| The result as data (a table, a path, a file's description) | standard output | the returned object; its `repr` | the returned object; its card |
| A result's summary (the summary of communities that `run communities` prints, the lines of `query path`) | standard output: it is the result; with `--top N` the table follows it; with `--json` the document's `summary` holds it | the returned object's `repr` | the returned object's card |
| The summary of an input (§ 3.4), plan, progress, result line, hints | standard error | `sys.stderr` | one card in the cell's output, updated in place |
| Warnings of an operation (§ 6.2) | standard error | `sys.stderr`, through the progress channel | a notice card |
| A result cut short (§ 6.2) | standard error | a `warnings` warning of category `eg.TruncatedWarning` | the result's card carries the "Cut short." note of `notebook.html` cell 8, and the same warning is issued (M-12) |
| Errors and refusals | standard error, and the exit code (§ 6.4) | an exception (§ 6.1) | the error card, traceback folded |
| A plan asked for on its own (`--dry-run`, `dry_run=True`, `explain`) | standard output, all of it — headline, summary, plan, `phases`, `result` and the closing `Dry run: nothing was written.` with its hint: the plan is the result | the returned `Plan`; nothing is printed | the returned `Plan`; its card |

- **S-1** Standard output of the command carries only what a program would consume: the
  result, and nothing else. `everygraph info`, `doctor`, `--help` and `--version` are
  results, and so is the summary a result prints above its table (the summary of
  communities). Everything about *how the work is going* is on standard error.
- **S-2** With `--json`, standard output is exactly one JSON document in every outcome:
  the result — the result document of `docs/api-sketch.md` § 5.3 for a table, the
  file document of § 4.7 for an operation that makes a file (`ingest`, `map`,
  `export`); the plan with `--dry-run`; the error document of § 6.1 when the command
  fails or is refused, a usage error included; the stop document of § 5.4 when it is
  stopped (exit status 130 or 143). Standard error keeps the plan, progress and
  warnings (with `--dry-run --json`, the text of the plan); it does not repeat the
  final message.
- **S-3** A library call prints only through the progress channel, and only when
  `progress=True` (the default). `progress=False` prints nothing; `progress=f` sends
  the events of § 4.6 to `f` and prints nothing. Results never print themselves. The
  one exception is a result cut short (§ 6.2): it is reported with Python's
  `warnings.warn(…, eg.TruncatedWarning)`, so Python's own warning filters show it,
  silence it or turn it into an error, whatever `progress` says.
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
- **S-10** Progress also takes the piped form — ASCII, nothing redrawn, no bars, one line
  per finished phase (§ 4.5) — on a terminal under `TERM=dumb`, and whenever
  `--progress=plain` is given. Redrawn bars are noise for a screen reader, and
  `TERM=dumb` alone only removed colour (added 2026-11-25, `docs/research/README.md`
  H6; `docs/research/sota-human-factors.md` § 3, challenge 6). The rest of the stream
  keeps the rendering the table above gives it: tables on standard output stay aligned
  on a terminal.

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
  blocks use a label field of 10 columns, so values start at column 13. The step-table
  form of an `explain` plan (§ 3.4) has its own grid, as typeset in `notebook.html`:
  labels `fits`, `why`, `estimate` in a field of 9 columns, values from column 12;
  step numbers at column 3, steps from column 6, then the place, the memory
  right-aligned, and the work, each separated by at least two spaces.
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
| Size | `B` (bytes, below 1 kB, a whole number), then `kB`, `MB`, `GB`, `TB`, powers of 10 (`GB` = 10⁹ bytes). Exact sizes and budgets: one decimal below 100. Estimates and what the machine reports: two significant digits. | `28 B`, `38.2 GB`, `8.0 GB`; `61 GB scratch`, `44 GB free` |
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
  count of a Parquet file) or an estimate marked with `~`. A sample that covers the
  whole file (a small text file) gives exact counts, with basis `exact` (P-7):
  `tiny.csv · 28 B · 6 rows`.
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
  | A probe of the machine | cores, memory installed, free and limited by a container, storage type, free disk in the scratch and output directories, and whether the scratch root is on a memory-backed file system (`tmpfs`, `ramfs`) | #45 |

  Free memory and free disk are read at that moment, not cached. The time spent planning
  is bounded: at most 1 s or 1 % of the estimated run, whichever is larger (*placeholder*).
- **P-3** The plan block and the progress lines are printed, without being asked,
  exactly when the operation is long (§ 1). A short operation prints neither: only its
  result. Three things do not depend on length: an operation that reads a file always
  prints its headline and summary (what it understood is worth four lines every time,
  and it is where a guessed direction is stated); a plan is always printed for
  `--dry-run`, and returned for `dry_run=True` and `explain`; a plan is always part of
  a refusal.
- **P-4** After the plan is printed the work starts at once (S-4).
- **P-5** Before starting, the engine checks, in this order, and refuses on the first
  failure (§ 3.5):
  1. the per-node state of every phase fits the memory budget — and, when the scratch
     root is on a memory-backed file system, the per-node state plus the upper bound
     of the scratch estimate fits it (P-18);
  2. the scratch directory has room for the upper bound of the scratch estimate;
  3. the output directory has room for the upper bound of the output estimate, and the
     output can be created there.
- **P-16** A memory budget larger than the machine's memory in all (or than the
  container's limit, when that is lower) is refused before any work, as a usage error
  (exit status 2; in Python a `ValueError` with the same text): what was asked, what the
  machine has, and the ways forward. It is never silently lowered.
- **P-17** A budget above what is free now but within the machine's memory is honoured.
  The plan says so on a second `budget` line (§ 3.4), because the system may have to
  swap or evict other programs' memory to give it.
- **P-18** Scratch on a memory-backed file system is memory. When the scratch root is
  one, the preflight counts the scratch's upper bound against the memory budget; if the
  two together do not fit, the refusal offers `--temp-dir` (`temp_dir=`) with a
  directory on a disk, and a larger budget only if one would work (P-15).

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
| `scratch`, `output` | Where scratch goes, where that choice came from, whether it is memory-backed, and how much is free there; what will be written and where. | `estimate`, `result` |
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
  without one. The ratio is `regime.working_bytes / budget.memory_bytes`, printed with
  one decimal, where `working_bytes` is the graph as the operation must handle it:
  for an operation on a graph, the size of the edge data it reads (the map example
  below: 9.3 GB of edges against 8.0 GB, `1.2×`); for a build, the edge records the
  build sorts, rows × bytes per record (the ingest example: 1,830,402,118 rows ×
  20 bytes — two 4-byte node numbers, an 8-byte weight, a 4-byte time — is
  36.6 GB against 8.0 GB, `4.6×`; the record size is invented, like every figure here).

### 3.3 Estimates and their uncertainty

- **P-7** Every estimate is a triple — `value`, `low`, `high` — with a `basis`:
  `exact` (known, `low` = `value` = `high`), `sampled` (from a sample of the input) or
  `model` (from the cost model alone). `[low, high]` is an interval with a stated
  target coverage, not a guarantee: on an idle machine, the actual figure lies inside
  it in at least 90 % of runs (*placeholder*), counted per quantity over the benchmark
  corpus. Wall time depends on what else the machine is doing, so its coverage is
  defined, and measured, on an idle machine only (`CLAUDE.md` § 3).
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
  `(actual − value) / actual` for each of the four quantities. When the actual is 0
  (an operation that used no scratch), the relative error is `0` if `value` is 0 too,
  and otherwise undefined: it is recorded as `null`, next to the absolute error
  `value − actual` in the quantity's unit, and it counts as a miss wherever a bound is
  checked. The bound of #40 — estimates within ±30 % of actuals on the tier-2 datasets —
  is checked where #40 asks for it, for an ingest, on the four quantities, by the
  benchmark harness (#4) on an idle machine. No test anywhere depends on a wall time
  measured under load.

### 3.4 Text form

The plan is at most four blocks, in this order, separated by one empty line.

1. **Headline.** The file's name in bold, its size, its rows or its nodes and edges.
2. **Summary** — only for an operation that reads an input the engine has not seen
   (`ingest`, `eg.read`), and then always (P-3): `ids`, `edges`, `columns`.
3. **Plan block** — when the operation is long, always these four lines: `machine`,
   `budget`, `plan`, `estimate`.
4. **Then** the progress lines (§ 4) for a run, or the two lines `phases` and `result`
   and a closing sentence for a dry run.

<!-- block: ingest-plan -->
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
  estimate  3 min 40 s · peak 5.9 GB · 61 GB scratch · result 9.3 GB
```

Rules for the lines:

| Line | Content, in order | Variants |
|---|---|---|
| headline | name · size · rows | `~1.83 B rows` when the count is sampled (a text file); for a graph: name · size · nodes · edges |
| `ids` | kind of identifier (width) · distinct count | kinds: `integers`, `hex addresses (20 bytes)`, `text` |
| `edges` | direction · repeated edges · self-loops | repeated edges `duplicates kept as events`, `duplicates collapsed` or `duplicates summed`, for the options `duplicates="events"`, `"collapse"`, `"sum"` (#36); `self-loops dropped` or `self-loops kept`; `directed (from the column names)` or `undirected (the column names give no direction)` when the caller did not say; a second line, aligned with the first value, when one does not fit |
| `columns` | each used column `name → role` · `ignored:` the rest | `a → source · b → target` first when they were detected rather than given; more than three ignored columns are counted: `ignored: 12 columns` |
| `machine` | physical cores · memory installed (free) · storage, free space | `(8.0 GB allowed by the container)` in place of the free figure when a limit is lower |
| `budget` | size `memory` (where it came from) | `(default: half of free RAM)`, `(--memory)`, `(memory=)`, `(EVERYGRAPH_MEMORY)`, `(set when the graph was opened)` for a default given to `eg.read` or `eg.open` (origin `graph`); when the budget is more than is free now (P-17), a second line aligned with the value: `more than the 16 GB free now: the system may swap` |
| `plan` | regime `—` reason | § 3.2 |
| `estimate` | time · `peak` memory · scratch, and `in` its root when that is not the default (`61 GB scratch in /data/tmp`) · `result` size | `no scratch` when none; `result 1.7 GB in transfers.eg` when the result is stored inside the graph file |

- **P-10** The summary states rules and estimates, never a count that needs the whole
  file (N-1). Exact counts of what was dropped appear in the result block (§ 4.4).
- **P-11** The default memory budget is half of the memory that is free when the plan is
  computed. "Free" is what the process could still obtain: the system's available
  memory or, inside a container with a lower limit, what remains under that limit. The
  `budget` line always says where the figure came from.

A dry run prints the same blocks, then what would happen instead of doing it. All of it
is on standard output, because the plan is the result of a dry run (§ 2.1):

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
  estimate  3 min 40 s · peak 5.9 GB · 61 GB scratch · result 9.3 GB

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
  estimate  about 4 min · peak 5.6 GB · 12 GB scratch · result 4.1 GB
```

The plan of an operation on a graph, **asked for on its own** — `g.explain("communities")`
or `dry_run=True` in Python, `--dry-run` on the command line — is the step-table form
typeset in `notebook.html`. It is the same text on both surfaces, except that the
command line writes the operation without parentheses and ends with the dry-run notice:

```text
Plan for communities() on transfers.eg — Leiden, resolution 1.0, deterministic
  fits     peak 5.6 GB of 8.0 GB budget

  1  keep per-node state              memory            5.1 GB
  2  move nodes between communities   edges from disk   0.5 GB   about 12 passes
  3  merge communities and repeat     memory          < 0.4 GB   4 levels
  4  write the result                 disk                   —

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
  Community detection has no lower-memory variant in 0.1 (`design/DECISIONS.md`, "A
  low-memory variant of community detection"), so its refusal offers a budget only.
  When the smallest budget that would work is more than the machine's memory in all,
  the refusal says so in plain words and offers no `--memory` line: no budget on this
  machine would work.

```console
$ everygraph run communities transfers.eg --memory 4GB
✗ Not enough memory for this step.

  communities needs 5.1 GB for per-node state (212,418,907 nodes × 24 bytes).
  The budget is 4.0 GB. The edges stay on disk either way.

  Ways forward:
    --memory 6GB            the smallest budget that works

Nothing was changed.
```

The arithmetic: 212,418,907 × 24 = 5,098,053,768 bytes of state; with the plan's fixed
buffers of 0.5 GB (invented) the smallest budget is 5,598,053,768 bytes, rounded up
to `6GB` (P-14).

When no budget on this machine is enough (an invented graph of 2.6 B nodes: 2.6 × 10⁹ ×
24 bytes = 62.4 GB, plus 0.5 GB of buffers, rounds up to 63 GB, more than 61 GB):

```console
$ everygraph run communities web.eg
✗ Not enough memory for this step.

  communities needs 62.4 GB for per-node state (2,600,000,000 nodes × 24 bytes).
  The budget is 8.0 GB, and this machine has 61 GB of memory in all, so no
  budget on it is enough. The edges stay on disk either way.

  Ways forward:
    a machine with at least 63 GB of memory

Nothing was changed.
```

When the scratch root is on a memory-backed file system (P-18), here `/tmp` given with
`--temp-dir`; up to 70 GB of scratch and a peak of 5.9 GB are 75.9 GB, more than this
machine has, so no budget is offered:

```console
$ everygraph ingest transfers.parquet --source from --target to \
      --temp-dir /tmp
✗ Not enough memory for this step.

  /tmp is held in memory (tmpfs), so its scratch counts against the budget:
  up to 70 GB of scratch and a peak of 5.9 GB do not fit 8.0 GB.

  Ways forward:
    --temp-dir DIR          a directory on a disk with 70 GB free
    leave out --temp-dir    scratch goes beside the output (1.4 TB free)

Nothing was written.
```

A budget larger than the machine (P-16) is a usage error, before any plan:

```console
$ everygraph run communities transfers.eg --memory 80GB
✗ The memory budget is more than this machine has.

  --memory 80GB asks for 80.0 GB; this machine has 61 GB of memory in all.

  Ways forward:
    --memory 16GB           what is free now
    leave out --memory      the default, half of free RAM: 8.0 GB now

Nothing was changed.
```

A budget above what is free but within the machine (P-17) runs, and the plan says so:

```console
  machine   16 cores · 61 GB RAM (16 GB free) · NVMe, 1.4 TB free
  budget    40.0 GB memory (--memory)
            more than the 16 GB free now: the system may swap
```

The same refusal in Python is an exception (§ 6.1 gives the pattern) and in a notebook
the error card of `notebook.html`, cell 7. With `--dry-run`, `dry_run=True` or `explain`
a plan that does not fit is *returned*, not raised: the refusal is its content, and the
command exits with status 4.

### 3.6 Machine-readable form

`Plan.to_dict()` in Python, the `plan` event (§ 4.6), and the standard output of
`--dry-run --json` are the same document. A refusal inside a plan is the error document
of § 6.1, by reference: there is one definition of it.

**Compatibility.** Every document carries `schema`, and these rules hold for all the
schemas of this contract — plan, error, event, stop and file (§ 4.7) — and for the
result document of `docs/api-sketch.md` § 5.3:

- Within version 1 a later release may **add an optional key**. It never removes or
  renames a key, never changes a key's type or meaning, never makes an optional key
  required, and never adds a value to an enumeration. Any of those increments `schema`.
- A **reader of version 1** must tolerate keys it does not know, at any depth, and may
  rely on every required key being present with its stated type and on every
  enumeration being complete for version 1.
- The schemas below are **closed** (`additionalProperties: false`, or
  `unevaluatedProperties: false` where a definition is shared) wherever this document
  fixes the keys, so the engine's own tests catch a misspelt key. They describe what
  one release writes: a reader that checks a document from a newer release must not
  use them as they are (it drops the closing keyword first). The open places are
  `parameters`, which holds the operation's own arguments, `arithmetic`, `counts`
  and `dropped` in the `result` event and the file document, and `summary`, `meta`
  and the rows in the result document.
- The enumerations are complete for 0.1, including the choices that #36 builds
  (`self_loops`: `dropped`, `kept`; `duplicates`: `kept as events`, `collapsed`,
  `summed`) and every error class that 0.1 can produce (§ 6.1), even where the
  approved defaults use only one.

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "urn:everygraph:schema:plan:1",
  "title": "Everygraph plan, version 1",
  "type": "object",
  "additionalProperties": false,
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
      "additionalProperties": false,
      "required": ["python", "command"],
      "properties": {
        "python": {"type": "string"},
        "command": {"type": "string"}
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
      "additionalProperties": false,
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
      "additionalProperties": false,
      "required": ["path", "from", "memory_backed", "free_bytes"],
      "properties": {
        "path": {"type": "string"},
        "from": {"$ref": "#/$defs/origin"},
        "memory_backed": {"type": "boolean"},
        "free_bytes": {"$ref": "#/$defs/count"}
      }
    },
    "output": {
      "type": "object",
      "additionalProperties": false,
      "required": ["path", "kind"],
      "properties": {
        "path": {"type": ["string", "null"]},
        "kind": {"enum": ["graph", "map", "result", "file", "none"]},
        "free_bytes": {"$ref": "#/$defs/count"}
      }
    },
    "fits": {"type": "boolean"},
    "refusal": {
      "oneOf": [
        {"type": "null"},
        {"$ref": "urn:everygraph:schema:error:1"}
      ]
    }
  },
  "$defs": {
    "count": {"type": "integer", "minimum": 0},
    "origin": {"enum": ["argument", "graph", "environment", "default"]},
    "estimate": {
      "type": "object",
      "additionalProperties": false,
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
      "additionalProperties": false,
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
      "additionalProperties": false,
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
      "additionalProperties": false,
      "required": ["ids", "edges", "columns"],
      "properties": {
        "ids": {
          "type": "object",
          "additionalProperties": false,
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
          "additionalProperties": false,
          "required": ["directed", "directed_from", "duplicates", "self_loops"],
          "properties": {
            "directed": {"type": "boolean"},
            "directed_from": {"enum": ["caller", "column names", "default"]},
            "duplicates": {
              "enum": ["kept as events", "collapsed", "summed"]
            },
            "self_loops": {"enum": ["dropped", "kept"]}
          }
        },
        "columns": {
          "type": "array",
          "items": {
            "type": "object",
            "additionalProperties": false,
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
      "additionalProperties": false,
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
          "additionalProperties": false,
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
      "additionalProperties": false,
      "required": [
        "memory_bytes", "memory_from", "above_free", "threads", "threads_from"
      ],
      "properties": {
        "memory_bytes": {"$ref": "#/$defs/count"},
        "memory_from": {"$ref": "#/$defs/origin"},
        "above_free": {"type": "boolean"},
        "threads": {"type": "integer", "minimum": 1},
        "threads_from": {"$ref": "#/$defs/origin"}
      }
    },
    "regime": {
      "type": "object",
      "additionalProperties": false,
      "required": ["name", "reason", "working_bytes", "ratio"],
      "properties": {
        "name": {"enum": ["in_memory", "mapped", "streamed", "on_disk"]},
        "reason": {"type": "string"},
        "working_bytes": {"$ref": "#/$defs/count"},
        "ratio": {"type": "number", "minimum": 0}
      }
    },
    "step": {
      "type": "object",
      "additionalProperties": false,
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
      "additionalProperties": false,
      "required": ["name", "kept", "seconds"],
      "properties": {
        "name": {"type": "string", "pattern": "^[a-z]{1,11}$"},
        "kept": {"type": "boolean"},
        "seconds": {"$ref": "#/$defs/estimate"},
        "regime": {"enum": ["in_memory", "mapped", "streamed", "on_disk"]}
      }
    }
  }
}
```

`regime.ratio` is `working_bytes / budget.memory_bytes` (P-6), unrounded; the text
rounds it. `budget.above_free` is true when P-17 applies. `scratch.path` is the scratch
root; the job's own directory inside it is named in § 5.3.

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
    "above_free": false, "threads": 16, "threads_from": "default"
  },
  "regime": {
    "name": "on_disk",
    "reason": "the graph is 4.6× the memory budget",
    "working_bytes": 36608042360,
    "ratio": 4.576
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
  "scratch": {
    "path": ".", "from": "default", "memory_backed": false,
    "free_bytes": 1400000000000
  },
  "output": {
    "path": "transfers.eg", "kind": "graph", "free_bytes": 1400000000000
  },
  "fits": true,
  "refusal": null
}
```

The `refusal` of § 3.5, inside a plan whose `fits` is `false`. It is the same document
that `--json` prints when the command is refused (§ 6.1), and it validates against the
error schema:

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
    {
      "command": "--memory 6GB",
      "python": "g.communities(memory=\"6GB\")",
      "note": "the smallest budget that works"
    }
  ],
  "changed": false
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
| | `identifiers` | distinct identifiers (unit `identifiers`) | identifiers, with the user's noun: `212,418,907 addresses` |
| | `building` | parts | `2,048 parts` |
| | `checking` | sections of the file (unit `sections`) | the checks that passed: `sorted · symmetric counts` |
| `map`, `g.map()` | `communities` | passes over the edges | `31,960 · modularity 0.603` (the noun is dropped for width: `design/DECISIONS.md`, "The word for what community detection finds") |
| | `hierarchy` | levels (unit `levels`) | `6 zoom levels` |
| | `layout` | regions (unit `regions`) | (empty) |
| | `tiles` | tiles | `1,204,118 tiles` |
| `run NAME`, `g.NAME()` | one phase, named after the operation | passes over the edges, or nodes | one or two exact facts about the result: `31,960 communities · modularity 0.603` where it fits |
| the first count of a view (`v.num_nodes`, `v.num_edges`) | `counting` | edges (unit `edges`) | the edges counted: `4,118,306 edges` (the nodes are in the view's headline) |
| `export`, `g.save()`, `t.write()`, `--out` | `writing` | rows (unit `rows`) | the rows written: `212,418,907 rows` |

A whole-graph algorithm is a single phase: `communities`, `pagerank`, `components`,
`kcore`, `triangles`, `degrees`, `bfs`. Point queries (`neighbors`, `khop`, `path`,
`common_neighbors`, `edges_between`) have no phases and no progress; they have `limit`
(and `khop` and `path` a `timeout`) instead (§ 6.2, "cut short").

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

<!-- block: states -->
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

<!-- block: ingest-done -->
```console
  reading      ████████████████  1,830,402,118 rows                 1 min 41 s
  identifiers  ████████████████  212,418,907 addresses                    38 s
  building     ████████████████  2,048 parts                              59 s
  checking     ████████████████  sorted · symmetric counts                12 s

✓ transfers.eg · 9.3 GB · 212,418,907 nodes · 1,830,397,917 edges · 3 min 30 s
  dropped: 4,201 self-loops
  next: everygraph map transfers.eg
```

- **G-11** An operation that makes a file ends with the result block. Its first line
  is `✓`, what was made (bold), its size, its exact counts (for a graph, nodes and
  edges; a map has none beyond its phase lines) and, always, the total time — the
  `map` example below, `4 min 12 s`, is the sum of its four phases; it is the only
  place the total of a file-making operation appears. A graph held in memory is named
  by its input and says `in memory` where the size would be
  (`✓ friends.csv · in memory · 68,304 nodes · 999,962 edges · 2 s`). An operation
  that returns a table or an object has no result block: its last phase line is the
  record, and the result follows on its own channel (§ 2.1).
- **G-12** `dropped:` follows when rows of the input did not become edges, with the
  exact count of each kind. It is left out when nothing was dropped. The kinds, in
  this order, joined by ` · `: `N self-loops` (`self_loops="drop"`, key
  `dropped.self_loops`) and `N duplicates` (`duplicates="collapse"`, the repeated
  rows dropped, key `dropped.duplicates`). Under `duplicates="sum"` the repeated rows
  are not dropped but merged into their pair's edge: a line `merged:` follows
  `dropped:` (or takes its place) with `N duplicates, summed into their pairs` (key
  `counts.merged`), left out when it is 0. With the edges of the `✓` line the two
  lines add up to the rows read (`docs/spec/graph-semantics.md` § 2.8).

```console
✓ transfers.eg · 8.1 GB · 212,418,907 nodes · 1,604,551,209 edges · 3 min 22 s
  dropped: 4,201 self-loops · 225,846,708 duplicates
  next: everygraph map transfers.eg
```

```console
✓ transfers.eg · 8.4 GB · 212,418,907 nodes · 1,604,551,209 edges · 3 min 25 s
  dropped: 4,201 self-loops
  merged: 225,846,708 duplicates, summed into their pairs
  next: everygraph map transfers.eg
```
- **G-13** At most two hints follow, each a label and something that can be run, the
  runnable part in bold: `next:` on the command line; in Python the hint is a call
  (`reopen later: eg.open("transfers.eg")`). A hint is never a sentence of advice.

```console
$ everygraph map transfers.eg
transfers.eg · 9.3 GB · 212,418,907 nodes · 1,830,397,917 edges

  machine   16 cores · 61 GB RAM (16 GB free) · NVMe, 1.4 TB free
  budget    8.0 GB memory (default: half of free RAM)
  plan      edges on disk, read in passes — the graph is 1.2× the memory budget
  estimate  about 4 min · peak 5.6 GB · 12 GB scratch · result 4.1 GB

  communities  ████████████████  31,960 · modularity 0.603          2 min 11 s
  hierarchy    ████████████████  6 zoom levels                            18 s
  layout       ████████████████                                           41 s
  tiles        ████████████████  1,204,118 tiles                     1 min 2 s

✓ transfers.egmap · 4.1 GB · 4 min 12 s
  open it:   everygraph open transfers.eg
  share it:  everygraph map transfers.eg --out shared.egmap --reveal summary
```

The working map `transfers.egmap` is full, so the `share it:` hint is the command that
writes a copy for sharing (`docs/api-sketch.md` § 3.8). It suggests `--reveal summary`,
regions and counts only; `--reveal full`, every identifier, is the other value, and
`--reveal` has no default (`design/DECISIONS.md`, "What a shared map reveals"). In
Python the hint is `share it: m.export("shared.egmap", reveal="summary")`.

### 4.5 The other renderings

**ASCII only.** The same lines through the table of § 2.3:

<!-- rendering: ascii of states -->
```text
  reading      ################  1,830,402,118 rows                 1 min 41 s
  identifiers  #########.......  61% | 5.6 M/s | about 15 s left          23 s
  building     ................
  checking     ................
```

**Piped.** Nothing is redrawn. The headline, summary and plan block are printed as they
are (in ASCII), then each phase prints one line when it finishes, without a bar, its
time ending at column 60; then the result block.

<!-- rendering: piped of ingest-plan + ingest-done -->
```text
transfers.parquet | 38.2 GB | 1,830,402,118 rows

  ids       hex addresses (20 bytes) | ~212.4 M distinct
  edges     directed | duplicates kept as events | self-loops dropped
  columns   value -> weight | block_time -> time | ignored: gas, nonce, input

  machine   16 cores | 61 GB RAM (16 GB free) | NVMe, 1.4 TB free
  budget    8.0 GB memory (default: half of free RAM)
  plan      build on disk - the graph is 4.6x the memory budget
  estimate  3 min 40 s | peak 5.9 GB | 61 GB scratch | result 9.3 GB

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

<!-- rendering: narrow of states -->
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
- **G-19** `job` is the first 16 hexadecimal digits of the job's fingerprint (R-4), and
  the scratch directory's `ID` is a prefix of it. It is the same for every event of one
  job, and two runs of the same job share it: a run that continues a stopped one
  carries the same `job`, and so does a later run of the same command, even after the
  input changed (the fingerprint leaves out the input's identity). `job` names the
  work, not the run.

Every event has these keys:

| Key | Type | Meaning |
|---|---|---|
| `event` | string | `plan`, `phase`, `warning`, `result`, `stopped` or `error` |
| `schema` | integer | `1` |
| `job` | string | 16 hexadecimal digits of the fingerprint: the same for every event of one job, and shared by every run of that job (G-19) |
| `operation` | string | as in the plan |
| `t` | number | seconds since this run of the operation started |

and, by kind:

| Event | Key | Type | Meaning |
|---|---|---|---|
| `plan` | `plan` | object | the plan document of § 3.6 |
| `phase` | `name` | string | the phase |
| | `index`, `count` | integer | position from 0, and the number of phases |
| | `state` | string | `running`, `done`, `kept` or `stopped` |
| | `unit` | string | `bytes`, `rows`, `identifiers`, `nodes`, `edges`, `parts`, `sections`, `passes`, `levels`, `regions`, `tiles`, `items` (§ 4.1) |
| | `done`, `total` | number; number or null | progress in that unit; the total if known |
| | `fraction` | number or null | `done / total`, from 0 to 1 |
| | `pass` | object or null | `{"index": 2, "of": 3}`; `of` is null when open-ended |
| | `rate` | number or null | units per second |
| | `eta_seconds` | number or null | time left, when G-7 allows a claim |
| | `elapsed_seconds` | number | time in this phase |
| | `detail` | string or null | the finished detail of § 4.1, for final states |
| | `peak_memory_bytes` | integer | peak so far in this phase |
| `warning` | `code` | string | a stable identifier: `slow-gzip`, `read-only`, `old-scratch` (§ 6.2, R-10) |
| | `message` | string | the headline sentence |
| | `why` | string or null | the consequence, with its numbers |
| | `ways_forward` | array | ways, as in § 6.1 |
| | `continuing` | string or null | what happens now |
| `result` | `output` | object | `path`, `kind`, `bytes` |
| | `counts` | object | exact counts: `nodes`, `edges`, `rows`, `merged` (G-12), … |
| | `dropped` | object | exact counts by kind: `self_loops`, `duplicates` (G-12) |
| | `elapsed_seconds`, `peak_memory_bytes`, `scratch_bytes` | number | the actual figures |
| | `estimate_error` | object | for each of the four estimates, `relative` (number or null) and `absolute` (P-9) |
| | `next` | array | hints: `label`, `python`, `command` |
| `stopped` | `stopped` | object | the stop object of § 5.4: also what `--json` prints after a stop |
| `error` | `error` | object | the error document of § 6.1 |

The same, as a schema. The stop object is defined here and used by the stop document
of § 5.4:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "urn:everygraph:schema:event:1",
  "title": "Everygraph progress event, version 1",
  "type": "object",
  "required": ["event", "schema", "job", "operation", "t"],
  "properties": {
    "event": {
      "enum": ["plan", "phase", "warning", "result", "stopped", "error"]
    },
    "schema": {"const": 1},
    "job": {"type": "string", "pattern": "^[0-9a-f]{16}$"},
    "operation": {"type": "string"},
    "t": {"type": "number", "minimum": 0}
  },
  "oneOf": [
    {"$ref": "#/$defs/plan_event"},
    {"$ref": "#/$defs/phase_event"},
    {"$ref": "#/$defs/warning_event"},
    {"$ref": "#/$defs/result_event"},
    {"$ref": "#/$defs/stopped_event"},
    {"$ref": "#/$defs/error_event"}
  ],
  "unevaluatedProperties": false,
  "$defs": {
    "count": {"type": "integer", "minimum": 0},
    "rate": {"type": ["number", "null"], "minimum": 0},
    "plan_event": {
      "properties": {
        "event": {"const": "plan"},
        "plan": {"$ref": "urn:everygraph:schema:plan:1"}
      },
      "required": ["plan"]
    },
    "phase_event": {
      "properties": {
        "event": {"const": "phase"},
        "name": {"type": "string", "pattern": "^[a-z]{1,11}$"},
        "index": {"$ref": "#/$defs/count"},
        "count": {"type": "integer", "minimum": 1},
        "state": {"enum": ["running", "done", "kept", "stopped"]},
        "unit": {
          "enum": [
            "bytes", "rows", "identifiers", "nodes", "edges", "parts",
            "sections", "passes", "levels", "regions", "tiles", "items"
          ]
        },
        "done": {"type": "number", "minimum": 0},
        "total": {"type": ["number", "null"], "minimum": 0},
        "fraction": {"type": ["number", "null"], "minimum": 0, "maximum": 1},
        "pass": {
          "oneOf": [
            {"type": "null"},
            {
              "type": "object",
              "additionalProperties": false,
              "required": ["index", "of"],
              "properties": {
                "index": {"type": "integer", "minimum": 1},
                "of": {"type": ["integer", "null"], "minimum": 1}
              }
            }
          ]
        },
        "rate": {"$ref": "#/$defs/rate"},
        "eta_seconds": {"$ref": "#/$defs/rate"},
        "elapsed_seconds": {"type": "number", "minimum": 0},
        "detail": {"type": ["string", "null"]},
        "peak_memory_bytes": {"$ref": "#/$defs/count"}
      },
      "required": [
        "name", "index", "count", "state", "unit", "done", "total",
        "fraction", "pass", "rate", "eta_seconds", "elapsed_seconds",
        "detail", "peak_memory_bytes"
      ]
    },
    "warning_event": {
      "properties": {
        "event": {"const": "warning"},
        "code": {"enum": ["slow-gzip", "read-only", "old-scratch"]},
        "message": {"type": "string"},
        "why": {"type": ["string", "null"]},
        "ways_forward": {
          "type": "array",
          "items": {"$ref": "urn:everygraph:schema:error:1#/$defs/way"}
        },
        "continuing": {"type": ["string", "null"]}
      },
      "required": ["code", "message", "why", "ways_forward", "continuing"]
    },
    "result_event": {
      "properties": {
        "event": {"const": "result"},
        "output": {
          "type": "object",
          "additionalProperties": false,
          "required": ["path", "kind", "bytes"],
          "properties": {
            "path": {"type": ["string", "null"]},
            "kind": {"enum": ["graph", "map", "result", "file", "none"]},
            "bytes": {"type": ["integer", "null"], "minimum": 0}
          }
        },
        "counts": {
          "type": "object",
          "additionalProperties": {"$ref": "#/$defs/count"}
        },
        "dropped": {
          "type": "object",
          "additionalProperties": {"$ref": "#/$defs/count"}
        },
        "elapsed_seconds": {"type": "number", "minimum": 0},
        "peak_memory_bytes": {"$ref": "#/$defs/count"},
        "scratch_bytes": {"$ref": "#/$defs/count"},
        "estimate_error": {
          "type": "object",
          "additionalProperties": false,
          "required": [
            "seconds", "peak_memory_bytes", "scratch_bytes", "output_bytes"
          ],
          "properties": {
            "seconds": {"$ref": "#/$defs/error_of"},
            "peak_memory_bytes": {"$ref": "#/$defs/error_of"},
            "scratch_bytes": {"$ref": "#/$defs/error_of"},
            "output_bytes": {"$ref": "#/$defs/error_of"}
          }
        },
        "next": {
          "type": "array",
          "maxItems": 2,
          "items": {
            "type": "object",
            "additionalProperties": false,
            "required": ["label", "python", "command"],
            "properties": {
              "label": {"type": "string"},
              "python": {"type": "string"},
              "command": {"type": "string"}
            }
          }
        }
      },
      "required": [
        "output", "counts", "dropped", "elapsed_seconds", "peak_memory_bytes",
        "scratch_bytes", "estimate_error", "next"
      ]
    },
    "error_of": {
      "type": "object",
      "additionalProperties": false,
      "required": ["relative", "absolute"],
      "properties": {
        "relative": {"type": ["number", "null"]},
        "absolute": {"type": "number"}
      }
    },
    "stopped_event": {
      "properties": {
        "event": {"const": "stopped"},
        "stopped": {"$ref": "#/$defs/stopped"}
      },
      "required": ["stopped"]
    },
    "error_event": {
      "properties": {
        "event": {"const": "error"},
        "error": {"$ref": "urn:everygraph:schema:error:1"}
      },
      "required": ["error"]
    },
    "stopped": {
      "type": "object",
      "additionalProperties": false,
      "required": [
        "reason", "during", "fraction", "kept", "dropped", "scratch",
        "partial", "written", "resume", "discard"
      ],
      "properties": {
        "reason": {"enum": ["interrupt", "terminate", "exception"]},
        "during": {"type": "string"},
        "fraction": {"type": ["number", "null"], "minimum": 0, "maximum": 1},
        "kept": {"type": "array", "items": {"type": "string"}},
        "dropped": {"type": "array", "items": {"type": "string"}},
        "scratch": {
          "oneOf": [
            {"type": "null"},
            {
              "type": "object",
              "additionalProperties": false,
              "required": ["path", "bytes"],
              "properties": {
                "path": {"type": "string"},
                "bytes": {"$ref": "#/$defs/count"}
              }
            }
          ]
        },
        "partial": {"type": ["string", "null"]},
        "written": {"type": "array", "items": {"type": "string"}},
        "resume": {
          "type": "object",
          "additionalProperties": false,
          "required": ["python", "command"],
          "properties": {
            "python": {"type": "string"},
            "command": {"type": "string"}
          }
        },
        "discard": {"type": "array", "items": {"type": "string"}}
      }
    }
  }
}
```

A running phase, as `f` receives it:

```json
{
  "event": "phase", "schema": 1, "job": "7c1e04b9a2f35d68",
  "operation": "ingest", "t": 124.2,
  "name": "identifiers", "index": 1, "count": 4, "state": "running",
  "unit": "identifiers", "done": 129600000, "total": 212400000,
  "fraction": 0.61, "pass": null, "rate": 5600000, "eta_seconds": 15,
  "elapsed_seconds": 23, "detail": null, "peak_memory_bytes": 4100000000
}
```

In that event `total` is the sampled estimate of distinct identifiers; the percentage
of G-5 is taken against it and held below 100 until the phase ends.

### 4.7 The file document

- **G-20** With `--json`, an operation that makes a file — `ingest`, `map`, `export` —
  prints on success one **file document** on standard output: the result block (G-11)
  as data. It says what was made (path, kind, size in bytes, and for a shared map what
  it reveals), its exact counts, what was dropped, which phases were kept from an
  earlier run or the graph file, and the time this run took. An operation whose result
  is a table (`run`, `query`, with or without `--out`) prints the result document of
  `docs/api-sketch.md` § 5.3 instead; `--out` is then named in its `stored_in`.

The continued ingest of § 5.4, with `--json`:

```json
{
  "schema": 1,
  "job": "7c1e04b9a2f35d68",
  "operation": "ingest",
  "made": {"path": "transfers.eg", "kind": "graph", "bytes": 9300000000},
  "counts": {"nodes": 212418907, "edges": 1830397917},
  "dropped": {"self_loops": 4201},
  "kept": ["reading"],
  "elapsed_seconds": 109
}
```

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "urn:everygraph:schema:file:1",
  "title": "Everygraph file document, version 1",
  "type": "object",
  "additionalProperties": false,
  "required": [
    "schema", "job", "operation", "made", "counts", "dropped", "kept",
    "elapsed_seconds"
  ],
  "properties": {
    "schema": {"const": 1},
    "job": {"type": "string", "pattern": "^[0-9a-f]{16}$"},
    "operation": {"type": "string"},
    "made": {
      "type": "object",
      "additionalProperties": false,
      "required": ["path", "kind", "bytes"],
      "properties": {
        "path": {"type": "string"},
        "kind": {"enum": ["graph", "map", "file"]},
        "bytes": {"$ref": "#/$defs/count"},
        "reveal": {"enum": ["full", "summary"]}
      }
    },
    "counts": {
      "type": "object",
      "additionalProperties": {"$ref": "#/$defs/count"}
    },
    "dropped": {
      "type": "object",
      "additionalProperties": {"$ref": "#/$defs/count"}
    },
    "kept": {"type": "array", "items": {"type": "string"}},
    "elapsed_seconds": {"type": "number", "minimum": 0}
  },
  "$defs": {
    "count": {"type": "integer", "minimum": 0}
  }
}
```

`made.kind` is `graph` for a `.eg`, `map` for a `.egmap` (with `reveal` when it was
written for sharing), `file` for anything `export` writes in another format. `kept` is
empty when every phase ran; a graph that was up to date and only opened (§ 6.3) has
every phase in it and the time of the opening.

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
| `ingest`, `eg.read` of a file | `reading`, `identifiers`, `building` | the job's scratch directory; the unfinished output as `NAME.eg.partial` beside its destination | #40 |
| `map`, `g.map()` | `communities`, `hierarchy`, `layout` | committed to the graph file as cached results when each phase finishes (#39, by the protocol of C-1): a later run finds them like any cached result | `tiles` writes `NAME.egmap.partial`, removed if interrupted; the phase starts again |
| a whole-graph algorithm (one phase) | — | nothing | The result is stored when the phase ends, or not at all. |
| point queries, `info`, `doctor`, `verify` | — | nothing | They write nothing. |

- **R-3** Only work that will be asked for again in the same form is kept. A phase's
  kept output is valid for any number of threads. Where the layout of a phase's kept
  output depends on the memory budget — the number of parts or buckets it was cut
  into — the manifest records that layout, and a run under another budget either
  reuses it, when the following phases can still work with it inside the new budget,
  or runs that phase again and says so (R-7). The result does not depend on the
  budget either way (`design/DECISIONS.md`, "What 'repeatable' promises").

### 5.2 What stopping leaves behind

Stopping means `Ctrl-C` (SIGINT), SIGTERM sent to the command, a `KeyboardInterrupt` in
Python (including "interrupt the kernel" in a notebook), or an exception from a progress
callback (G-18). These are *clean* stops. SIGKILL, a power cut and a kill by the
operating system are *hard* stops: nothing can be printed, and the guarantees must hold
anyway.

- **C-1** **A graph file is never left corrupt.** At every instant a file named
  `NAME.eg` either does not exist or is complete and passes `everygraph verify`.
  - A new graph is written as `NAME.eg.partial` in the destination directory, made
    durable, then renamed to `NAME.eg` in one step.
  - A result added to an existing graph is appended; the file's table of contents is
    switched to include it only once the result is durable (the protocol of #28). Until
    that switch the file reads exactly as before.
  - The same holds for `NAME.egmap`.
- **C-2** **A stop never changes what an existing graph file already held.** After a
  clean or a hard stop, every section the file had before the operation is unchanged
  and the file is valid. What a finished phase committed as a cached result (the first
  three phases of `map`) stays in it, and the stop block names it on its `written`
  line; nothing else is added. After a hard stop, bytes appended but never referenced
  may remain at the file's end until the next write removes them.
- **C-3** After a clean stop the only things left are the ones the stop block names
  (§ 5.4): cached results committed by finished phases, the job's scratch directory
  holding finished phases only, and possibly `NAME.eg.partial`. The interrupted phase's
  files are removed.
- **C-4** A clean stop is prompt: workers check for cancellation at chunk boundaries, at
  least every 64 MiB of input or 10⁶ items of work (*placeholders*), and the stop block
  is printed within 2 s of the signal on an idle machine (*placeholder*). This is the
  one rule for interrupt checks; in Python the same boundaries are where the engine
  asks Python whether a signal has arrived (C-6).
- **C-5** A second `Ctrl-C` while the first is being handled ends the process at once,
  with no message. It is a hard stop.
- **C-6** In Python the engine installs **no signal handler**. Python's own handler turns
  `Ctrl-C` into `KeyboardInterrupt`; a long call polls Python's signal check at the
  chunk boundaries of C-4 and, when it raises, stops its workers, cleans up, prints the
  stop block and re-raises that same exception — a plain `KeyboardInterrupt`, never a
  wrapped one. An exception raised by a signal handler the program installed itself is
  handled the same way (reason `exception`). SIGTERM is not handled by the library: a
  Python process that receives it with no handler of its own ends as a hard stop. The
  Python process and any graph object in it remain usable after a clean stop.
- **C-7** The command handles SIGINT and SIGTERM itself and exits with status 130 after
  SIGINT and 143 after SIGTERM. With `--json` its standard output is then the stop
  document of § 5.4 (S-2).
- **C-8** A failure is not a stop. When an operation fails (§ 6), its scratch is removed
  unless finished phases can still be used by the same command once the cause is fixed
  (the disk filled up during `building`); the error's last line says which.
- **C-9** On success the scratch directory is removed.

### 5.3 The scratch directory, the manifest and staleness

The scratch directory of a job is `.everygraph-ID` inside the scratch root (§ 1): by
default the directory of the output, so the final rename of C-1 stays on one file
system and nothing of an operation on a file is left in the system temporary
directory. `ID` is the first four
hexadecimal digits of the job's fingerprint (more digits if that name is taken by a
different job, which the manifest's full fingerprint shows). The leading dot hides it
on Unix; on Windows the directory is given the hidden attribute. "The same command
again" finds it by computing the same fingerprint: there is no registry.

- **R-4** The fingerprint is a hash of: the operation; every parameter that changes what
  a phase writes, the input's path included (so not the memory budget, not the thread
  count, not `progress`); and the output path. It does **not** include the identity of
  the input or the versions of the phases' algorithms: those are recorded in the
  manifest and checked against it (R-7), so that a changed input or a newer Everygraph
  finds the old directory, says why it cannot be used, and removes it.
- **R-5** The directory holds `manifest.json`, rewritten atomically at each phase
  boundary, and the kept files it lists.

| Manifest key | Meaning |
|---|---|
| `schema` | `1` |
| `fingerprint` | the full job fingerprint |
| `operation`, `parameters`, `call` | as in the plan; `call` is what the R-10 warning and the stop block quote |
| `engine`, `phase_versions` | the Everygraph version that wrote it; the algorithm version of each phase |
| `input` | `path`, `bytes`, `modified` (nanoseconds), `sample_hash` |
| `output` | the destination path and the `.partial` file, if any |
| `phases` | for each phase: `name`, `state` (`done` or absent), `finished`, `elapsed_seconds`, `detail`, `layout` (what of its output depends on the budget, such as `{"parts": 2048}`, and the memory the following phases need to read it), and `files`: name, bytes and content hash of each kept file |
| `budget_bytes`, `threads`, `deterministic` | what the run that wrote it used; recorded, never compared except through `layout` |
| `started`, `stopped` | timestamps of the first start and of the last stop |
| `owner` | host name and process id of the run that holds the directory, while it runs |

- **R-6** The identity of an input file is its size, its modification time and the hash
  of a fixed sample of its bytes (the first and last 1 MiB — *placeholder*). The whole
  file is not hashed again to decide.
- **R-7** A kept phase is used only if all of these hold, checked in this order;
  otherwise it and every later phase run again:

  | Check | If it fails |
  |---|---|
  | A directory whose manifest has this fingerprint exists | A new run. Nothing is said. |
  | No live process owns the directory | Error: `Another run of this job is in progress.` (exit 1). Nothing is touched. |
  | The input's identity matches the manifest's | The kept work is stale: the directory is removed, a line says so, the run starts from the first phase. |
  | Each phase's algorithm version matches the manifest's | That phase and the later ones run again; a line says so. |
  | Each phase's recorded `layout` can be read by the following phases within the new budget | That phase and the later ones run again; a line says so. |
  | Every kept file exists with the recorded size and hash | That phase and the later ones run again; a line says so. |

- **R-8** A deterministic run that continues produces a result byte for byte equal to an
  uninterrupted deterministic run with the same parameters (the acceptance test of #40),
  whatever the budget and thread count of either. Ingest is always deterministic. A run
  with `deterministic=False` that continues produces a valid result of the same
  operation, not necessarily the same bytes.
- **R-9** Kept scratch is never removed by time. It is removed by a successful run, by a
  run that finds it stale, or by the person, who deletes the directory the stop block
  names.
- **R-10** Scratch that no run will find again is still reported. A long operation that
  is about to write scratch looks in its scratch root for other `.everygraph-*`
  directories with no live owner, and prints the warning of § 6.2 (`old-scratch`) for
  each, at most three (*placeholder*), then `and N more`. It never removes them: the
  command that made one may still be run again. This includes the resumable scratch
  of stopped runs of *other* commands in the same scratch root, on every long run that
  writes scratch there: that is intended — it is the only reminder that the space is
  held — and is not to be filtered out (#98).

### 5.4 The words

**On a clean stop**, after the terminal's own `^C`. Stopping is not an error and gets no
symbol.

```console
  reading      ████████████████  1,830,402,118 rows                 1 min 41 s
  identifiers  █████████░░░░░░░  61% · 5.6 M/s · about 15 s left          23 s
^C
Stopped at your request, during identifiers.

  kept      reading · 31 GB of scratch in .everygraph-7c1e
  dropped   identifiers at 61% — that phase starts again from its beginning
  written   nothing: there is no transfers.eg yet

  continue:  run the same command again
  discard:   delete the directory .everygraph-7c1e
```

| Line | Rule |
|---|---|
| first | `Stopped at your request, during PHASE.` After SIGTERM to the command: `Stopped by a signal, during PHASE.` |
| `kept` | The finished phases; then, for those kept as scratch, the size and place of the scratch. `nothing` when no phase had finished. |
| `dropped` | The interrupted phase and how far it was, and always the words `that phase starts again from its beginning`. |
| `written` | What exists of the output. `nothing: there is no NAME.eg yet`; for an operation on an existing graph `nothing: NAME.eg is unchanged`; when finished phases were committed to the graph file as cached results, their names: `communities, hierarchy kept in NAME.eg; no NAME.egmap yet`. |
| `continue:` | `run the same command again`; in Python `run the same call again`; in a notebook `run the cell again`. |
| `discard:` | What can be deleted to throw the kept work away: `delete the directory .everygraph-7c1e`, and `and NAME.eg.partial` when there is one. Paths are relative to the working directory when they are inside it, absolute otherwise. Left out when no scratch and no `.partial` file was kept; cached results in the graph file are not discarded from here. |

The `discard:` line names what to delete rather than a shell command, so it is right on
every platform (`choices.md` D6); no command of 0.1 lists or removes scratch (§ 10).

A `map` stopped during its third phase keeps the first two in the graph file:

```console
$ everygraph map transfers.eg
  communities  ████████████████  31,960 · modularity 0.603          2 min 11 s
  hierarchy    ████████████████  6 zoom levels                            18 s
  layout       ██████░░░░░░░░░░  40% · about 25 s left                    16 s
^C
Stopped at your request, during layout.

  kept      communities, hierarchy
  dropped   layout at 40% — that phase starts again from its beginning
  written   communities, hierarchy kept in transfers.eg; no transfers.egmap yet

  continue:  run the same command again
```

(Shown from the phase lines onward; the headline and plan block of § 4.4 come before
them.) Running it again shows `communities` and `hierarchy` as `kept` (G-3) and runs
`layout` and `tiles`.

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

**With `--json`**, the block above is still printed on standard error, and standard
output carries the stop document; the exit status is 130 or 143. For the ingest
stopped above (the two strings of `resume` are shortened here with `…`; the real ones
are the whole call and the whole command):

```json
{
  "schema": 1,
  "job": "7c1e04b9a2f35d68",
  "operation": "ingest",
  "stopped": {
    "reason": "interrupt",
    "during": "identifiers",
    "fraction": 0.61,
    "kept": ["reading"],
    "dropped": ["identifiers"],
    "scratch": {"path": ".everygraph-7c1e", "bytes": 31000000000},
    "partial": null,
    "written": [],
    "resume": {
      "python": "eg.read(\"transfers.parquet\", source=\"from\", …)",
      "command": "everygraph ingest transfers.parquet --source from …"
    },
    "discard": [".everygraph-7c1e"]
  }
}
```

Its schema; the stop object itself is the one of the event schema (§ 4.6):

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "urn:everygraph:schema:stop:1",
  "title": "Everygraph stop document, version 1",
  "type": "object",
  "additionalProperties": false,
  "required": ["schema", "job", "operation", "stopped"],
  "properties": {
    "schema": {"const": 1},
    "job": {"type": "string", "pattern": "^[0-9a-f]{16}$"},
    "operation": {"type": "string"},
    "stopped": {"$ref": "urn:everygraph:schema:event:1#/$defs/stopped"}
  }
}
```

A stop is not an error on any surface. In Rust a planned function returns an outcome
that is either its result or `Stopped`, carrying the reason the cancellation handle was
given (`interrupt`, `terminate`, `exception`; `docs/api-sketch.md` § 4); its JSON form is
this document. In Python the reason decides what is raised after the clean stop: a
plain `KeyboardInterrupt` for `interrupt`, and for `exception` the very exception that
the progress callback or the program's own signal handler raised (G-18, C-6).

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

When kept work cannot be used, the line says so instead (one line per case, in the
order of R-7, wrapped at 80 columns), and a full run follows with its summary and plan;
phases that could be kept still show `kept`:

```console
  The file has changed since the run stopped 6 min ago: starting again.
  Everygraph has changed since the run stopped 3 days ago: starting again
  from identifiers.
  The kept reading was laid out for a larger memory budget: starting again
  from reading.
  Part of the scratch from the run stopped 6 min ago is missing: starting
  again from identifiers.
```

The four cases, in that order: the input changed (the directory is removed); a phase's
algorithm changed; a phase's layout does not fit the new budget (R-3); a kept file is
missing or damaged. When the kept work is in the graph file (`map`), there is no
manifest to check: the cached results are found like any other (#39) and their phases
show `kept`.

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

Nothing was changed.
```

- **M-4** In a notebook the error is a card with the explanation first and the traceback
  folded beneath it (`notebook.html`, cell 7 and "The error card with the traceback
  unfolded"); each way forward has a copy button.
- **M-5** With `--json` the error is one document on standard output. This is the only
  definition of an error or a refusal: the plan's `refusal` (§ 3.6), the `error` event
  (§ 4.6) and `to_dict()` of every Python exception are this document. Its key names
  are fixed: `budget_bytes` is the memory budget; `needs_bytes` what the step needs;
  `smallest_budget_bytes` the exact smallest budget that would work (P-14), or null when
  none on this machine would; `available_bytes` only ever free disk (for
  `InsufficientDisk`) or free machine memory; `machine_memory_bytes` the machine's
  memory in all; `holds`, for `OutputExists`, what the file in the way holds (`other
  graph` or `not ours`); `column`, for an `InvalidInput` about a column rather than
  rows, the column's name. Every way forward has the same three keys in every document:
  `command` (what the command line shows under `Ways forward:` — an option alone, a
  whole command, or a phrase, as in M-2), `python` (the same for Python) and `note`
  (its consequence, possibly empty), and may carry `needs_bytes`. In Python
  `ways_forward` is a list of objects with these same fields (`docs/api-sketch.md`
  § 3.10). The `error` key names one of the classes of the table below; that list is
  complete for 0.1.

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "urn:everygraph:schema:error:1",
  "title": "Everygraph error, version 1",
  "type": "object",
  "additionalProperties": false,
  "required": ["schema", "error", "message", "ways_forward", "changed"],
  "properties": {
    "schema": {"const": 1},
    "error": {
      "enum": [
        "InsufficientBudget", "InsufficientDisk", "InvalidInput",
        "InvalidExpression", "UnknownNode", "Unsupported", "OutputExists",
        "CorruptFile", "IoError", "UsageError", "EverygraphError"
      ]
    },
    "message": {"type": "string"},
    "why": {"type": "string"},
    "step": {"type": "string"},
    "needs_bytes": {"$ref": "#/$defs/count"},
    "budget_bytes": {"$ref": "#/$defs/count"},
    "smallest_budget_bytes": {"type": ["integer", "null"], "minimum": 0},
    "available_bytes": {"$ref": "#/$defs/count"},
    "machine_memory_bytes": {"$ref": "#/$defs/count"},
    "arithmetic": {
      "type": "object",
      "required": ["count", "unit", "bytes_each"],
      "properties": {
        "count": {"$ref": "#/$defs/count"},
        "unit": {"type": "string"},
        "bytes_each": {"$ref": "#/$defs/count"}
      }
    },
    "path": {"type": "string"},
    "holds": {"enum": ["other graph", "not ours"]},
    "section": {"type": ["string", "null"]},
    "column": {"type": "string"},
    "node": {"type": "string"},
    "operation": {"type": "string"},
    "reason": {"type": "string"},
    "expression": {"type": "string"},
    "position": {"type": ["integer", "null"], "minimum": 0},
    "problems": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
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
    "invalid_rows": {"$ref": "#/$defs/count"},
    "rows_checked": {"$ref": "#/$defs/count"},
    "ways_forward": {
      "type": "array",
      "items": {"$ref": "#/$defs/way"}
    },
    "changed": {"type": "boolean"},
    "kept": {
      "oneOf": [
        {"type": "null"},
        {
          "type": "object",
          "additionalProperties": false,
          "required": ["phases", "path", "bytes"],
          "properties": {
            "phases": {"type": "array", "items": {"type": "string"}},
            "path": {"type": "string"},
            "bytes": {"$ref": "#/$defs/count"}
          }
        }
      ]
    }
  },
  "allOf": [
    {
      "if": {"properties": {"error": {"const": "InsufficientBudget"}}},
      "then": {
        "required": [
          "step", "needs_bytes", "budget_bytes", "smallest_budget_bytes"
        ]
      }
    },
    {
      "if": {"properties": {"error": {"const": "InsufficientDisk"}}},
      "then": {"required": ["step", "needs_bytes", "available_bytes", "path"]}
    },
    {
      "if": {"properties": {"error": {"const": "InvalidInput"}}},
      "then": {
        "anyOf": [
          {"required": ["problems", "invalid_rows", "rows_checked"]},
          {"required": ["column"]}
        ]
      }
    },
    {
      "if": {"properties": {"error": {"const": "UnknownNode"}}},
      "then": {"required": ["node"]}
    },
    {
      "if": {"properties": {"error": {"const": "OutputExists"}}},
      "then": {"required": ["path", "holds"]}
    },
    {
      "if": {"properties": {"error": {"const": "IoError"}}},
      "then": {"required": ["path"]}
    }
  ],
  "$defs": {
    "count": {"type": "integer", "minimum": 0},
    "way": {
      "type": "object",
      "additionalProperties": false,
      "required": ["command", "python", "note"],
      "properties": {
        "command": {"type": "string"},
        "python": {"type": "string"},
        "note": {"type": "string"},
        "needs_bytes": {"$ref": "#/$defs/count"}
      }
    }
  }
}
```

The refusal of § 3.5 with `--json` is the document shown at the end of § 3.6. The one
where no budget on the machine is enough:

```json
{
  "schema": 1,
  "error": "InsufficientBudget",
  "message": "Not enough memory for this step.",
  "why": "communities needs 62.4 GB for per-node state.",
  "step": "communities",
  "needs_bytes": 62400000000,
  "budget_bytes": 8000000000,
  "smallest_budget_bytes": null,
  "machine_memory_bytes": 61000000000,
  "arithmetic": {"count": 2600000000, "unit": "nodes", "bytes_each": 24},
  "ways_forward": [
    {
      "command": "a machine with at least 63 GB of memory",
      "python": "a machine with at least 63 GB of memory",
      "note": ""
    }
  ],
  "changed": false
}
```

An ingest whose `transfers.eg` already holds a graph built from another input
(`docs/api-sketch.md` § 2.7, "Files that exist"), exit status 1:

```json
{
  "schema": 1,
  "error": "OutputExists",
  "message": "transfers.eg exists and holds another graph.",
  "path": "transfers.eg",
  "holds": "other graph",
  "ways_forward": [
    {
      "command": "--out FILE",
      "python": "eg.read(\"transfers.parquet\", out=\"FILE\", …)",
      "note": "write the graph under another name"
    },
    {
      "command": "--overwrite",
      "python": "eg.read(\"transfers.parquet\", overwrite=True, …)",
      "note": "replace the other graph"
    }
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
| `InvalidInput` | `Cannot read FILE: N lines are not valid rows.` (or the one thing that is wrong); for a weight column that cannot be used, `Cannot use COLUMN as a weight: it has negative values.` (or `NaN values`, `infinite values`) |
| `InvalidExpression` | `Cannot understand the filter.` |
| `UnknownNode` | `ID is not in FILE.` |
| `Unsupported` | `OPERATION cannot be done on FILE.` |
| `OutputExists` | `FILE exists and holds another graph.` or `FILE exists and was not written by Everygraph.` |
| `CorruptFile` | `FILE is damaged.` |
| `IoError` (command line only; in Python the operating system's own `OSError` subclass — `FileNotFoundError`, `PermissionError`, … — with its own text) | `Cannot read FILE: REASON.` or `Cannot write FILE: REASON.`, the reason as the operating system gives it |
| `UsageError` (command line only; in Python the same case is a `TypeError` or `ValueError` with the same text) | the one thing that is wrong: `Unknown option --memroy.`, `The memory budget is more than this machine has.` (P-16) |
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

**Worked example: scratch left by another run** (R-10). Printed, like every warning,
before the plan block (M-9) of a long operation that is about to write scratch in the
same place:

```console
! Another stopped run left 12 GB of scratch here, 3 days ago.

  It is .everygraph-31a0, from: everygraph ingest transfers.parquet --undirected

  Running that command again continues it; to free the space instead,
  delete the directory .everygraph-31a0

  Continuing with this run.
```

**Worked example: a result cut short.** A point query stops at its `limit` (a count
of the nodes or rows it returns; for `path`, of the nodes its search visits) or its
`timeout` (seconds) and returns what it has. The result itself is marked — that is the
rule (`design/preview/choices.md` E1) — and the warning says where it stopped and how
to go further. The point queries that can be cut short are `neighbors`, `khop`, `path`,
`common_neighbors` and `edges_between` (`docs/api-sketch.md` § 2.4).

- **M-12** In Python the warning is issued with `warnings.warn(text,
  eg.TruncatedWarning)`, where `eg.TruncatedWarning` derives from `UserWarning`, and is
  attributed to the caller's line. It is not printed by the progress renderer and does
  not depend on `progress=`. Python's filters therefore apply: by default it is shown
  once per calling line, so a loop of truncated calls prints it once;
  `warnings.simplefilter("ignore", eg.TruncatedWarning)` silences it, `"error"` makes it
  raise. Python's own formatting puts `FILE:LINE: TruncatedWarning: ` before the text,
  which therefore carries no `!` and no "cut short:" prefix of its own. The result's
  text form says it too, in its footer: `cut short at limit=10,000, in hop 2`
  (`docs/api-sketch.md` § 2.5). In a notebook the result's card carries the "Cut
  short." note of `notebook.html` cell 8 — the same sentence and the call to go
  further — and the warning is issued as well, so a loop or a script that never
  displays the result still hears of it.

```text
>>> near = g.khop("0x28c6…1d60", k=2, limit=10_000)
<stdin>:1: TruncatedWarning: khop() stopped at its limit of 10,000 nodes,
part of the way through hop 2. The result is partial and marked
truncated=True.
  to go further: g.khop("0x28c6…1d60", k=2, limit=100_000)
>>> near.truncated
True
```

On the command line the warning has the structure of M-8:

```console
$ everygraph query khop transfers.eg \
      0x28c6c06298d514db089934071355e5743bf21d60 --k 2 --limit 10000 --top 3
node                                        distance
0x21a31ee1afc51d94c2efccaa2092ad1028285549         1
0xdac17f958d2ee523a2206206994597c13d831ec7         1
0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48         1

3 of 10,000 rows shown
! cut short: khop stopped at its limit of 10,000 nodes.

  It was part of the way through hop 2, so the result is partial.

  To go further:
    --limit 100000
```

The table is on standard output; the footer and the warning under it are on standard
error. The warning has no last line of what happens now: the query has finished.

- **M-10** "Cut short" means the answer is partial. It is never used for a table of
  which only some rows are displayed: that is said as `3 of 10,000 rows shown`.
- **M-11** A result that is cut short is still a success: exit status 0, and
  `"truncated"` in the JSON document (`docs/api-sketch.md` § 2.5).

Stable codes of the warnings of an operation in 0.1 (the `code` of the `warning` event):
`slow-gzip`, `read-only` (a result could not be stored in the graph file and is kept
for the session only) and `old-scratch` (R-10). A result cut short is not one of them:
point queries take no `progress=`, so they have no event stream; the mark is on the
result (`truncated`) and the warning is Python's (M-12). A direction
that was guessed is not a warning: the `edges` line of the summary states it (§ 3.4),
and the summary is always printed (P-3).

### 6.3 Notices

Plain sentences with no symbol, each fixed:

| When | Text |
|---|---|
| Clean stop | `Stopped at your request, during PHASE.` and its block (§ 5.4) |
| Continuing | `Continuing the run stopped AGE ago: the file has not changed since.` |
| Kept work cannot be used | the four lines of § 5.4, "When kept work cannot be used" |
| Dry run | `Dry run: nothing was written.` |
| A file was up to date | `transfers.eg is up to date with transfers.parquet: opened, not rebuilt.` |
| A result came from the file | `full result: transfers.eg (cached) · export with --out pagerank.parquet` (dim, under a table on a terminal) |
| Serving the map | `Serving on http://127.0.0.1:7461 (this computer only) — opening your browser.` then `Ctrl-C to stop.` (dim) |

### 6.4 Exit status

| Status | Meaning |
|---|---|
| 0 | Done. Includes a result cut short, and a dry run whose plan fits. |
| 1 | Failed for a reason not listed below: an operating-system error reading or writing a file (`IoError`), an output file that may not be replaced (`OutputExists`), another run of the same job in progress or an internal error (`EverygraphError` with no more specific class). |
| 2 | Usage: the command line could not be understood or cannot be honoured — an unknown command or option, a missing argument such as `--reveal` with `map --out` (`UsageError`), a budget larger than the machine's memory, a filter that does not parse (`InvalidExpression`), an operation the graph does not support (`Unsupported`). |
| 3 | The input is not valid (`InvalidInput`): rows that cannot be read, or a weight column with negative, NaN or infinite values where an operation needs weights. |
| 4 | Refused before starting for lack of memory or disk (`InsufficientBudget`, `InsufficientDisk`), or the disk fills during a run (`InsufficientDisk`). Also a dry run whose plan does not fit. |
| 5 | A graph or map file is damaged (`CorruptFile`), or `everygraph verify` found a problem. |
| 6 | No such node: an identifier that is not in the graph or the view (`UnknownNode`; `design/DECISIONS.md`, "Exit status for 'no such node'"). |
| 130 | Stopped by `Ctrl-C` (SIGINT). |
| 143 | Stopped by SIGTERM. |

With `--json`, every one of these outcomes also puts exactly one document on standard
output (S-2): the result document or the file document, the plan, the error document,
or the stop document.

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
- **T-8** One word, one meaning: "budget" is memory; "limit" is a count of nodes or
  rows a query returns, never memory or time; "timeout" is seconds; "scratch" is temporary disk; "kept" is finished work that will
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
- a result cut short is shown on the result's own card by the approved "Cut short."
  note of `notebook.html` cell 8, and Python's warning is issued as well (M-12), so a
  loop that never displays a result still hears of it;
- a display with no JavaScript (a notebook rendered as a static page, a log) falls back
  to the piped text form.

---

## 8. Test hooks

**H-0, the precondition.** All of this text is produced in one place by a renderer that
is a pure function: `render(events, surface, rendering, width) → text`, with no clock and
no probe of its own. The planner takes the machine and the clock as inputs. Every
message string lives in one catalogue. Without this the tests below are not possible;
with it they need no real graph.

**No test depends on wall time measured under load.** Tests in the suite use a fake
clock or count work; figures in seconds are measured only by the benchmark harness (#4)
on an idle machine (`CLAUDE.md` § 3).

| Rules | Mechanical test | Owner |
|---|---|---|
| S-1, S-2, G-20 | Run each command with standard output and standard error captured separately. With `--json`, standard output parses as exactly one JSON document in every outcome — a result (the file document for `ingest`, `map` and `export`, the result document for `run` and `query`), a plan, an error (a usage error included), and a stop: SIGINT and SIGTERM sent during a run give the stop document, valid against its schema, with exit status 130 and 143. Without `--json` and piped, standard output is the result in its piped form — a tab-separated table with a header, the lines of a result's summary or of `info`, a dry run's plan — or empty. Standard error never contains a result row; the summary of an input is on standard error and the summary of communities on standard output. | #98 |
| S-3, M-12 | A Python test with `progress=False` asserts nothing is written to either stream; with `progress=f`, that `f` received events and nothing was written. A `khop` stopped at its limit, inside `warnings.catch_warnings(record=True)`, records exactly one `eg.TruncatedWarning` attributed to the calling line, and the result's text form ends with the `cut short at limit=` footer; with `simplefilter("ignore", eg.TruncatedWarning)` nothing is written; with `"error"` it raises. In the widget's tests (#76) the result's card carries the "Cut short." note. | #81 |
| S-4, P-4 | Every command runs to completion with standard input closed. The message catalogue contains no question mark at the end of a line. | #98 |
| S-6 – S-9, W-1 – W-6, G-14 | **Golden transcripts.** Recorded event streams (one JSON object per line) are rendered for each surface in four renderings (`colour`, `plain`, `ascii`, `piped`) at widths 80 and 60 and compared with stored files, colour instructions included. A checker asserts for every golden file: no line over 80 columns; no redrawn line over 78; the `ascii` and `piped` files are pure ASCII; the `plain` file has no escape sequence; the `colour` file uses only the six instructions of S-7. | #98, #81 |
| §§ 2.5, 2.6, N-1, N-2 | Unit tests of the formatters on a table of inputs and expected strings (every example in § 2.6 is a row). A property test: a value with basis `sampled` or `model` never renders without `~` or "about". | #98 |
| P-1, P-5, P-16 – P-18 | With the scratch and output directories on a file system that records writes, a refused operation performs none. Fault injection: each of the three checks of P-5 fails in turn and the matching refusal is produced. With a fake probe: a budget above the machine's memory is a usage error with exit status 2 before any plan; a budget above the free memory runs and its plan has the second `budget` line; a scratch root reported as `tmpfs` adds the scratch's upper bound to the memory check and, when that fails, the refusal's ways forward start with `--temp-dir`; a smallest budget above the machine's memory gives a refusal with no `--memory` way and `smallest_budget_bytes` null. | #40, #45 |
| P-2 | Planning an ingest reads no more than the sample: counted through the I/O layer's read counters (#29). | #40 |
| P-3 | With a fake cost model, an operation estimated just under and just over `T_PLAN` prints no plan block and no progress, respectively both. An ingest of a ten-line file prints its headline and summary. | #45, #40 |
| P-6 – P-8, P-10 – P-12 | Golden transcripts of the plan for: ingest of a Parquet file, of a text file (sampled row count), an operation on a graph, an `explain`, a plan with a kept phase. A unit test: the printed ratio is `working_bytes / budget.memory_bytes` rounded to one decimal. | #40, #45 |
| § 3.6, P-13 – P-15, M-5, G-15 – G-17 | **Schemas.** The plan, error, event, stop and file schemas, and the result schema of `docs/api-sketch.md` § 5.3, are kept as files beside the tests, identical to the ones in the two documents. Every plan, error, event, stop, file and result document produced in the test suite is validated against them, closed as they are; the examples in the two documents are validated by `docs/tools/check_schemas.py`. Every error class the code can raise or the command can report is in the enumeration of the error schema. A sequence check on recorded streams: `plan` first, exactly one final event, one final `phase` event per phase. A test that no refusal for `communities` offers anything but `--memory` or a phrase. | #45, #81, #98 |
| P-7, P-9 | **Estimates against actuals.** In the suite: the ingest report, each result's provenance and the `result` event record `estimate` and `actual` for time, peak memory, scratch and output size, and a table-driven test checks the relative error, including `actual` = 0 (P-9). With the harness (#4), on an idle machine: the coverage of `[low, high]` is measured per quantity over the benchmark corpus and stored with the run, so the model can be tuned; the ±30 % bound is asserted only where #40 asks for it, for ingest on the tier-2 datasets. | #40, #45, #4 |
| P-11 | With a fake probe reporting a given available memory, container limit and container usage, the budget is half of what is free by the definition of P-11. | #45 |
| G-1, G-2 | Every phase name in the registry matches `^[a-z]{1,11}$`; every operation's emitted phases equal its plan's. | #45 |
| G-5 – G-9 | Property tests on recorded streams: percentages never decrease and never reach 100 before the final event; no `eta_seconds` before the thresholds of G-7; none when `total` is null; a final event's detail contains no `~`. | #81 |
| G-10 | A fake clock drives 1,000 updates in one second; the renderer emits at most 4 redraws, and one when a second passes with no update. | #98 |
| G-18, G-19, C-6 | A callback that raises on its third call: the exception propagates, a `stopped` event with reason `exception` precedes it, and the conditions of C-2 and C-3 hold. `signal.getsignal` returns the same handlers before, during and after a long call. `_thread.interrupt_main()` during a call yields the stop block and a plain `KeyboardInterrupt`, after which the graph object still answers a query. Every event's `job` is the first 16 digits of the fingerprint the manifest records, the same in every event of the run and in a second run of the same job. | #81 |
| R-1 – R-3, R-7, R-8, C-1 – C-3 | **Interruption tests.** For `ingest` (#40) and for `map` (#98, with #39 for the cached results), at each phase: send SIGINT at a random moment inside the phase; assert exit status 130, the stop block (compared with numbers masked), `everygraph verify` on any `.eg` present, and that the directory listing and the graph file's table of contents equal what the block named — for `map`, every section the file had before is unchanged and the finished phases are committed. Run the same command again with a different `--threads`; assert the `kept` lines and that the output of a deterministic run equals, byte for byte, the output of an uninterrupted one. Run it again under a smaller `--memory` than a kept layout needs; assert the R-7 line and that the phase runs again. | #40, #98 |
| C-1, C-2, R-8 | **Hard stops.** `kill -9` at ten random points of an ingest (#40) and of a `map` (#98); every following run completes, a deterministic result is byte-identical to an uninterrupted run, and a graph file that existed before still has every section it had, unchanged, and passes `verify`. Crash injection between every write step of the container (#28). | #40, #98, #28 |
| C-4, C-5, C-7 | An instrumented run counts the work done between two cancellation checks and asserts it never exceeds the bounds of C-4; two SIGINTs 10 ms apart leave the conditions of C-1 true; exit statuses are 130 and 143. The 2 s bound is measured by the harness on the tier-1 datasets on an idle machine, not asserted in the suite. | #98, #4 |
| R-4 – R-7, R-10 | Unit tests of the fingerprint: changing `--memory`, `--threads`, `progress`, the input's bytes or a phase's algorithm version does not change it; changing any other parameter, the input's path or the output path does. Table-driven tests of R-7, one per row, each asserting the line printed and, for a changed input, that the old directory is gone. A scratch root holding a directory of another fingerprint with no live owner — another command's stopped job included — gives the `old-scratch` warning and leaves the directory in place. | #40 |
| M-1 – M-4, M-6 – M-10, § 6.3 | Golden transcripts of every worked example in § 6, on each surface. The corpus of malformed inputs (#32) produces `file:line` locations that are checked against the corpus's own index. | #32, #98, #81 |
| § 6.4, M-11 | A table-driven test: each exception class raised inside the command, and each usage error, yields its exit status. A query stopped at its limit exits with 0 and has a non-null `"truncated"` in its `--json` document. | #98 |
| G-3, G-4, G-11 – G-13 | Golden transcripts of: a run that continues (a `kept` phase); `map` when communities exist already; a result block with and without `dropped:`, with `duplicates` under `"collapse"` and with `merged:` under `"sum"`, whose counts add up to the rows read; a graph kept in memory; an operation that returns a table (no result block). Every `✓` line ends with a duration. In the `colour` files a bar contains no instruction other than faint. A check on the registry of hints: no operation has more than two. | #98, #81 |
| C-8, C-9, R-9 | **Clean-up tests.** After a successful run the scratch root holds no `.everygraph-*` directory. With a failure injected in each phase in turn (invalid row, disk full), the scratch is gone, or holds exactly the finished phases when the error's last line says they were kept. A scratch directory with an old modification time is still there after an unrelated run. | #40 |
| S-5 | The same command under two locales (`LANG=C.UTF-8`, `LANG=it_IT.UTF-8`) prints identical bytes. | #98 |
| S-10 | A long command run in a pseudo-terminal with `TERM=dumb`, and again with `--progress=plain`, writes progress on standard error with no carriage return, no cursor movement and no bar: one line per finished phase, as in the piped rendering. | #98 |
| T-7, T-8 | A vocabulary lint over the catalogue and over the public names: "budget" never appears in an entry about nodes or seconds, "limit" never in one about memory, "groups" never appears; a user's noun reaches a message only through the one function that derives it; no public name contains one. | #98, #81 |
| T-1, T-6 | Not mechanical. Every new or changed catalogue entry appears in a golden file, so it is visible in the diff and is read against these two rules in review. | reviewer |
| T-2 – T-5 | A lint over the message catalogue: forbidden words, exclamation marks, a full stop at the end of each sentence, every error entry has at least one way forward. | #98 |
| this document | `python3 docs/tools/check_transcripts.py docs/ux-contract.md`: every code block within 80 columns; the ASCII rendering of every transcript pure ASCII and within 80; redrawn lines within 78; the JSON blocks parse; every numbered rule is cited in this table; and each ASCII, piped and narrow block of § 4.5 equals the rendering the checker computes from the Unicode blocks it is marked as derived from. `docs/tools/check_schemas.py docs/ux-contract.md` (and `docs/api-sketch.md`): the five schemas of this document and the result schema of the sketch are valid, the cross-references between them resolve, every example validates, and negative controls (a missing key, an unknown error class, a misspelt key, a `way` without `python`, an `OutputExists` without `holds`, a file document with a misspelt key, a result document with a misspelt key) are rejected. | #13 |

---

## 9. Decided here

Choices this document had to make because the previews were silent or disagreed, and
the places where it departs from their wording. Each is one behaviour, with one line of
reasoning. This list is part of `design/DECISIONS.md` by reference (the row "Choices
made while writing the two M0 specifications"); none changes an answer recorded there,
and items 14 – 17 follow rows that settle a question this document had raised.

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
   long; reading a file always prints its summary.** `cli.md` shows `everygraph map`
   going straight to its phases, but the decision is that long operations print their
   plan. `T_PLAN` = 5 s is a placeholder. *The rule has no exception to remember, and
   `choices.md` A4 needs the summary line to say when a direction was guessed.*
7. **Standard output is for results, standard error for everything else** (§ 2.1), and
   `--json` puts exactly one document on standard output in every outcome: result,
   plan, error (a usage error included, as `UsageError`), or the stop document after
   `Ctrl-C` or SIGTERM. The previews did not say. *It is what makes
   `everygraph run … | other-tool` and `> file` safe, and a script never has to guess
   whether to parse.*
8. **Exit statuses 0 – 6, 130, 143** (§ 6.4), shared with `docs/api-sketch.md`; 6 is "no
   such node" (`design/DECISIONS.md`). #98 asks for "meaningful exit codes" without
   listing them. *Refusal (4), invalid input (3) and a missing node (6) are the ones a
   script must tell apart from a crash or a typo.*
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
13. **Names of what is left on disk**: the scratch directory `.everygraph-ID`,
    `manifest.json` inside it, and `NAME.eg.partial` beside the destination.
    *A person will see these in a listing; they should explain themselves.*
14. **Scratch goes beside the output, not in `/tmp`** (`design/DECISIONS.md`, "Where
    scratch goes"). Departs from `cli.md` and `terminal.html`, which show `61 GB scratch
    in /tmp` and `/tmp/everygraph-7c1e`: the directory is `.everygraph-7c1e` beside the
    output, and the `estimate` line names the scratch root only when it is not the
    default (`61 GB scratch`, the form `api.md` already printed). A memory-backed
    scratch root counts against the budget (P-18). *`/tmp` is memory on many systems,
    and the output's file system is where the final rename is atomic.*
15. **"Communities", never "groups"** (`design/DECISIONS.md`, "The word for what
    community detection finds"). Departs from `terminal.html` and `notebook.html`: the
    `map` phase line reads `31,960 · modularity 0.603` (the noun dropped, because
    `31,960 communities · modularity 0.603` is 36 columns and the detail has 32); the
    `explain` steps read `move nodes between communities` and `merge communities and
    repeat`, with the step column widened to fit; the `communities` line of
    `everygraph info` reads `31,960 · 2 min 11 s · 2026-11-24`, because with
    "deterministic" in its label (`docs/api-sketch.md` § 9) `cli.md`'s
    `31,960 communities · 2 min 11 s · 2026-11-24` makes the line 85 columns, and the
    decision drops the noun where a line has no room. *Golden files freeze whatever is
    merged.*
16. **No `--precision low` way forward for communities** (`design/DECISIONS.md`, "A
    low-memory variant of community detection"). Departs from `cli.md` § 2,
    `api.md` § 6, `terminal.html` and `notebook.html` cell 7, whose refusals offered it:
    the refusal offers `--memory 6GB` only. *Nothing in the plan builds the variant
    (#57 needs f64 weights), and P-15 forbids offering what does not exist.*
17. **Every `✓` line carries the total time** (G-11). Departs from `cli.md` and
    `terminal.html`, whose `map` result line was `✓ transfers.egmap · 4.1 GB`; it is
    now `✓ transfers.egmap · 4.1 GB · 4 min 12 s`. *One rule for every result line, and
    the total is the one figure the phase lines do not give.*
18. **`discard:` names what to delete instead of `rm -r`.** Departs from `terminal.html`
    (`discard: rm -r /tmp/everygraph-7c1e`): the line reads `delete the directory
    .everygraph-7c1e`. *`rm -r` is wrong on Windows (`choices.md` D6), and no issue
    builds a clean-up command to name instead.*
19. **In Python a result cut short is also a `warnings` warning** (`eg.TruncatedWarning`,
    M-12). At a prompt this departs from `api.md`, which printed
    `! cut short: khop() stopped …`: Python's own formatting shows
    `FILE:LINE: TruncatedWarning: khop() stopped …`. In a notebook the approved
    "Cut short." note of `notebook.html` cell 8 stays on the result's card, and the
    warning is issued as well. The command line keeps the approved form, laid out as
    every warning is (M-8). *Point queries take no `progress=`, so only Python's
    filters can silence or deduplicate a warning in a loop, and a script that never
    displays the result must still hear of it.*
20. **A finished phase of `map` is committed to the graph file.** C-2 used to promise a
    byte-for-byte unchanged file after any stop; it now promises that what the file held
    before is unchanged and valid, and the stop block names the cached results a stop
    leaves (`written   communities, hierarchy kept in transfers.eg; …`). *Throwing away
    two minutes of communities because the tiles were interrupted would contradict
    R-1.*
21. **Budgets the machine cannot give** (P-16, P-17). A budget above the machine's
    memory in all is a usage error before any work; one above the free memory is
    honoured, with a second `budget` line; a refusal whose smallest budget is above the
    machine's memory says so and offers no `--memory`. The previews were silent. *A
    budget is a promise the engine keeps, so it must not accept one it cannot.*
22. **What the plan's numbers mean.** `regime.ratio` is the working data over the budget
    (P-6); `[low, high]` is an interval with a target coverage of 90 % on an idle
    machine (P-7, *placeholder*), not a guarantee; the relative error is defined when
    the actual is 0 (P-9). The previews printed the figures without saying. *A number
    that a test or a user will compare against must have a definition.*
23. **`job` names the work, not the run** (G-19): it is the fingerprint's first 16
    digits, so a continued run has the same `job` as the one that stopped. *It lets a
    log reader join a stop and its continuation without a registry.*
24. **The map's `share it:` hint is the export command**, `everygraph map
    transfers.eg --out shared.egmap --reveal summary` (`m.export("shared.egmap",
    reveal="summary")` in Python), with `--reveal full` named as the other value in
    the text. Departs from `cli.md` § 1 and `terminal.html`, whose hint was
    `upload transfers.egmap and viewer.html to any static host`. *The working map is
    full (`design/DECISIONS.md`, "What a shared map reveals"): uploading it would
    share every identifier; the old hint was also not runnable (G-13).*
25. **Every error class 0.1 can produce is in the closed enumeration** (§ 6.1):
    `OutputExists` (exit status 1, with `path` and `holds`) and `IoError`, the
    command line's class for an error of the operating system (exit status 1, with
    `path`; Python keeps the operating system's own `OSError` subclasses), join the
    others. A negative, NaN or infinite weight is `InvalidInput` (exit status 3): it
    is a property of the data. *Adding a value to an enumeration later would change
    the schema's version.*
26. **`--json` of an operation that makes a file** is the file document of § 4.7, and
    a result's summary (the summary of communities) is the result, on standard
    output, while the summary of an input stays on standard error (§ 2.1). A dry
    run's plan, its closing notice included, is on standard output. *The previews
    did not say; one document per outcome needs a document for this outcome too.*
27. **Where scratch goes when there is no output file**: beside the graph file; for a
    graph held in memory, in the system temporary directory, where P-18 applies.
    *There is no file of the user's to put it beside, and the memory-backed case is
    already counted against the budget.*
28. **The result block's words for repeated rows** (G-12): `dropped: … · N
    duplicates` under `duplicates="collapse"` and a `merged: N duplicates, summed
    into their pairs` line under `"sum"`, keys `dropped.duplicates` and
    `counts.merged` of `docs/spec/graph-semantics.md` § 2.8 (its § 9, item 9). The
    previews show only `dropped: N self-loops`. *Merged rows are not lost — their
    weights and times are in the summed edge — so they are not called dropped; and
    every row read is accounted for on the screen, as the report's identity says.*

## 10. Open points and obligations for other issues

**Obligations.** § 8 gives test hooks and duties to issues whose files do not yet
contain them. This pull request does not edit `plan/issues/`; the coordinator amends
each file with the sentence given here, then republishes.

| Issue | Obligation |
|---|---|
| #40 (`m2-13-preflight-resume.md`) | Ingest's preflight and resume as § 3 and § 5: scratch in `.everygraph-ID` beside the output (P-5, P-18); the fingerprint of R-4 with input identity, phase versions and budget-dependent layout checked against the manifest (R-3, R-7); the `old-scratch` warning (R-10); and the § 8 tests it owns (interruption at each phase with a resume under another `--threads` and a smaller `--memory`, `kill -9`, fingerprint and R-7 table tests, clean-up tests). |
| #40 (`m2-13-preflight-resume.md`), acceptance | The ±30 % bound applies to wall time, peak memory, scratch and output size of an ingest on the tier-2 datasets, measured by the benchmark harness on an idle machine (P-7, P-9); no test in the suite asserts a wall time. |
| #45 (`m3-02-planner.md`) | Plans and refusals as § 3: documents valid against the closed plan, error and event schemas; P-15 (only ways forward that exist), P-16 (a budget above the machine is a usage error), P-17 (above free memory is honoured, with a line), P-18 (memory-backed scratch counts against the budget); estimate and actual in each result's provenance (P-9). |
| #98 (`m8-01-cli.md`) | No `--yes` (S-4); `--json` gives exactly one document on standard output in every outcome, a stop included (S-2, C-7), and the file document for `ingest`, `map` and `export` (G-20); exit statuses of § 6.4, with `IoError` and `OutputExists` at 1; a result's summary on standard output, a dry run's plan wholly on standard output (§ 2.1); the § 8 tests it owns: interruption and `kill -9` of `everygraph map`, the C-4 work-between-checks test, the golden transcripts. |
| #81 (`m6-01-python-api.md`) | No signal handler installed; long calls poll Python's signal check so `Ctrl-C` arrives as a plain `KeyboardInterrupt` after a clean stop (C-6); a result cut short is reported with `warnings.warn(…, eg.TruncatedWarning)` (M-12); events valid against the event schema (§ 4.6). |
| #39 (`m2-12-artifact-cache.md`) | The `communities`, `hierarchy` and `layout` phases of `map` are committed as cached results as each finishes, so a stopped `map` keeps them and a later run shows them as `kept` (§ 5.1, C-2). |
| #28 (`m2-01-container-spec.md`) | Appending a result never changes a section the file already held; checked with #98 by the `kill -9` test of `map` (C-2, § 8). |
| #69 (`m5-03-tile-builder.md`) | The pyramid is written as `NAME.egmap.partial` and renamed when complete; a stop removes the partial file (§ 5.1, C-1, C-3). |
| #4 (`m0-03-bench-harness.md`) | Records the estimates and actuals of P-9 for every run, reports per quantity the share of runs whose actual lies in `[low, high]`, and measures the 2 s stop bound of C-4 on the tier-1 datasets. |
| #36 (`m2-09-graph-semantics.md`) | The plan reports the treatment chosen with the values of § 3.6: `self_loops` `dropped` or `kept`; `duplicates` `kept as events`, `collapsed` or `summed` (options `"events"`, `"collapse"`, `"sum"`). |
| #32 (`m2-05-text-reader.md`) | The malformed-input corpus carries an index of the `file:line` of every bad row, against which the messages of M-6 are checked. |
| #76 (`m5-10-jupyter-widget.md`) | The cards of § 7, driven only by the events of § 4.6; interrupting the kernel ends the card in the stop block. |

**Open points.** Found while writing; none blocks this contract.

| Point | Where it shows | Suggested owner |
|---|---|---|
| `terminal.html` still describes seven colour instructions and keeps blue "until that is decided"; `SPEC.md` has decided. It also still shows "a fifth of free RAM", "groups", `--precision low`, `/tmp/everygraph-7c1e`, `rm -r`, and a `map` result line without its time (§ 9, items 2, 14 – 18). | `design/preview/style/terminal.html` | #121 |
| `notebook.html` uses `budget=10_000` for `khop` (now `limit=`), "start over" on interruption, bullets in the exception text, "cut short" for display, "groups" in the step table and `precision="low"` in the error card. | `design/preview/style/notebook.html` | #121, #76 |
| The option to skip invalid rows and count them (#32) has no approved name. If it is added (`--skip-invalid` is the obvious one), it becomes a third way forward in the invalid-input error and a `dropped:` kind in the result block. | § 6.1 | #32, then a preview |
| A phase that lasts hours prints nothing in a job log until it ends (the approved piped form has no heartbeat). | § 4.5 | #98 |
| No command lists or removes scratch left by stopped runs; the stop block's `discard:` line and the `old-scratch` warning (R-10) are the only pointers. `everygraph doctor` could list them. | § 5.3 | #98 |
