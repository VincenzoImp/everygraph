# Everygraph

**Every graph, every size, every machine.** One embeddable engine to ingest, analyze,
query and map graphs from a thousand to a hundred billion edges on a single computer —
using all the hardware it is given, and staying inside the memory budget it is given.

> **Status: early development.** There is no installable software here yet. This
> repository holds the product and architecture baseline, the evidence from a feasibility
> lab, the prototypes that produced it, the full work plan as GitHub milestones and
> issues, and the first production crates (file format, I/O, readers).

![A map of com-Orkut (3.1 M nodes, 117 M edges) from the lab prototype](docs/img/map-overview-orkut.png)

## Why

Graph work today means choosing between tools that are comfortable but do not scale and
tools that scale but are not comfortable, then chaining several of them to prepare,
analyze, query and look at one graph. Nothing free does all four at any size on one
machine.

## Three promises

1. **Open anything fast** — a file becomes an analysis-ready graph at about 100 M edges/s
   while what the ingest writes fits in memory or in the drive's fast-write burst; an
   ingest that writes more goes at the drive's sustained write rate (about 60 M edges/s
   for 1.8 B edges on the lab machine).
2. **Never run out of memory** — you set a budget; the engine plans first and stays inside it.
3. **See it and trust it** — a zoomable map of the whole graph, exact numbers at every
   level, identical results at any thread count, shareable as one static file.

## What is new, and what is not

Most of the parts have been published before. Prior art includes:

