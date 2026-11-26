# Roadmap

Two goals order the work:

1. **Version 0.1, when the product is really usable:** a file becomes a graph, the kernels
   and community detection run inside a memory budget, the map opens in a browser, and
   the command line and the Python package are installable.
2. **Eligibility for the full LDBC Graphalytics suite on one machine:** all six algorithms
   on every official dataset up to graph500-30 (447 M nodes, 17 B edges), which does not
   fit in this machine's memory.

The issues are in `plan/issues/` and on GitHub. `python3 plan/next.py` lists the issues
whose dependencies are closed.

## Milestones

Work is ordered by risk: what could invalidate the product comes first.

| Milestone | What it delivers | Exit criterion |
|---|---|---|
| **M0 · Foundations and decisions** | Workspace, CI, benchmark and correctness harnesses, ratified decisions, legal checks | Clone, build and test with one command; every decision has an ADR |
| **M1 · De-risking lab, round 2** | Experiments on what the first lab left open | Every open risk has a number and a decision |
| **M2 · Format and ingest** | `.eg` container, compressed adjacency, I/O layer, ingest for integer, hex and string ids | Friendster → `.eg` in < 60 s at an 8 GB budget |
| **M3 · Engine and kernels** | Planner, memory accountant, edge-map framework, core kernels | Identical results in every regime and thread count; never over budget |
| **M4 · Communities and hierarchy** | Production Leiden, out-of-core, quality gates, map hierarchy | Friendster communities under 16 GiB, quality mode within 1% of the better of NetworKit PLM and GVE-Leiden run on the same machine (DESIGN § 6) |
| **M5 · The map** | Layout, tiles, `.egmap`, viewer, static export | A 1 B-edge graph opens from a static URL, first view < 250 KB |
| **M6 · Python and interop** | Python package, Arrow interop, import/export, notebook widget | `pip install` on three OSes; zero-copy into Polars/pandas/DuckDB |
| **M7 · Queries and views** | Point queries, shortest paths, filtered and temporal views, live mode of the map | Neighbours < 1.5 µs p99 in RAM; path p50 < 2 ms on 10^9 arcs |
| **M8 · CLI, docs and v0.1 launch** | CLI, docs, public benchmarks, showcase maps, Graphalytics eligibility, release | v0.1.0 published with reproducible benchmarks |
| **M9 · After v0.1** | Browser tier, MCP server, query language, temporal segments, NetworkX backend, sustainability | Unscheduled |

Every milestone ends with a dogfood pass, a cleanup and test audit, one official
measurement window and a report in `docs/reports/` (CLAUDE.md § 6, § 10).

## Dependencies between milestones

```
M0 ─► M2 ─► M3 ─► M4 ─► M5 ─► M7 ─► M6 ─► M8
 └─► M1 ───────────┴─────┘
```

The line is the order in which milestones can end, not the order in which they start.

- M1 runs beside M2 and M3: it uses the lab prototypes, not production code. Its results
  gate parts of M4 (quality on six graph families) and M5 (edges, layout area, frame
  budgets).
- M5 is the static map: layout, tiles, file format, viewer, export. It needs M4.
- M6 and M7 start once M3 exists and run beside M4 and M5. They end after M5: the Jupyter
  widget (M6) and the live mode and overlays of the map (M7) need the viewer. M6 ends
  after M7: its tutorials and its subgraph bridge use queries and views.
- A first slice of the Python package and of the command line exists from M2, so that
  tests and users can drive the engine; M6 and M8 finish them.

## Critical path

To 0.1, the longest chain of issues that each wait for the one before:

```
production Leiden (#57) ─► hierarchy artifact (#62) ─► map hierarchy (#63) ─► layout (#67)
  ─► tile builder (#69) ─► viewer core (#71) ─► viewer experience (#72) ─► honest map (#78)
  ─► showcase map (#101) ─► release (#105)
```

Beside it, and needed before the release:

- the engine under a budget: the build's columns (#246) and the allocator policy (#167);
  the planner (#45) and the semi-external source (#46); the no-OOM and determinism checks
  in CI (#53, #54); weights (#58), then Leiden beyond RAM (#60);
- the tile format (#68), which waits for the edge experiment (#20);
- queries: point queries (#90), shortest paths (#91), views (#93);
- the packages: Python API (#81), Arrow interop (#83), wheels (#82), the rest of the
  command line (#247);
- the documentation site (#99) and the public benchmark report (#100).

To Graphalytics eligibility (#209):

```
in-adjacency (#38) + columns (#246) ─► semi-external source (#46)
  ─► LCC (#51), CDLP (#65), BFS and SSSP past RAM (#240) ─► block cache (#178) ─► #209
```

Three items on these paths need people or hardware this host does not have: the edge
tasks with three to five users (#20), the usability check of the viewer (#72), and the
frame budgets on an integrated GPU and on Safari (#26). They are listed in
`docs/STATE.md`, "Needs the owner's hands".

## Effort

Estimates for one full-time engineer, given by the agents that built each prototype. None
has been tested against real progress, and estimates made right after a successful
prototype tend to be low.

| Area | Estimate |
|---|---|
| Ingest, kernel framework, memory accountant, planner | 4–5 months |
| Adjacency format reader/writer and streaming | 3–5 weeks |
| Community detection beyond the prototype | 3–4 weeks |
| Map: layout, tiles, viewer | 6–10 weeks |
| Dictionary, out-of-core queries, columns and views | ~3 months |
| **Sum of the above** | **about 11–13 months** |

Not in the sum: M0, M1, the Python API and wheels, the CLI, documentation, the showcase
maps and the release. With them, 0.1 is an estimated 16–21 months for one engineer. The
NetworkX backend (a further 2–3 months, estimate) is after 0.1 and not counted. The
project is built by parallel agents, so these figures size the work; they are not a
schedule.

## The three risks that decide the plan

1. **Out-of-core community detection.** If Leiden cannot run on a 1.8 B-edge graph inside
   16 GiB with good quality, the laptop claim shrinks to "analyze yes, map only with
   enough RAM". Measured feasible in the lab for one case (DESIGN § 5, item 1); the
   product is #60.
2. **Map usefulness.** Hierarchy and edges must carry meaning, not only render. Tested
   with user tasks, not only metrics (#20, #72).
3. **One maintainer.** Answered by scope discipline (each milestone ships something
   usable) and by funding and co-maintainer work that starts before the release (#115).
