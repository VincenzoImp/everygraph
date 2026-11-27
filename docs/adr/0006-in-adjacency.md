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
  1.5 ms warm, single thread (FINDINGS § F1; `lab/f1-query/results/pq_twitter-dir_warm.json`;
  shared machine). Idle (#118, 2026-11-25, FINDINGS § M1-13,
  `lab/m1-13-compressed/results/pq.jsonl`): 0.44–0.47 ms on raw arrays, 0.56–0.59 ms on the
  compressed default (p50, heap / mmap).
- Compressed in-adjacency (#118; `lab/m1-13-compressed/results/encode.jsonl`; idle):
  15.94 bits per arc (the out-adjacency 16.54), encoded in 0.99 s at 16 threads from an
  in-memory raw transpose. The transpose itself was not re-measured.
- Not measured: building a compressed in-adjacency directly; whether directed community
  detection or SCC need it in practice (#24 decides the directed semantics).

## Consequences

- It is a derived artifact (#38). Its speed target (twitter-2010 transpose ≤ 15 s at 16
  threads, in line with the lab's 12.3 s at 32 threads) is #240's.
- Undirected graphs store one symmetric adjacency and no transpose.

## What would change our mind

- The build of #38 cannot make it within 15 s on twitter-2010 at 16 threads in RAM (#240),
  or its compressed
  size exceeds the out-adjacency's, making an on-demand transpose per session cheaper than
  storing it.
