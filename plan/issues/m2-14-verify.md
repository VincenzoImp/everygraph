---
title: Verification and integrity: `eg verify`
milestone: M2 · Format and ingest
labels: type:feature, area:format, P1, risk:low, size:S, lab-evidence
---
**Why.** Users must be able to check a file they received, and the engine must detect corruption instead of computing on it.

**Scope**
- [ ] Fast check: header, TOC, section lengths, hashes of small sections
- [ ] Deep check: all content hashes; adjacency invariants (strictly increasing lists, ids in range, no self-loops when declared, symmetry by pair hash for undirected graphs, out/in consistency for directed ones); column lengths; dictionary bijection
- [ ] Parallel and streaming, usable on files larger than RAM
- [ ] Machine-readable report

**Acceptance**
- Deep verify of Friendster in < 60 s on the lab machine
- Every single-bit flip injected into a test file is detected

**Pointers.** `glab verify` and `glc checksym` in the lab prototypes.
