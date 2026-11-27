# ADR 0009 — Properties are Arrow-compatible columns

- **Status:** accepted, partly validated
- **Decision:** D9 in [DESIGN.md](../DESIGN.md#4-decisions)
- **Date:** 2026-11-25

## Context

Kernels read a few small properties (weight, timestamp) on every edge they visit; users
attach many wide or rarely read ones. An undirected graph stores each edge twice in its
adjacency, so a property can be duplicated per adjacency entry or stored once per edge
behind an edge id.

## Decision

Properties are Arrow-compatible columns. Small hot columns (weight, timestamp) are
duplicated per adjacency entry; wide or cold columns are stored once per edge behind a
u32 edge id.

## Evidence

From the properties experiment (FINDINGS § F1; shared machine; orkut; one weighted
PageRank iteration with an f32 weight; `lab/f1-query/results/props_orkut_t32.json`,
`lab/f1-query/results/props_orkut_t4.json`):
- Through a u32 edge id: 0.247 s vs 0.117 s with the weight duplicated per entry at 32
  threads (2.10×); 0.668 vs 0.391 s at 4 threads (1.71×). This "1.7–2.1×" is **one kernel,
  one graph, two thread counts**.
- Space: duplication costs 2·m·P bytes, indirection 2·m·4 + m·P; indirection is smaller
  when a property set exceeds 4 bytes per edge (note in the same file). Orkut with three
  columns (f32 weight, u32 timestamp, u8 type): 2.11 GB duplicated vs 1.99 GB indirect.
- Arrow compatibility: the lab wrote raw little-endian columns, the same bytes as an Arrow
  primitive values buffer, and mmapped them (same file); zero-copy export to Arrow
  consumers was verified in § F2 (ADR 0019).

## Consequences

- #37 specifies the column sections and #246 writes them in the build. The measurement
  this ADR lacks for the duplicated path, weighted PageRank on a per-pair weight column
  within 15% of the unweighted kernel on orkut, is #240's.
- Which columns are "hot" is decided at ingest (`weight=` and time columns), not guessed.

## What would change our mind

- #240 shows weighted PageRank with a duplicated weight column more than 15% slower than
  unweighted on orkut, which would remove most of the advantage over indirection.
- Typical inputs carry several hot columns per edge, so duplication costs more memory than
  the measured slowdown of indirection is worth.

## Amendment (2026-11-25, ADR 0028): per event or per pair, not per adjacency entry

An adjacency stores pairs, and the events of a pair are found through its `.events`
section (`docs/spec/eg-adjacency.md` § 3.1). "Duplicated per adjacency entry" above
becomes: hot columns are aligned with an adjacency and duplicated across its ends, **per
event** (indexed by event offset: a time, each row's weight) or **per pair** (indexed by
entry offset: the aggregates kernels read, such as a pair's summed weight). On a graph
without repeated pairs the two are the same, and so is the measurement above. Wide or
cold columns stay behind an edge id; the edge id is itself a per-event column (indexed by
event offset, one value per event), and since a graph can have 2^32 events or more it is
`u64` there, not always `u32`. #37 builds both.

## Closed by

Weighted PageRank on a per-pair column against the unweighted kernel (#240).
