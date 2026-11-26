# The correctness oracle

Everygraph computes the same answer through many paths: in memory, memory-mapped or
streamed; from raw or compressed storage; on 1 or N threads; natively or in WebAssembly.
The oracle checks that every one of those paths gives the right answer, and the same
answer (issue #6, DESIGN D11, D12, D14).

It has two halves:

| Where | Language | What |
|---|---|---|
| [`crates/everygraph-oracle`](../crates/everygraph-oracle) | Rust (`publish = false`) | generators, the corpus, the exchange form, the analytic graph, in-process references, comparators, invariant checkers, the `KernelUnderTest` trait and harness, the external-kernel protocol, a toy kernel with seeded bugs, golden-file checks, the `everygraph-oracle` command |
| `oracle/` (this directory) | Python, managed by `uv` with a lock file | reference runners on NetworkX 3.7, python-igraph 1.0.0 and scipy 1.18.1 (`oracle-ref`), golden-file builder, semantics probe |

Python exists only here. No engine crate depends on it, and nothing in it is shipped.
python-igraph is GPL-2.0-or-later: that is acceptable for a test tool that is never
distributed, and it must never move into a shipped package.

## How a kernel is checked

For each graph of a corpus and each kernel the graph lists, the harness
(`plugin::run`):

1. runs the kernel under every configuration the implementation declares (threads,
   chunk size, storage, regime);
2. **cross-configuration identity**: every result equals the first one bit for bit
   (D14);
3. **in-process reference** (`reference.rs`): the result matches it under the
   comparator's rules;
4. **Python references**: it matches each library's answer in `reference.json`;
5. **invariants** (`invariants.rs`): it satisfies every invariant of its kernel, which
   needs no reference at all.

The four families are independent: an invariant knows nothing of the references, and a
reference knows nothing of configurations. A finding names the check that fired, the
graph (reproducible from its name), the kernel and the configuration.

The in-process references are textbook sequential code, so property-based tests can
compare against something without leaving Rust. They are checked against the three
libraries on every CI run (`check --plugin reference`).

**One kernel has no Python witness.** Quotient is checked by the in-process reference
and the invariants only (`plugin::python_computes`). Adjacency (the CSR itself, which
the point queries list) has NetworkX's sorted neighbours as its witness.
Graphs with more than 2^20 nodes, the sparse-id families, are not run through Python at
all: their per-node answers would make `reference.json` hundreds of MB. They are checked
by the in-process references, the invariants and cross-configuration identity.

References and invariants run on **every distinct result** of a kernel, not only on the
first configuration's. A bug confined to one configuration (say, compressed storage) is
then seen by every family, not only by cross-configuration identity.

## Running it

```sh
oracle/run-ci.sh          # what CI runs on every pull request (budget: 10 minutes)
# Both under a memory limit on a shared machine:
#   systemd-run --user --scope -p MemoryMax=8G -p MemorySwapMax=0 oracle/run-ci.sh
oracle/run-nightly.sh     # larger corpus, 2,000 proptest cases, all golden files, lab prototype
```

Or step by step:

```sh
cargo build -p everygraph-oracle --release
target/release/everygraph-oracle corpus --tier ci --out /tmp/oracle/ci   # 289 graphs
cd oracle && uv sync --locked
uv run oracle-ref run /tmp/oracle/ci                                     # reference.json per graph
cd ..
target/release/everygraph-oracle check --corpus /tmp/oracle/ci --plugin reference --require-python
target/release/everygraph-oracle check --corpus /tmp/oracle/ci --plugin toy --require-python
target/release/everygraph-oracle seeded-bugs --corpus /tmp/oracle/ci
target/release/everygraph-oracle golden --dir oracle/golden --data-root ROOT   # see Golden files
```

**What runs where.**

- **CI**, job `oracle` of `.github/workflows/ci.yml`, which runs `run-ci.sh`:
  - the CI corpus through the Python references, the in-process references, the toy and
    the seeded bugs;
  - the semantics probe;
  - the golden files of karate and of the nine generated tier-1 datasets;
  - the 14 LDBC Graphalytics test graphs against their reference outputs (§ LDBC
    Graphalytics below).

  After `uv sync` it downloads only the LDBC test graphs (10 kB). The oracle's Rust tests (unit tests, 6 properties
  at 96 cases each, the seeded-bug tests, the karate golden file) run in the `test` job
  with the rest of the workspace; run locally, `run-ci.sh` runs them too.
