---
title: M2 cleanup: code size, duplicated helpers and a test audit
milestone: M2 · Format and ingest
labels: type:infra, area:engine, P1, risk:low, size:M
---
**Why.** CLAUDE.md § 6 ends every milestone with a cleanup issue, and M2 had none. The owner named heavy, unmaintainable code and weak tests as the project's main risks (2026-11-26). Line counts on main, 2026-11-26 (`find crates/<c> -name '*.rs'`, `src` includes in-file unit tests): `everygraph-bench` 12,249 src + 1,605 tests — the largest crate, larger than the format and the io crates together; `everygraph-ingest` 10,584 + 2,567 (text reader and identifier dictionary); `everygraph-oracle` 5,823 + 907; `everygraph-core` 3,640; `everygraph-format` 2,706 + 450; `everygraph-io` 1,658 + 704. Again after the night's merges (main at 1bd9cd9, `src` only, `find crates/<c>/src -name '*.rs' | xargs cat | wc -l`): `everygraph-ingest` 14,171 (the build of #35 added `build/`, 2,440 without its tests); `everygraph-format` 9,453 (columns 2,498, the container 2,918 with results, `verify.rs` 666); `everygraph-core` 6,079 (`par.rs` 1,479, `seg_vec.rs` 1,178); `everygraph-io` 2,568; `everygraph-cli` 2,619.

**Method**
- [ ] For each crate: what it owns (crate docs), lines per module, and for every module above ~800 lines a one-line answer to "what would break if this were half the size"
- [ ] Product crates first (core, format, io, ingest). The harness (`everygraph-bench`) is a tool: only remove what no spec or CI job uses, if cheap; no size target (owner, 2026-11-26: tools may be rough, product code must be clean)
- [ ] `everygraph-ingest`: duplicated helpers between the text reader and the dictionary (buffers, scratch, partitioning), merged into one place
- [ ] The allocation lints for `everygraph-format` and `everygraph-ingest`: a `clippy.toml` that disallows sizing a buffer directly, as in the kernel crates (CONTRIBUTING.md, "The allocation rule"), the lint forbidden in each crate root, and `tools/check_portability_canary.py` extended to catch every entry. Each existing call that sizes a buffer is either routed through the accountant or allowed in one place with the reason.
- [ ] The three buffers allocated outside the memory accountant, marked `TODO(#200)`: the bounce buffer of `SegmentReader` (`everygraph-core`, `segment.rs`) and the direct-I/O bounce buffers of `FileSource` and `BlockWriter` (`everygraph-io`, `file.rs`, `sink.rs`). Route each through the accountant or record why it is allowed
- [ ] Duplicated test helpers across crates (byte generators, temporary files, child-process runners) moved to one test-support module
- [ ] `cargo mutants` over `everygraph-core`, `-format`, `-io`, `-ingest` (`tools/mutants.sh crate CRATE`, or `gh workflow run mutants.yml -f package=CRATE`; CONTRIBUTING.md, "Mutation testing"): the report cites each run's `missed.txt`; every surviving mutant killed by a test or explained in the report; every test that no mutant can fail rewritten or removed
- [ ] `lab/` prototypes whose production counterpart has landed (b1-core text reader, codec, dictionary) marked frozen in their README

**Done when.** A short report in the PR: lines before and after per crate, mutants survived per crate with explanations, tests removed and added; CI time before and after.

**Feeds.** The M2 milestone report; the same issue shape for M3–M9.

**Pointers.** CLAUDE.md § 6 "Keep the code small" and "Quality gates"; `docs/research/sota-testing.md` (Just et al. on what mutants cannot represent).

**Depends on** {{#m3-17-cargo-mutants}}
