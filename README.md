# Everygraph

**Every graph, every size, every machine.** One embeddable engine to ingest, analyze,
query and map graphs from a thousand to a hundred billion edges on a single computer —
using all the hardware it is given, and staying inside the memory budget it is given.

> **Status: design and planning.** There is no installable software here yet. This
> repository holds the product and architecture baseline, the evidence from a feasibility
> lab, the prototypes that produced it, and the full work plan as GitHub milestones and
> issues.

![A map of com-Orkut (3.1 M nodes, 117 M edges) from the lab prototype](docs/img/map-overview-orkut.png)

## Why

Graph work today means choosing between tools that are comfortable but do not scale and
tools that scale but are not comfortable, then chaining several of them to prepare,
analyze, query and look at one graph. Nothing free does all four at any size on one
machine.

## Three promises

1. **Open anything fast** — a file becomes an analysis-ready graph at about 100 M edges/s.
2. **Never run out of memory** — you set a budget; the engine plans first and stays inside it.
3. **See it and trust it** — a zoomable map of the whole graph, exact numbers at every
   level, identical results at any thread count, shareable as one static file.

## What the feasibility lab measured

One workstation (Ryzen 9 7950X, 61 GB RAM, NVMe). Rows marked † were measured while other
experiments shared the machine and are noisy; the rest are idle-machine runs. These are
research prototypes working on uncompressed arrays, from plain text, without `fsync`.
Details, methods and caveats are in [docs/FINDINGS.md](docs/FINDINGS.md).

| Question | Result |
|---|---|
| Text edge list → graph | 35 M edges in 0.42 s; 117 M in 0.89 s; 1.8 B in 21 s (52 s with an 8 GB budget) |
| PageRank on 1.8 B edges | 3.4 s/iteration in RAM; 3.7 s/iteration under a 2 GB memory limit. Cheap single-pass kernels do not get this: components take 0.34 s in RAM and 3.6 s out of core, bounded by reading the file |
| Kernels vs NetworKit (117 M edges) | PageRank (f64, 20 iterations) 0.43 s vs 2.79 s; components 13 ms vs 247 ms; k-core 0.28 s vs 1.23 s |
| Communities (Leiden, 35 M edges) | 0.76 s at modularity 0.750 (GVE-Leiden: 1.02 s at 0.722); 2.0 s at 0.765 with three iterations (NetworKit: 6.0 s at 0.769) |
| Determinism | identical partition, layout and map file at 1–32 threads; costs 2.4–3× for Leiden |
| Storage † | 16–17 bits/edge on social graphs (raw: 33); at 32 threads scans faster than raw arrays on twitter-2010, slower on Orkut |
| Shortest path between two nodes † | 0.3 ms median on LiveJournal, in-process (Neo4j 6.4 ms and embedded engines 0.3–4.8 s on the same graph, measured through their query interfaces); 1.5 ms on twitter-2010 |
| Map of a 4 M-node graph † | layout 3.3 s, tiles 8.5 s; first view 152 KB; search in 2 requests |
| Same core in the browser † | 35 M edges end to end in ~3 s in Chromium |

## What is not solved yet

Community detection beyond RAM (Leiden needs 23.6 GB on the 1.8 B-edge graph), the quality
of the map's hierarchy and edges on dense graphs, and the cost of determinism. The headline
"a billion edges to a map on a 16 GB laptop" has not been run end to end. These are the
subject of the first work milestone. See [docs/DESIGN.md § 5](docs/DESIGN.md).

## Building

The Rust workspace is a skeleton: the crate layout, the coding rules, and the first
pieces of the portable core (`crates/everygraph-core`). It needs Rust 1.85 or newer and
nothing else.

```sh
cargo build --workspace
cargo test --workspace
cargo build -p everygraph-core --target wasm32-unknown-unknown --no-default-features
```

The full list of checks a change must pass, and the rules behind them (portability of
the core, integer widths, `unsafe`, licences), are in [CONTRIBUTING.md](CONTRIBUTING.md).

## Where to look

- [docs/DESIGN.md](docs/DESIGN.md) — product, architecture and 23 decisions with their evidence
- [docs/FINDINGS.md](docs/FINDINGS.md) — what the lab measured, including negative results
- [docs/ROADMAP.md](docs/ROADMAP.md) — milestones and their exit criteria
- [plan/](plan/) — every issue as a file, and the script that publishes them
- [crates/](crates/) — the Rust workspace; rules in [CONTRIBUTING.md](CONTRIBUTING.md)
- [lab/](lab/) — the prototypes behind the numbers (research code, not the product)

## Licence

Dual-licensed under Apache-2.0 or MIT, at your option.
