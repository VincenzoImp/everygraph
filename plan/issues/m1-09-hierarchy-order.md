---
title: Renumber nodes by community hierarchy: compression and speed for free?
milestone: M1 · De-risking lab, round 2
labels: type:experiment, area:format, P1, risk:low, size:S, lab-evidence
---
**Question.** LLP ordering improved BV compression on orkut by 26% (15.25 → 11.32 bits/arc), varint by 18%, Stream VByte by 14%, and made raw PageRank 32% faster — but LLP took ~9 minutes. A flat community id gained almost nothing (−3%). Does a lexicographic key over *all* levels of the Leiden hierarchy, which the product computes anyway, get close to LLP?

**Method**
- [ ] Orderings: original, LLP (webgraph CLI), flat community, full Leiden-hierarchy path, hierarchy path + degree within the finest community, hierarchy path + BFS within the finest community
- [ ] Graphs: orkut, twitter-sym, lj, uk-2005
- [ ] Measure bits/edge for Stream VByte and BV, PageRank and Leiden time, cost of the permute + re-encode pass

**Decision it feeds.** Whether `eg` renumbers after community detection by default, and whether the layout order and the storage order should be the same permutation ({{#m2-03-adjacency-codec}}).

**Done when.** A table ordering × graph × {bits/edge, PageRank time}, and a yes/no on default renumbering.
