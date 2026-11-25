# ADR 0006 — Directed graphs store out- and in-adjacency

- **Status:** accepted
- **Decision:** D6 in [DESIGN.md](../DESIGN.md#4-decisions)
- **Date:** 2026-11-25

## Context

On a directed graph, in-neighbours, pull-style kernels over reversed edges and
bidirectional search all need the transpose. Computing it per query is not possible;
storing it doubles the topology.

## Decision

A directed graph stores its out-adjacency and, as a derived artifact built after ingest
and cached in the `.eg` file, its in-adjacency.

## Evidence

- **Cost** (FINDINGS § F1; shared machine; twitter-2010, 41.7 M nodes, 1.47 B arcs,
  32 threads, raw u32 arrays; `lab/f1-query/results/csc_twitter_t32.json`): the transpose
  took 12.3 s and adds 4.23 bytes per arc (6.2 GB).
- **Use**: bidirectional BFS on twitter-2010 (which uses the in-adjacency) had a p50 of
  1.5 ms warm, single thread (FINDINGS § F1; `lab/f1-query/results/pq_twitter-dir_warm.json`).
- Not measured: the size and build time of a *compressed* in-adjacency; whether directed
  community detection or SCC need it in practice (#24 decides the directed semantics).

## Consequences

- #38 builds it as a derived artifact; its acceptance (twitter-2010 transpose ≤ 15 s at
  16 threads) is in line with the lab's 12.3 s at 32 threads.
- Undirected graphs store one symmetric adjacency and no transpose.

## What would change our mind

- #38 cannot build it within 15 s on twitter-2010 at 16 threads in RAM, or its compressed
  size exceeds the out-adjacency's, making an on-demand transpose per session cheaper than
  storing it.
