# The community hierarchy: layout, version 1

Status: specification for 0.1, written by #62 (`plan/issues/m4-06-hierarchy-artifact.md`).
Implemented by `crates/everygraph-community/src/hierarchy/`. It is a result kept in the
graph file, with parts (`eg-container.md` § 6, "Results" and "Parts of a result"). The
semantics it stores are those of `graph-semantics.md` § 5.5 (S5, S6).

A **hierarchy** is a sequence of `L ≥ 1` nested partitions of the nodes of a graph, as
community detection makes them: each level's communities are unions of the communities of
the level below. Level 1 is the coarsest (the partition, `docs/api-sketch.md` § 2.12),
level `L` the finest. Every level holds exact counts and its **quotient graph**.

Contents: 1 Names · 2 Numbering · 3 What a writer and a checker guarantee · 4 Versions ·
5 Decided here.

---

## 1. Names

The result is `results/communities/<key>` (operation `communities`, version 1). Its key
holds every option of the run that can change the partition: `method` (`leiden`),
`iterations`, `resolution`, `seed`, `refinement`, `prolong` and `split`, with the
reserved `graph`, `version` and `deterministic`. Its sections, all referring to the
result's record:

| Section | Kind, type | Values |
|---|---|---|
| `<result>` | 10, `u64` | `L`, then `n`, the nodes of the graph |
| `<result>/membership` | 10, `u32` | `n` values: the level-`L` community of each node |
| `<result>/<ℓ>/nodes` | 10, `u32` | one per community of level `ℓ`: its nodes |
| `<result>/<ℓ>/internal` | 10, `u64` | `L(c)`: the weight of the pairs inside, a kept loop once |
| `<result>/<ℓ>/volume` | 10, `u64` | `S(c)`: the sum of its nodes' strengths, a kept loop's `2w` included |
| `<result>/<ℓ>/parent` | 10, `u32` | for `ℓ ≥ 2`: the level-`ℓ − 1` community holding it |
| `<result>/<ℓ>/quotient.index`, `.data`, `.events` | 11, 12, 13 | the quotient graph of level `ℓ` |

`ℓ` is written in decimal, from 1. The number of communities of level `ℓ` is the length of
its `nodes`. The weight of the pairs between a community and the rest of the graph is
`S(c) − 2·L(c)` and is not stored. On an unweighted graph every weight is a count of pairs
(`internal_edges` and `external_edges` of `c.table()`).

**The quotient graph** of level `ℓ` is an undirected adjacency (`eg-adjacency.md`) over
the communities of the level: an entry between `a ≠ b` in both lists when some pair joins
them, and a loop at `a` when some pair lies inside it, kept loops of the input included
(S6). An entry's number of events is its weight: the weight of the pairs between the two
communities, or `L(a)` for the loop. When every entry weighs 1 there is no `.events`
section (`eg-adjacency.md` § 3.1). So the quotient's total weight is the graph's `W`, and
its modularity under singletons equals that of the level (S5).

## 2. Numbering

The communities of every level are numbered from 0 by decreasing `nodes`; of two with the
same count, the one holding the smaller node comes first. The numbering is a function of
the partition alone. So a run that gives the same partition — any deterministic run of the
same input and seed (DESIGN D14) — gives every community at every level the same id, and a
region can be cited by its level and id.

## 3. What a writer and a checker guarantee

A writer checks each level as it writes it, and `everygraph verify --deep` checks every
stored hierarchy of the file's graph again:

1. The `nodes` of each level add up to `n`, and each community's to the sum of its
   children's (or of its members' in `membership`); no community is empty; the numbering is
   that of § 2.
2. The `internal` and `volume` of each level equal those of its quotient graph under
   singletons: the loop's weight, and the sum of the row with the loop counted twice.
3. They also equal those of the partition that `parent` makes of the level below's
   quotient graph, and at level `L` those of `membership` on the graph (`adjacency.out`,
   when undirected). So at every level the volumes add up to `2W`, internal and cut
   weights to `W`, and the modularity of the level equals that of the level below under
   the partition it represents, bit for bit (every sum is an integer).

## 4. Versions

The layout is version 1 of the operation `communities`; a change of layout or of what
fills it is a new version, and so a new key (`eg-container.md` § 6).

## 5. Decided here

- **One result with parts**, not a section per array with keys of their own: a hierarchy
  is found, replaced and listed as one thing.
- **Weights as events of a compressed adjacency**, not a per-entry weight column: they
  are integers, and an Elias–Fano sequence of event offsets takes about
  `2 + log2(events / entries)` bits per entry against 64 for a `u64` or an `f64`. The
  quotient graph is then read by every kernel that reads an adjacency.
- **Levels numbered from the top**, as the API numbers them, so that level 1 is the
  partition whatever the depth.
- **The cut is derived, not stored**: `S(c) − 2·L(c)` is exact, and a stored copy could
  only disagree.
