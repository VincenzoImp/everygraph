---
title: Freeze the .eg format for 0.1: open byte-level decisions taken, a compatibility corpus
milestone: M2 · Format and ingest
labels: type:spec, area:format, P0, risk:medium, size:S
---
**Why.** Release 0.1 promises that files it writes stay readable ({{#m8-08-release}}). After that, every change to the bytes of an `.eg` file costs a format version and a reader for the old one. The state-of-the-art review of 2026-11-26 (`docs/research/sota-review-2026-11.md` § 0 item 8, S10) found one open decision of this kind for `.eg`: chunked records for hub lists ({{#m1-17-chunked-hub-records}}). `docs/spec/eg-adjacency.md` § 4 says today that no list is ever split. The other two format decisions of the review are not in `.eg`: zoom data in the tiles is {{#m5-02-tile-format}}'s, and the Arrow export of the structure is {{#m6-03-arrow-interop}}'s.

**Scope**
- [ ] The go/no-go of {{#m1-17-chunked-hub-records}} is written into `docs/spec/eg-adjacency.md`. On go: the record layout with restart points, as a codec value or as version 2 of the adjacency section, specified, implemented and fuzzed. On no-go: § 4 keeps "no list is ever split", with the measurement that decided it
- [ ] Each specification under `docs/spec/` that describes `.eg` bytes states the format version it freezes, and which fields a later minor version may still use (the rules of `docs/spec/eg-container.md` § 10)
- [ ] A small corpus of `.eg` files written by the release candidate (undirected, directed, weighted, with events, with a result section), kept as test data. Every later reader opens it with `verify --deep` in CI

**Not in scope.** A new compact codec. Codec values that a 0.1 reader refuses can be added later without breaking the promise, and the codec census ({{#m1-16-codec-0124}}) decides that one.

**Done when.** No open issue proposes a change to the bytes that 0.1 writes, the specifications say "frozen" with their version, and the corpus test runs in CI.

**Depends on** {{#m1-17-chunked-hub-records}}, {{#m2-01-container-spec}}, {{#m2-03-adjacency-codec}}
