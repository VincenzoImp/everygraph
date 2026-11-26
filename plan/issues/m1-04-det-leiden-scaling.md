---
title: Deterministic Leiden: recover scaling beyond 8 threads
milestone: M1 · De-risking lab, round 2
labels: type:experiment, area:community, P1, risk:medium, size:M, lab-evidence
---
**Question.** Can the deterministic mode reach ≤ 1.5× the asynchronous time at 16 threads for Leiden? Today it is 2.4–3.0× at 16 threads (FINDINGS "Idle-machine re-runs") and stops scaling at ~8 (lj local moving: 2.14 s at 8 threads, 1.88 s at 16).

**Suspected cause (not profiled).** On coarse passes the hashed sub-rounds contain only a few thousand vertices, so barriers dominate.

**What the literature says** (added 2026-11-25, `docs/research/sota-kernels.md` § 3 D14, challenge 1; `docs/research/sota-communities.md` § 3 D14, challenge 1; paper numbers, their machines). The suspected cause matches the code. The lab uses (n/16384) rounded to a power of two, clamped to **256–1,024 sub-rounds per sweep at every level** (`lab/b2-leiden/src/main.rs` l. 890–895), and sub-rounds under 1,024 vertices run on one thread. On a coarse level of 100,000 vertices that is about 400 vertices per sub-round, all run serially. Mt-KaHyPar-SDet (Euro-Par'22 / PPoPP'22, 2 × 64-core EPYC 7742) uses a constant 16 sub-rounds for its Louvain preprocessing and 3 for coarsening, and reports that "the number of sub-rounds either does not affect solution quality, or using fewer sub-rounds even leads to better quality". Its deterministic coarsening is 16–24% slower than the nondeterministic one (as reported in the DetJet paper, arXiv 2025, 64 threads on a 96-core EPYC). Its self-speed-up is 12.8× at 16 threads (geometric mean). DSLM (Euro-Par 2018) uses 4 sub-rounds; fewer fail to converge.

**Method**
- [ ] **First, the cheap A/B** (no new code: the lab binary has `--sub-rounds`): interleaved in `lab/b2-leiden`, `--sub-rounds` ∈ {4, 8, 16, 32, 64, auto} on lj and orkut (then Friendster), at 1, 4 and 16 threads, 1 and 3 iterations. Metrics: time per phase, det/async ratio, Q, passes, and the result hash at 1/4/16 threads. The sub-round count must stay a constant or a function of the level's size, never of threads or budget. Risk to watch: oscillation with large synchronous batches (DSLM: fewer than 4 fails to converge; Mt-KaHyPar: 1 oscillates); our smaller-label rule covers only singleton swaps
- [ ] Only if the A/B does not reach the target: an afterburner filter (pLouvain) or deterministic colouring (Grappolo, Vite) instead of hashed sub-rounds (`docs/research/sota-communities.md` challenge 11), and the parallel spanning-tree refinement of pLeiden (challenge 6)
- [ ] Profile where time goes per pass and per phase at 8, 16 threads (barrier wait vs decide vs commit)
- [ ] Sub-round count as a function of the *current level's* size instead of the input size; merge sub-rounds when active vertices fall below a threshold; run tiny passes on one thread (already done below 1,024 active vertices — tune the threshold)
- [ ] Deterministic refinement: measure proposal rejection rate; try two-phase accept
- [ ] Keep bit-identity: every variant must produce the same hash at 1, 2, 4, 8, 16 threads
- [ ] Run deterministic mode on Friendster for the first time (untested at 10^9 edges)

**Decision it feeds.** D14: whether "deterministic by default" stays, and what the documented price is. D14's cost (2.4–3.0× at 16 threads) is re-measured after this A/B. D14 is not narrowed before it (`docs/research/README.md` B5).

**Done when.** A table det/async by thread count for lj, orkut and Friendster on an idle machine, with the hash check, and a recommendation.

**Answered by PR #208 (2026-11-26), exploratory.** The rule: sub-rounds per level = clamp(next_pow2(n_level / 65536), 4, 1024), adopted in {{#m4-01-leiden-production}} (`lab/m1-04-det-leiden-scaling/README.md`). Closed on its load-independent results (hashes, modularity, sub-round counts) and interleaved ratios (CLAUDE.md § 6). The idle-machine table (`bench/specs/det-leiden.toml`) runs in the M4 milestone window, and D14's documented price changes only from it.

**Pointers.** `lab/b2-leiden`, FINDINGS § B2 determinism table; literature: Mt-KaHyPar synchronized moves (arXiv 2112.12704), LD-Leiden (note 2026-11-25: a dynamic method, not deterministic — the survey in `docs/research/memory-bounded-algorithms.md` finds it misplaced here; Mt-KaHyPar remains the reference) (arXiv 2502.18497).
