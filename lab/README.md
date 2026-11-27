# lab — research prototypes and experiments

Research code written to answer specific questions. It is the evidence behind
`docs/FINDINGS.md`, not the product: it contains absolute paths (`/home/tux/graphlab`),
lab-only build flags (`target-cpu=native`), lightly tested options and `unsafe`
shared-memory code that has not been through Miri or ThreadSanitizer.

**Frozen.** A prototype is frozen once its production counterpart lands (CLAUDE.md § 6),
and an experiment is frozen once its issue closes: it is not extended, and bugs in it are
not fixed. The "Frozen" column says which parts are frozen. Some binaries stay in use as
the A/B reference of a benchmark spec (`bench/specs/*.toml`, the "Reference in" column),
so they must keep building.

The first lab (a few hours per experiment):

| Directory | Question | FINDINGS | Production counterpart | Frozen | Reference in |
|---|---|---|---|---|---|
| `a-baselines` | What do NetworKit, igraph, GVE and DuckDB do on this machine? | § B1, § B2, § Idle-machine re-runs | none (reference tools) | yes | `baselines.toml` |
| `b1-core` | Ingest, kernels, larger-than-RAM execution | § B1 | text reader #32, I/O #29, the build #35, PageRank, BFS and SSSP (#47, #49), components, core numbers and degrees (#48, #50) | yes, except triangles (#51) and centralities (#52) | `headline.toml`, `tracer.toml`, `kernels.toml`, `edge-map.toml`, `components-kcore.toml` |
| `b2-leiden` | Parallel and deterministic Louvain/Leiden, hierarchy | § B2 | Leiden #57, weights #58, parallel aggregation #59 | yes, except the hierarchy (#62, #63) | `tracer.toml`, `leiden.toml`, `det-leiden.toml`, `hierarchy-order.toml`, `mmap-hints.toml`, `references.toml` |
| `c-compression` | Adjacency codecs, ordering, cold scans | § C | codec #30 | yes, except node ordering (#179) | — |
| `e-map` | Layout, tile pyramid, PMTiles container, WebGL2 viewer | § E | the `.egmap` format #68 | no: layout, tiles and viewer (#67, #69–#71) | `tracer.toml`, `mmap-hints.toml`, `hierarchy-shaping.toml` |
| `f1-query` | Id dictionary, point queries, properties, views | § F1 | dictionary #34, property columns #37 | yes, except queries and views (#90, #93) | `id-dictionary.toml`, `mmap-hints.toml` |
| `f2-python` | Query-engine comparison, PyO3 binding, NetworkX backend | § F2 | the first slice of the Python package (part of #81) | no: the Python API (#81) and the NetworkX backend (#86) | `engines.toml` |
| `g-wasm` | The same core in the browser | § G | none yet | no: the browser tier (#108) | — |

Second-round experiments and measurements of production code. Every one is frozen, as its
issue is closed, except `m3-12-reference-runs`, whose issues are open:

| Directory | Issue | FINDINGS | Reference in |
|---|---|---|---|
| `m1-01-tracer` | #15, Friendster from text to a map | § M1-01 | `tracer.toml`, `mmap-hints.toml` |
| `m1-02-ooc-leiden` | #16, out-of-core Leiden | § M1-02 | `ooc-leiden.toml`, `det-leiden.toml`, `hierarchy-shaping.toml`, `mmap-hints.toml` |
| `m1-03-louvain-collapse` | #17, the async Louvain collapse on Friendster | § M1-03, its own `README.md` | — |
| `m1-04-det-leiden-scaling` | #18, the sub-round rule of deterministic Leiden, A/B | its own `README.md` (exploratory) | `det-leiden.toml`, `leiden.toml` |
| `m1-05-hierarchy-shaping` | #19, a hierarchy the map can use | § M1-05 | `hierarchy-shaping.toml` |
| `m1-06-edge-rendering` | #20, connections without a selection (the preview `design/preview/style/overview.html`); popping in the lab tiles | `design/DECISIONS.md` (2026-11-26), `results/` (exploratory) | — |
| `m1-07-layout-area` | #21, layout area and the gap between top regions | its own `results/README.md` | — |
| `m1-09-hierarchy-order` | #23, node orders against our codec and kernels | its own `results/README.md` (exploratory) | `hierarchy-order.toml` |
| `m1-12-viewer-no-gpu` | #26, the lab viewer without a discrete GPU: entities against pixels, drawing modes in `e-map/viewer` | `results/` (exploratory) | — |
| `m1-13-compressed` | #118, the default storage end to end | § M1-13 | — |
| `m1-15-mmap-hints-block-cache` | #172, mmap hints and a budgeted block cache | ADR 0029, its own `results/README.md` | `mmap-hints.toml` |
| `m1-18-layout-baseline` | #175, our map layout against published large-graph layouts | § M1-18, its own `results/README.md` | — |
| `m1-20-ldbc-graphalytics` | #177, LDBC Graphalytics as oracle and workload | § M1-20 | — |
| `m2-03-codec` | #30, the production codec measured | `docs/spec/eg-adjacency.md` § 7, § 8 | `codec.toml` |
| `m2-04-segvec` | #31, segmented arrays against `Vec` | ADR 0002 | `segvec.toml` |
| `m2-08-build` | #35, the size of `.events` on a temporal graph (results only) | `docs/spec/eg-adjacency.md` § 7 | — |
| `m2-28-par-scatter` | #189, safe scattered writes, A/B | `plan/issues/m2-28-par-scatter.md` | — |
| `m2-35-random-read-probe` | #263, random-read rate by size and queue depth, `pread` pool against io_uring | its own `README.md` (exploratory) | — |
| `m3-12-reference-runs` | #55 and #265 (open), production kernels against the best-known kernel codes | its own `README.md` | `references.toml` |
| `m3-27-frontier-counters` | #266, frontier driver and decode counters | its own `README.md` | — |
| `m5-02-tile-format` | #68, the tile payload against MapLibre Tile and Arrow IPC on the lab tiles (bytes only) | `docs/spec/egmap.md` § 10, its own `README.md` (exploratory) | — |

Raw result files are under each `results/` directory; datasets and build outputs are not
included. The first lab's prototypes read graphs in a format of their own, `glab-csr-v0`,
one directory per graph under `$EVERYGRAPH_DATA/csr/`: `meta.json` (n, edges, flags),
`offsets.u64` (n + 1 little-endian values), `targets.u32` (symmetric, sorted per node, no
duplicates, no self-loops) and `ids.u64` (the input id of each dense id).

Of the first lab, only `e-map` has a report of its own (`REPORT.md`); the others are
reported in `docs/FINDINGS.md`. Source headers that say "see REPORT.md" mean the FINDINGS
section of the first table.
