---
title: Deterministic Leiden: recover scaling beyond 8 threads
milestone: M1 · De-risking lab, round 2
labels: type:experiment, area:community, P1, risk:medium, size:M, lab-evidence
---
**Question.** Can the deterministic mode reach ≤ 1.5× the asynchronous time at 16 threads for Leiden? Today it is 2.3–3.0× at 16 threads and stops scaling at ~8 (lj local moving: 2.14 s at 8 threads, 1.88 s at 16).

**Suspected cause (not profiled).** On coarse passes the hashed sub-rounds contain only a few thousand vertices, so barriers dominate.

**Method**
- [ ] Profile where time goes per pass and per phase at 8, 16 threads (barrier wait vs decide vs commit)
- [ ] Sub-round count as a function of the *current level's* size instead of the input size; merge sub-rounds when active vertices fall below a threshold; run tiny passes on one thread (already done below 1,024 active vertices — tune the threshold)
- [ ] Deterministic refinement: measure proposal rejection rate; try two-phase accept
- [ ] Keep bit-identity: every variant must produce the same hash at 1, 2, 4, 8, 16 threads
- [ ] Run deterministic mode on Friendster for the first time (untested at 10^9 edges)

**Decision it feeds.** D14: whether "deterministic by default" stays, and what the documented price is.

**Done when.** A table det/async by thread count for lj, orkut and Friendster on an idle machine, with the hash check, and a recommendation.

**Pointers.** `lab/b2-leiden`, FINDINGS § B2 determinism table; literature: Mt-KaHyPar synchronized moves (arXiv 2112.12704), LD-Leiden (note 2026-11-25: a dynamic method, not deterministic — the survey in `docs/research/memory-bounded-algorithms.md` finds it misplaced here; Mt-KaHyPar remains the reference) (arXiv 2502.18497).
