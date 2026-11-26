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
5 Decided here · 6 The map tree.

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

A run with prolongation (`prolong`) stores one level, its partition: its finer levels
are the coarsening the run went through before it moved nodes again, so they do not nest
in the partition it returns. `ℓ` is written in decimal, from 1. The number of communities of level `ℓ` is the length of
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
4. The entries of each quotient graph are those its partition makes of the level below
   (`verify` only): with a random 64-bit weight `h(c)` for each community of the level,
   the sums `Σ_b A[a][b]·h(b)` over the stored row of `a` equal the sums over the rows of
   `a`'s children in the level below (or its nodes in the graph), each neighbour mapped
   to its community, modulo the prime `2^64 − 59` (`h(c)` is the draw modulo it). For an
   error that does not depend on the weights (a bug, damaged bytes), a wrong entry passes
   with probability about 2^-64 for one draw, whatever the size of the error: modulo 2^64
   an error by a multiple of 2^k would pass with probability 2^(k−64). The weights come
   from `everygraph_core::rng` with the purpose `HCHK`, counter `[community, level, 0, 0]`
   and seed 0, so the check is the same on every run, and a file crafted with the weights
   in hand can pass it. A quotient graph of `2^62` events or more is refused.

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

## 6. The map tree

The **map tree** of a partition (DESIGN D16, ADR 0016) is a tree of regions: the root is
the graph, and the regions without children, the leaves, partition its nodes. It is
the result `results/map/<key>` (operation `map`, version 1). Its key holds the options
of the Leiden run that found the partition (as in § 1), and `fan_out` (the most children
of a region), `leaf` (the most nodes of a region that is not split) and `super` (`auto`,
`always` or `never`). A rebuild with the same options replaces the map; whether the
map it replaces had a super level is an input of the rebuild, not part of the key.

**Numbering.** Region 0 is the root. The regions are numbered breadth first: every
region after its parent, and the children of a region together, by decreasing node
count, then by smallest node. So the numbering is a function of the tree.

| Section | Kind, type | Values |
|---|---|---|
| `<result>` | 10, `u64` | the regions `R`, the nodes `n`, and 1 when the map has a super level, else 0 |
| `<result>/parent` | 10, `u32` | `R` values: the parent of each region; 0 for the root |
| `<result>/kind` | 10, `u32` | the kind of each region (below) |
| `<result>/nodes` | 10, `u32` | the nodes inside each region |
| `<result>/internal` | 10, `u64` | `L(r)`: the weight of the pairs inside each region, a kept loop once |
| `<result>/volume` | 10, `u64` | `S(r)`: the sum of the strengths of its nodes, a kept loop's `2w` included |
| `<result>/membership` | 10, `u32` | `n` values: the leaf holding each node |
| `<result>/siblings.index`, `.data`, `.events` | 11, 12, 13 | the sibling graph |

| Code | Kind | Drawn as (`design/preview/style/SPEC.md`, "The map") |
|---|---|---|
| 0 | root: the graph | — |
| 1 | super: a group of communities of the partition, found by Leiden on the whole graph at a lower resolution | solid outline |
| 2 | community: a community of the partition | solid outline |
| 3 | subcommunity: a part of a community, split again for the map | dashed outline |
| 4 | aggregate: the smaller children of one parent, "+N smaller communities"; or the communities without an edge, under the root | hatched; not drawn when its parent is an aggregate |

**The sibling graph** is an undirected adjacency (`eg-adjacency.md`) over the `R`
regions. It has an entry between two children `a ≠ b` of one region when some pair of
nodes joins them, in both lists; its number of events is the weight of the pairs
between them. A pair of nodes counts once: between the two children of its leaves'
lowest common ancestor that hold its two nodes. A pair inside a leaf counts in that
leaf's `internal` only. When every entry weighs 1 there is no `.events` section. The
graph between the children of any region, which the layout places, is that region's
part of it.

The community of the partition that holds a node is the region of kind 2 on its path
from the root; there is exactly one.

`everygraph verify --deep` checks every stored map of the file's graph:

1. The parents and the numbering are those above; no region has more than `fan_out`
   children (from the record); kind 0 is the root's only.
2. On every path from the root, a super region and a community follow no community, a
   subcommunity follows one, an aggregate follows anything; every leaf has a community
   on its path. `membership` names leaves only.
3. The `nodes` of each leaf are its members in `membership`, and those of every other
   region the sum of its children's; no region is empty.
4. Every entry of the sibling graph joins two children of one region. For every region
   `p` with children: `volume(p)` is the sum of its children's, and `internal(p)` the
   sum of its children's plus half the weight of the entries between them. So the
   root's are `W` and `2W`, with integer sums that hold bit for bit.
5. Against the graph (`adjacency.out`, when undirected and in memory): the `internal`
   and `volume` of every leaf equal those of `membership`; and for every region `a`,
   the sums `Σ_b S[a][b]` and `Σ_b S[a][b]·h(b)` over its row equal those over the pairs
   of nodes that the tree gives to `a`, with a random 64-bit `h` per region, modulo
   `2^64 − 59`, as in § 3, from the purpose `HCHK`, counter `[region, 0, 0, 1]` and seed
   0.

**Decided here.**

- **A tree with parent pointers and one sibling graph**, not a hierarchy by levels:
  the map's levels are not partitions of the nodes, and the sibling graph is what the
  layout reads. Every pair of nodes counts once, so its entries are at most the
  graph's.
- **The kind is stored, the aggregate's count is not**: the communities under an
  aggregate are its descendants of another kind through aggregates, and a stored copy
  could only disagree.
