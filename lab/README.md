# lab — research prototypes and experiments

Research code written to answer specific questions. It is the evidence behind
`docs/FINDINGS.md`, not the product: it contains absolute paths (`/home/tux/graphlab`),
lab-only build flags (`target-cpu=native`), lightly tested options and `unsafe`
shared-memory code that has not been through Miri or ThreadSanitizer.

**Frozen.** A prototype is frozen once its production counterpart lands (CLAUDE.md § 6):
it is not extended, and bugs in it are not fixed. Some binaries stay in use as the A/B
reference of a benchmark spec (`bench/specs/*.toml`), so they must keep building.

The first lab (a few hours per experiment):

| Directory | Question | FINDINGS | Production counterpart | Reference in |
|---|---|---|---|---|
| `a-baselines` | What do NetworKit, igraph, GVE and DuckDB do on this machine? | § B1, § B2, § Idle-machine re-runs | none (reference tools) | `baselines.toml` |
| `b1-core` | Ingest, kernels, larger-than-RAM execution | § B1 | text reader #32, I/O #29, the build #35, PageRank, BFS and SSSP (#47, #49), components, core numbers and degrees (#48, #50) landed; triangles and centralities not yet | `headline.toml`, `tracer.toml` |
| `b2-leiden` | Parallel and deterministic Louvain/Leiden, hierarchy | § B2 | not yet (#57) | `tracer.toml` |
| `c-compression` | Adjacency codecs, ordering, cold scans | § C | codec #30 landed; ordering not yet | — |
| `e-map` | Layout, tile pyramid, PMTiles container, WebGL2 viewer | § E | not yet (#67–#71) | `tracer.toml` |
| `f1-query` | Id dictionary, point queries, properties, views | § F1 | dictionary #34, property columns #37 landed; queries not yet | `id-dictionary.toml` |
| `f2-python` | Query-engine comparison, PyO3 binding, NetworkX backend | § F2 | the first slice of the Python package (part of #81) landed; NetworkX backend not yet (#86) | `engines.toml` |
| `g-wasm` | The same core in the browser | § G | not yet (#108) | — |

Second-round experiments and measurements of production code:

| Directory | Issue | FINDINGS | Reference in |
|---|---|---|---|
| `m1-01-tracer` | #15, Friendster from text to a map | § M1-01 | `tracer.toml` |
| `m1-02-ooc-leiden` | #16, out-of-core Leiden | § M1-02 | `ooc-leiden.toml` |
| `m1-03-louvain-collapse` | #17, the async Louvain collapse on Friendster | § M1-03 | — |
| `m1-04-det-leiden-scaling` | #18, the sub-round rule of deterministic Leiden, A/B | its own `README.md` (exploratory) | `det-leiden.toml` |
| `m1-05-hierarchy-shaping` | #19, a hierarchy the map can use | § M1-05 | `hierarchy-shaping.toml` |
| `m1-06-edge-rendering` | #20, connections without a selection (the preview `design/preview/style/overview.html`); popping in the lab tiles | `design/DECISIONS.md` (2026-11-26), `results/` (exploratory) | — |
| `m1-09-hierarchy-order` | #23, node orders against our codec and kernels | its own `results/README.md` (exploratory) | `hierarchy-order.toml` |
| `m1-12-viewer-no-gpu` | #26, the lab viewer without a discrete GPU: entities against pixels, drawing modes in `e-map/viewer` | `results/` (exploratory) | — |
| `m1-13-compressed` | #118, the default storage end to end | § M1-13 | — |
| `m1-15-mmap-hints-block-cache` | #172, mmap hints and a budgeted block cache | ADR 0029, `results/README.md` | `mmap-hints.toml` |
| `m1-18-layout-baseline` | #175, our map layout against published large-graph layouts | § M1-18 | — |
| `m1-20-ldbc-graphalytics` | #177, LDBC Graphalytics as oracle and workload | § M1-20 | — |
| `m2-03-codec` | #30, the production codec measured | `docs/spec/eg-adjacency.md` § 7, § 8 | `codec.toml` |
| `m2-04-segvec` | #31, segmented arrays against `Vec` | ADR 0002 | `segvec.toml` |
| `m2-08-build` | #35, the size of `.events` on a temporal graph (results only) | `docs/spec/eg-adjacency.md` § 7 | — |
| `m2-28-par-scatter` | #189, safe scattered writes, A/B | `plan/issues/m2-28-par-scatter.md` | — |
| `m3-27-frontier-counters` | #266, frontier driver and decode counters | `plan/issues/m3-27-frontier-counters.md` | — |

Raw result files are under each `results/` directory; datasets and build outputs are not
included. The first lab's prototypes read graphs in a format of their own, `glab-csr-v0`,
one directory per graph under `$EVERYGRAPH_DATA/csr/`: `meta.json` (n, edges, flags),
`offsets.u64` (n + 1 little-endian values), `targets.u32` (symmetric, sorted per node, no
duplicates, no self-loops) and `ids.u64` (the input id of each dense id).

Only `e-map` has its own `REPORT.md`; `m1-03-louvain-collapse` and
`m1-04-det-leiden-scaling` have a `README.md`, and `m1-15-mmap-hints-block-cache` and
`m1-09-hierarchy-order` and `m1-18-layout-baseline` a results `README.md`. The other experiments are reported in
`docs/FINDINGS.md`. Some source headers still say "see REPORT.md": that report is the
FINDINGS section of the table above.
