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
| `runners/` | Comparison runners for other tools: a `uv`-locked Python project | yes (not `.venv/`) |

The **work directory** holds builds, decompressed inputs, cold-cache copies and logs
(after `headline`, about 80 GB). It is `--work`, else `$EVERYGRAPH_BENCH_WORK`, else
`$EVERYGRAPH_SCRATCH/bench-work`, else `bench/work/` in the checkout (ignored by git).
It must be on a disk: a work directory on tmpfs or ramfs is refused before anything is
written, because what the harness puts there would sit in RAM outside every memory
limit, and eviction cannot work there. `everygraph-bench clean` deletes it (only a
directory the harness has marked as its own; `--spec NAME` for one spec's part,
`--dry-run` to see the size).

## Running

```sh
everygraph-bench check                       # every spec parses; list the cases
everygraph-bench check headline --commands   # the expanded commands, for this machine
everygraph-bench selftest                    # on an idle machine, before official runs
everygraph-bench run headline --only kernels-orkut --repetitions 1   # exploratory
everygraph-bench run headline --official     # an official run (see below)
everygraph-bench report bench/results/headline/*.jsonl --json chart.json
everygraph-bench report --compare A.jsonl B.jsonl --max-diff 0.05
everygraph-bench run SPEC --interleave --seed 1 --compare-by build=main   # an A/B
everygraph-bench clean
```

`--data` (or `$EVERYGRAPH_DATA`) is the data root; on the lab machine
`/home/tux/graphlab/data`. Progress goes to standard error, the report of the run to
standard output. Exit status: 0; 1 (failure, a row that did not complete, a report
with a reference off or unchecked, a comparison over the limit or not comparable, a
failed self-test); 2 (usage); 3 (an official run refused to start).

## Protocol

**Official and exploratory runs.** A run is exploratory unless `--official` is given.
Issues close on exploratory evidence; official runs happen once per milestone, in one
planned window, and before a number goes into README, DESIGN, a paper or a release
(CLAUDE.md § 6). An official run, in this order:

1. takes **the** lock, `/run/user/<uid>/everygraph-bench-official.lock`, without
   waiting, before any other check. It is the only lock in the harness and is taken in
   one place (`harness::run::run`). A second official run, from another shell or
   nested in a benchmark command, fails at once (exit 3) and names the holder — it
   never queues, because a nested lock once deadlocked a lab run for half an hour. The
   lock is an `flock`: the kernel releases it however the holder ends. (Builds with
   the `test-hooks` feature, which only the crate's own tests enable, read another
   path from `$EVERYGRAPH_BENCH_LOCK`; release builds never do.)
2. refuses a working tree with uncommitted changes to tracked files (`--allow-dirty`
   overrides it and every row then says `dirty`), and refuses a spec file that is not
   tracked by git or differs from its version at `HEAD`, whatever `--allow-dirty` says;
3. refuses `--max-load` above the spec's `[gate] max_load` (it may make the gate
   stricter, never looser);
4. refuses to start when the 1-minute load average is above the threshold (default
   1.0) or `MemAvailable` is below what the run needs (its largest memory limit, or
   `[gate] min_mem_available`). Every row records the gate as applied: the lock path,
   both thresholds, the needed memory and the readings;
5. writes its rows to `results/`, to be committed.

Exploratory runs skip all five and go to `exploratory/`. Their rows record the same
readings; their report says not to quote them.

**Every repetition.** Before each repetition the harness empties the case's work
directory, writes back dirty pages (`sync`: an ingest leaves gigabytes whose write-back
would otherwise run during the next repetition), and sleeps through a settle window
(1 s, `[defaults] settle`) while it counts the cores other processes keep busy
(`/proc/stat`, user and system time) and the machine's storage traffic
(`/proc/diskstats`, physical devices only). The command then runs as

```
systemd-run --user --scope -p MemoryMax=<limit> -p MemorySwapMax=0 \
    -p OOMPolicy=continue -- everygraph-bench measure --report … -- <command>
```

(`OOMPolicy=continue`: otherwise systemd stops the whole scope when the OOM killer
ends one of its processes, and the OOM would go unrecorded.) A run shorter than 1 s
gets a second settle window just after it, since nothing can be measured during it.

