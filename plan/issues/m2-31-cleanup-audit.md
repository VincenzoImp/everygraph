---
title: M2 cleanup: code size, duplicated helpers and a test audit
milestone: M2 · Format and ingest
labels: type:infra, area:engine, P1, risk:low, size:M
---
**Why.** CLAUDE.md § 6 ends every milestone with a cleanup issue, and M2 had none. The owner named heavy, unmaintainable code and weak tests as the project's main risks (2026-11-26). Line counts on main, 2026-11-26 (`find crates/<c> -name '*.rs'`, `src` includes in-file unit tests): `everygraph-bench` 12,249 src + 1,605 tests — the largest crate, larger than the format and the io crates together; `everygraph-ingest` 10,584 + 2,567 (text reader and identifier dictionary); `everygraph-oracle` 5,823 + 907; `everygraph-core` 3,640; `everygraph-format` 2,706 + 450; `everygraph-io` 1,658 + 704.

**Method**
- [ ] For each crate: what it owns (crate docs), lines per module, and for every module above ~800 lines a one-line answer to "what would break if this were half the size"
- [ ] Product crates first (core, format, io, ingest). The harness (`everygraph-bench`) is a tool: only remove what no spec or CI job uses, if cheap; no size target (owner, 2026-11-26: tools may be rough, product code must be clean)
- [ ] `everygraph-ingest`: duplicated helpers between the text reader and the dictionary (buffers, scratch, partitioning), merged into one place
- [ ] Duplicated test helpers across crates (byte generators, temporary files, child-process runners) moved to one test-support module
- [ ] `cargo mutants` over `everygraph-core`, `-format`, `-io`, `-ingest` ({{#m3-17-cargo-mutants}} for the tooling): every surviving mutant killed by a test or explained in the report; every test that no mutant can fail rewritten or removed
- [ ] `lab/` prototypes whose production counterpart has landed (b1-core text reader, codec, dictionary) marked frozen in their README

**Done when.** A short report in the PR: lines before and after per crate, mutants survived per crate with explanations, tests removed and added; CI time before and after.

**Feeds.** The M2 milestone report; the same issue shape for M3–M9.

**Pointers.** CLAUDE.md § 6 "Keep the code small" and "Quality gates"; `docs/research/sota-testing.md` (Just et al. on what mutants cannot represent).
