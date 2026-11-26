---
title: Beyond 2^32 nodes and server-class tuning
milestone: M9 · After v0.1
labels: type:feature, area:engine, P2, risk:medium, size:L, lab-evidence
---
**Why.** Dense u32 ids are the default because u64 targets cost +71% (f64) to +87% (f32) on PageRank (orkut, 32 threads, idle; DESIGN D3), and they cover every graph a laptop or workstation can hold. Web-scale graphs (Software Heritage: tens of billions of nodes) need more, on machines with hundreds of gigabytes and many NUMA nodes.

**Scope**
- [ ] `NodeId` generic over u32/u64 selected at open time; the gap codec is already width-agnostic (identical bits/edge with ids shifted by 2^33 in the lab)
- [ ] Hybrid: u32 local ids within partitions to keep kernels fast
- [ ] NUMA-aware allocation and scheduling; huge pages for state arrays (≤ 10% in the lab on one socket)
- [ ] Validation on a ≥ 10 B-edge public web graph on a rented large-memory server. The Web Data Commons 2012 page graph (3,563 M nodes, 128,736 M arcs) is a near-limit u32 test, not a test above 2^32 nodes: 3.56 B is 83% of 2^32 (`docs/research/memory-bounded-algorithms.md`, addendum item 18)
- [ ] The graph for the acceptance below, named: a synthetic RMAT graph above 2^32 nodes, generated deterministically from a seed (Seraph, FAST 2024, ran RMAT with 8.6 B nodes and 112 B edges on 8-byte ids, their machine), or the Software Heritage graph (tens of billions of nodes); the choice and its download or generation cost recorded here before work starts
- [ ] Fully external regime when even the O(n) state does not fit (not measured in the lab)

**Acceptance**
- A graph with > 2^32 nodes (the synthetic RMAT graph or Software Heritage, as named above; not Web Data Commons) ingests and runs WCC and PageRank; u32 graphs show no regression

**Depends on** {{#m3-04-pagerank}}, {{#m3-05-components}}
