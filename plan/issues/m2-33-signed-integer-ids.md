---
title: Negative integers as identifiers in text edge lists
milestone: M2 · Format and ingest
labels: type:feature, area:ingest, P2, risk:low, size:S
---
**Why.** The text reader detects an identifier column as integers only when every field of its sample is an unsigned decimal integer (`IdType::Integer`: below 2^64; `KeyKind::Integers`: "text never negative"). So a column with one negative value, as in `-1,5`, is read as text: the graph's `node` column is `utf8`, and `5` and `-1` come back as the strings `"5"` and `"-1"`. `docs/api-sketch.md` § 2.2 says that an integer column gives integers of the same type. In a text file the column has no type of its own, so the rule needs a decision for signed values (review of PR #229, S11). The Python package finds such a node from an `int` through its decimal text, so lookups already work.

**Scope**
- [ ] `IdType::Auto` detects signed decimal integers that fit in `i64` as integers. The dictionary stores them as `ColumnType::Int { bits: 64 }` with the sign-flipped normalized key `KeyKind::Integers` already defines for signed columns.
- [ ] A column that mixes values above `i64::MAX` with negative ones stays text, and the summary says why.
- [ ] Graph order is unchanged for an all-unsigned column: the same bytes as today.

**Acceptance**
- `-1,5` / `5,6` reads to a graph whose `node` column is `int64` with the values −1, 5, 6. Tested in `everygraph-ingest` and through `eg.read`.
- The oracle corpus builds the same files as before (hash of #54).

**Depends on** {{#m2-07-id-mapping}}
