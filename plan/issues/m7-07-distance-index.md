---
title: Distance index for out-of-core path queries
milestone: M7 · Queries and views
labels: type:experiment, area:query, P2, risk:high, size:L
---
**Question.** Out-of-core shortest path costs ~0.8 s p50 on Friendster because bidirectional BFS touches many cold blocks. Can a compact index (landmarks / ALT bounds, or pruned landmark labeling on the core) bring it to interactive latency within a memory budget?

**Method**
- [ ] Landmark-based distance bounds with k = 16–64 landmarks (k·n bytes) to prune bidirectional search
- [ ] Pruned landmark labeling on the k-core or on the community quotient graph, with exact search inside communities
- [ ] Measure index size, build time, and query p50/p99 warm and cold on twitter-2010 and Friendster

**Done when.** A go/no-go with numbers; if go, a follow-up feature issue.

**Depends on** {{#m7-02-shortest-paths}}, {{#m7-03-ooc-queries}}