- **Nightly**, `run-nightly.sh` (not scheduled yet; run by hand or by the benchmark
  machine):
  - the nightly corpus: 1,576 graphs, up to 100,000 rows, hubs with 2^16 neighbours, and
    sparse ids up to n = 2^24. It gets the same checks as CI, plus the lab prototype
    through its adapter;
  - 2,000 cases per property;
  - all 17 tier-1 edge-list datasets of the registry and the three extra SNAP graphs,
    fetched and verified, with all 21 golden files checked;
  - LDBC Graphalytics graph500-22 and dota-league against their reference outputs.

Measured times and peaks are in the pull request that introduced the oracle (#6).

**Memory.** Reference implementations are for small and medium graphs; large graphs are
checked by invariants. `oracle-ref run` bounds memory twice:

- It skips any graph with more rows than `--max-rows` (default 2,000,000) without
  loading it.
- Each worker process runs under an address-space limit (`--max-memory-gib`, default
  3 GiB), with one BLAS thread. A kernel that would grow past the limit fails with
  `MemoryError`, which is recorded as a skipped case; the run then exits non-zero.

The summary line reports the largest worker resident set. Workers default to 2
(`--jobs`), so the default total stays near 6 GiB. On the lab machine, run anything that
is not a unit test under a hard limit as well:

```sh
systemd-run --user --scope -p MemoryMax=8G -p MemorySwapMax=0 uv run oracle-ref run DIR --jobs 2
```

The Rust commands that load graphs (`golden`, `check`, `seeded-bugs`) take
`--max-memory` (GiB, default 4) as well. The crate forbids `unsafe`, so they cannot set
RLIMIT_AS; instead they refuse a source whose estimate exceeds the limit, before loading
it. The estimate is 400 bytes per row plus 160 per node
(`everygraph_oracle::golden::estimate`), calibrated on web-google's measured 1.76 GB.
`tests/golden.rs` reads registry data only from `$EVERYGRAPH_ORACLE_DATA`, an explicit
opt-in, never from `$EVERYGRAPH_DATA`.

This is a lesson learned the hard way. The first version computed scipy's triangle
counts as `diag(A³)/2`. On the nightly tier's star with 2^16 leaves, `A @ A` has 2^32
entries: one worker reached 61.5 GB before it was stopped. The graph has only 65,536
edges. Every product is now taken over the degree-ordered orientation, which is bounded
by O(m^1.5).

## Exchange formats

One directory per graph, written by `everygraph-oracle corpus` and read by Rust, by
Python and by external kernels (`exchange.rs`, `graph.py`):

- **`graph.json`** holds:
  - `format` (`"everygraph-oracle-graph"`) and `version` (1);
  - `name`, `n` (isolated nodes included), `directed`, `rows`;
  - `columns` (`src`, `dst`, then optional `weight` and `time`);
  - `semantics` (`{"self_loops": "drop" | "keep", "duplicates": "events" | "collapse" |
    "sum"}`; `duplicates` defaults to `events` when absent);
  - `partition` (file name or null);
  - `kernels`: the kernels to check, for example `{"kernel": "bfs", "source": 3}`;
  - `provenance`: the generator spec, which regenerates the graph exactly.
- **`edges.tsv`** has one row per event, `src<TAB>dst[<TAB>weight][<TAB>time]`, with ids
  in `0..n` and one leading `#` comment line. Weights are written in the shortest form
  that reads back to the same double.
- **`partition.txt`** holds one community label per node, one per line.
- **`reference.json`** is written by `oracle-ref run`:
  - `libraries` with their versions;
  - `adjustments`, see SEMANTICS.md;
  - `results`: per kernel, each library's answer and whether they agree;
  - `disagreements` and `errors`.

**Output form.** Each output is an object with one key:

| Key | Meaning |
|---|---|
| `ints` | one integer per node, `null` = no value |
| `reals` | one real per node, `null` = no value |
| `labels` | one label per node, compared after relabelling each class by its smallest member |
| `count` | a single count |
| `real` | a single real |
| `csr` | `offsets` and `targets` of an adjacency |
| `counts` | named counts: the ingest report (`counts.rows`, `dropped.self_loops`, ...), compared exactly; the Python side's answer is under the name `python` |
| `quotient` | community `sizes` and weighted `edges` |

**External kernels** (`external.rs`) are programs run once per (graph, kernel,
configuration) as `<program> [args] --graph DIR --kernel '<json>' --config '<json>'`.
They print one output as JSON. This is how the lab prototype is checked today
(`lab/b1-core/crates/glab-oracle`), and how the Python binding can be checked from M2.

## Semantics

Kernels run on the *analytic graph* (`projection.rs`, `graph.py`), the rules of
`docs/spec/graph-semantics.md` applied to the rows:

- self-loops are dropped and counted, unless the graph keeps them; a kept loop is the
  pair {v, v}, `A[v][v] = 2w` undirected and `w` directed for degrees, PageRank,
  modularity and the quotient, ignored by core numbers, triangles, clustering,
  transitivity, components and traversals;
- a repeated pair is analysed once, with its multiplicity, summed weight and smallest
  weight (`duplicates = events`); `collapse` keeps the pair's first row only; `sum`
  analyses as `events`, except that a step's cost is the summed weight;
- the undirected projection of a directed graph has {u, v} if either arc exists.

The ingest report (`ingest_report` kernel) and its two identities are checked on every
graph. Where NetworkX and
igraph disagree, [`SEMANTICS.md`](SEMANTICS.md) records the case, what each does and what
Everygraph does. It ends with the decisions (Q1–Q13, 2026-11-25) taken where no earlier rule applied.

## Tolerances

`tolerances.json` is read by both halves. A real `a` passes against a reference `b` when
`|a − b| ≤ abs + abs_over_n / n + rel·|b|`. Integers, labels, counts and adjacency are
compared exactly; results of one kernel under two configurations are compared bit for
bit.

| Quantity | Tolerance | Why |
|---|---|---|
| PageRank, per node | `rel = 1e-9`, `abs_over_n = 1e-9` | The references are at the fixed point to about 10⁻¹¹ relative or better (PRPACK; NetworkX at `tol = 1e-14 / n`, see SEMANTICS.md; the in-process power iteration shrinks the initial error by 10⁻¹⁷). Values scale as 1/n, so a bare absolute bound would be loose on large graphs and meaningless on small ones: the bound is 10⁻⁹ relative, floored at 10⁻⁹ of the uniform value 1/n. m3-04 asks for agreement "within 1e-9"; this is that, made scale-free. On the CI corpus every comparison passes, including those of the lab prototype, which runs a fixed 300 iterations |
| PageRank, sum | `abs = 1e-9` | n additions of values that sum to 1 in exact arithmetic |
| local clustering | `abs = 1e-12` | `2t / (d(d−1))` is one division of exact integers, so every correct implementation gives the same correctly rounded double. The slack only covers a different but equivalent formula |
| transitivity | `abs = 1e-12` | same reason, one division |
| modularity | `abs = 1e-10` | a sum over communities whose order differs between libraries: about n·ε relative error, which is below 10⁻¹¹ for n ≤ 10⁵ |
| weighted distances | `rel = 1e-12`, `abs = 1e-12` | sums along paths; two shortest paths of equal length may round differently. The corpus mostly uses dyadic weights (multiples of 1/16), whose sums are exact |

## Golden files

`golden/*.json` hold known answers on real graphs: Zachary's karate club, **all 17 tier-1
datasets of the registry** (`datasets/datasets.toml`, #5), and three small SNAP graphs the
registry does not list. Each file is about 2–3 KB. Values are compact:

- counts: nodes, rows, self-loop and duplicate rows, pairs or arcs (`m`), the maximum
  degree;
- components: weak and strong, count and largest;
- triangles: the total, and the SHA-256 of the per-node counts;
- core numbers: the SHA-256 of the per-node values, the maximum and the histogram;
- three exact samples (first node, last node, node of largest degree): degree, core
  number and triangles.

Per-node arrays are hashed as little-endian `u64` in node order. Each file carries:

- its `parse` block, the few rules (separator, comment prefix, id columns, row filter,
  integer or string ids) by which Rust and Python read the same rows from the same
  bytes;
- the SHA-256 of the file read and of the download;
- the library versions;
- SNAP's published figure where SNAP gives one (checked against the dataset pages on
  2026-11-25).

Node ids are the rank of the original id among those that appear in a row.

**Witnesses depend on size,** so that every run stays within its memory limit:

| Graph | Source | Kind | n | m | Triangles | = SNAP | Max core | Python witnesses |
|---|---|---|---|---|---|---|---|---|
| karate | vendored (`golden/karate.tsv`, identical in NetworkX and igraph) | u | 34 | 78 | 45 | – | 4 | NetworkX, igraph, scipy |
| email-Eu-core | SNAP download | d | 1,005 | 24,929 | 105,461 | yes | 34 | all three |
| ca-GrQc | SNAP download | u | 5,242 | 14,484 | 48,260 | yes | 43 | all three |
| p2p-Gnutella08 | SNAP download | d | 6,301 | 20,777 | 2,383 | yes | 10 | all three |
| ego-facebook | registry | u | 4,039 | 88,234 | 1,612,010 | yes | 115 | all three |
| soc-epinions1 | registry | d | 75,879 | 508,837 | 1,624,481 | yes | 67 | all three |
| soc-sign-bitcoin-otc | registry | d | 5,881 | 35,592 | 33,493 | – | 21 | all three |
| college-msg | registry | d | 1,899 | 20,296 (from 59,835 messages) | 14,319 | – | 20 | all three |
| wiki-clickstream-ptwiki-2024-01 | registry (string ids) | d | 288,605 | 983,234 | 529,749 | – | 19 | igraph, scipy |
| com-youtube | registry | u | 1,134,890 | 2,987,624 | 3,056,386 | yes | 51 | igraph, scipy |
| roadnet-ca | registry | u | 1,965,206 | 2,766,607 | 120,676 | yes | 3 | igraph, scipy |
| web-google | registry | d | 875,713 | 5,105,039 | 13,391,903 | yes | 44 | igraph, scipy |
| 9 generated `synth-*` | registry (made by `everygraph-bench`, no network) | u | 20–1,000 | 91–5,000 | 0–1,140 | – | 1–19 | all three |

- **Up to 600,000 pairs**, NetworkX, igraph and scipy must agree on every value.
- **Above that**, NetworkX is not run (its graph objects take several GB). igraph and
  scipy must agree on counts, components and triangles. Core numbers have igraph as their
  one Python witness; the Rust check (`everygraph-oracle golden`, in-process reference)
  is the second.
- **No kernel is skipped on any tier-1 graph.**

Measured on this machine, under `MemoryMax=8G`:

- writing all 21 files: 30 s, largest resident set 1.66 GB (web-google);
- the Rust check of all 21: 13.8 s, 1.76 GB.

The SNAP figures confirm every published triangle count, largest WCC and largest SCC.
Where SNAP's edge count differs it is because SNAP counts self-loops as edges
(email-Eu-core: 642, ca-GrQc: 12; recorded in each file's `notes`).

**Where the sources are:**

- The registry's datasets come from a data root filled by `everygraph-bench fetch tier1
  --data ROOT` (about 72 MB). Pass `--data-root ROOT`, or set `$EVERYGRAPH_ORACLE_DATA`
  (then `$EVERYGRAPH_DATA`).
- The three extra SNAP graphs come from `uv run oracle-ref golden --fetch`, into
  `oracle/.cache/`.
- Karate is vendored.

A golden file whose source is missing is skipped, not failed; `--require-all` fails it.
CI checks karate and the nine generated datasets, which need no network. Nightly fetches
tier 1 and checks all 21.

Commands:

```sh
target/release/everygraph-bench fetch tier1 --data ROOT
uv run oracle-ref golden --write --data-root ROOT     # recompute (after a library upgrade)
uv run oracle-ref golden --check --data-root ROOT
target/release/everygraph-oracle golden --dir oracle/golden --data-root ROOT --require-all
```

**Golden checks of a future kernel (design note, for #45).** Today the golden files are
checked against the in-process references only. A `KernelUnderTest` will be checked the
same way:

1. Read the source with the file's `parse` block (`golden::read_rows`); ids then are
   ranks, the convention every value uses.
2. Build the engine graph from those rows, so that node `i` of the engine is rank `i`.
   An engine that renumbers internally maps its output back to ranks before hashing.
3. Run the kernel and compute each value as `golden::compute` does: counts and the
   SHA-256 of the per-node arrays as little-endian `u64` in rank order.
4. Compare with the file.

A `golden::check_kernel(plugin, roots)` helper next to `check_dir` will do this; it is
not built yet.

## LDBC Graphalytics

LDBC Graphalytics publishes graphs up to billions of edges with the reference output of
six deterministic algorithms (issue #177; specification v1.0.5, arXiv 2011.15028). They
are the oracle's references above 10^5 edges. The module `ldbc`
(`crates/everygraph-oracle/src/ldbc/`) holds:

- `read`: the data set's `.properties`, `.v`, `.e` and `<g>-<ALGO>` files. Vertex ids
  become dense indices in increasing id order, so "smallest label" means "smallest id".
  It refuses self-loops, repeated edges and unknown vertices, which the specification
  excludes.
- `spec`: BFS, PageRank (fixed iterations, sinks spread uniformly, f64), WCC, CDLP, LCC
  (the directed definition included) and SSSP, sequential, written from § 2.3.
- `validate`: the match rules of § 2.4: exact (BFS, CDLP), equivalence (WCC), and
  `|r - s| <= 1e-4 |r|` (PageRank, LCC, SSSP).
- `run`: runs `spec`, `first-seen` (CDLP with initial labels in input order) and
  `oracle` (this crate's references, Everygraph's semantics; graphs up to 2^20 edges).

```sh
everygraph-bench fetch ldbc-test --data ROOT        # then unpack: oracle/ldbc-unpack.sh
everygraph-oracle ldbc --graphs DIR --expect oracle/ldbc-expected.json [--json OUT]
```

`DIR` holds one unpacked data set per subdirectory. `spec` must always match. The other
outcomes, mismatches included, are pinned in `oracle/ldbc-expected.json` with their
causes, so a change either way fails the run. Kernels of other crates and the lab
prototypes are checked by calling `ldbc::run::row` on their result (the proof kernels:
`crates/everygraph-kernels/tests/ldbc.rs`, run by both scripts;
the lab: `lab/m1-20-ldbc-graphalytics`). Results checked this way are **not LDBC Benchmark
Results**; the name is reserved for audited runs.

## Plugging in a kernel (issue #45 onwards)

A kernel crate adds `everygraph-oracle` as a dev-dependency and implements
`KernelUnderTest` in a test:

- `supports(kernel, case)` says which kernels and graph kinds it handles;
- `configurations()` lists what must not change the answer: thread counts, chunk sizes
  (including one at 64 entries, the size the adversarial families aim at), storage (raw,
  compressed) and regime (in memory, mmap, stream);
- `run(case, kernel, config)` returns the result in canonical form.

Three tests then cover it:

- `plugin::run` over `corpus::entries(Tier::Ci)`, which needs no Python;
- the same over a corpus directory loaded with `Prepared::load` after `oracle-ref run`,
  which adds the three libraries;
- property tests over `strategies::arb_graph`.

From M2 the minimal Python binding can be checked the same way through the external
protocol.

Two things the toy does not need and an engine kernel will:

- **A way to set the thread count of `everygraph_core::par`'s pool from a test.** Only
  `everygraph-core` may depend on rayon (deny.toml), so this needs a small API in the
  core, or a test-only exception.
- **The storage and regime axes**, which mean something only once the codec (#30) and
  the I/O backends (#29) exist.

## Seeded bugs

`toy.rs` is a chunked, threaded kernel with five switchable bugs. `everygraph-oracle
seeded-bugs` prints which checks catch each one, and `tests/seeded_bugs.rs` asserts it.
The fourth bug is the reason the cross-configuration check exists: its drift is a few
units in the last place, far inside every reference tolerance.

A lesson from building the toy: a reduction whose blocks follow the *scan's* chunks gives
a different rounding when the chunk size changes, and with it when the storage block size
changes. The toy's first version did this, and the harness flagged it at once. The fixed
version reduces over blocks of a constant number of nodes. Production kernels will meet
the same trap whenever raw and compressed storage cut lists differently.

The lab prototype falls into it too. On the nightly tier its PageRank differs bit-wise
between chunk sizes 64 and 65,536 on 56 graphs with isolated nodes: its dangling-mass
sum follows the chunks. Threads and regimes do not change it, and every other lab kernel
is bit-identical in every configuration. `run-nightly.sh` reports the prototype's
findings without failing on them.