**Every run is under a hard memory limit**: a spec whose benchmark has no
`memory_limit` (its own or `[defaults]`) is an error, and the limit the kernel applied
(`memory.max`, `memory.swap.max`) is recorded next to the one asked for. `measure` is
the first process of the scope; it starts the command, samples `memory.stat` every
20 ms, and when the command ends reads, from inside the still-living cgroup,
`memory.peak` (exact peak of anonymous + file-backed + kernel memory), `memory.events`
(limit hits, OOM kills), `cpu.stat` (user and system time of everything in the scope),
and its own `/proc/self/io` (bytes the command and its descendants read from and wrote
to storage). Those counters include a descendant only once it is reaped, and a
descendant whose parent ends first — on a timeout or a stop every process of the scope
is signalled at once; a command may also exit before its children — would be reaped by
init and its traffic lost (tracer row 27, `tiles-16g`, recorded 86,016 bytes read
after an hour of page faults). So `measure` makes itself a child subreaper
(`PR_SET_CHILD_SUBREAPER`): orphans become its children, and after the scope is empty
it reaps every one before it reads the counters. When that cannot be done (a descendant
moved itself to another cgroup and still runs 5 s later, or the `prctl` fails) the row's
`io.unknown` says why, the byte counts are a lower bound and no metric, and others'
storage traffic during the run is not computed. The cgroup's own `io.stat` would avoid
all this, but on the lab machine systemd does not delegate the io controller to user
scopes (`cgroup.controllers` of `user@1000.service`: `cpu memory pids`). Anonymous and file-backed peaks are kept apart because a memory-mapped
graph counts as RSS but is reclaimable page cache (FINDINGS § B1): RSS alone misleads.
They are sampled, so a spike shorter than the interval can be missed; `memory.peak` is
not sampled.

**Processes left behind.** When the command exits, any other process still in its
scope (a backgrounded child, a daemon) is killed with SIGKILL, by the ids of
`cgroup.procs`, until the scope is empty; the row lists them in `left_processes` and its
status is `left_processes`, not `ok`: they would have disturbed what came next.

**Others while it ran.** For runs of 1 s and more, the machine's user time minus the
scope's own gives the cores others used, and the machine's storage traffic minus the
run's own bytes gives their traffic. User time only, because the kernel's work for the
run (write-back, reclaim) is system time charged outside the scope; interrupt time is
left out of every window for the same reason. `everygraph-bench selftest` checks this
accounting on the machine: a busy loop on every CPU must find others at ≤ 0.25 cores
and its own cgroup at about threads × seconds, and a 1 GiB synced write must find
others' traffic at ≤ 10 MB/s.

**Disturbed rows.** In an official run, a repetition is marked `disturbed` when, in a
settle window around it or while it ran, others kept more than 0.5 cores busy (`[gate]
disturbed_above_cores`) or moved more than 50 MB/s to or from storage (`[gate]
disturbed_above_io_mb_s`). The row says which reading crossed which threshold
(`disturbance.reasons`); it is kept, left out of every summary, and listed at the end
of the run. One busy core skewed lab timings by 10–40%.