- hierarchical graph maps served as data tiles (Uncharted's Aperture Tiles, 2015);
- a fan-out cap of 64 (ASK-GraphView, 2006);
- compressed adjacency that is faster than raw arrays in parallel (Ligra+, 2015);
- recursive re-clustering of oversized communities, Elias–Fano offsets, and seeded node
  relabelling;
- estimating memory and refusing a run before it starts (Neo4j GDS).

What we claim is the combination, and what holds it together:

- identical results at any thread count, memory budget and execution regime — new for
  a graph engine as far as we read, though the idea exists elsewhere: sums independent of
  how the work is decomposed in high-performance computing (Hallberg and Adcroft, 2014)
  and "data independence" for floating-point aggregates in databases (Müller et al.,
  2018);
- a deterministic, budgeted build from text to graph;
- a map of a 1.8 B-edge graph on one machine under a memory budget, with stated omissions
  (the aim: not yet run end to end, see below).

The literature reviews behind this are in [docs/research/](docs/research/README.md).

## What the feasibility lab measured

One workstation (Ryzen 9 7950X, 61 GB RAM, NVMe). Rows marked † were measured while other
experiments shared the machine and are noisy; the rest are idle-machine runs. Unless a row
is marked ‡, it was measured on the default storage — compressed adjacency, graph in RAM —
with the same kernel on uncompressed arrays given next to it where the two differ; rows
marked ‡ were measured on uncompressed arrays only and not re-measured. These are research
prototypes, reading plain text, without `fsync`. Details, methods and caveats are in
[docs/FINDINGS.md](docs/FINDINGS.md) (the default storage: § M1-13).

| Question | Result |
|---|---|
| Text edge list → graph ‡ | to uncompressed arrays, 32 threads, through the benchmark harness: 35 M edges in 0.39 s and 117 M in 0.78 s (90 M and 150 M edges/s, median of 6); 1.8 B in 29 s with a 32 GB budget and 37 s with 8 GB when ingests run back to back (62 M and 49 M edges/s, median of 3), bounded by the drive's sustained write rate, not the CPU — 17.7–19.5 s (92–102 M edges/s) on a rested drive. Encoding them to the default storage took a further 0.06 s, 0.17 s and 3.4 s at 16 threads, measured separately from ingest |
| PageRank on 1.8 B edges | 3.2 s/iteration in RAM on the default storage (uncompressed: 3.4), f32, 16 threads. Under a 2 GB memory limit: 3.6 s/iteration with the uncompressed-format streaming reader (32 threads) ‡; the default storage has no streaming reader yet and takes 4.3 s/iteration through plain mmap (16 threads). Cheap single-pass kernels do not get this: components take 0.50 s in RAM on the default storage (uncompressed: 0.36, 16 threads) and 3.6 s out of core ‡, bounded by reading the file |
| Kernels vs NetworKit (117 M edges) | lab at 16 threads, NetworKit at 32: PageRank (f64, 20 iterations) 0.31 s (uncompressed: 0.42 s) vs 2.79 s; components 17 ms (uncompressed: 12 ms) vs 247 ms; k-core 0.22–0.36 s in either storage (run-to-run spread) vs 1.23 s |
| Communities (Leiden, 35 M edges) | one iteration, 16 threads: 0.79 s at modularity 0.749 on the default storage (uncompressed: 0.76 s at 0.750); GVE-Leiden, 32 threads: 1.02 s at 0.722. Three iterations: 2.0 s at 0.765 ‡ (NetworKit: 6.0 s at 0.769) |
| Determinism | identical partition at 1–32 threads ‡ and on uncompressed and compressed storage (layout and map file: † shared machine); costs 2.4–3.0× for Leiden at 16 threads ‡ |
| Storage | 16.1–18.1 bits/edge on LiveJournal, Orkut and twitter-2010, 23.2 on Friendster (uncompressed: 32.8–35.7); at 16 threads scans run faster than uncompressed arrays on all four (14.9–27.5 vs 12.4–13.7 G entries/s), at 1 thread 2.2–2.5× slower |
| Shortest path between two nodes | in-process, single thread, median: 0.16–0.17 ms on LiveJournal on the default storage (uncompressed: 0.13–0.14 ms); 0.56–0.59 ms on twitter-2010 (uncompressed: 0.44–0.47 ms). Neo4j 6.4 ms and embedded engines 0.3–4.8 s on LiveJournal, measured through their query interfaces at 3 threads † |
| Map of a 4 M-node graph † ‡ | layout 3.3 s, tiles 8.5 s; first view 152 KB; search in 2 requests |
| Same core in the browser † ‡ | 35 M edges end to end in ~3 s in Chromium |

## What is not solved yet

Community detection beyond RAM works in the lab but not yet in the product: one Leiden
iteration on the 1.8 B-edge graph runs under a 16 GiB limit at 1.17× and under 8 GiB at
2.34× the time of the same algorithm in RAM (idle machine, 16 threads, default storage,
0 disconnected communities, the deterministic partition identical to the in-RAM one), but
only that graph and one iteration were measured. Also open: the quality of the map's
hierarchy and edges on dense graphs, and the cost of determinism. The headline
"1.8 B edges to a map on a 16 GB laptop" has not been run end to end: the tile builder
does not yet fit 16 GiB, and the layout fits only with the graph memory-mapped. The second lab
round (milestone M1) took these on first; see [docs/DESIGN.md § 5](docs/DESIGN.md#5-what-is-not-solved-yet).

## Building

The Rust workspace holds the crate layout, the coding rules, the portable core
(`crates/everygraph-core`), and the first product pieces: the `.eg` container and the
compressed adjacency (`everygraph-format`), the I/O layer (`everygraph-io`), and the text
reader and identifier dictionary (`everygraph-ingest`). It needs Rust 1.87 or newer and
nothing else.

```sh
cargo build --workspace
cargo test --workspace
cargo build -p everygraph-core --target wasm32-unknown-unknown --no-default-features
```

The full list of checks a change must pass, and the rules behind them (portability of
the core, integer widths, `unsafe`, licences), are in [CONTRIBUTING.md](CONTRIBUTING.md).

## Where to look

- [docs/DESIGN.md](docs/DESIGN.md) — product, architecture and 25 decisions with their evidence (one ADR each under `docs/adr/`; ADR 0026 words promise 1)
- [docs/FINDINGS.md](docs/FINDINGS.md) — what the lab measured, including negative results
- [docs/ROADMAP.md](docs/ROADMAP.md) — milestones and their exit criteria
- [plan/](plan/) — every issue as a file, and the script that publishes them
- [crates/](crates/) — the Rust workspace; rules in [CONTRIBUTING.md](CONTRIBUTING.md)
- [lab/](lab/) — the prototypes behind the numbers (research code, not the product)

## Licence

- **Code** — every source file, script and build file, everything under `crates/` and
  `lab/`, and code samples wherever they appear — is dual-licensed under
  [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option.
- **Documentation and format specifications** — `docs/`, `design/` and the Markdown
  files outside `crates/` and `lab/` — are under [CC BY 4.0](LICENSE-DOCS), which lists
  the exact paths.
- **Third-party files** (the fonts of the design previews, one generated script in the
  browser experiment) keep their own licences: see
  [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).

The format specifications are public and versioned, and no feature will be held back for
a paid edition. Everygraph has no telemetry and will not get any: the software sends
nothing anywhere unless you ask it to.

Contributions come in under the same licences, with a Developer Certificate of Origin
sign-off and no contributor licence agreement ([CONTRIBUTING.md](CONTRIBUTING.md)). How
the project is run, and by whom, is in [GOVERNANCE.md](GOVERNANCE.md); how to report a
vulnerability is in [SECURITY.md](SECURITY.md).
