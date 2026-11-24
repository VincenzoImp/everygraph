---
title: Beyond 2^32 nodes and server-class tuning
milestone: M9 · After v0.1
labels: type:feature, area:engine, P2, risk:medium, size:L, lab-evidence
---
**Why.** Dense u32 ids are the default because u64 targets cost +70–85% on PageRank, and they cover every graph a laptop or workstation can hold. Web-scale graphs (Software Heritage: tens of billions of nodes) need more, on machines with hundreds of gigabytes and many NUMA nodes.

**Scope**
- [ ] `NodeId` generic over u32/u64 selected at open time; the gap codec is already width-agnostic (identical bits/edge with ids shifted by 2^33 in the lab)
- [ ] Hybrid: u32 local ids within partitions to keep kernels fast
- [ ] NUMA-aware allocation and scheduling; huge pages for state arrays (≤ 10% in the lab on one socket)
- [ ] Validation on a ≥ 10 B-edge public web graph on a rented large-memory server
- [ ] Fully external regime when even the O(n) state does not fit (not measured in the lab)

**Acceptance**
- A graph with > 2^32 nodes ingests and runs WCC and PageRank; u32 graphs show no regression
