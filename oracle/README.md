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
compare against something without leaving Rust. They are never the only witness: every
CI run checks them against the three libraries (`check --plugin reference`).

## Running it

```sh
oracle/run-ci.sh          # what CI runs on every pull request (budget: 10 minutes)
oracle/run-nightly.sh     # larger corpus, 2,000 proptest cases, SNAP golden files, lab prototype
```

Or step by step:

```sh
cargo build -p everygraph-oracle --release
target/release/everygraph-oracle corpus --tier ci --out /tmp/oracle/ci   # 288 graphs
cd oracle && uv sync --locked
uv run oracle-ref run /tmp/oracle/ci                                     # reference.json per graph
cd ..
target/release/everygraph-oracle check --corpus /tmp/oracle/ci --plugin reference --require-python
target/release/everygraph-oracle check --corpus /tmp/oracle/ci --plugin toy --require-python
target/release/everygraph-oracle seeded-bugs --corpus /tmp/oracle/ci
target/release/everygraph-oracle golden --dir oracle/golden
```

**What runs where.**

- **CI**, the whole of `run-ci.sh`:
  - `cargo test -p everygraph-oracle`: unit tests, 6 properties at 96 cases each, the
    seeded-bug tests on the CI corpus, and the karate golden file;
  - the CI corpus through the Python references, the in-process references, the toy and
    the seeded bugs;
  - the semantics probe and the golden check of whatever is cached.

  It needs no network after `uv sync`.
- **Nightly**, `run-nightly.sh`:
  - the nightly corpus: 1,572 graphs, up to 20,000 nodes and 100,000 rows, hubs with 2^16
    neighbours;
  - 2,000 cases per property;
  - the download and checksum check of the SNAP sources, and their golden files;
  - the lab prototype through its adapter.

Measured runtimes are in the pull request that introduced the oracle (#6). They were
taken on a loaded machine, so they are upper bounds, not benchmarks.

## Exchange formats

One directory per graph, written by `everygraph-oracle corpus` and read by Rust, by
Python and by external kernels (`exchange.rs`, `graph.py`):

- **`graph.json`** holds:
  - `format` (`"everygraph-oracle-graph"`) and `version` (1);
  - `name`, `n` (isolated nodes included), `directed`, `rows`;
  - `columns` (`src`, `dst`, then optional `weight` and `time`);
  - `semantics` (`{"self_loops": "drop" | "keep"}`);
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
  - `ingest`: the Python side's own count of rows, self-loop rows, duplicate rows, pairs
    and isolates;
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
| `quotient` | community `sizes` and weighted `edges` |

**External kernels** (`external.rs`) are programs run once per (graph, kernel,
configuration) as `<program> [args] --graph DIR --kernel '<json>' --config '<json>'`.
They print one output as JSON. This is how the lab prototype is checked today
(`lab/b1-core/crates/glab-oracle`), and how the Python binding can be checked from M2.

## Semantics

Kernels run on the *analytic graph* (`projection.rs`, `graph.py`), the rules of
`plan/issues/m2-09-graph-semantics.md` applied to the rows:

- self-loops are dropped and counted, unless the graph keeps them;
- a repeated pair is analysed once, with its multiplicity, summed weight and smallest
  weight;
- the undirected projection of a directed graph has {u, v} if either arc exists.

The ingest counts add up to the rows (a property test checks it). Where NetworkX and
igraph disagree, [`SEMANTICS.md`](SEMANTICS.md) records the case, what each does and what
Everygraph does. It ends with the open questions the coordinator must confirm.

## Tolerances

`tolerances.json` is read by both halves. A real `a` passes against a reference `b` when
`|a − b| ≤ abs + abs_over_n / n + rel·|b|`. Integers, labels, counts and adjacency are
compared exactly; results of one kernel under two configurations are compared bit for
bit.

| Quantity | Tolerance | Why |
|---|---|---|
| PageRank, per node | `rel = 1e-9`, `abs_over_n = 1e-9` | The references are at the fixed point to about 10⁻¹⁵ (PRPACK; NetworkX at `tol = 1e-15`; the in-process power iteration shrinks the initial error by 10⁻¹⁷). Values scale as 1/n, so a bare absolute bound would be loose on large graphs and meaningless on small ones: the bound is 10⁻⁹ relative, floored at 10⁻⁹ of the uniform value 1/n. m3-04 asks for agreement "within 1e-9"; this is that, made scale-free. On the CI corpus every comparison passes, including those of the lab prototype, which runs a fixed 300 iterations |
| PageRank, sum | `abs = 1e-9` | n additions of values that sum to 1 in exact arithmetic |
| local clustering | `abs = 1e-12` | `2t / (d(d−1))` is one division of exact integers, so every correct implementation gives the same correctly rounded double. The slack only covers a different but equivalent formula |
| transitivity | `abs = 1e-12` | same reason, one division |
| modularity | `abs = 1e-10` | a sum over communities whose order differs between libraries: about n·ε relative error, which is below 10⁻¹¹ for n ≤ 10⁵ |
| weighted distances | `rel = 1e-12`, `abs = 1e-12` | sums along paths; two shortest paths of equal length may round differently. The corpus mostly uses dyadic weights (multiples of 1/16), whose sums are exact |

## Golden files

`golden/*.json` hold known answers on small public graphs: node and pair counts,
self-loop and duplicate rows, components (weak and strong), triangles (total and per
node) and core numbers (per node, maximum, histogram). Every value was computed by at
least two of NetworkX, igraph and scipy, which agreed, and is stored with the library
versions, the source's URL and SHA-256, and SNAP's published figure where SNAP gives one.

| Graph | Source | n | m | Triangles (= published) | Notes |
|---|---|---|---|---|---|
| karate | vendored (`golden/karate.tsv`; the 78-edge Zachary graph, identical in NetworkX and igraph) | 34 | 78 | 45 | |
| email-Eu-core | SNAP, directed | 1,005 | 24,929 | 105,461 (yes) | SNAP's 25,571 edges include 642 self-loops |
| ca-GrQc | SNAP | 5,242 | 14,484 | 48,260 (yes) | SNAP's 14,496 edges include 12 self-loops |
| ego-Facebook | SNAP | 4,039 | 88,234 | 1,612,010 (yes) | |
| p2p-Gnutella08 | SNAP, directed | 6,301 | 20,777 | 2,383 (yes) | |

The SNAP files (63–219 KB compressed) are not redistributed. `uv run oracle-ref golden
--fetch` downloads them into `oracle/.cache/` (ignored by git;
`$EVERYGRAPH_ORACLE_CACHE` overrides it) and verifies their SHA-256. Checks of a file
whose source is missing are skipped, not failed. **When the dataset registry (#5)
lands**, the `DATASETS` list in `golden.py` moves into its `datasets.toml`, `--fetch`
calls the registry's fetcher, and each golden file's `source` block gains the registry
entry's name. The tier-1 graphs (lj and others) then get golden files too, with their
published triangle counts as a third witness (m3-08).

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
