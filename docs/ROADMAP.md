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
| **M4 · Communities and hierarchy** | Production Leiden, out-of-core, quality gates, map hierarchy | Friendster communities under 16 GiB at modularity ≥ 0.60 |
| **M5 · The map** | Layout, tiles, `.egmap`, viewer, static and live modes, Jupyter widget | A 1 B-edge graph opens from a static URL, first view < 250 KB |
| **M6 · Python and interop** | Python package, Arrow interop, import/export, NetworkX backend | `pip install` on three OSes; zero-copy into Polars/pandas/DuckDB |
| **M7 · Queries and views** | Point queries, shortest paths, filtered and temporal views | Neighbours < 1.5 µs p99 in RAM; path p50 < 2 ms on 10^9 arcs |
| **M8 · CLI, docs and v0.1 launch** | CLI, docs, public benchmarks, three showcase maps, release | v0.1.0 published with reproducible benchmarks |
| **M9 · After v0.1** | Browser tier, MCP server, query language, temporal segments, sustainability | Unscheduled |

## Dependencies between milestones

```
M0 ──► M1 ──► M4 ──► M5 ──► M8
 │      │      ▲      ▲
 └──► M2 ──► M3 ──────┤
        │      └──► M7 ┘
        └──► M6 (a minimal Python binding starts with M2; the product finishes in M6)
```

M1 runs in parallel with M2 and M3: it uses the lab prototypes, not production code. Its
results gate M4 (out-of-core Leiden, hierarchy shaping) and M5 (edges, layout).

## Effort

Estimates from the people — and agents — who built the prototypes, for productionizing
each area: ingest and kernel framework 4–5 engineer-months; community detection 3–4 weeks
beyond the prototype once the out-of-core design is settled; map 6–10 weeks; dictionary,
out-of-core queries, columns and views about 3 months together; browser tier 5–6 weeks;
NetworkX backend 2–3 months for ~60 functions. Summed with integration, documentation and
launch work, v0.1 is on the order of **12–18 months for one full-time engineer**, less
with a second. These are estimates, not commitments, and none has been tested against
real progress yet.

## The three risks that decide the plan

1. **Out-of-core community detection** — if Leiden cannot run on a 1.8 B-edge graph inside
   16 GiB with good quality, the laptop claim shrinks to "analyze yes, map only with enough
   RAM". Addressed first in M1.
2. **Map usefulness** — hierarchy and edges must carry meaning, not just render. Addressed
   in M1 with user tasks, not only metrics.
3. **One maintainer** — addressed by scope discipline (each milestone ships something
   usable) and by starting funding and co-maintainer work early (M9, in parallel).
