---
title: Name the source, target, weight and time columns of a file without a header by position
milestone: M2 · Format and ingest
labels: type:feature, area:ingest, P2, risk:low, size:S
---
**Why.** `everygraph ingest --weight NAME --time NAME` and `eg.read(weight=, time=)` take a column name. A file without a header has no names: its columns are `column 1`, `column 2`, … in messages, and asking for `--time at` gives "it has no header, so there is no column named at". So the weight or time of a headerless edge list (SNAP's `CollegeMsg.txt`, `src dst unix_ts`) cannot be read without editing the file (found in PR #269). The text reader already takes a position (`text::Column::Index`); the public options do not.

**Scope**
- [ ] A public form for a column by position on the command line and in Python, for `--source`, `--target`, `--weight` and `--time` and their keywords. Proposed: an integer from 1, as the messages count (`--time 3`, `eg.read(time=3)`); a header name that is itself an integer keeps its meaning by name, and the summary says which was used. This is a choice of something a user types: write it into `design/DECISIONS.md` and show it in the `columns` line of the summary (`docs/ux-contract.md` § 3.4)
- [ ] The stored column is named `column N`, as messages name it; the name is part of the build's parameters, so the cache key does not change with how the column was asked for
- [ ] `docs/api-sketch.md` § 3.1 and § 5.2 give the form

**Acceptance**
- `everygraph ingest CollegeMsg.txt --time 3` builds a graph whose `adjacency.out.event/arrow/column 3` holds the input's times, and `verify --deep` finds it intact
- `eg.read(path, weight=3)` and `everygraph ingest path --weight 3` make the same file; on a file with a header, a position past the header is refused as today (`BeyondHeader`)

**Depends on** {{#m2-34-build-columns}}
