# Roadmap

Ten milestones. Each ends with something that can be shown and measured. Work is ordered by
risk: what could invalidate the product comes first. The issues are in `plan/issues/` and
on GitHub.

| Milestone | What it delivers | Exit criterion |
|---|---|---|
| **M0 · Foundations and decisions** | Workspace, CI, benchmark and correctness harnesses, ratified decisions, legal checks | Clone, build, test and re-run every lab number with one command |
| **M1 · De-risking lab, round 2** | Experiments on what the first lab left open | Every open risk has a number and a decision |
| **M2 · Format and ingest** | `.eg` container, compressed adjacency, I/O layer, ingest for integer, hex and string ids | Friendster → `.eg` in < 60 s at an 8 GB budget |
| **M3 · Engine and kernels** | Planner, memory accountant, edge-map framework, core kernels | Identical results in every regime and thread count; never over budget |
| **M4 · Communities and hierarchy** | Production Leiden, out-of-core, quality gates, map hierarchy | Friendster communities under 16 GiB, quality mode within 1% of the better of NetworKit PLM and GVE-Leiden run on the same machine (DESIGN § 6, restated 2026-11-25) |
| **M5 · The map** | Layout, tiles, `.egmap`, viewer, static and live modes, Jupyter widget | A 1 B-edge graph opens from a static URL, first view < 250 KB |
| **M6 · Python and interop** | Python package, Arrow interop, import/export, notebook widget | `pip install` on three OSes; zero-copy into Polars/pandas/DuckDB |
| **M7 · Queries and views** | Point queries, shortest paths, filtered and temporal views | Neighbours < 1.5 µs p99 in RAM; path p50 < 2 ms on 10^9 arcs |
| **M8 · CLI, docs and v0.1 launch** | CLI, docs, public benchmarks, three showcase maps, release | v0.1.0 published with reproducible benchmarks |
| **M9 · After v0.1** | Browser tier, MCP server, query language, temporal segments, sustainability | Unscheduled |

## Dependencies between milestones

```
M0 ─► M2 ─► M3 ─► M4 ─► M5 ─► M8
 │                 ▲     ▲      ▲
 └─► M1 ───────────┴─────┘      │
           M3 ─► M6 ────────────┤
           M3 ─► M7 ────────────┘
```

- M1 runs in parallel with M2 and M3: it uses the lab prototypes, not production code. Its
  results gate M4 (out-of-core Leiden, hierarchy shaping) and M5 (edges, layout).
- M5 is the *static* map: layout, tiles, file format, viewer, export. It needs M4 only.
- M6 and M7 can start once M3 exists and run alongside M4 and M5. The map features that
  need them live there: the Jupyter widget is in M6; live mode and attribute/time overlays
  are in M7.
- A minimal Python binding is built during M2 so tests can drive the engine; M6 finishes
  the Python product.

## Effort

Estimates given by the agents that built each prototype, for productionizing their area:

| Area | Estimate |
|---|---|
| Ingest, kernel framework, memory accountant, planner | 4–5 months |
| Adjacency format reader/writer and streaming | 3–5 weeks |
| Community detection beyond the prototype | 3–4 weeks |
| Map: layout, tiles, viewer | 6–10 weeks |
| Dictionary, out-of-core queries, columns and views | ~3 months |
| **Sum of the above** | **about 11–13 months** |

Not included in any of those estimates: M0 (foundations), M1 (the second lab round), the
Python API and wheels, the CLI, documentation, the three showcase maps and the release.
Adding them, **v0.1 is more plausibly 16–21 months for one full-time engineer**. Estimates
made right after a successful prototype tend to be optimistic, and none of these has been
tested against real progress. The NetworkX backend (a further 2–3 months) was moved to
after v0.1 by an approved design choice (2026-11-25) and is not counted here.

## The three risks that decide the plan

1. **Out-of-core community detection** — if Leiden cannot run on a 1.8 B-edge graph inside
   16 GiB with good quality, the laptop claim shrinks to "analyze yes, map only with enough
   RAM". Addressed first in M1.
2. **Map usefulness** — hierarchy and edges must carry meaning, not just render. Addressed
   in M1 with user tasks, not only metrics.
3. **One maintainer** — addressed by scope discipline (each milestone ships something
   usable) and by starting funding and co-maintainer work early (M9, in parallel).
