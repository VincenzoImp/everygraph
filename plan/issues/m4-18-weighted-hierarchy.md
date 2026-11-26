---
title: Weighted hierarchies in the graph file
milestone: M4 · Communities and hierarchy
labels: type:feature, area:community, P1, risk:medium, size:S
---
**Why.** Leiden reads real weights as fixed-point integers ({{#m4-02-weights-resolution}}), but `hierarchy::store` refuses a weighted result: the stored volumes and quotient weights would be in units of `2^-s`, the key does not name the weight column, and `verify --deep` checks the conservation against the unweighted graph only.

**Scope**
- [ ] The key of a weighted hierarchy names its weight column and its direction (`use_direction`); the result records `weight_scale` in its provenance (`docs/research/sota-numerics.md`, challenge 8)
- [ ] The quotient graph's events are its integer weights at that scale, as today for unweighted graphs; a reader converts volumes and internal weights with `2^-s` (`docs/spec/eg-communities.md` gains the rule)
- [ ] `hierarchy::check` and `verify --deep` read the weight column and rebuild the same integers, so conservation is checked bit for bit
- [ ] Directed weights: the skews per community are stored, or the Leicht–Newman value is recomputed from the graph; decide by size

**Acceptance**
- A weighted hierarchy, stored and read back, gives the table and the modularity of the run, bit for bit, on the oracle's weighted graphs with loops
- `verify --deep` refuses a file whose quotient weights were changed by one unit

**Depends on** {{#m4-02-weights-resolution}}, {{#m4-06-hierarchy-artifact}}
