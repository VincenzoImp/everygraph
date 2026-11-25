---
title: Research: a differentially private map export
milestone: M9 · After v0.1
labels: type:experiment, area:map, P2, risk:high, size:M
---
**Question.** Can a map be shared with a formal privacy guarantee, beyond the k-threshold merging of `reveal="summary"` ({{#m5-15-summary-export}})? Not in v0.1 (decided 2026-11-25, `docs/research/README.md` H9): it breaks promise 3's exact numbers, and the literature's DP community detection is weak at useful settings — LouvainDP and ModDivisive (Nguyen et al., WPES'16) need ε of 0.1–0.5·ln n (0.5·ln n = 7.0 on youtube, 1.1 M nodes) to be useful, with modularity well below non-private Louvain (read from their figures). The partition itself would have to be private; local edge-DP (Mundra et al., PVLDB'25) assumes an untrusted curator, the wrong model for an analyst who holds the graph.

**Method**
- [ ] A labelled `reveal="dp"` design: consistent noisy region counts over a public or DP hierarchy, with children summing to their parent by post-processing (Hay et al., PVLDB'10; Kuo et al., PVLDB'18)
- [ ] How it meets D14: seeded noise is fine only if the seed never enters the file; repeated exact releases of the same graph allow differencing, so say what a user may publish twice
- [ ] Utility at ε ∈ {0.5, 1, ln n / 2} on lj and orkut: count error per level and how many top-level regions survive

**Done when.** A note with the design, the utility table and a go/no-go for a post-0.1 issue.

**Pointers.** `docs/research/sota-human-factors.md` § 2 ("Privacy of released graph summaries"), § 3 (the D14 row), § 4 challenge 9.

**Depends on** {{#m5-15-summary-export}}
