# State of the art: testing a graph engine (oracles, fuzzing, crash consistency, budgets)

Second-wave review W2 (`docs/research/sota-coverage.md` § (c) topic 2, § (d) W2), 2026-11-25,
Opus. Scope: differential and metamorphic testing of graph kernels and of the NetworkX backend;
property-based graph generation; structure-aware fuzzing of the readers, the `.eg` container,
the adjacency and tile decoders; decompression bombs; crash consistency of append-and-commit
files; fault injection (ENOSPC, EIO, memory limits) for the no-OOM promise; mutation testing;
what the oracle (`crates/everygraph-oracle`, m0-05, m2-16) lacks. Nothing was built, measured
or loaded; every number below is from a paper, its artifact, or our repository, and comes with
its setup. Papers are in `/home/tux/graphlab/papers/testing/` (text in `txt/`).

> **Work in development that this touches — read first.**
> - **PR #170, `.eg` container spec v1.0 (branch `m2-01-container-spec`).** No v1.0 file
>   exists, so spec changes are still free. This review supports the reviewer's must-fixes M1–M3
>   and S1–S3 with literature, and adds six spec/test changes (Challenge 1): a "no
>   amplification" rule for every pointer in the format (M2 is Fifield's overlapping-file
>   bomb in another container); EIO on one header block treated as a torn block (S2; CORDS: btrfs
>   turns corruption into EIO); a fallback that is reported and blocks updates (S3; CORDS:
>   RethinkDB's silent rollback of acknowledged data); crash images for lost *synced* writes,
>   all 128 header cuts, garbage in unsynced regions, and crash → recover → commit → crash; a
>   `persist()` helper for rename + directory fsync with an ALICE-style multi-file model; and
>   a written rule for what a failed sync means (Rebello et al.).
> - **#44, edge-map framework.** The relations of Challenge 2 should run through the
>   framework's sources (slices, mmap, streams, compressed) so that one relation tests all
>   four; the framework's "validate targets before unchecked indexing" item needs a fuzz
>   target of hostile-but-decodable adjacency (Challenge 6).

---

## 1. Scope and reading log

Read level: **full** = method and evaluation read; **partial** = abstract, method and the
result tables, the rest skimmed; **abstract/results**; **not accessible**. Rows marked (†)
were read in full by a delegated reader under my brief, from the same text files; I checked
their numbers against the files where I quote them.

| Work | Venue, year | Verified link | Read level | PDF |
|---|---|---|---|---|
| **Graph-algorithm oracles** | | | | |
| Mang, Ba, He, Rigger: Finding logic bugs in graph-processing systems via Graph-cutting (GSlicer) | SIGMOD 2025 (PACMMOD 3(3) art. 163) | doi.org/10.1145/3725300 | **not accessible** (ACM DL returns 403 from this host); read instead: the poster, and the artifact's `oracles.py`, `README.md` and `found-bugs/overall.csv` (github.com/joyemang33/GSlicer) | `2025-mang-gslicer-poster.pdf` |
| Yan, Rigger, Wirth, Pham: GraphFuzz, differential fuzzing with lightweight feedback | arXiv 2502.15160, 2025 (no venue found) | arxiv.org/abs/2502.15160 | full | `2025-yan-graphfuzz.pdf` |
| Zhuang et al.: Testing graph database systems via graph-aware metamorphic relations (Gamera) | PVLDB 17(4), 2023 | doi.org/10.14778/3636218.3636236 | abstract/results (†) | `2023-zhuang-gamera.pdf` |
| Chen et al.: Metamorphic testing, a review of challenges and opportunities | ACM CSUR 51(1), 2018 | doi.org/10.1145/3143561 | abstract/results (†); it has **no** taxonomy of relation patterns (defers to Segura et al., TSE 2016, not read) | `2018-chen-metamorphic-review.pdf` |
| Kamm, Rigger, Zhang, Su: Testing graph database engines via query partitioning (GDBMeter) | ISSTA 2023 | doi.org/10.1145/3597926.3598044 | abstract only (search result) | — |
| GDsmith (ISSTA 2023), Grand (ISSTA 2022) | ISSTA | doi.org/10.1145/3597926.3598046 ; doi.org/10.1145/3533767.3534409 | abstract only (coverage audit) | — |
| **Property-based testing and generation** | | | | |
| Löscher, Sagonas: Targeted property-based testing | ISSTA 2017 | doi.org/10.1145/3092703.3092711 | full (†) | `2017-loscher-targeted-pbt.pdf` |
| Groce et al.: Swarm testing | ISSTA 2012 | cs.utah.edu/~regehr/papers/swarm12.pdf | abstract/results (†) | `2012-groce-swarm.pdf` |
| Goldstein et al.: Property-based testing in practice | ICSE 2024 | harrisongoldste.in/papers/icse24-pbt-in-practice.pdf | abstract/results (†) | `2024-goldstein-pbt-practice.pdf` |
| **Fuzzing: formats, resources, CI** | | | | |
| Dutra, Gopinath, Zeller: FormatFuzzer | ACM TOSEM 2023 | doi.org/10.1145/3628157 ; arxiv.org/abs/2109.11277 | partial (abstract, §1, §5 decision seeds) | `2023-dutra-formatfuzzer.pdf` |
| Fifield: A better zip bomb | USENIX WOOT 2019 | usenix.org/conference/woot19/presentation/fifield | full (construction §3–5 skimmed) | `2019-fifield-zipbomb.pdf` |
| Wen et al.: MemLock, memory usage guided fuzzing | ICSE 2020 | doi.org/10.1145/3377811.3380396 | partial (abstract, §2–3, results summary) | `2020-wen-memlock.pdf` |
| Lemieux et al.: PerfFuzz | ISSTA 2018 | doi.org/10.1145/3213846.3213874 | full (†) | `2018-lemieux-perffuzz.pdf` |
| Wei, Chen, Feng, Dillig: Singularity, pattern fuzzing for worst-case complexity | ESEC/FSE 2018 | doi.org/10.1145/3236024.3236039 | full (†) | `2018-wei-singularity.pdf` |
| Petsios et al.: SlowFuzz | CCS 2017 | arxiv.org/abs/1708.08437 | abstract/results (†) | `2017-petsios-slowfuzz.pdf` |
| Klees et al.: Evaluating fuzz testing | CCS 2018 | arxiv.org/abs/1808.09700 | full (†) | `2018-klees-evaluating-fuzz.pdf` |
| Klooster et al.: Effectiveness and scalability of fuzzing in CI/CD pipelines | arXiv 2205.14964 (SBFT 2023) | arxiv.org/abs/2205.14964 | abstract/results | `2023-klooster-ci-fuzzing.pdf` |
| **Crash consistency and fault injection** | | | | |
| Pillai et al.: All file systems are not created equal (ALICE, BOB) | OSDI 2014 | usenix.org/conference/osdi14/technical-sessions/presentation/pillai | full (§1–4; §4.5–4.6 skimmed) | `2014-pillai-alice.pdf` |
| Mohan et al.: Bounded black-box crash testing (B3, CrashMonkey, ACE) | OSDI 2018 | usenix.org/conference/osdi18/presentation/mohan | partial (§1–4, Table 4) | `2018-mohan-crashmonkey.pdf` |
| Bornholt et al.: Specifying and checking file system crash-consistency models (Ferrite) | ASPLOS 2016 | doi.org/10.1145/2872362.2872406 | full (†) | `2016-bornholt-ferrite.pdf` |
| Zheng et al.: Torturing databases for fun and profit | OSDI 2014 | usenix.org/conference/osdi14/technical-sessions/presentation/zheng_mai | full (†) | `2014-zheng-torturing-databases.pdf` |
| Zheng et al.: Understanding the robustness of SSDs under power fault | FAST 2013 | usenix.org/conference/fast13/technical-sessions/presentation/zheng | full (†) | `2013-zheng-ssd-power-fault.pdf` |
| Ganesan et al.: Redundancy does not imply fault tolerance (CORDS) | FAST 2017 | usenix.org/conference/fast17/technical-sessions/presentation/ganesan | full (†) | `2017-ganesan-cords.pdf` |
| Rebello et al.: Can applications recover from fsync failures? | USENIX ATC 2020 | usenix.org/conference/atc20/presentation/rebello | partial (abstract, §1, findings #1–#9) | `2020-rebello-fsync-failures.pdf` |
| Gunawi et al.: EIO, error handling is occasionally correct | FAST 2008 | usenix.org/legacy/event/fast08/tech/full_papers/gunawi/gunawi.pdf | full (†) | `2008-gunawi-eio.pdf` |
| LeBlanc et al.: Chipmunk | EuroSys 2023 | doi.org/10.1145/3552326.3567498 | partial (†) | `2023-leblanc-chipmunk.pdf` |
| LeBlanc et al.: SquirrelFS | OSDI 2024 / arXiv 2406.09649 | arxiv.org/abs/2406.09649 | abstract/results (†) | `2024-leblanc-squirrelfs.pdf` |
| Zhou et al.: FoundationDB (§4 simulation testing, §6.2) | SIGMOD 2021 | doi.org/10.1145/3448016.3457559 | partial (†): §4 and §6.2 only | `2021-zhou-foundationdb.pdf` |
| **Mutation testing** | | | | |
| Just et al.: Are mutants a valid substitute for real faults? | FSE 2014 | doi.org/10.1145/2635868.2635929 | full (†) | `2014-just-mutants-real-faults.pdf` |
| Petrović, Ivanković, Fraser, Just: Practical mutation testing at scale, a view from Google | IEEE TSE 2021 / arXiv 2102.11378 | arxiv.org/abs/2102.11378 | full (†) | `2021-petrovic-mutation-google.pdf` |
| cargo-mutants book (`in-diff.md`, `shards.md`, `ci.md`) | tool docs | github.com/sourcefrog/cargo-mutants/tree/main/book/src | read (docs, not a paper) | — |
| **Our side** | | | | |
| PR #170: `docs/spec/eg-container.md`, `tests/container.rs` (branch `m2-01-container-spec`), review comment | 2026-11-25 | `gh pr view 170 --comments` | full | — |
| `crates/everygraph-oracle`, `oracle/README.md`, `crates/everygraph-format/fuzz`, `crates/everygraph-ingest/src/text/decompress.rs`, `.github/workflows/ci.yml`, ruzstd 0.9.1 source | repository | — | read the parts cited | — |

Not done: OpenAlex was not used (0 requests). DBLP is blocked from this host (coverage audit).

---

## 2. The state of the art, as a map

| Approach | Best known work | What it found, on what | What it says to us |
|---|---|---|---|
| **Differential testing of graph algorithms** | GraphFuzz (2025) | 12 new bugs, 6 logic, in 9 implementations of NetworkX and igraph (initial study: 18 algorithms, 9 problems, 5 trials × 2 h each, AWS 16 vCPU / 32 GB). 24 h campaign over 17 more problems: 18 more bugs, 15 of them crashes. | Coverage feedback saturates: 95–100% of reachable lines with **< 10 graphs** within minutes (corpus 2–9 graphs). Feedback from the **output** (for SCC: number of components and largest size; for MST: total weight, bucketed) found the same bugs up to **4,493×** faster (Bug-2, small seed corpus; Table 3) and ran ~40× more graphs per hour than coverage+output. Bugs live in tiny corner cases: a zero-weight cycle (`≤` for `<`), self-loops, a node absent from the graph, an infinite loop in max-flow. |
| **Metamorphic testing of graph algorithms** | Graph-cutting / GSlicer (SIGMOD 2025) | 39 new bugs (34 fixed or confirmed) in NetworkX (25), Neo4j (4), Kùzu (10); of the 34, 11 logic, 22 crashes, 1 hang; ≥ 8 logic bugs not found by baselines; "a few concrete relationships" cover > 100 NetworkX APIs. | Relation: cut V into V1, V2; induced subgraphs G1, G2, cut edges E_c. Lossless (E_c = ∅) → results combine exactly; lossy → monotone bounds (`node_dict_smaller/greater`, `sum_greater` in `oracles.py`). Of the 25 NetworkX bugs, 20 are crashes and **18 are on degenerate graphs** — no edges, one node, one edge, fewer than 3 nodes, a looped single node (my count of `found-bugs/overall.csv`; the poster's Table 2 counts 4 logic + 18 crashes among NetworkX's fixed or confirmed ones). |
| Graph-aware MRs for graph DBs | Gamera (PVLDB 2023) | 39 bugs (15 confirmed, 3 fixed) in 7 GDBs, 100-node/200-edge graphs, 48 h in total (6-core i7, 32 GB); **compound relations (fusion, partitioning) found 30 of 39**, elementary 5, dynamic 4, "update" and "irrelevant manipulation" 0; larger graphs had "negligible impact". The differential tester Grand gave 1,412 false positives in 1,420 reports. | Union/partition relations are the productive ones, on small graphs — the same lesson as Graph-cutting. |
| **Property-based generation** | Targeted PBT (ISSTA 2017); swarm testing (ISSTA 2012) | Targeted PBT: hill climbing on a user "utility value" found a 42-node graph with a ≥ 21-hop shortest path after 17,666 tests on average (1,000 runs), where random PBT found none in 100,000 tests × 1,000 repetitions (i7-4600U, one core). Swarm (each feature omitted with p = 0.5 per configuration): Csmith on 17 compilers, one week per arm, 104 vs 73 distinct crash bugs; a stack bug 1 in 370,000 tests without, ~1 in 16 with; but a bad set of 20 configurations made a red-black-tree test worse. | We have proptest and a fixed corpus; no search, no feature omission. Use hundreds of swarm configurations, not tens. |
| **Structure-aware format fuzzing** | FormatFuzzer (TOSEM 2023); libFuzzer + `arbitrary` in Rust | FormatFuzzer compiles a binary template into a generator driven by a *decision seed* (a byte string); a mutation of the seed is a valid-format mutation; found memory errors in ffmpeg and timidity. | Our adjacency fuzz target already does the hybrid ("odd byte: encode a small CSR, then damage it"). The container has no fuzz target. |
| **Resource-exhaustion fuzzing** | SlowFuzz (CCS 2017), PerfFuzz (ISSTA 2018), Singularity (FSE 2018), MemLock (ICSE 2020) | MemLock keeps an input that raises peak heap or stack even without new coverage; vs AFL, AFLFast, PerfFuzz, FairFuzz, Angora, QSYM: 40.5% more unique crashes, ≥ 2.07× faster to a given bug, inputs using ~150× more memory; 15 CVEs. PerfFuzz keeps an input that sets a new maximum for *any* key of a map (per-edge hit counts): vs SlowFuzz 5–69× hotter spots, vs AFL 2–18× (§5.2; 4 libraries, 6 h × 20 runs); it proposes bytes-per-malloc-site keys but never evaluated them. Singularity evolves small *generator programs* (graph components: add node, add edge, grow edge, self-loop, bridge, merge, …) and fits cost against size (R² > 0.95): 16 of 17 textbook worst cases in 3 h each (fitness at n = 250), and a push-relabel max-flow in JGraphT at O(n⁵) against an expected O(n³) (Xeon E5-1620 v3). SlowFuzz: wall-clock fitness never passed 23% slowdown (noise); bzip2 showed nothing in the first hour, then 300×. | The toolbox for m3-10: fitness = heap bytes outside the ledger, counted deterministically (never RSS or time); search over graph generators; Singularity's size fit lets small graphs predict the large-graph OOM (Challenge 3). Searches warm up slowly: nightly, not per PR. |
| **Decompression bombs** | Fifield (WOOT 2019) | One DEFLATE stream cannot exceed a ratio of **1032**; overlapping files in the zip container reach 10 MB → 281 TB (28 M×), 42 kB → 5.5 GB, 46 MB → 4.5 PB with Zip64. | The danger is **pointers that let several entries share bytes**, not the codec. In our `.eg`: overlapping names (reviewer M2) are exactly this. In PMTiles: run-length and shared tile offsets. Streams are bounded by output caps (we have one for zstd). |
| **Crash consistency of applications** | ALICE/BOB (OSDI 2014); Ferrite (ASPLOS 2016); Zheng et al. (OSDI 2014) | ALICE: 60 static vulnerabilities in 11 applications (LevelDB, LMDB, SQLite, PostgreSQL, Git, …), over 4,000 failing crash states; persistence properties differ across 16 configurations of 6 Linux file systems. Ferrite: ext4 `data=ordered` left a 4096-byte file of 2500 "a" + 1596 NULs (size before data); replace-via-rename without fsync is atomic on btrfs only, and btrfs does not guarantee create-via-rename. Zheng: 12 of 15 database/file-system pairs violate durability under prefix power faults (0.05–19.15% of injected faults). | The application protocol, not the file system, is where bugs are; directory operations are the most common durability hole (7 of 11 ALICE applications, "often due to directory operations not being flushed"). LMDB, the closest design to ours (shadow paging), needs its 106-byte meta-page write to be atomic; our header blocks are hashed and do not. |
| **Crash consistency of file systems** | CrashMonkey/ACE (OSDI 2018); Chipmunk (EuroSys 2023) | B3: 24 of 26 known bugs reproduced, 10 new, from 3.37 M workloads of ≤ 3 operations in two days on 65 nodes; every reported bug follows a persistence point. Chipmunk: 23 new bugs in 5 PM file systems; replaying one in-flight write onto the persistent state found 10 of 11 mid-syscall bugs, two found all. | Small bounded-exhaustive workloads find almost everything (the small-scope effect); our in-process image enumeration is the cheap version. Both need root/kernel modules: not for our CI. |
| **Devices and errors** | Zheng et al. (FAST 2013); CORDS (FAST 2017); Rebello et al. (ATC 2020); Gunawi et al. (FAST 2008) | 13 of 15 SSDs (4 with power-loss protection, 5 vendors) broke semantics under power cuts: lost or reordered writes despite flushes on 8; torn writes at 512 B multiples with *old* bytes behind; bit corruption on 3. CORDS: one bad block of one node causes data loss or unavailability in 8 replicated stores; ext4/XFS return corrupt data silently, btrfs/ZFS return EIO. Rebello: after a failed fsync, ext4, XFS and btrfs mark pages clean; ext4/XFS keep the new data in the cache, btrfs reverts; retry is useless; PostgreSQL used fsync wrongly for 20 years. Gunawi: 13% of error-propagating calls in Linux 2.6.15 file systems drop the error (sync/wait paths far worse than reads). | Flushes can lie; EIO is a corruption signal; a failed sync is final; the writer's "poison on error" is right. |
| **Deterministic simulation** | FoundationDB (SIGMOD 2021) | All nondeterminism behind interfaces; real code runs in a simulator that injects faults ("buggification", swarm-toggled); > 0.5 M disk-years without a data-corruption event on CloudKit. | Our `ByteSource`/`ByteSink` and `MemoryAccountant` are the same seams: a fault-injecting implementation costs little (Challenge 4). |
| **Mutation testing** | Just et al. (FSE 2014); Petrović et al. (TSE 2021) | Just: 357 real faults in 5 Java programs; 73% coupled to at least one mutant; mutation score correlates with real-fault detection independently of coverage (4 of 5 programs); 17% of faults cannot be coupled to any mutant — mostly "algorithm modification" (37 of 63) and numerical overflow/NaN. Google: diff-only, one mutant per line, > 100 "arid node" rules: median **7 mutants per change** (820 with all operators), productivity raised from 15% to 89% by suppression; 82% of surfaced mutants judged useful. | Mutation scores are evidence of test strength, not of correctness on algorithmic bugs; diff-scoped they fit CI. |
| **Fuzzing evaluation and CI** | Klees et al. (CCS 2018); Klooster et al. (2023) | Klees: 32 papers surveyed, none evaluated soundly; single runs mislead (nm: one run 1,200 vs 800 crashes, medians of 30 runs 400 vs ~1,250); 57,142 "unique" crashes in cxxfilt were 9 bugs; rules: ≥ 30 trials, Mann–Whitney U and Â12, ground-truth bugs, ≥ 24 h. Klooster: 15-minute campaigns in CI catch regressions; ~63% of sessions skippable in 3 of 9 Magma libraries by triaging commits. | A CI fuzz run is a regression gate with canary bugs, not evidence. |

---

## 3. Decision by decision

"Level" = we do what the best work does; "behind" = a known method we lack; "ahead" = we do
more than the works read.

| Ours | What the best work does | Verdict | Evidence |
|---|---|---|---|
| **D7 / #28 `.eg` update protocol** (append, TOC, sync, other header block, sync; fall back to the older commit) | LMDB-style shadow paging with two meta pages (ALICE Fig. 4B) requires an atomic 106-byte overwrite; LogCabin alternates two pointer files (CORDS). | **Ahead** on tears (header blocks hashed, each in its own 4 KiB page: no atomicity assumed); **behind** on three points: a TOC fallback that is silent and can be overwritten (S3 — RethinkDB's rollback of acknowledged data in CORDS is this bug), EIO on block 1 failing open (S2 — CORDS: on btrfs a corrupt block *is* an EIO), and rename/directory fsync left to callers (S6 — ALICE's most common durability vulnerability). | Spec §§ 2, 7, 8; review of PR #170; ALICE § 4.3–4.4; CORDS § 4; Zheng 2014 § 5.1 (SQLite's unlink lived in memory until the next fsync). |
| **#28 crash tests** (recording sink; per step: all, nothing, prefixes, each write alone, cuts at 1, ½, last byte, 12 random sector subsets) | ALICE: APM with micro-ops incl. *size change* and *random* block content; Chipmunk: every subset of in-flight writes, plus post-recovery mutation; Zheng 2013: tears at 512 B with old bytes; flush violations on real SSDs. | **Level** with ALICE for one file (stronger than Zheng's prefix-only model), **behind** on: lost synced writes (lying flush), garbage instead of old bytes in unsynced appends, all 128 header cut positions (reviewer S5), bit flips, double crash, and anything involving the directory. | `tests/container.rs` lines 614–735 (branch): torn or lost writes leave the old bytes; no fate drops a synced op; `check_crashes` updates after recovery but does not crash again. |
| **kill -9 test** (`everygraph-io/tests/container.rs`, ux-contract C-1/C-2) | — | Correct as a process-death test only: the page cache survives SIGKILL (reviewer S5). Power loss can only be modelled (no root, no fault rig). | Zheng 2013 needed a custom power-cutting circuit; CrashMonkey a kernel module. |
| **m2-14 verify** ("every single-bit flip detected") | CORDS injects zeros, junk, EIO per block. | **Level** for sections (BLAKE3) once verify also checks header and TOC padding (reviewer S5a: TOC padding is unchecked). | Spec § 7; review S5. |
| **D11 / m2-04 / m3-10 no-OOM** (accountant, `InsufficientBudget`, cgroup matrix, counting allocator) | MemLock: memory as a fuzzing fitness; Singularity: input patterns + growth extrapolation; PerfFuzz: per-site maxima. | **Behind**: the matrix is fixed (kernel × graph × budget); nothing *searches* for the input that maximizes untracked memory, and nothing predicts the large-n failure from small n. The counting allocator (m3-10 last item; `everygraph-ingest/tests/heap_peak.rs`) is the right instrument and already exists. | m3-10 text; FINDINGS: "the only OOM was O(n) state at 1 GB"; MemLock § 3.2 seed retention by memory. |
| **D2 fallible allocation** | FoundationDB buggification; CORDS single-fault injection. | **Behind**: no test makes the k-th reservation fail. Error paths that are never exercised drop errors (Gunawi: 13% overall, worse on sync paths). | No `fail_at`/injection in the workspace (`grep`). The container's `Recording` sink fails at one op — the pattern exists, for writes only. |
| **D14 / m3-11 determinism** (hashes across threads, platforms, regimes, budgets) | Differential execution across configurations is known in databases (query-plan differential testing, Rigger's group); "identity is not correctness" (coverage audit). | **Level**, and the budget/regime axes are uncommon in graph engines. Keep the oracle beside it: identity across configurations cannot see a bug common to all. | `oracle/README.md` "four families"; m3-11. |
| **m0-05 / m2-16 oracle** (NetworkX, igraph, scipy references; in-process references; invariants; cross-config identity; golden files; seeded bugs caught by ≥ 2 families) | GraphFuzz: differential + output-feedback search; GSlicer: graph-cutting MRs; targeted PBT; swarm testing. | **Ahead** on references and invariants (PageRank fixed-point bound in `invariants.rs`, semantics table, degenerate graphs in the CI corpus: empty, isolates, single edge, `corpus.rs` 60–111). **Behind** on: metamorphic relations on the kernel under test (one exists, `answers_do_not_depend_on_node_ids`, on the *references* only, and compares sorted summaries); any search (corpus fixed: 289 CI graphs, 1,576 nightly; proptest 96 cases per property in CI); bounded-exhaustive tiny graphs; hang detection (GraphFuzz Bug-5, Kùzu hang); hostile adjacency. | `crates/everygraph-oracle/tests/properties.rs:107–133`; `oracle/README.md`. |
| **D20 / m6-06 NetworkX backend** ("reproduce NetworkX semantics exactly"; run NetworkX's backend test-suite) | GSlicer runs MRs through NetworkX's public API; 18 of its 25 NetworkX bugs are on degenerate graphs that NetworkX's own tests missed. | **Behind** (not built). NetworkX's suite will not cover degenerate inputs (GSlicer shows it did not). "Exactly" must include *which exception* NetworkX raises, and a policy for NetworkX's own bugs. | `found-bugs/overall.csv`: e.g. `ZeroDivisionError` in `current_flow_betweenness_centrality` on a one-node graph; `degree_pearson_correlation_coefficient` on a graph with no edges. |
| **m2-03 codec fuzzing** (cargo-fuzz target `adjacency.rs`: raw and encode-then-damage modes; all decoders equal the scalar one; APIs agree) | FormatFuzzer's decision seeds = our "odd byte" mode. | **Level** in design, **behind** in use: no CI job runs it (no `fuzz` in `.github/workflows/*.yml`), no committed corpus replay. | `crates/everygraph-format/fuzz/fuzz_targets/adjacency.rs`; `ci.yml`. |
| **m2-05 text reader, compressed input** | Fifield: the bound must be on output. | **Level**: zstd frames decoded in parallel only within `expansion_cap = 16 × compressed + 64 KiB`, larger frames streamed; RLE-bomb test exists; lines capped at 1 MiB. Checked in ruzstd 0.9.1: a fresh `StreamingDecoder` does not pre-reserve the frame's declared window (≤ 128 MiB default); `DecodeBuffer::reset` does, so **reusing decoders would request up to 128 MiB per concurrently decoded frame outside the ledger** (on Linux mostly untouched, hence not resident; on wasm32 a real allocation against the 4 GiB space) — pin it with a test. gzip is single-stream (≤ 1032× per stream; bounded by streaming). | `decompress.rs:296–330`; `text/tests.rs:595`; ruzstd `frame_decoder.rs:25,100–118`, `decode_buffer.rs:46–50`. |
| **m2-06 Parquet** (not built) | — | No plan item for hostile files. A Parquet page header declares its uncompressed size; a reader that pre-allocates from it is MemLock's "allocation without checking" class. | m2-06 scope has no fuzzing line. |
| **m5-02 tile format / viewer** (not built) | Fifield. | Spec should bound decoded tile size (the 32 KB budget is for gzip size) and forbid directory structures whose expansion is unbounded. | m5-02. |
| **m2-13 resume** (manifests, `kill -9` at ten random points) | ALICE: update-via-rename and directory entries are where applications fail. | **Behind** by design: ten random SIGKILLs test process death; manifests written by rename need the multi-file model (Challenge 1f). | m2-13 acceptance. |
| **cargo-mutants test audit** (CLAUDE.md § 6; DECISIONS 2026-11-25; no issue file) | Google: diff-based mutants at code review, arid-node suppression; Just: mutant detection correlates with real-fault detection. | **Behind**: it exists only as a rule, not as a CI job or an issue; only one seeded-bug suite (`toy.rs`, 6 bugs). | `grep -rl mutants plan/` → nothing. |
| **m0-02 CI < 15 min** | Klooster: 15-minute fuzzing campaigns in CI find regressions; ~63% of fuzz sessions skippable in 3 of 9 libraries by triaging unaffected commits. | **Level** on structure (change detection already gates the oracle job). Fuzzing is missing from it. | `ci.yml` "Which jobs this run needs". |

---

## 4. Challenges, ranked

Each: change · expected gain (source) · cost and risk against the promises · cheapest
experiment · issue.

### 1. Close the container's crash and hostile-input gaps before v1.0 is frozen (PR #170) — spec changes are free now

| Item | Change | Source |
|---|---|---|
| a | **No amplification.** A spec-wide rule: every offset/length in the format points to bytes no other pointer of the same kind points to (sections already; names now — reviewer M2), and what a reader holds after open is at most a constant times the bytes it read. Test: open every fuzz input under the counting allocator, assert held ≤ 2 × (header + TOC) bytes. | Fifield: overlap is what beats the 1032 limit. |
| b | **Generation**: 0 invalid on read, `checked_add` on write (M1). | Review M1. |
| c | **Fallback is a state, not a detail**: `open` returns it, `update` refuses it unless asked (S3), `verify` reports it; the newer commit's bytes are never overwritten automatically. Spec § 8 says so. | CORDS § 4 (RethinkDB rolls back acknowledged data; Kafka/LogCabin truncate after a corrupt entry). |
| d | **EIO on a header block = torn block**; read the blocks separately after any error (S2). Decide and write down: EIO on the newest TOC falls back *and reports* (media) — transient network errors are the `ByteSource`'s business (retry inside the HTTP source), not the container's. | CORDS § 2.1/4.3: btrfs/ZFS return EIO for corruption; 8.5% of near-line disks develop latent sector errors (cited field data). |
| e | **Crash model additions** (all in `tests/container.rs`, seconds): (1) fates that drop a write *before* the last sync — expect the older commit or a refusal, never a mix (Zheng 2013: 8 of 15 SSDs lost or reordered writes across flushes); (2) all 128 cut positions of the header write (S5; 128 extra images); (3) unsynced appended bytes filled with garbage, not old bytes or zeros (ALICE "random" micro-op); (4) crash → open → commit → crash (Chipmunk's post-recovery check; Zheng 2014's "back and forth" — add monotonicity over prefix images); (5) one bit flipped in each section, header and TOC → `verify` catches it (m2-14). | as cited |
| f | **One `persist()` in `everygraph-io`** (rename `.partial` → final, `fsync` the directory; Windows `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH`), used by ingest, `map`, `compact`, resume manifests; and an **ALICE-style model test** of it: record logical ops (create, write, set_len, fsync file, rename, fsync dir, unlink) and enumerate states under a pessimistic APM (any un-fsynced directory op lost; any un-fsynced append's size persisted with garbage data). | ALICE: 7 of 11 applications lose durability, "often due to directory operations not being flushed"; Ferrite: btrfs guarantees replace- but not create-via-rename. |
| g | **What a failed sync means**, in spec § 8: the commit's outcome is unknown; the writer is poisoned (it already is); the caller reports failure and never retries the sync; a reader in the same boot may see the new header from the page cache although it is not durable, so the CLI must not report "written". Test with a sink that after a failed sync serves the *new* bytes (ext4/XFS behaviour) and one that serves the *old* (btrfs). | Rebello et al. findings #1–#3, #8–#9. |
| g′ | Keep `sync_all` (fsync), never `sync_data` (fdatasync), after any `set_len` — level today (the reviewer checked `sync_all`), worth a sentence in § 8. Say in § 7 that a reader trusts the TOC's offsets and lengths, never the file's length, and whether a file shorter than its committed end is valid (reviewer N2). | Zheng 2014 § 5.1: LightningDB's `fdatasync` did not wait for the journal commit carrying the length (ext3, ext4 similar); Ferrite: ext4 persisted a 4096-byte size before its data. Zheng 2014 also found fixed-location metadata ("repetitive LBA") among the most vulnerable fault points — our header blocks are that pattern, hence (e)(2). |
| h | **Fuzz target `container.rs`**: `Container::open`, `verify_all`, `compact`, `Provenance::decode` on (a) raw bytes and (b) a decision-seed mode: `arbitrary` ops (add, remove, commit, reopen) build a valid file, then the tail of the input damages it. | FormatFuzzer § 5; our adjacency target. Reviewer "not checked: fuzzing of `decode_toc`". |

Gain: removes the three classes of format bugs that the literature shows recur (amplifying
pointers, silent rollback, directory durability), at the only moment it is free. Cost: spec
text (~1 page), ~300 test lines, one fuzz target, one helper. Risk: none to the promises; (c)
changes an API (`open` returns a status). Cheapest experiment: the reviewer's probes plus (e)(1)
— if a dropped pre-sync TOC write ever yields a "valid newest commit" whose sections fail
hashes, the spec's § 7 fallback rule is load-bearing and must be tested on every step.
Issue: PR #170 (#28); new "`persist`: rename and directory sync in one place"; m2-13.

### 2. Metamorphic relations on the kernel under test, as a fifth oracle family

Add `metamorphic.rs` to the oracle: for each kernel, a list of relations `G → (G', f)` such
that `run(G')` must equal `f(run(G))` (exact) or satisfy a bound. Run them through the
plug-in harness (every configuration, so through #44's four sources), on the CI corpus and in
proptest. Proposed set (exact = bit-identical comparison of the canonical output):

| Relation | Kernels | Expected | Exact? | Bug class it reaches |
|---|---|---|---|---|
| R1 Relabel by a permutation π | all per-node kernels | `out(πG) = π(out G)`; labels up to canonical relabelling | integers exact; PageRank within the fixed-point tolerance (summation order changes) | id-dependent logic, chunk boundaries aligned with ids, the "smallest id" label rule |
| R2 Disjoint union G1 ⊔ G2 (graph-cutting, E_c = ∅) | degree, WCC, SCC, BFS/SSSP (source in G1), core, triangles, clustering; totals add; transitivity = (3·Σt)/(Σtriples) | per-node equal; PageRank: `PR(v) = (n1/n)·PR_1(v)` when neither part has dangling nodes (uniform teleport) | integers exact | state shared across components, union-find merging, frontier bookkeeping |
| R3 Lossy cut (induced subgraph G[V1]) | core, degree, triangles: ≤; BFS/SSSP distances: ≥; WCC of G[V1] refines G's restricted | bounds | exact bounds | GSlicer's triangle-listing class |
| R4 Add one edge | triangles ≥, each core number +0 or +1, #WCC −0 or −1, distances ≤ | bounds | exact | incremental bookkeeping, the k-core update rule |
| R5 Reverse all arcs | SCC equal; BFS `direction=in` on G = `out` on Gᵀ; in-degree ↔ out-degree | equal | exact | **the derived in-adjacency (D6, m2-11)** |
| R6 Undirected G vs its symmetric directed form | WCC = SCC; BFS equal; PageRank equal with each undirected loop weight doubled | equal | integers exact; PageRank tolerance | the loop convention S1 (A[v][v] = 2w vs w) |
| R7 Scale all weights by 2^k | SSSP ×2^k; weighted PageRank, modularity, **deterministic Leiden partition** unchanged | equal | **bit-exact** (power-of-two scaling is exact in binary floating point barring overflow/underflow) | weight normalization, resolution in weight units, mixing weighted and unweighted sums |
| R8 Subdivide every edge | BFS distances ×2 on original nodes; WCC unchanged on them | equal | exact | frontier/level logic |
| R9 Add a kept self-loop | core, triangles, clustering, transitivity, WCC, BFS unchanged; degree +2 (undirected) | equal | exact | already an invariant in m2-16 for references; move it to kernels |
| R10 Shuffle rows; split a pair's events (`events`/`sum` modes) | adjacency, ingest report, every kernel | equal (not for `collapse`, whose kept row depends on order) | exact | ingest canonicalization |
| R11 Path decomposition: for c on the returned path a→b, d(a,c) + d(c,b) = d(a,b); undirected d(a,b) = d(b,a); add a zero-weight arc u→v ⇒ d(v) ≤ d(u) | SSSP, BFS, bidirectional BFS (m7-02) | equal / bound | exact for integer weights; dyadic weights exact | the GraphFuzz zero-weight-cycle class; Chen et al. § 4.1 (a real fault exposed by adding a zero-weight edge) |
| R12 Irrelevant manipulation: add a disjoint component or isolated nodes | every per-node kernel on the old nodes (PageRank: see R2's condition) | equal | exact | state indexed by n, "first component" assumptions |
| R13 Leiden quality | Leiden | `Q(partition) = Q(quotient, singletons)` (exists); relabel: Q within the documented run-to-run spread, not equal (visiting order) — statistical, not exact | bound | — |

Gain: GSlicer reports ≥ 8 logic bugs that baselines (including differential testing against
other systems) did not find, and a few relations cover > 100 APIs; in Gamera the compound
union/partition relations found 30 of 39 bugs on 100-node graphs. Chen et al. cite a study
(their ref. [55], not read) in which 3–6 diverse relations reached ≥ 90% of the faults an
oracle could detect. Prioritize R2/R3 (union and cut), R5, R7, R11. For us the relations
also reach graphs above the Python cap (n > 2^20, where only invariants and identity run
today), because they need no reference. Cost: ~400 lines in the oracle, one extra kernel run
per relation on small graphs (seconds for the CI corpus, estimate). Risk: R1/R2 on PageRank
need the tolerance, not identity — do not let them be read as a determinism claim. Cheapest
experiment: implement R1, R2, R5, R7 against the toy kernel and the existing seeded bugs;
count which bugs each relation catches alone (acceptance "≥ 2 independent checks" becomes
measurable per relation). Issue: new "Oracle: metamorphic relations on kernels under test"
(feeds m0-05, m2-16, m2-11, m4-01).

### 3. Make budget fuzzing (m3-10) find the O(n)-state OOM on small graphs

The lab's only OOM was untracked anonymous O(n) state (WCC under 1 GB). That bug has a
signature visible at any size: **untracked heap grows with n**. So:

1. **Slope test (Singularity's idea applied to memory).** For each kernel and each generator
   family, run at n = 2^10 … 2^16 in one process under the counting allocator (m3-10's last
   item; `ingest/tests/heap_peak.rs` shows the mechanism), record
   `U(n) = peak(heap) − peak(ledger)`, fit U against n. Required: U bounded by a constant (or
   by threads × per-thread buffers), slope ≈ 0. A slope of b bytes/node predicts
   `b × 65.6 M` bytes untracked on Friendster (extrapolation, labelled as such) — the test
   fails long before the graph exists. Seconds per kernel; no cgroup.
   Two warnings from Singularity: a pattern that only appears above a size threshold is
   missed when fitness is measured below it (its textCrunchr failure: fitness at 1 KB, a
   400 KB budget) — so the sizes must straddle our spill, chunk (2^16 entries) and segment
   thresholds; and the generator's component library caps what can be found (its
   Hopcroft–Karp failure) — so include hubs, long chains, duplicates, sparse id ranges,
   isolates, many small components.
2. **Search, not a matrix (MemLock / targeted PBT / Singularity).** Fitness = U(n)/n, or
   the fitted slope over three sizes, or `heap peak / budget`; always byte counts from the
   counting allocator, never RSS or wall time (SlowFuzz: wall-clock fitness stalled at 23%).
   Search space = generator parameters (Singularity's graph components: add node, add edge,
   grow edge, self-loop, bridge, merge, update weight) plus configuration: hub size (2^16
   neighbours exists in the nightly tier), number of components, path length (recursion and
   frontier depth — MemLock's uncontrolled recursion; NetworkX's recursive Tarjan was
   dropped after GraphFuzz's Bug-2), id sparsity, key length and type, duplicate
   multiplicity, weight columns, chunk size, thread count, budget. Keep a configuration when
   it raises the fitness even with no new outcome (MemLock § 3.2). In CI: targeted-PBT hill
   climbing, fixed seed, 1–3 minutes on ≤ 10^4 edges (targeted PBT found in ~17,700 tests
   what 10^5 random tests never did); nightly: the longer evolutionary search, whose
   findings are committed as patterns that CI replays at three sizes.
3. **Attribute untracked bytes to a phase** (PerfFuzz's map of maxima, cheaply): the
   accountant sets a thread-local "phase" tag when a reservation is made; the counting
   allocator keeps a per-tag maximum of untracked bytes, and the search keeps an input that
   raises *any* tag's maximum (PerfFuzz: a scalar objective gets stuck in local maxima).
   The report then names the phase that leaks, not just the kernel; deduplicate findings by
   phase, never by input (Klees: 57,142 "unique" crashes were 9 bugs).
4. **Budget boundary.** For each (kernel, graph) binary-search the smallest budget that
   succeeds and compare it with the planner's declared minimum: declared < true is an OOM
   in waiting; declared ≫ true is a refusal users will hit for nothing (report the ratio).
   Below the declared minimum, the refusal itself must allocate O(1) (counting allocator
   during planning).
5. **Fail the k-th reservation** (D2's fallible allocation): for k = 1…K, make
   `try_reserve` fail at the k-th call; every run must end in a typed error with no panic,
   no leaked scratch file, and the accountant back at zero.
6. The cgroup matrix stays, nightly, for what only the kernel sees: allocator retention
   (m2-19), page cache vs anonymous, thread stacks.

Gain: finds the bug class that actually happened, in CI time, without large graphs. Cost:
~300 lines (slope fit, search, tags); the counting allocator needs a crate with `unsafe`
already registered (exists in ingest tests). Risk: U(n) depends on the allocator (glibc
arenas) — measure *requested* bytes in the counting allocator, which is allocator-
independent, and leave RSS to the cgroup job. Cheapest experiment: run the slope test on the
lab WCC that OOM-ed (FINDINGS: O(n) state at 1 GB) — it must show a slope of ~4–8 bytes/node
(estimate: one u32/u64 per node) before the fix and ≈ 0 after. Issue: m3-10 (scope gains 1–5);
m2-04 (k-th reservation failure).

### 4. One fault-injecting I/O layer ("buggified" sink and source) for ENOSPC, EIO, short reads

`ByteSource`/`ByteSink` are already the seam (D2). Add, in a test-support module, a wrapper
that returns `ENOSPC` at the k-th write or `set_len`, `EIO` on the k-th read or sync, a short
read, or (for HTTP) a transient error, driven by a seed; and run ingest, the dictionary
build, spills, the container and `compact` over every k up to the op count of a small run.
Assertions: typed error, clean scratch (ux-contract C-8/C-9), the previous `.eg` intact,
never a "success" after a failed sync. Real ENOSPC on Linux without root: an unprivileged user
namespace with a `size=`-limited tmpfs (`/proc/sys/kernel/unprivileged_userns_clone` = 1 here;
not tried); on GitHub-hosted runners `sudo mount -t tmpfs -o size=…`. Gain: Gunawi et al.
found 13% of error-propagating calls dropped in Linux file systems, 19.6% on
sync/wait/write paths; untested error paths are where that happens. FoundationDB attributes
its record to exactly this style. Cost: ~200 lines; seconds per run. Risk: none. Cheapest
experiment: wrap the text reader's spill and the container writer; count error paths reached
(a `TEST()`-style counter, FoundationDB § 4). Issue: new "Fault-injecting ByteSource/ByteSink
for tests" (feeds m2-13, m2-04, #28, m8-01 C-8).

### 5. Fuzzing in CI: replay on every PR, short campaigns when a target's crate changes, long ones nightly

Today the adjacency target exists but no workflow runs it. Proposal, under the 15-minute PR
budget: (a) every PR: replay the committed corpus of every fuzz target as a normal test
(`cargo fuzz run T corpus/T -- -runs=0`, seconds); (b) PRs that touch a target's crate: 5
minutes per target, in parallel jobs (Klooster: 15-minute campaigns find regressions; triage
saved ~63% of sessions in 3 of 9 libraries); (c) nightly, on the scheduled run: 1 h per
target, new corpus entries committed by PR. Targets: adjacency (exists), container (1h), text
reader (raw bytes and gzip/zstd frames, including a frame declaring a 128 MiB window: the
counting allocator must not see it), Parquet wrapper (when built), tile payload (when built),
the oracle's `graph.json` loader is not worth it. Klees et al.'s rules (≥ 30 trials, ≥ 24 h,
Mann–Whitney U, ground-truth bugs) apply to *claims* about fuzzers, not to regression
replay: do not cite CI fuzz results as evidence of absence; keep canary bugs (the historical
O(n) untracked state behind a test-only flag; a re-introduced off-by-one in the codec) and
measure time-to-detect offline, nightly. Gain:
the decoder's "never reads out of bounds under fuzzing" acceptance (m2-03) becomes a
continuous property. Cost: one workflow job, corpora in the repo (KBs). Risk: nightly
toolchain for cargo-fuzz (already used locally); keep it out of the required checks.
Issue: m0-02 follow-up "Fuzz targets in CI"; m2-03; m2-05; m2-06.

### 6. Hostile-but-decodable adjacency for the kernels (#44)

The codec validates framing, not graph invariants (review of PR #157, recorded in m3-01).
Add an oracle family "invalid CSR": unsorted lists, duplicates, targets ≥ n, a loop on a
graph declared loop-free, asymmetric undirected lists. Expected: a typed error at open (if
validated once and cached in provenance) or at the driver, never a panic, never UB under
Miri. Gain: closes the path from a damaged or hostile file to `unsafe` indexing. Cost: small
(generators exist). Issue: #44 (m3-01) acceptance.

### 7. Bounded-exhaustive tiny graphs and swarm generation

(a) **Exhaustive tier**: every labelled undirected graph on ≤ 5 nodes with or without loops
(2^15 = 32,768), every directed graph on ≤ 4 nodes with loops (2^16 = 65,536), each under
`drop`/`keep`, through the in-process references and the kernel under test. The small-scope
evidence: B3 reproduced 24 of 26 reported file-system bugs with ≤ 3 operations; GraphFuzz's
seed corpus was one vertex and two of its bugs were found faster from it than from larger seeds; 18 of GSlicer's 25 NetworkX bugs are on graphs with at most 2 nodes or no edges.
Seconds (estimate: microseconds per graph per kernel). (b) **Swarm**: each proptest case
draws a random subset of features to *omit* (no loops, no isolates, no reciprocal arcs,
only stars, no weights), as Groce et al. Gain: corner cases by construction, not by luck.
Cost: ~100 lines. Issue: m0-05 follow-up.

### 8. Mutation testing as a CI signal, not only a milestone ritual

cargo-mutants supports `--in-diff FILE` (mutants overlapping the PR diff only), `--shard k/n`,
`--baseline=skip`, `--in-place`, and nextest. Proposal: on PRs that touch `everygraph-kernels`,
`-format`, `-core::memory`, run `cargo mutants --in-diff` on those crates, sharded over the
parallel jobs, time-boxed to 10 minutes, advisory at first; full run at each milestone (the
existing rule). Report missed mutants in the PR as test gaps, not failures. Evidence: Just
et al. — 73% of 357 real faults are coupled to a mutant and the score tracks real-fault
detection independently of coverage, but 17% are not coupled to any mutant, mostly
algorithm changes and numerical overflow/NaN: exactly the kernel bugs (float accumulation,
weight overflow, tie-breaking) that need the oracle and relations, so **a mutation score is
never presented as a correctness argument**. Petrović et al. — diff-only, one mutant per line
and arid-node suppression bring a change to a median of 7 mutants (820 unfiltered) and
raise usefulness from 15% to 89%; relational, logical and statement-deletion operators are
the most productive. **Invert one of Google's rules:** they treat `reserve`/capacity
arithmetic as arid; for us those mutants test the memory promise — keep them, and kill them
with a test that asserts the ledger covers what the counting allocator saw. Treat logging,
progress output and message strings as arid (`#[mutants::skip]` or the config file). Cost:
one job; runtime is the crate's test time × mutants on the diff. Risk: long tests (the
oracle's 90 s seeded bugs) dominate — scope the test command per crate. Issue: new
"cargo-mutants on PR diffs" (m0-02 follow-up); the milestone cleanup issues.

### 9. The NetworkX backend: degenerate-graph differential tests and a bug policy

(a) Run every dispatched function on the oracle's degenerate graphs and on the exhaustive
tier through both NetworkX's own implementation and the backend; compare results **and
exception types** (a drop-in raises what NetworkX raises). (b) Apply R1–R4 through the
public `nx.*` API (GSlicer's harness shape) so relations cover many functions at once. (c)
Add to `oracle/SEMANTICS.md` a rule for NetworkX's own bugs: follow the fixed upstream
behaviour, list the version, keep a test. Gain: GSlicer: 18 NetworkX bugs on degenerate
inputs its suite missed — our backend would either inherit or diverge silently. Cost: test
code only. Issue: m6-06.

### 10. Decompression and declared-size bounds in every reader

A rule for all readers (text, Parquet, `.eg`, tiles): no allocation from a size the input
declares before the bytes that justify it are read and counted; stream past a cap. Pin the
ruzstd behaviour found above with a regression test (frame declaring a 128 MiB window, read
under the counting allocator). For Parquet (m2-06): page headers declare uncompressed sizes;
fuzz the wrapper before relying on `arrow-rs`. For tiles (m5-02): a decoded-size cap per tile
in the spec, and a bound on PMTiles run-length expansion. Gain: closes MemLock's
"allocation without checking" class (its motivating CVE is a length field from the input).
Cost: small per reader. Issue: m2-05 (test), m2-06, m5-02.

### 11. Typestate for the commit order (optional)

SquirrelFS enforces its persistence order with Rust typestate at compile time (≈ 10 s), and
Chipmunk then found no ordering bug in it, only bugs in unchecked rebuild code. Our writer's
order (TOC → sync → header → sync) could be encoded the same way (`Pending → TocSynced →
Committed`). Gain: small (the tests already cover the order). Cost: an API refactor. Issue:
none now; note for #28's follow-ups.

### What fits a pull-request CI under 15 minutes

Durations are estimates (nothing was run); each item runs in a parallel job or inside an
existing one.

| Every PR (required) | When the crate changed | Nightly (scheduled run, not required) |
|---|---|---|
| Container crash images incl. Challenge 1e (seconds; already in `nextest`) | 5 min per fuzz target (adjacency, container, text reader) | 1 h per fuzz target; new corpus entries come back by PR |
| Fuzz corpus replay (seconds) | `cargo mutants --in-diff`, sharded, ≤ 10 min, advisory | Full cargo-mutants per crate (milestone rule) |
| Metamorphic relations R1–R12 on the CI corpus (≈ 1–2 min) | Targeted-PBT memory search, 1–3 min, fixed seed | Evolutionary memory search (PerfFuzz/Singularity style); time-to-detect canaries, ≥ 10 trials |
| Bounded-exhaustive tiny graphs (seconds) | Fault-injection sweep over every k (seconds to a minute) | cgroup budget matrix on tier-2 graphs (m3-10), allocator retention (m2-19) |
| Untracked-heap slope tests per kernel (seconds) | — | Oracle nightly tier, golden files, ALICE-style multi-file model with larger workloads |

---

## 5. What we have that the literature does not — novelty checked

| Claim | Verdict |
|---|---|
| In-process crash-image enumeration over an abstract sink, every step, with sector-random tears, as a unit test | **Not new in idea** (ALICE's APMs, BOB's block reordering, CrashMonkey's replay, FoundationDB's simulated disk). Uncommon in practice for a single-file format at unit-test speed; our model is stronger than Zheng 2014's prefix-only one. Claim "tested against a model of power loss", never "power-loss safe". |
| Cross-configuration identity (threads, chunk size, storage, regime, budget) as an oracle family | In Chen et al.'s terms this is differential testing (same input, several "versions"), not a metamorphic relation; differential testing across execution plans exists in databases (Rigger's group). Across **memory budgets and out-of-core regimes** for graph kernels: not found in the works read. A plausible claim, worded as "not found". |
| Seeded bugs that at least two independent oracle families must catch | A small, hand-picked mutation analysis; level with the literature, not new. |
| PageRank checked by a fixed-point residual bound instead of a reference value | The certifying-algorithms idea (checker instead of reference); not new; good. |
| Untracked-heap slope as a no-OOM test (Challenge 3.1) | Not found in MemLock, PerfFuzz, SlowFuzz or Singularity, which search for worst-case *time* or absolute memory; Singularity extrapolates time growth. Applying growth extrapolation to "memory outside a ledger" appears new — claim cautiously, after it works. |
| Graph-cutting relations for an engine whose kernels also run out of core | GSlicer tested in-memory libraries and GDBMSes; running the same relations across regimes (#44's sources) is our extension, not a new method. |

---

## 6. What to ignore, and why

- **CrashMonkey, Chipmunk, dm-log-writes on our CI** — they test file systems (kernel modules, root); our bugs are in our protocol, which the in-process model covers.
- **Hardware power-fault rigs** (Zheng 2013) — evidence for the model (lying flushes, 512 B tears), not a test we can run.
- **Coverage-guided fuzzing of kernels** — GraphFuzz: coverage saturates with < 10 graphs; use output feedback and relations.
- **FormatFuzzer's toolchain** (010 Editor templates → C++) — the decision-seed idea is what transfers, via Rust `arbitrary`.
- **GDsmith, Grand, GDBMeter, Gamera's query relations** — query-language testing; revisit for m9-03 (GQL subset) only.
- **Full verification** (FSCQ-style, Alloy models) — cost out of proportion for one append-only container (SquirrelFS reports FSCQ ≈ 11 h to verify).
- **Statistical fuzzer comparisons** (Klees' 30 trials × 24 h) — needed only to claim one fuzzer beats another, which we do not.
- **LLM-generated oracles** — no evidence in the works read that they beat relations we can derive from the semantics spec.
