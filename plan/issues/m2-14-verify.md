---
title: Verification and integrity: `everygraph verify`
milestone: M2 · Format and ingest
labels: type:feature, area:format, P1, risk:low, size:S, lab-evidence
---
**Why.** Users must be able to check a file they received, and the engine must detect corruption instead of computing on it.

**Scope**
- [ ] Fast check: header, TOC, section lengths, hashes of small sections
- [ ] Deep check: all content hashes; adjacency invariants (strictly increasing lists, ids in range, no self-loops when declared, symmetry by pair hash for undirected graphs, out/in consistency for directed ones); the `.events` section of each adjacency that has one (`docs/spec/eg-adjacency.md` § 5: flag bit 1 and the section agree, a strictly increasing sequence, out and in with equal `events`); column lengths; dictionary bijection
- [ ] Parallel and streaming, usable on files larger than RAM
- [ ] Machine-readable report

**Acceptance**
- Deep verify of Friendster in < 60 s on the lab machine
- Every single-bit flip injected into a test file is detected

**Left after PR #212 (2026-11-26).** PR #212 built `everygraph_format::verify`, `Dictionary::verify` and `everygraph verify FILE [--deep] [--strict] [--json]`, and showed the single-bit-flip criterion. Still open:
- [ ] The Friendster time: `systemd-run --user --scope -p MemoryMax=8G -p MemorySwapMax=0 /usr/bin/time -v target/release/everygraph verify --deep friendster.eg`, on an idle machine, once {{#m2-08-csr-build}} and the ingest pipeline can write the file
- [ ] The content checks of the `.events` section (strictly increasing, equal `events` in out and in), with {{#m2-08-csr-build}}, which builds the section
- [ ] Column lengths, with {{#m2-10-property-columns}}
- [ ] A guard on huge lists in the codec: one list decodes into one buffer of 4·d bytes. `verify` reserves that buffer against its budget and reports a list too large as unchecked, but the decoding API offers kernels no such bound; a crafted list of about 2^32 entries asks for 16 GiB (review of PR #212)

**Pointers.** `glab verify` and `glc checksym` in the lab prototypes.
