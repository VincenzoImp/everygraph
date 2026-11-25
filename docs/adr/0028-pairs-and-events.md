# ADR 0028 — Pairs in the adjacency, events beside it

- **Status:** accepted, partly validated
- **Decision:** D9 in [DESIGN.md](../DESIGN.md#4-decisions) (amends ADR 0009: hot columns are per event or per pair, not "per adjacency entry"), with the storage of D4 (`docs/spec/eg-adjacency.md` revision 2) and rules under D14 for files built by appends
- **Date:** 2026-11-25

## Context

Two specifications disagreed, and the disagreement made the default event model
impossible to store.

- `docs/spec/graph-semantics.md` § 2.5 (revision of #36) said: "The adjacency stores **one
  entry per event** … within a node's list, entries are sorted by (neighbour in graph
  order, time, input order) … so the events of a pair are contiguous". `duplicates="events"`
  is the default (`design/preview/choices.md` A1; `design/DECISIONS.md`, "All remaining
  preview choices": "duplicates kept as events").
- `docs/spec/eg-adjacency.md` § 1 codes each list as **strictly increasing**
  (`w_j = x_j − x_{j−1} − 1`), and the only encoder refuses anything else
  (`crates/everygraph-format/src/adjacency/encode.rs`, `EncodeError::NotIncreasing`).

So any input with a repeated pair — every transaction, message or citation-with-time
dataset, exactly the temporal ones — could not be written. Nothing in the format, in #35
(the build) or in #37 (property columns) said where multiplicities and per-event times
live. The defect was found by the review of graphs that change
(`docs/research/sota-dynamic.md` § 0.1) and confirmed by the coordinator against the
encoder. It needs fixing whether or not segments are ever built; segments (#111) make the
choice between the two ways out sharper, because an append adds events to old pairs.

## Decision

**(A) Pairs in the adjacency, events beside it.** An adjacency stores pairs: each list
holds each neighbour once, coded exactly as in revision 1 of `eg-adjacency.md`. When some
pair has more than one event, the section `<name>.events` (container kind 5) holds an
Elias–Fano sequence of the per-entry event offsets, `len = entries + 1`, `U = events`
(`eg-adjacency.md` § 3.1), and index flag bit 1, `events`, is set exactly when that section
exists. Entry `e` has `ev[e + 1] − ev[e]` stored events, which is the pair's multiplicity
except under `duplicates="sum"`, where the multiplicity is the `edges` column, so that
`"sum"` and `"events"` give the same pair results (`graph-semantics.md` § 2.4, § 2.5).
Lists stay in increasing internal id of the neighbour, which is graph order until a
renumbering; results listed "in graph order" are sorted by graph position. The events
of a pair are ordered by (time, input order), nulls of time last, identically at both ends
of an undirected pair. **Per-event columns** (a time; each row's weight under
`duplicates="events"`) are indexed by event offset; **per-pair columns** (aggregates over a
pair's events, among them the pair sum that weighted kernels read; the `edges` column of
`duplicates="sum"`) by entry offset. A graph whose every pair has one event has no
`.events` section, and its event offsets are its entry offsets.

Hot columns (D9) stay aligned with an adjacency and duplicated across its two ends and
across out- and in-adjacency, as ADR 0009 measured — but per event or per pair, as the
column is, instead of "per adjacency entry". Wide or cold columns stay behind an edge id,
which is itself a per-event column (indexed by event offset, `num_edges` values) and is
`u64` when the graph has 2^32 events or more.

Adopted with it, from `docs/research/sota-dynamic.md` § 0.3–0.4 (reservations and rules;
segments themselves are not built, #111 builds them):

- **A1.** Flag bit 0 of the adjacency index, `sparse`, is reserved for segments
  (`eg-adjacency.md` § 3).
- **Index flag bit 1, `events`** (added by the review of this decision): the section's
  presence is part of the index, so a lost `.events` section is an error rather than
  another valid graph, and the bit is the must-understand signal — revision-1 readers
  refuse every flag. With it, cross-section checks (`eg-adjacency.md` § 5): out and in
  both have `.events` or neither, with equal `events`; names and kind 5 agree; per-event
  columns have `events` rows.
- **A2.** Blocks are the unit of I/O and of encoding; the order of result-bearing
  reductions is fixed by reduction blocks of node ranges that are a function of the
  graph — never of the file's blocks, the scan's chunk size, the thread count, the budget
  or the storage format (#44, #47; `eg-adjacency.md` § 4). Revision 1 said blocks "make the chunking of
  scans a property of the file", which contradicted "any valid directory may be written"
  as soon as two writers exist.
- **A3.** A merged run is byte-identical to the run an encoder writes for the same pairs
  and events (`eg-adjacency.md` § 4).
- **S1.** A pair's sum is exact over its events in every run: the newest run that touches
  a pair stores that pair's aggregates over all runs, recomputed from exact stored
  values, and supersedes older values; `duplicates="sum"` with a floating-point weight
  makes a graph that cannot be appended (`Unsupported`), and so does an append that would
  overflow a pair sum's stored type (`graph-semantics.md` § 4.3).
- **S2.** Column statistics are recorded per run and for the whole graph; the whole
  graph's are updated exactly and incrementally (exact accumulators per run; the largest
  multiplicity from the pairs a run touches; under `"sum"`, `min` and `max` of pair sums
  recomputed from the per-pair column when a superseded value was an extreme), never by
  rescanning older runs (`graph-semantics.md` § 2.7).
- **S3.** Graph order after appends: the base, then each segment's new identifiers in
  the order its input first names them (`graph-semantics.md` § 2.3; ADR 0008).
- **S4.** For a file built by appends, results depend on its batches, not on how its runs
  are merged or compacted (`design/DECISIONS.md`, "What 'repeatable' promises, for files
  built by appends"). A renumbering is not covered: a renumbered file may reduce over
  other node ranges.
- **S5.** The dictionary of a segmented file is layered, and a merge that rebuilds it
  keeps every id (ADR 0008, as a rule for #111).

**Versions.** The index and data keep version 1. Revision 1's own text already required
strictly increasing lists, so an equal neighbour was never a valid version-1 list,
whoever wrote it, and revision 2 reads every valid revision-1 file as before. What it adds
is a defined flag (bit 1), which revision-1 readers refuse, and a section. The
specification is revision 2 (`eg-adjacency.md` § 6). The `.events` section is new and
versioned on its own.

**Container kinds.** Kind 5 is the `.events` section and kind 6 the segment manifest
(reserved, not in 1.0), in `eg-container.md` (pull request 170) §§ 5, 10, not here. Kind 5
is not announced with a required feature of the container: required bits protect readers
older than a feature, and kind 5 is defined in 1.0, before any graph reader exists, so
the graph kinds of 1.0 (2–5) are not optional for a reader that reads the graph (only
container-level tools — list, verify, copy, compact — treat them as opaque). Inside the
adjacency, index flag bit 1 carries the same signal. Kind 6 is announced by the
container's required feature bit 0, `segments`: a 1.0 reader that skipped the manifest
would read the base run, an older graph, and nothing would tell it so.

## Evidence

Nothing was measured for this decision; what follows is the defect, arithmetic from the
format, and the earlier measurement that still carries D9.

- **The defect:** `encode` returns `EncodeError::NotIncreasing` for a list with an equal
  neighbour (`crates/everygraph-format/src/adjacency/encode.rs`, documented on `encode`);
  `graph-semantics.md` § 2.5 required equal neighbours for every repeated pair under the
  default option.
- **Cost of (A), arithmetic from `eg-adjacency.md` § 2, not measured.** On a graph with no
  repeated pair: zero (no section). That covers the four social graphs whose sizes are in
  `eg-adjacency.md` § 7 and ADR 0004 (lj, orkut, twitter-sym, Friendster). With
  `r = events / entries`: `1 + r` bits per entry for `r < 2`, between `2 + ⌊log2 r⌋` and
  `3 + ⌊log2 r⌋` for `r ≥ 2`.
- **Cost of (B), the same arithmetic.** An equal neighbour is a zero word: 1 data byte and
  2 control bits, so `10 (r − 1)` bits per entry. For the structure alone, against (A)'s
  `1 + r` (for `r < 2`), (B) is smaller only while `r < 11/9`, fewer than about one
  repeated event per 4.5 pairs. That leaves out the per-pair columns (A) also stores and
  (B) need not: with a `float64` pair sum (64 bits per entry per adjacency), (A) costs
  `65 + r` bits per entry against (B)'s `10 (r − 1)`, and (B) is smaller up to
  about `r ≈ 7.9` (with the `r ≥ 2` cost of (A); `75/9 ≈ 8.3` from the `r < 2`
  formula). So the decision does not rest on space: it rests on kernel time (every
  scan under (B) decodes, deduplicates and sums every event) and on appends (below).
- **Kernels on pairs** keep the bits per entry and the times of ADR 0004 (#118: social
  graphs, no repeated pairs, where (A) and revision 1 are the same bytes). Under (B), a
  graph with `r` events per pair makes every scan decode `r` times the entries, and every
  kernel deduplicate them and count multiplicities: `degree` would no longer be
  `entry[v + 1] − entry[v]`. Not measured: no temporal graph has been encoded.
- **D9's measurement still applies** (FINDINGS § F1, ADR 0009: weighted PageRank with the
  weight duplicated per entry, orkut, shared machine): a kernel under (A) reads a per-pair
  column by entry offset, which on a graph without repeated pairs is that experiment.

## Alternatives

- **(B) Allow equal neighbours** (`w_j = x_j − x_{j−1}` under a flag). One section fewer.
  Costs (`docs/research/sota-dynamic.md` § 0.1): every kernel pays a per-entry dedupe and
  a multiplicity count on every pass, and decodes every event (on Ethereum-like data with
  many events per pair, many times the pairs); degrees come from the lists instead of the
  index; appended events for an old pair cannot live in a separate run without re-sorting
  the pair's list in the base. Rejected: it moves the cost of the history onto the
  analysis that does not use it.
- **Events only behind an edge id** (cold columns, D9), with no per-entry offsets.
  Rejected: windows and `edges()` would need an id per event *and* a way from a pair to
  its ids, which is the `.events` sequence again plus a permutation.
- **Store pairs and drop events by default** (`duplicates="collapse"` or `"sum"`).
  Rejected: departs from the approved default A1 and loses time windows.

## Consequences

- **#35 (build)** groups each pair's events, writes each list with each neighbour once,
  and writes `<name>.events` (out, and in on a directed graph) when some pair repeats,
  setting index flag bit 1;
  per-event columns are carried in event order, per-pair aggregates computed from each
  pair's range. The streaming `.events` writer is #35's; `encode` of #30 keeps refusing
  equal neighbours, which is now correct. A graph with repeated pairs becomes writable.
- **#37 (columns)** distinguishes per-event from per-pair columns, indexes them by event
  or entry offset, decides which aggregates are stored per pair (the pair sum of a weight
  column is), and keeps its acceptance (weighted PageRank on a per-pair weight column
  within 15% of unweighted on orkut).
- **Kernels** (#44 onwards) read pairs, multiplicities (from the `.events` sequence or a
  per-pair column, as #37 decides) and per-pair columns, never per-event columns; their
  reductions follow framework node ranges, never file blocks (A2).
- **Time windows and views** (#93, #94, m7-05) filter events through the event offsets and
  recompute multiplicities and sums on the events they keep (`graph-semantics.md` § 6).
- **Segments** (#111): an append writes a run with its own lists and `.events` for the
  pairs it touches; the base is not rewritten; a pair's aggregates are computed over its
  events in every run (S1, S2); graph order and the dictionary grow by layers (S3, S5);
  merges are checkable byte for byte (A3); results do not depend on how runs are merged
  (S4). The sparse index flag (A1) keeps a small run small.
- `crates/everygraph-format/src/adjacency/index.rs` says, of `entry_offset`, "Edge-indexed
  columns are indexed by it": true of per-pair columns only. Corrected by #35, which
  touches that crate (code is not changed here).

## What would change our mind

- #35, on a temporal graph with repeated pairs, measures the `.events` section above
  `3 + log2 r` bits per pair, or a window over it slower than scanning a per-entry event
  list by more than the kernels gain.
- A kernel that must see events one by one (beyond `num_edges`, `edges()`, event counts
  and time filters) turns out to be common enough that every pass would pay the
  indirection.

## Closed by

#35 (build writes and measures the `.events` section), #37 (per-event and per-pair
columns).
