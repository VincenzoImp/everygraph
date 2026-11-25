# Benchmarks

Everything Everygraph says about speed and memory is measured here, with
`everygraph-bench` (crate `crates/everygraph-bench`), on an idle machine
(CLAUDE.md § 3). This file is the measurement protocol and the reference for the spec
and result formats.

| Path | What | In git |
|---|---|---|
| `specs/*.toml` | Benchmark specs: what to run, on what, under which limits | yes |
| `results/<spec>/<timestamp>-<revision>.jsonl` | Rows of **official** runs | yes: every number quoted in a document cites one of these files |
| `exploratory/<spec>/…` | Rows of exploratory runs | no (ignored) |
| `work/` | Default work directory: builds, decompressed inputs, cold-cache copies, logs | no (ignored) |
| `runners/` | Comparison runners for other tools: a `uv`-locked Python project | yes (not `.venv/`) |

## Running

```sh
everygraph-bench check                       # every spec parses; list the cases
everygraph-bench check headline --commands   # the expanded commands, for this machine
everygraph-bench run headline --only kernels-orkut --repetitions 1   # exploratory
everygraph-bench run headline --official     # an official run (see below)
everygraph-bench report bench/results/headline/*.jsonl --json chart.json
everygraph-bench report --compare A.jsonl B.jsonl --max-diff 0.05
```

`--data` (or `$EVERYGRAPH_DATA`) is the data root; on the lab machine
`/home/tux/graphlab/data`. `--work` (or `$EVERYGRAPH_BENCH_WORK`) moves the work
directory, which must be on a real disk, not tmpfs: the cold-cache protocol cannot
evict tmpfs pages, and says so in the rows (`not_cold`). Progress goes to standard
error, the report of the run to standard output. Exit status: 0, 1 (failure, or a row
that did not complete), 2 (usage), 3 (an official run refused to start).

## Protocol

**Official and exploratory runs.** A run is exploratory unless `--official` is given.
An official run:

1. refuses a working tree with uncommitted changes to tracked files (`--allow-dirty`
   overrides it and every row then says `dirty`);
2. takes **the** lock, `/run/user/<uid>/everygraph-bench-official.lock`, without
   waiting. It is the only lock in the harness and is taken in one place
   (`harness::run::run`). A second official run, from another shell or nested in a
   benchmark command, fails at once (exit 3) and names the holder — it never queues,
   because a nested lock once deadlocked a lab run for half an hour. The lock is an
   `flock`: the kernel releases it however the holder ends;
3. refuses to start when the 1-minute load average is above 1.0 (`[gate] max_load`,
   `--max-load`) or `MemAvailable` is below what the run needs (its largest memory
   limit, or `[gate] min_mem_available`);
4. writes its rows to `results/`, to be committed.

Exploratory runs skip all four and go to `exploratory/`. Their rows record the same
readings; their report says not to quote them.

**Every repetition.** Before each repetition the harness empties the case's work
directory, writes back dirty pages (`sync`: an ingest leaves gigabytes whose write-back
would otherwise run during the next repetition), and sleeps through a settle window
(1 s, `[defaults] settle`) while it counts the cores other processes keep busy
(`/proc/stat`). The command then runs as

```
systemd-run --user --scope -p MemoryMax=<limit> -p MemorySwapMax=0 -- \
    everygraph-bench measure --report … -- <command>
```

**Every run is under a hard memory limit**: a spec whose benchmark has no
`memory_limit` (its own or `[defaults]`) is an error, and the limit the kernel applied
(`memory.max`, `memory.swap.max`) is recorded next to the one asked for. `measure` is
the first process of the scope; it starts the command, samples `memory.stat` every
20 ms, and when the command ends reads, from inside the still-living cgroup,
`memory.peak` (exact peak of anonymous + file-backed + kernel memory), `memory.events`
(limit hits, OOM kills), `cpu.stat` (user and system time of everything in the scope),
and its own `/proc/self/io` (bytes the command and its descendants read from and wrote
to storage). Anonymous and file-backed peaks are kept apart because a memory-mapped
graph counts as RSS but is reclaimable page cache (FINDINGS § B1): RSS alone misleads.
They are sampled, so a spike shorter than the interval can be missed; `memory.peak` is
not sampled. While the command runs, `/proc/stat` minus the scope's own CPU time gives
the cores used by others (for runs of 1 s and more; shorter runs rely on the settle
window).

**Disturbed rows.** In an official run, a repetition during which, or just before
which, other processes kept more than 0.5 cores busy on average (`[gate]
disturbed_above_cores`) is marked `disturbed`: kept in the results, left out of every
summary, and listed at the end of the run. One busy core skewed lab timings by 10–40%.

