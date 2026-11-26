# The `.egmap` map file: version 1.0

Status: specification for 0.1, written by #68 (`plan/issues/m5-02-tile-format.md`).
Implemented by `crates/everygraph-map/src/egmap/`. An independent reader written from
this text alone is `tools/egmap.py`; `tools/check_egmap_readers.py` compares it with the
Rust reader and opens the file with the official PMTiles readers for Python and
JavaScript. The decisions it implements are DESIGN D18 with ADR 0018 (data tiles in
PMTiles v3), ADR 0017 (nested discs), ADR 0016 with the map tree of #63 (its region
kinds), and the rows of `design/DECISIONS.md` on the viewer, the super level, the
connections panel (#20) and WebGL 2 (#26).

A map file is a **PMTiles v3 archive** of data tiles. A tile holds what is drawn at its
zoom: regions and nodes as discs, and the labels chosen for it at build time. Besides the
tiles, the file holds **arrays**: the region tree with its exact counts, node positions,
per-node adjacency and the sibling matrices of the connections panel. A **section
table** that standard PMTiles readers do not see lists the arrays. A viewer opens the
file with one request for its first 16 KiB, then fetches one tile, or one chunk of an
array, per request.

Contents: 1 Conventions · 2 The container · 3 What the map holds · 4 The tile payload ·
5 Arrays · 6 Zoom consistency · 7 Summary maps · 8 Versions · 9 Hostile input ·
10 Decided here.

```
offset 0       PMTiles v3 header          127 bytes
       127     root directory             gzip
               section table              48 + 64 × n bytes; ends at or before 16384
               unused bytes up to 16384 (zeros)
       16384   tile data                  each tile once, in tile-id order, gzip
               arrays                     chunks (gzip), then a chunk index
               leaf directories           gzip
               metadata                   JSON, gzip
```

The order after byte 16384 is the writer's; a reader goes by the offsets.

---

## 1. Conventions

- Integers are little-endian. Offsets and lengths are `u64` bytes.
- Floating-point values are IEEE 754 binary32, little-endian, finite, and never −0.0
  (written as +0.0). The same content is then the same bytes on every platform.
- **Coordinates** are in units of the map's width: the map is the square `[0, 1]²`, `x`
  to the right, `y` down, as PMTiles tiles are numbered. Tile `(z, x, y)` is the closed
  square `[x / 2^z, (x + 1) / 2^z] × [y / 2^z, (y + 1) / 2^z]`. A tile is 256 pixels
  wide, so at zoom `z` a length `d` is `d · 256 · 2^z` pixels.
- **Precision.** The deepest zoom is 16. Between 0.5 and 1 two neighbouring binary32
  values differ by 2^-24 of the map's width, which is one pixel at zoom 16; below 0.5
  they are closer. So at every zoom of a file two positions that differ by one pixel
  have different stored values, and a WebGL 2 shader, also binary32, draws them apart.
- **Gzip** is RFC 1952: one member, nothing after it. It is the only compression in the
  file.
- **The cap** is 1 MiB (1,048,576 bytes): no tile payload, directory, metadata or chunk
  decodes to more. A reader stops inflating at the cap and refuses the item.
- `pad8(n)` is `n` rounded up to a multiple of 8. Padding and reserved bytes are zero;
  readers check that they are.
- "Refuse" means the reader returns an error for the item and does not use any of it.
- **Arithmetic** on offsets, lengths and counts read from a file is checked: a sum or
  product that does not fit in 64 bits is a refusal, never a wrap.

## 2. The container

### 2.1 Header

The first 127 bytes are a PMTiles v3 header (PMTiles specification, version 3). A map
file sets these fields as follows; a reader refuses a file where one differs, except
where the table says "any".

| Field | Value |
|---|---|
| magic, version | `PMTiles`, 3 |
| root directory offset | 127 |
| root directory length | ≥ 1; the root ends at or before byte 16384 |
| metadata offset, length | any, length ≥ 1 |
| leaf directories offset, length | any (length 0 when there are no leaves) |
| tile data offset, length | any |
| addressed tiles, tile entries, tile contents | the same number: the number of tiles |
| clustered | 1 |
| internal compression, tile compression | 2 (gzip) |
| tile type | 0 (other) |
| min zoom, max zoom | the shallowest and the deepest zoom of a tile (both 0 when there is none); max zoom ≤ 16 |
| bounds, center | any; writers put the whole Web Mercator square, center zoom 0 |

Every region the header names lies inside the file. The header, the root directory and
the section table (from byte 0 to the end of the table), the metadata, the leaf
directories, the tile data and every array do not overlap. Bytes outside all of them
are not read; writers write zeros there.

### 2.2 Metadata

The PMTiles metadata is a JSON object, gzip, at most the cap decoded. Its keys:

| Key | Value |
|---|---|
| `format` | `"everygraph-egmap"` |
| `version` | `"1.0"`: the version of this specification the writer followed; a reader accepts `"1.<n>"` |
| `name` | the graph's name, a string |
| `description` | one line on what the map shows, a string |
| `attribution` | where the graph comes from, a string; in a summary map, a label only (§ 7) |
| `zoom` | an object with four finite numbers, the thresholds of § 3.1: `region_px`, `open_px`, `node_px`, `label_off_px` |

All six keys are required, and other keys are allowed, in the object and in `zoom`. The whole-file check (§ 2.4)
refuses metadata that is not such an object. The metadata
describes; it does not locate anything: everything a reader needs to find tiles and
arrays is in the header and the section table, and the exact counts are in the arrays,
not in the JSON.

### 2.3 Directories

Directories are PMTiles v3 directories: a varint count (unsigned LEB128), then the
tile-id deltas (the first one is the first id), the run lengths, the lengths and the
offsets, each as varints. An offset is stored as `offset + 1`, or as 0 for an entry
that starts where the previous one ends; the first entry's is never 0. A varint takes
at most 10 bytes and fits in 64 bits, and the directory ends with its last offset.

The **tile id** of `(z, x, y)` is the number of tiles of zooms `0 … z − 1`,
`(4^z − 1) / 3`, plus the position of `(x, y)` on the Hilbert curve of order `z`
(PMTiles specification, "Tile IDs"). A map file narrows the rules:

- Ids increase strictly within a directory, and each entry has length ≥ 1.
- The root holds tile entries (run length 1) or leaf entries (run length 0). A leaf
  holds tile entries only: one level of leaves, no runs of repeated tiles.
- A leaf entry's id is the first id of its leaf, and a leaf holds only ids from its own
  id up to, not including, the next root entry's id.
- Leaves fill the leaf directories in id order, each byte once: the first starts at
  offset 0 of the leaf directories, each next one where the one before ended, and the
  last ends at their end.
- Tiles fill the tile data in the same way, in tile-id order. No two entries share
  bytes.
- A tile's payload names its own `(z, x, y)` (§ 4.1); it must be the tile its id stands
  for.

A writer puts all entries in the root when they fit; otherwise it cuts them into leaves
of 4,096 entries, doubling the leaf size until the root fits, up to 65,536 entries a
leaf (at most 15 bytes an entry, so a leaf decodes within the cap).

### 2.4 Reading

**Open.** Read the first `min(16384, file length)` bytes. Check the header (§ 2.1).
Inflate and parse the root directory (§ 2.3); check that each tile entry lies inside the
tile data and that the leaf entries fill the leaf directories. The section table starts
at `127 + root length` (§ 5.1); check it and the types and lengths of its arrays (a
chunk index is read with its chunks, not on open). Check
that no two regions overlap (§ 2.1).

**A tile.** Find the last root entry whose id is at most the tile's id. A tile entry
with that id is the tile; a leaf entry: fetch, inflate and check the leaf, then search
it. Fetch the tile's bytes, inflate within the cap, decode and check the payload (§ 4),
and check that it names the tile asked for, that its zoom is within the header's zooms,
that no label is drawn past `max zoom + 1`, that its region ids and its nodes' region
ids are below the table's `regions` and its node ids below `nodes`, that only a summary map has regions of kind
other, and, in a summary map, that it holds no node.

**The whole file** (`MapReader::verify`, and `tools/egmap.py dump`): the metadata
(§ 2.2); every tile in tile-id order, checking that tiles and leaves fill their regions
(§ 2.3), that the number of tiles is the header's, and that the shallowest and deepest
zoom of a tile are the header's; every chunk of every array (§ 5.2) and the rules of
§ 5.3; the rules of § 3 that tie tiles to the arrays and to each other (§ 3.6); and the
census of § 6.

## 3. What the map holds

The map draws the hierarchy of the graph's communities, the **map tree** of #63, as
**nested discs**: each region a disc inside its parent's disc, each node a small disc
inside its region's (ADR 0017). The **entities** of the map are its regions and its
nodes. Facts about a region — its parent, kind and exact counts — are in the region
table (§ 5.3), once. A tile holds the entities drawn at its zoom that meet it, with what
drawing them needs.

The rules of this section are the builder's (#69). Readers check those that one tile
shows (§ 4.3) and those of § 3.6; the census of § 6 and the builder's tests check the
others (the values of `zmin` and `z_off`, and that every tile a disc meets holds it).

### 3.1 Zoom and `zmin`

Each entity has a **minimum zoom** `zmin`. It is drawn at every zoom from `zmin` to the
map's deepest zoom, and at no zoom before. `zmin` depends on the geometry and the
hierarchy only — never on which tile, which other entities, or which budget — so an
entity drawn at one zoom is drawn at every deeper zoom (Been, Daiches, Yap, TVCG 2006;
Been, Nöllenburg, Poon, Wolff, CGTA 2010; MSAGL.js § 3.1).

With `size(r, p)` the least `z ≥ 0` with `r · 256 · 2^z ≥ p` (exact: each side is a
binary32 value times a power of two, computed in binary64), and the four thresholds of
the metadata (`zoom` in § 2.2):

- a region `R` of radius `r` under the root: `zmin(R) = min(size(r, region_px), zmax)`;
- a region `R` of radius `r` with parent `P` of radius `r_P` other than the root:
  `zmin(R) = min(max(size(r, region_px), zmin(P), size(r_P, open_px)), zmax)` —
  it is large enough, and its parent is drawn and **open**;
- a node of radius `r` in region `R` of radius `r_R`:
  `zmin = min(max(size(r, node_px), zmin(R), size(r_R, open_px)), zmax)`.

`zmax` is the map's deepest zoom: at `zmax` every entity is drawn. The builder chooses
the thresholds; the file records them, and readers do not recompute `zmin`.

### 3.2 Regions

A region is a node of the map tree. Its **kind** is one of:

| Code | Kind | What |
|---|---|---|
| 0 | community | a community of the partition the map draws (#63's `Community`) |
| 1 | subcommunity | a part of a community that the map tree split again, drawn dashed |
| 2 | aggregate | the smaller communities of one parent drawn as one region, "+N smaller communities" |
| 3 | other | in a summary map only: the regions and pairs below its `k` (§ 7) |
| 4 | super | a region of the super level above the partition (`design/DECISIONS.md`, "The map's super level") |
| 5 | root | region 0, the whole graph, never drawn |

Regions are numbered from 0, breadth first: region 0 is the root; the children of a
parent have consecutive ids, larger regions first (by node count; of two with the same
count, the one the map tree numbers first), and a parent's id is below its children's.
So the parent ids, read in id order, never decrease, and a region's **rank** among its
siblings — the key `#3` of the approved design — is its id minus the first id with the
same parent, plus 1. The tree is at most 255 regions deep below the root. A super
region's parent is the root or a super region.

The **colour** of a region is the index (0–4) of its top region's colour in the
five-colour assignment, computed once at build time: a child of the root has its own,
every other region its parent's.

### 3.3 Nodes

A node record holds its node id in the graph, the deepest region that holds it, its
disc, its original identifier as text (`name`), its degree in the undirected projection
with a kept loop counted twice (`degree`, `graph-semantics.md`), the colour of its
region, and its `zmin`. Its tree path is its region and that region's ancestors, from
`regions.parent` (§ 5.3).

### 3.4 Labels

Labels are chosen at build time, so that what a tile shows does not depend on the
frame, and each label has a **zoom interval** `[z_on, z_off)`: it is drawn at every
zoom from `z_on` up to, not including, `z_off`.

- A label is anchored to its entity: `centre` (the text centred on the disc's centre) or
  `below` (centred under the disc, its top on the disc's lowest point). Its box is the
  text's box in the viewer's font metrics (#121, `m5-14-design-system`), in pixels.
- The builder visits zooms from 0 to `zmax`. At zoom `z` the candidates are the entities
  drawn at `z` that have a text, in a fixed order: depth (regions before nodes, shallow
  before deep), then node count or degree (larger first), then id. A candidate already
  on stays on. A new candidate is accepted if its box overlaps no box accepted at `z`;
  it then has `z_on = z`.
- Anchors are points of the map that do not move with the zoom, and boxes keep their
  size in pixels, so the distance in pixels between two anchors doubles at each deeper
  zoom: two boxes that do not overlap at `z` do not overlap deeper. A label once on
  therefore never collides again.
- `z_off` is `zmax + 1` for a node, and for a region the first zoom at which its radius
  reaches `label_off_px` pixels (`size(r, label_off_px)`, at most `zmax + 1`): its
  children carry the labels from then on.

A tile at zoom `z` holds a label exactly when its entity is in the tile and
`z_on ≤ z < z_off`, with the same text, interval and anchor in every tile.

### 3.5 Placement and the budget

Let `w = 1 / 2^z`, `x0 = x·w`, `x1 = (x + 1)·w` and `y0`, `y1` the same in `y`, for tile
`(z, x, y)`, and `(cx, cy, r)` a disc. In binary64, from the stored binary32 values, each
operation one IEEE 754 operation rounded to nearest:

- the disc **meets** the tile when `dx·dx + dy·dy ≤ r·r`, with
  `dx = max(x0 − cx, cx − x1, 0)` and `dy` the same in `y`;
- the disc **covers** the tile when `ex·ex + ey·ey ≤ r·r`, with
  `ex = max(cx − x0, x1 − cx)` and `ey` the same in `y`: every corner is inside.

An entity is in tile `(z, x, y)` when `zmin ≤ z`, its disc meets the tile, and its disc
does not cover the tile. A region that covers a tile has no outline in it, and the
approved viewer draws no tint for a region that covers the view; the outline of a region
is in some tile at every zoom from its `zmin`.

**Containment.** A disc `(cx, cy, r)` is **inside** a disc `(px, py, pr)` when
`r ≤ pr` and `(cx − px)² + (cy − py)² ≤ (pr − r)²`, computed in the same way. The
builder makes each region's disc inside its parent's, and each node's disc inside its
region's, in the stored values; the root has no disc.

The builder keeps a **budget** per tile as a guard. When a tile would pass it, the
builder leaves entities out, last in this order first: regions before nodes, then
`zmin` (smaller first), then depth, then node count or degree (larger first), then id.
What it leaves out is counted in the tile's `omitted` figures, never dropped silently:
`regions` and `nodes` left out, and `region_nodes`, the nodes inside the regions left
out, each node once. A budget that leaves an entity out of a tile at `z` and keeps it at
`z − 1` makes it pop (§ 6); if the deepest tiles pass 32 KB under the rule above, the
budget becomes nested: an entity kept in a tile is kept in every tile below it that it
is placed in.

### 3.6 What ties tiles to the arrays

The whole-file check (§ 2.4) refuses a map where:

- a region of a tile has a node count or a kind other than the region table's;
- a region's colour is not its parent's (below the top), or a node's colour is not its
  region's;
- a region's disc is not inside its parent's, or a node's is not inside its region's;
- a region's `zmin` is below its parent's, or a node's below its region's;
- the same entity has two different records in two tiles, or the same label two
  different texts, intervals or anchors.

Each check needs the entity and its parent or region seen in some tile; a summary map
and the budget can leave some out, and those are not checked.

### 3.7 Edges

A tile holds no edges. The approved viewer draws links for the selection only, from the
per-node adjacency (§ 5.3); the connections panel draws the sibling matrices of § 5.3,
and names their rows from the region table (`design/DECISIONS.md`, 2026-11-26, #20). The
figures the caption shows are exact: the weight inside and between regions is in the
region table, and a selection's links are its whole adjacency list.

## 4. The tile payload

A tile is stored gzip. Its payload — what the gzip decodes to — is at most the cap.

### 4.1 Header

```
offset  size  field
0       4     magic                 "EGMT"
4       2     major version         1
6       2     minor version         0
8       1     z                     0 … 16
9       3     reserved
12      4     x                     below 2^z
16      4     y                     below 2^z
20      4     reserved
24      8     omitted regions
32      8     omitted region nodes
40      8     omitted nodes
48            blocks, to the end of the payload
```

### 4.2 Blocks

A block is a 16-byte header and a body:

```
offset  size  field
0       4     tag      bit 31: required; bits 0–30: the block number, from 1
4       4     rows     ≥ 1
8       8     length   of the body, a multiple of 8
16            body
```

Block numbers start at 1 and increase strictly through the payload; a block number 0 is
refused. A reader that does not know a block's number skips the block if bit 31 is
clear and refuses the tile if it is set. The blocks of this version carry bit 31; one of
their numbers without it is refused. A block of zero rows is not written; a missing
block holds zero rows.

A body is a sequence of **columns**, one value per row each, every column padded with
zero bytes to `pad8` of its length. The length of a known block is exactly the sum of
its padded columns.

| Tag | Block | Columns, in order |
|---|---|---|
| `0x80000001` | regions | `id` u32, `x` f32, `y` f32, `r` f32, `nodes` u64, `kind` u8, `colour` u8, `zmin` u8 |
| `0x80000002` | nodes | `id` u32, `region` u32, `x` f32, `y` f32, `r` f32, `name` u32, `degree` u64, `colour` u8, `zmin` u8 |
| `0x80000003` | labels | `entity` u32, `text` u32, `z_on` u8, `z_off` u8, `anchor` u8 |
| `0x80000004` | strings | `end` u32 (one per string: the end of its bytes), then the UTF-8 bytes of all strings, one after the other |

`name` and `text` are indexes into the strings. A label's `entity` is an index into the
tile's entities: `i` below the number of regions is region `i` of the tile, and
`regions + j` is node `j`. `anchor` is 0 for centre, 1 for below.

Columns are aligned to 8 bytes within the body, and the header and block headers are
multiples of 8, so a JavaScript reader can view each column of the inflated payload as a
typed array without a copy.

### 4.3 Rules

A reader refuses a tile that breaks any of these; a writer refuses to write one.

1. The header: magic, major version 1 (another major version is a later format:
   refused as such), zero reserved bytes, `z ≤ 16`, `x` and `y` below `2^z`.
2. The framing of § 4.2: block numbers from 1 and increasing, known lengths exact,
   padding zero, rows ≥ 1, string ends not decreasing and the last equal to the bytes'
   length, every string UTF-8.
3. Regions: ids increasing strictly and ≥ 1; `kind` ≤ 4; `colour` ≤ 4; `zmin ≤ z`;
   `nodes ≥ 1`.
4. Nodes: ids increasing strictly; `region ≥ 1`; `colour` ≤ 4; `zmin ≤ z`; `name` a
   string of the tile.
5. Labels: entities increasing strictly, each an entity of the tile; `text` a string of
   the tile; entity `zmin ≤ z_on ≤ z < z_off ≤ 17`; `anchor` ≤ 1.
6. Strings: increasing strictly in byte order, so each string once.
7. Geometry: `x` and `y` in `[0, 1]`, `0 < r ≤ 1`, and the disc meets the tile and does
   not cover it (§ 3.5).

The bytes of a tile are a function of its content: the only freedom is the minor
version, and the optional blocks a later writer may add after the known ones.

## 5. Arrays

### 5.1 The section table

The section table starts right after the root directory, at `127 + root length`, and
ends at or before byte 16384, so the first request of a viewer holds it.

```
offset  size  field
0       8     magic            "EGMAPTBL"
8       2     major version    1
10      2     minor version    0
12      4     entries          n
16      8     nodes            of the graph
24      8     regions          of the map tree, the root included (≥ 1)
32      8     edges            pairs of the graph, a kept loop once
40      4     summary k        0 for a full map; k ≥ 1 for a summary map (§ 7)
44      4     reserved
48      64·n  entries
```

An entry:

```
offset  size  field
0       32    name            1–32 bytes of a–z, 0–9, ".", "_", "-", then zero bytes
32      1     element         1 u8, 3 u32, 4 u64, 5 f32
33      3     reserved
36      4     chunk           values per chunk, ≥ 1; chunk × width ≤ the cap
40      8     count           values in all
48      8     offset          of the array in the file
56      8     length          of the array in bytes
```

Names increase strictly in byte order. A file whose table has another magic is not a
map; another major version is a later format. A reader ignores arrays whose names it
does not know.

### 5.2 Chunked arrays

An array of `count` values is cut into `k = ceil(count / chunk)` chunks of `chunk`
values, the last one shorter. The array's bytes are the chunks, each the gzip of its
values' little-endian bytes, then the **chunk index**: `k + 1` u64 offsets, relative to
the start of the array, of the start of each chunk and of the end of the last. So the
index starts at `length − 8 (k + 1)`; its first offset is 0, its offsets increase
strictly, and its last is where the index starts. A chunk decodes to exactly
`(values in it) × width` bytes; f32 values are finite and not −0.0.

To read values `i … j`, fetch index entries `i / chunk` to `j / chunk + 1` (16 bytes
for one chunk), then the chunks between them.

### 5.3 The arrays of version 1.0

**The region table**, required in every map, one value per region by id (`regions`
values each):

| Name | Element | Values |
|---|---|---|
| `regions.parent` | u32 | the parent's id; 0 for the root |
| `regions.kind` | u8 | the kind code of § 3.2 |
| `regions.nodes` | u64 | its nodes, exactly; the root's is `nodes` |
| `regions.internal` | u64 | the weight of the pairs inside it, a kept loop once |
| `regions.volume` | u64 | the sum of its nodes' degrees, a kept loop's twice |
| `regions.communities` | u32 | for an aggregate or other, the communities it holds; 0 otherwise |
| `regions.partner` | u32 | the sibling it shares the most weight with (the glyph's tick); 0 for none |

The weight of the pairs between a region and the rest of the graph is
`volume − 2 · internal`, as in `eg-communities.md` § 1; it is not stored. On an
unweighted graph every weight is a count of pairs. The rules:

- `parent[0] = 0` and `kind[0]` is root; for every other id, `parent < id`, the parents
  never decrease along the ids, the kind is not root, and a region has at most the node
  count of the sibling before it;
- the tree is at most 255 deep below the root; a super region's parent is the root or a
  super region; kind other only in a summary map;
- `nodes ≥ 1` below the root, `2 · internal ≤ volume`, `communities ≥ 1` exactly for
  aggregate and other, and `partner` 0 or a sibling (another region with the same
  parent); the root's `partner` is 0;
- the children's `nodes`, `internal` and `volume` add up to at most their parent's.

**Nodes and links**, each optional (a map without `adjacency.*` draws no links), and
never in a summary map:

| Name | Element | Count | Values |
|---|---|---|---|
| `positions` | f32 | `2 · nodes` | `x` then `y` of each node, by node id, in `[0, 1]` |
| `adjacency.offsets` | u64 | `nodes + 1` | where each node's neighbours start in `adjacency.targets`; from 0, not decreasing, ending at its count |
| `adjacency.targets` | u32 | any | each node's neighbours in the undirected projection, increasing strictly, below `nodes`, never the node itself |

The two adjacency arrays come together. The adjacency is symmetric: `v` is a neighbour
of `u` exactly when `u` is one of `v`. `MapReader::verify` checks this in one pass with
a hash of the pairs, which a file crafted against its fixed mixes can pass; the harm of
such a file is a link drawn from one of its ends only. `tools/egmap.py` checks it
exactly.

**The connections panel**, optional, the three together (#20):

| Name | Element | Count | Values |
|---|---|---|---|
| `matrix.offsets` | u64 | `regions + 1` | where each region's pairs start in `matrix.pairs`; from 0, not decreasing, ending at its count |
| `matrix.pairs` | u32 | any | the pairs of a parent's children with weight ≥ 1: `(i << 16) | j`, the children's ranks, `1 ≤ i < j ≤` its number of children, increasing strictly within a parent |
| `matrix.weights` | u64 | that of `matrix.pairs` | the weight of the pairs between children `i` and `j` |

A child of rank `i` of parent `p` is region `first(p) + i − 1`, where `first(p)` is the
first id whose parent is `p`; its name, counts and kind are in the region table, so the
panel names every row, also of children that no loaded tile holds. Ranks fit 16 bits:
#63 bounds a parent to 64 children, and a map tree with a parent of more than 65,535
children has no matrix for it. The lab
measured the matrices at 11.2 MB of gzip on lj (1.3% of its map; 8.5 MB on orkut), with
the largest parent's block at 11.9 KB (`lab/m1-06-edge-rendering/results/overview_lj.json`).

Search arrays (`search.*`) are #70's; attribute summaries (`attributes.*`) are optional
and come with #79.

## 6. Zoom consistency

`Census` counts, over a set of tiles — a whole pyramid, or the tiles a scripted dive
fetches — what breaks § 3:

| Count | What |
|---|---|
| `popping` | entities seen at a zoom, not at a deeper one, then seen again |
| `vanished` | entities not seen at the deepest zoom |
| `late` | entities first seen after their `zmin` |
| `inconsistent` | entities or labels whose record differs between two tiles |
| `label_popping` | labels seen, not seen deeper, then seen again |
| `label_cut` | labels not seen at every zoom of their interval |

The target for a built map is zero on each count, over the whole pyramid (#69 checks
lj and orkut). For scale: the lab's per-tile top-k selection makes 36,293 lj nodes pop
(0.91%) and 20,544 orkut nodes (0.67%), almost all from zoom 7 to 8
(`lab/m1-06-edge-rendering/results/popping_lj.json`, `popping_orkut.json`).

## 7. Summary maps

A map built with `reveal="summary"` (#196, `m5-15-summary-export`) has `summary k ≥ 1`
in its section table, and:

- no node entities in any tile, and no `positions`, `adjacency.*` or `search.*`
  arrays (readers check both);
- regions under `k` nodes merged into an aggregate; pairs below `k` in the matrices
  merged into a region of kind **other**, with totals that still add up;
- `attribution` a label only, and `k` shown in the caption.

The rules that make these merges, and the test that the summary cannot be undone, are
#196's.

## 8. Versions

The tile payload, the section table and the metadata each carry a version. A minor
version adds what an older reader can do without — an optional block, an array, a
metadata key — and readers accept any minor version of their major. A major version
changes what is there; readers refuse it as a later format.

## 9. Hostile input

A map file comes from anywhere: a URL, a mail. What bounds a reader's work and memory:

- **No declared size becomes memory before the bytes are there.** A directory's entry
  count is checked against its length (each entry takes at least 4 bytes) before the
  entries are allocated; every region, tile, leaf and array is checked to lie inside the
  file before it is read; a compressed item larger than the cap plus 4 KiB is refused
  unread (deflate stores incompressible data at 5 bytes per 65,535). The whole-file
  check keeps a few bytes per region and per entity it has seen, reserved through the
  memory budget.
- **Nothing decodes past the cap.** The 32 KB budget of #69 bounds a tile's gzip size,
  not what it decodes to. One DEFLATE stream cannot exceed a ratio of 1,032; the danger
  is pointers that let entries share bytes: overlapping entries in a zip container
  turned 42 kB into 5.5 GB (Fifield, WOOT'19). Hence one level of leaves, run length 1,
  and tiles and leaves that fill their regions each byte once (§ 2.3): reading every
  tile inflates each byte of the file once, at most 1,032 times its size.
- **The fuzz target** `egmap` (`crates/everygraph-format/fuzz`) decodes tile payloads,
  whole files raw and damaged, and files it lays out itself from uncompressed root,
  leaf and tile bytes, so that the directory rules are reached behind gzip's check. A
  payload that decodes must encode back to the same bytes up to its minor version and
  the optional blocks it skipped.

## 10. Decided here

- **PMTiles v3, no MVT.** The container is what PMTiles readers, CDNs and range servers
  already serve. MVT has no columns and no exact-integer guarantee for the counts
  (decided 2026-11-26, `docs/research/sota-review-2026-11.md` S12).
- **The custom payload, not MapLibre Tile (MLT) or Arrow IPC.** The three payloads were
  compared on the content of every lab tile of lj (203,941 tiles) and orkut (142,710),
  each converted to the records of § 4 (`lab/m5-02-tile-format/README.md`).
  Exploratory, load-independent byte counts, gzip level 6:

  | Graph | Payload | Gzip total | Median tile | Largest tile |
  |---|---|---|---|---|
  | lj | this payload | 378.8 MB | 1,139 B | 14,477 B |
  | lj | MLT (`mlt-core` 0.24, profile Web 4G) | 379.2 MB (+0.1%) | 1,220 B (+7.1%) | 14,011 B |
  | lj | Arrow IPC (arrow-rs 58.4, two streams) | 454.1 MB (+20%) | 1,562 B (+37%) | 14,040 B |
  | orkut | this payload | 273.9 MB | 1,188 B | 14,563 B |
  | orkut | MLT (profile Web 4G) | 273.5 MB (−0.1%) | 1,264 B (+6.4%) | 14,057 B |
  | orkut | Arrow IPC | 324.9 MB (+19%) | 1,610 B (+36%) | 14,133 B |

  Over HTTP the two are within 2% of each other in total (MLT's `MinSize` profile is
  0.2% smaller on lj and 2.0% on orkut), and MLT is lossless only with `x`, `y` and `r`
  kept as f32 property columns: its geometry is integer coordinates within the tile's
  extent. So the payload stays ours: it needs no decoder library in a viewer with a
  150 KB budget (#77), and its columns are typed-array views of the inflated bytes. The
  question is closed for this major version; the decode time in JavaScript was not
  measured.
- **The region table in arrays, not in tiles.** A region's parent, kind and counts are
  written once, not in every tile and at every zoom it is drawn; the panel names the rows
  of children no tile holds; tiles need not carry ancestors. The table is 8.25 MB of
  gzip on lj (1,637,273 regions) and 4.95 MB on orkut (872,753); with it, the tiles and
  the table together are 15% (lj) and 14% (orkut) smaller than tiles that carry the
  facts and the ancestors themselves (`lab/m5-02-tile-format/README.md`).
- **No edges in tiles** (§ 3.7): they were most of the lab's bytes (830.6 MB of gzip
  tiles on lj with edges and 721.1 MB on orkut, `lab/m5-02-tile-format/README.md`).
- **A region that covers a tile is not in it** (§ 3.5): at deep zooms the large regions
  would otherwise be repeated in every tile inside them. On the lab's tiles this drops
  567,871 region records on lj and 496,654 on orkut.
- **Arrays in a table after the root directory**, not in the JSON metadata: offsets and
  counts stay exact integers, a reader needs no JSON parser to find anything, and the
  first 16 KiB a PMTiles client fetches already holds the table. Standard readers never
  look between the root directory and the regions the header names.
- **Chunked arrays with the index at the end**, so that a writer streams an array
  without knowing its length first, and a viewer reads one chunk with two small reads.
- **Zoom 16 at most, binary32 coordinates** (§ 1), rather than quantized tile-local
  coordinates: one coordinate system for tiles, arrays and shaders.
- **The cut is derived, not stored**, as in `eg-communities.md`.
