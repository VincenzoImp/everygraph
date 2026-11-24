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

One workstation (Ryzen 9 7950X, 61 GB RAM, NVMe), idle machine unless noted.
Details, methods and caveats are in [docs/FINDINGS.md](docs/FINDINGS.md).

| Question | Result |
|---|---|
| Text edge list → graph | 35 M edges in 0.42 s; 117 M in 0.89 s; 1.8 B in 21 s (52 s with an 8 GB budget) |
| PageRank on 1.8 B edges | 3.4 s/iteration in RAM; 3.7 s/iteration under a 2 GB memory limit |
| Kernels vs NetworKit (117 M edges) | PageRank 0.37 s vs 3.42 s; components 13 ms vs 302 ms (reference measured on a shared machine) |
| Communities (Leiden, 35 M edges) | 0.76 s at modularity 0.750; 2.0 s at 0.765 with three iterations (best reference: 0.770 in 6.0 s) |
| Determinism | identical partition, layout and map file at 1–32 threads; costs 2.4–3× for Leiden |
| Storage | 16–17 bits/edge on social graphs, scanning faster than raw arrays at high thread counts |
| Shortest path between two nodes | 0.3–1.5 ms in RAM (Neo4j 6.4 ms; embedded engines 0.3–4.8 s on the same graph) |
| Map of a 4 M-node graph | layout 3.3 s, tiles 8.5 s; first view 152 KB; search in 2 requests |
| Same core in the browser | 35 M edges end to end in ~3 s in Chromium |

## What is not solved yet

Community detection beyond RAM (Leiden needs 23.6 GB on the 1.8 B-edge graph), the quality
of the map's hierarchy and edges on dense graphs, and the cost of determinism. The headline
"a billion edges to a map on a 16 GB laptop" has not been run end to end. These are the
subject of the first work milestone. See [docs/DESIGN.md § 5](docs/DESIGN.md).

## Where to look

- [docs/DESIGN.md](docs/DESIGN.md) — product, architecture and 23 decisions with their evidence
- [docs/FINDINGS.md](docs/FINDINGS.md) — what the lab measured, including negative results
- [docs/ROADMAP.md](docs/ROADMAP.md) — milestones and their exit criteria
- [plan/](plan/) — every issue as a file, and the script that publishes them
- [lab/](lab/) — the prototypes behind the numbers (research code, not the product)

## Licence

Dual-licensed under Apache-2.0 or MIT, at your option.
