# graphlab — feasibility lab for a new graph engine

## Why this lab exists

We are designing, from a blank sheet, a new open-source **unified graph engine**:
one tool to ingest, store, analyze, query and visualize graphs of *any* size
(10^3 … 10^11 edges) on *any* single machine (browser → laptop → workstation →
big server), using all the hardware it is given (all cores, RAM, NVMe, optional
GPU) and degrading gracefully when the graph does not fit in RAM. Think "the
DuckDB/Polars moment for graphs", plus a zoomable hierarchical map of the graph
(Google-Maps style, exact aggregate numbers at every level).

Working assumptions to be *validated or refuted* here (nothing is sacred):
Rust core; immutable CSR/CSC topology + Arrow property columns; adaptive
planner choosing between in-RAM / compressed / mmap / semi-external execution;
parallel deterministic algorithms; hierarchical community detection feeding a
multilevel layout and a tile pyramid; Python/CLI/SQL/WASM front-ends.

Each experiment answers concrete **decision questions**. A clear negative
result ("this bet does not work, here is the number") is as valuable as a
positive one. Do not make results look better than they are.

## Machine

- AMD Ryzen 9 7950X: 16 cores / 32 threads (cpu i and i+16 are SMT siblings), AVX-512, 64 MiB L3
- 61 GB RAM (about 45 GB usable by the lab in total, shared by all experiments), 63 GB swap (do NOT rely on swap)
- Samsung 990 PRO NVMe, ~1.4 TB free, ext4 on /home
- NVIDIA RTX 4090 (optional; the product must never *require* a GPU)
- Debian 13, kernel 6.12, rustc 1.96, gcc 14, clang, cmake, node 26, uv (Python), no sudo
- `systemd-run --user --scope -p MemoryMax=16G -p MemorySwapMax=0 <cmd>` works for hard memory limits

## Shared data (READ-ONLY — never modify or delete)

`/home/tux/graphlab/data/raw/`
- `com-lj.ungraph.txt.gz`        SNAP com-LiveJournal, undirected text edge list
- `com-orkut.ungraph.txt.gz`     SNAP com-Orkut
- `com-friendster.ungraph.txt.gz` SNAP com-Friendster, 9.4 GB gz (~31 GB text), 65.6M nodes, 1.806B edges
- `twitter-2010.graph` + `.properties`  LAW twitter-2010 in WebGraph BVGraph format, 41.65M nodes, 1.468B arcs (directed), no .offsets/.ef file yet

`/home/tux/graphlab/data/csr/{lj,orkut}/` — format **glab-csr-v0**:
- `meta.json`     {n, m_undirected, entries, directed, symmetric, sorted, self_loops}
- `offsets.u64`   n+1 little-endian u64; neighbors of node i are targets[offsets[i]..offsets[i+1]]
- `targets.u32`   little-endian u32, symmetric (both directions stored), sorted per node, deduplicated, no self-loops
- `ids.u64`       original SNAP id of each dense node id

lj: n=3,997,962 m=34,681,189 — orkut: n=3,072,441 m=117,185,083.
(Reference point: the naive numpy converter that produced these took 51 s and 197 s single-threaded.)

If you produce a new graph in glab-csr-v0 that others could use (e.g. friendster,
twitter), write it to `/home/tux/graphlab/data/csr/<name>/` (write to a temp dir
and `mv` atomically; never overwrite an existing one) and mention it in your report.

## Rules (several experiments run concurrently on this one machine)

1. Work ONLY inside your own directory `/home/tux/graphlab/exp/<your-exp>/`
   (plus the shared csr output above). Never touch other experiments or anything
   outside `/home/tux/graphlab`. Put Python envs (uv venv), cargo target dirs and
   downloads inside your own directory.
2. **CPU**: during development, pin runs to your assigned cores with
   `taskset -c <cores>` and build with `nice -n 10 cargo build --release -j 6`.
   For measurements that need the whole machine (thread-scaling, headline
   numbers), wrap the command in `flock /home/tux/graphlab/bench.lock <cmd>` so
   only one "official" run happens at a time — keep each such run short
   (< 10 min). Numbers will still be noisy because others are compiling; say so.
   The coordinator will re-run your `bench.sh` on an idle machine at the end.
3. **RAM**: stay under your assigned budget. Any run that needs more than 10 GB
   must be wrapped in `flock /home/tux/graphlab/bigmem.lock <cmd>` (one big-memory
   run at a time on the machine) and must stay under 34 GB.
4. **Network exposure**: this host has a PUBLIC IP. Any server you start must
   bind to `127.0.0.1` only. Never bind `0.0.0.0`.
5. No sudo. Never use `pkill`/`killall` by name or pattern; kill only PIDs you started.
   Do not touch the `llama`, `mongod`, `docker` processes of the host.
6. Disk: clean large temporaries you no longer need; keep your dir under ~150 GB.
7. Time: aim to be done in roughly 60–90 minutes of wall-clock. Answer the
   primary decision questions first; stretch goals only if time remains. Report
   partial results rather than overrunning.

## Deliverables (in your experiment directory)

- `REPORT.md` — decision questions → answers; tables of measured numbers
  (state threads, dataset, command, whether the machine was shared/noisy);
  what failed and why; concrete recommendations for the product design;
  open risks; effort estimate to productionize.
- `bench.sh` — one re-runnable script that reproduces the headline numbers and
  writes machine-readable `results/*.json`. It must run from a clean shell,
  finish in < 20 min, and not need network.
- The code, buildable (`cargo build --release` / `uv run`).

Your final message to the coordinator: the 10–20 most important numbers, the
answers to the decision questions, and your design recommendations — concise,
honest, with caveats. Raw facts over prose.