**Warm and cold cache.** A warm case runs once untimed first (`warmup`, default 1).
A warm-up that fails ends the case: its row carries the whole measurement — status
`oom`, `timeout` or `failed` as for a repetition, memory, I/O, OOM events, exit — with
`repetition` 0 and `warmup` set to its number, so it is never counted, and the
repetitions are not run. A
cold case (`cache = "cold"`) reads a private copy of its dataset, made once under
`work/copies/` by a read/write loop (not `copy_file_range`, which may share blocks on
a reflinking file system) and reused while the source is unchanged. Before each
repetition the copy is synced and evicted with `posix_fadvise(POSIX_FADV_DONTNEED)`;
then `mincore(2)`, through util-linux `fincore`, must find at most 1% of it resident,
or the repetition is not run and its row says `not_cold`. Private, because pages that
another process maps cannot be evicted. There are at least two copies
(`cold_copies`), and consecutive cold repetitions of a dataset — across cases and
across runs (the copies' directory keeps the index of the copy read last) — take them
in turn: reading the same blocks twice in a row measures the drive's own cache
(FINDINGS § F1). There is no `sudo` and no `drop_caches`. A cold dataset on tmpfs is
refused.

**Write-heavy cases.** A solid-state drive writes its first gigabytes after an idle
spell much faster than its sustained rate (on the lab machine: ~10 GB at ~5 GB/s after
5 idle minutes, then ~1.5 GB/s). A case that writes tens of gigabytes per repetition
therefore needs a warm-up run (`warmup = 1`), so that every measured repetition starts
from the same, sustained drive state; otherwise the first repetition is faster than the
others by a third (Friendster ingest, 2026-11-25).

**Order: consecutive or interleaved.** By default each case's repetitions run back to
back, the cases in spec order. `run --interleave [--seed S]` runs in rounds instead
(randomized multiple interleaved trials: Abedi and Brecht, ICPE'17): round *r* runs
every case with at least *r* repetitions once. Odd rounds take a new random order from
the seed; each even round reverses the round before it, so two cases run A B B A or
B A A B and slow drift weighs on both alike. Every row records `seed`, `round` and
`position`. A case's warm-ups run just before its first repetition; a case whose
warm-up fails leaves the later rounds. Interleaving changes the drive state and the
cold copy each repetition starts from: a spec with cold-cache cases must opt in with
`interleave = true`, and `interleave = false` forbids it (both refuse with exit 3).
Interleave every A/B (CLAUDE.md § 6): put the two binaries or settings in a matrix
variable and add `--compare-by VAR=BASELINE` to get the ratio after the run.

**Repetitions and intervals.** The report gives each median with an exact rank interval
`[x(j), x(n+1-j)]` at the largest `j` whose confidence, `1 - 2 P(Binomial(n, 1/2) < j)`,
is at least 95% (Le Boudec, 2010, Theorem 2.1; Hoefler and Belli, SC'15). It holds for
any continuous distribution. Below 6 counted repetitions no interval reaches 95%: the
report then shows `[min, max]` and its confidence (75% for 3) and says "no 95%
interval". The default is therefore 6 repetitions (`repetitions = 3` stays allowed).
The column `n for ±2%` (`--target` to change it) estimates the repetitions a 95%
interval within ±2% of the primary metric's median needs: `(1.96 · sqrt(π/2) · σ / (h ·
median))²`, with σ = 1.4826 × the median absolute deviation, at least 6. It is a
normal-theory estimate; `tests/stats.rs` checks all three statistics on normal,
lognormal and exponential samples.

**What a row ran.** Every row records the spec's SHA-256 and path, the SHA-256 of each
binary the spec declares (`binaries`) and of the program of an argument-list command,
and a digest of its dataset's listing (relative path, size and modification time of
every file): equal digests mean the same files, without hashing gigabytes.

**Stop latency (ux-contract C-4).** `interrupt = { after = "1s" }` sends SIGINT (or
`TERM`) to every process of the run's scope after the delay — what a terminal's Ctrl-C
does — and records the time to the exit as `stop_seconds`. The stop is clean only with
exit status 128 + the signal (130 for INT, 143 for TERM, C-7; `expect_exit` to change
it): any other ending makes the row `failed`. A reference with `max = 2.0` makes the
report check the bound.

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
interleave = true          # optional: allow --interleave with cold cases; false forbids it
binaries = ["{glab}"]      # hashed into every row (top-level keys come before tables)

[defaults]                 # inherited by every benchmark
memory_limit = "8G"        # required here or in each benchmark; systemd syntax, 1024-based
repetitions = 6            # default 6: the fewest with a 95% median interval
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
disturbed_above_io_mb_s = 50
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
conditions = { graph = "orkut", precision = "integer" }   # printed with every number
when = [{ match = { algo = "pr32" }, conditions = { precision = "f32" } }]
metric = [
  { name = "seconds", json = "/seconds_best", unit = "s" },                # JSON pointer, last JSON line of stdout
  { name = "s_per_iter", json = "/iter_seconds", reduce = "median", unit = "s" },
  { name = "rss", json = "/peak_rss_gb", scale = 1073741824, unit = "B" },
  { name = "ms", regex = 'took ([0-9.]+) ms', from = "stderr", optional = true },
]
primary = "seconds"        # default: the first metric
reference = [{ match = { algo = "pr32", threads = 32 }, metric = "seconds", value = 0.38, tolerance = 0.10, source = "…" }]
interrupt = { after = "1s", signal = "INT" }               # expect_exit defaults to 130
plan = { estimate = "/estimate", actual = { seconds = "wall_seconds" } }
runner = "networkit"       # a comparison runner: it must record what ran, below
record_json = { tool = "/tool", version = "/version", algorithm = "/algorithm", options = "/options" }
# or record = { tool = "GVE-Leiden", … }  (literal templates, for tools that print no JSON)
binaries = ["{repo}/bench/runners/run.py"]
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
version 1 (`schema`); the fields, from `harness::row::Row`. A field added later is read
with a default from rows written before it, and keeps the version (so far: `warmup` and
`io.unknown`, #142; `seed`, `round` and `position`, #195; all absent or `null` meaning
what older rows meant):

- identity: `run_id`, `spec`, `spec_sha256`, `spec_path`, `official`, `case`,
  `benchmark`, `group`, `vars`, `runner`, `repetition`, `repetitions`, `warmup` (the
  number of the failed warm-up a row records, with `repetition` 0; `null` otherwise),
  `seed`, `round`, `position` (of an interleaved run; `null` otherwise), `started_at`;
- outcome: `status` (`ok`, `failed`, `timeout`, `oom`, `not_cold`, `left_processes`,
  `unavailable`, `harness_error`) and `error`;
- where: `machine` (CPU model, cores, threads, SMT, memory, kernel, governors, boost,
  transparent huge pages, and an `id` hashed from the hardware), `git` (`revision`,
  `dirty`), `gate` (official runs: lock path, thresholds, readings);
- what: `command` (fully expanded), `wrapper` (the `systemd-run` and `measure`
  prefix), `binaries` (SHA-256 by path), `cwd`, `env`, `dataset` (name, path read,
  bytes, listing digest), `conditions`, `threads`, `memory_limit_bytes`, `cache` (mode,
  copy, resident bytes after eviction), `recorded` (what a runner says it ran);
- the machine around it: `load_start`, `load_end`, `mem_available_start_bytes`,
  `disturbance` (busy cores and storage traffic before, after, and of others during;
  thresholds; reasons; verdict);
- measured: `time` (wall, user, system), `io` (read, written, and `unknown`: why the
  run's own traffic could not be established, `null` when it was), `memory` (limit and
  swap limit applied, `peak_bytes`, `anon_peak_bytes`, `file_peak_bytes`, samples),
  `oom` (`max_events`, `oom_events`, `oom_kills`), `exit` (code, signal, timed out),
  `stop`, `left_processes`;
- read from the output: `metrics`, `units`, `primary`, `references`;
- plans: `estimate`, `actual`, `estimate_error`.

## Report

`everygraph-bench report FILES…` prints, per run, a header (official or exploratory,
machine, revision) and one table per group: for each case its conditions, threads,
memory limit and cache state, how many repetitions count and why the others do not,
the confidence of their median intervals, the repetitions needed for ±2%, then each
metric as `median [low, high]`, and the reference check. Then the benchmarks
not run (with their reasons), the rows that did not complete, the estimate coverage,
and the verdict on the references: `k within, j off, m unchecked`. A reference whose
case has no counted repetition (all disturbed or failed) is unchecked, never a pass;
the report exits 1 if any reference is off or unchecked. `--json FILE` writes the same
numbers as chart-ready series (one per case and metric, with conditions, values,
interval, confidence and repetitions needed).

`--compare` gives, for the primary metric, the ratio of medians (case / baseline) with a
95% percentile-bootstrap interval (10,000 seeded resamples, each side resampled on its
own; Efron and Tibshirani, 1993, § 13.3). A pair passes when the interval lies within
1 ± δ; δ is `--max-diff` (default 0.05), or the case's entry in `--deltas FILE`. Two
modes:
- across runs, each later run against the first: only official runs of the same spec
  (same SHA-256) on the same machine. To calibrate δ, compare two official runs of one
  revision (an A/A comparison) with `--write-deltas FILE`: it writes, per case, the
  smallest δ its interval meets, rounded up to 0.1%.
- `--by VAR=BASELINE`, within each run: the cases with that matrix value against those
  that differ from them only in `VAR`. Exploratory runs are allowed here: their ratios,
  never their times, may be quoted (CLAUDE.md § 3).

Each side needs at least two counted repetitions; the command exits 1 when an interval
leaves 1 ± δ or a pair is not comparable.

## Comparison runners

`runners/run.py TOOL TASK --graph DIR …` runs one task of one tool and prints one JSON
line naming exactly what it called (`algorithm`, `options`) and the tool's version;
the specs store those fields in every row. `seconds` is the tool's call alone: loading
is `load_seconds`, and turning the answer into a result (summing scores, modularity)
happens after the clock stops. The environment is `runners/uv.lock`; the specs'
preparation step runs `uv sync --frozen --project bench/runners` (it needs the network
the first time). The F2 engines run on cores 0–2 (`taskset`), as the lab pinned them.

| Tool | Spec | Version | Status |
|---|---|---|---|
| NetworKit | `baselines` | 11.3 (FINDINGS § A) | runs |
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