**Warm and cold cache.** A warm case runs once untimed first (`warmup`, default 1). A
cold case (`cache = "cold"`) reads a private copy of its dataset, made once under
`work/copies/` by a read/write loop (not `copy_file_range`, which may share blocks on
a reflinking file system) and reused while the source is unchanged. Before each
repetition the copy is synced and evicted with `posix_fadvise(POSIX_FADV_DONTNEED)`;
then `mincore(2)`, through util-linux `fincore`, must find at most 1% of it resident,
or the repetition is not run and its row says `not_cold`. Private, because pages that
another process maps cannot be evicted. There are at least two copies
(`cold_copies`), and consecutive cold repetitions of a dataset — across cases too —
take them in turn: reading the same blocks twice in a row measures the drive's own
cache (FINDINGS § F1). There is no `sudo` and no `drop_caches`.

**Stop latency (ux-contract C-4).** `interrupt = { after = "1s" }` sends SIGINT (or
`TERM`) to every process of the run's scope after the delay — what a terminal's Ctrl-C
does — and records the time to the exit as `stop_seconds`. A reference with
`max = 2.0` makes the report check the bound.

**Estimates and actuals (ux-contract P-7, P-9).** A benchmark with `plan = {}` reads
the `estimate` object (`seconds`, `peak_memory_bytes`, `scratch_bytes`,
`output_bytes`, each `{value, low, high, basis}`) from the command's last JSON line,
records the actual of each quantity (by default: wall time, `memory.peak`, and metrics
named `scratch_bytes` and `output_bytes`), and the P-9 error of each: relative
`(actual − value) / actual` (`0` when both are 0, `null` when only the actual is),
absolute `value − actual`, and whether the actual lies in `[low, high]`. The report
gives, per quantity, the share of rows inside the interval; wall time counts only on
official, undisturbed rows. No command produces a plan yet; the report is tested on
synthetic rows.

## Spec format

A spec is `specs/<name>.toml`; `name` must equal the file name. See `headline.toml`
for a full example.

```toml
schema = 1
name = "demo"
description = "what it measures"
reference = "where the numbers it reproduces are published"   # optional

[defaults]                 # inherited by every benchmark
memory_limit = "8G"        # required here or in each benchmark; systemd syntax, 1024-based
repetitions = 3
warmup = 1                 # default 1 for warm cache, 0 for cold
cache = "warm"             # or "cold"
cold_copies = 2
threads = [32]             # each count makes a case
timeout = "10m"
settle = "1s"
sample_interval = "20ms"
clean_work = true

[gate]                     # official runs
max_load = 1.0
disturbed_above_cores = 0.5
min_mem_available = "16G"  # default: the largest memory limit selected

[vars]                     # placeholders for every command
glab = "{repo}/lab/b1-core/target/release/glab"

[[prepare]]                # untimed, before the benchmarks, under a memory limit
name = "build"
cwd = "{repo}/lab/b1-core"
command = ["cargo", "build", "--release", "--offline"]
creates = "{work}/x"       # skipped when it exists
needed_by = ["k"]          # skipped unless a selected case belongs to these benchmarks

[[benchmark]]
name = "k"
group = "kernels"          # one report table per group
dataset_path = "csr/orkut" # under the data root, or absolute; or dataset = "<registry name>"
matrix = { algo = ["pr32", "wcc"] }          # cartesian product, with threads
exclude = [{ algo = "wcc", threads = 1 }]
memory_limit = "{algo}"    # limits and dataset names may use matrix values
command = ["{glab}", "run", "--graph", "{dataset}", "--algo", "{algo}", "--threads", "{threads}"]
# or: shell = "…"  (run with sh -c)
cwd = "{case_work}"        # the default
env = { OMP_NUM_THREADS = "{threads}" }
conditions = { graph = "orkut", precision = "f32" }   # printed with every number
metric = [
  { name = "seconds", json = "/seconds_best", unit = "s" },                # JSON pointer, last JSON line of stdout
  { name = "s_per_iter", json = "/iter_seconds", reduce = "median", unit = "s" },
  { name = "rss", json = "/peak_rss_gb", scale = 1073741824, unit = "B" },
  { name = "ms", regex = 'took ([0-9.]+) ms', from = "stderr", optional = true },
]
primary = "seconds"        # default: the first metric
reference = [{ match = { algo = "pr32", threads = 32 }, metric = "seconds", value = 0.38, tolerance = 0.10, source = "…" }]
interrupt = { after = "1s", signal = "INT" }
plan = { estimate = "/estimate", actual = { seconds = "wall_seconds" } }
runner = "networkit"       # for comparison runners
unavailable = "why it cannot run here"   # recorded as a skipped row, never dropped
```

