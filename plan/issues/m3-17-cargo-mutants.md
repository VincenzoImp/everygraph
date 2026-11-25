---
title: cargo-mutants on PR diffs
milestone: M3 · Engine and kernels
labels: type:infra, area:engine, P2, risk:low, size:S
---
**Why.** CLAUDE.md § 6 asks for a `cargo-mutants` test audit at the end of each milestone, but it exists only as a rule: no CI job and no issue (`grep -rl mutants plan/` found nothing). Mutants on the diff fit a PR: Google's diff-only mutants, one per line with arid-node suppression, bring a change to a median of 7 mutants (820 unfiltered) and raise the share judged useful from 15% to 89% (Petrović et al., TSE'21). The score is evidence of test strength, not of correctness: 73% of 357 real faults were coupled to at least one mutant, but 17% to none, mostly algorithm changes and numerical overflow or NaN (Just et al., FSE'14) — exactly the kernel bugs the oracle and its relations ({{#m3-16-metamorphic-oracle}}) must catch. Decided 2026-11-25 (`docs/research/README.md` T4).

**Scope**
- [ ] On PRs that touch `everygraph-kernels`, `-format` or `-core::memory`: `cargo mutants --in-diff` on those crates, sharded (`--shard k/n`) over parallel jobs, with nextest, time-boxed to 10 minutes, **advisory**: missed mutants are reported in the PR as test gaps, not failures
- [ ] The full run per crate stays the milestone rule, run nightly or at the milestone cleanup
- [ ] Keep `reserve`/capacity arithmetic mutants, which Google treats as arid: for us they test the memory promise; kill them with a test that the ledger covers what the counting allocator saw ({{#m3-10-no-oom-ci}})
- [ ] Treat logging, progress output and message strings as arid (`#[mutants::skip]` or the config file)
- [ ] Scope the test command per crate so the oracle's 90 s seeded-bug suite does not dominate
- [ ] A sentence in CONTRIBUTING: a mutation score is never presented as a correctness argument

**Acceptance**
- The job runs on PRs within its 10-minute box and posts the missed mutants
- The milestone cleanup issues cite its reports

**Pointers.** `docs/research/sota-testing.md` § 2 (mutation-testing row), § 3 (cargo-mutants row), § 4 challenge 8; the cargo-mutants book (`in-diff.md`, `shards.md`, `ci.md`).

**Depends on** {{#m0-02-ci}}