Placeholders: `{repo}`, `{data}`, `{work}`, `{spec}` everywhere; in benchmarks also
`{case_work}` (emptied before each repetition), `{dataset}` (the resolved path, the
private copy for cold cache), `{threads}`, `{rep}`, `{benchmark}` and the matrix
keys; `{{` and `}}` are literal braces. Metric patterns are not templates.

Metrics the harness measures for every row: `wall_seconds`, `user_seconds`,
`system_seconds`, `memory_peak_bytes`, `anon_peak_bytes`, `file_peak_bytes`,
`read_bytes`, `write_bytes`, and `stop_seconds` for interrupted runs.

## Rows

One JSON object per line, one line per repetition, written as each finishes. Schema
version 1 (`schema`); the fields, from `harness::row::Row`:

- identity: `run_id`, `spec`, `spec_sha256`, `official`, `case`, `benchmark`, `group`,
  `vars`, `runner`, `repetition`, `repetitions`, `started_at`;
- outcome: `status` (`ok`, `failed`, `timeout`, `oom`, `not_cold`, `unavailable`,
  `harness_error`) and `error`;
- where: `machine` (CPU model, cores, threads, SMT, memory, kernel, governors, boost,
  transparent huge pages, and an `id` hashed from the hardware), `git` (`revision`,
  `dirty`);
- what: `command` (fully expanded), `wrapper` (the `systemd-run` and `measure`
  prefix), `cwd`, `env`, `dataset` (name, path read, bytes), `conditions`, `threads`,
  `memory_limit_bytes`, `cache` (mode, copy, resident bytes after eviction);
- the machine around it: `load_start`, `load_end`, `mem_available_start_bytes`,
  `disturbance` (settle-window busy cores, others' cores during the run, threshold,
  verdict);
- measured: `time` (wall, user, system), `io` (read, written), `memory` (limit and
  swap limit applied, `peak_bytes`, `anon_peak_bytes`, `file_peak_bytes`, samples),
  `oom` (`max_events`, `oom_events`, `oom_kills`), `exit` (code, signal, timed out),
  `stop`;
- read from the output: `metrics`, `units`, `primary`, `references`;
- plans: `estimate`, `actual`, `estimate_error`.

## Report

`everygraph-bench report FILES…` prints, per run, a header (official or exploratory,
machine, revision) and one table per group: for each case its conditions, threads,
memory limit and cache state, how many repetitions count and why the others do not,
then each metric as `median (min–max)`, and the reference check. Then the benchmarks
not run (with their reasons), the rows that did not complete, and the estimate
coverage. `--json FILE` writes the same numbers as chart-ready series (one per case and
metric, with conditions and values). `--compare` puts the primary metric of each case
side by side across runs, against the first, and exits 1 when a difference reaches
`--max-diff`.

## Comparison runners

`runners/run.py TOOL TASK --graph DIR …` runs one task of one tool and prints one JSON
line naming exactly what it called (`algorithm`) and the tool's version. The
environment is `runners/uv.lock`; the specs' preparation step runs
`uv sync --frozen --project bench/runners` (it needs the network the first time).

| Tool | Spec | Version | Status |
|---|---|---|---|
| NetworKit | `baselines` | 11.3 | runs (version the lab used not recorded) |
| python-igraph | `baselines`, `engines` | 1.0.0 | runs |
| GVE-Louvain, GVE-Leiden | `baselines` | commits of 2025-04-08, pinned in the spec | built by the spec from source |
| DuckDB (ingest) | `baselines` | 2.0.1 | runs |
| rustworkx | `engines` | 0.18.1 | runs |
| LadybugDB | `engines` | 0.22.3 (PyPI `ladybug`) | runs |
| GFQL (graphistry + polars) | `engines` | 0.62.2 + 2.1.0 | runs |
| DuckDB + DuckPGQ | `engines` | — | **unavailable**: needs DuckDB 1.3.2 next to 2.0.1 |
| Neo4j 5.26 + GDS | `engines` | — | **unavailable**: a server lifecycle not written yet |

Known traps, kept in the runners: igraph's PageRank is an exact PRPACK solve, not 20
iterations; NetworKit's triangle count is `TriangleEdgeScore` summed and divided by 3
(not `LocalClusteringCoefficient`); rustworkx's 2-hop is a Python loop over a native
`neighbors()`; GFQL has no shortest-path operator.
