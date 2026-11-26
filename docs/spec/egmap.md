# The `.egmap` map file: version 1.0

Status: specification for 0.1, written by #68 (`plan/issues/m5-02-tile-format.md`).
Implemented by `crates/everygraph-map/src/egmap/`. An independent reader written from
this text alone is `tools/egmap.py`; `tools/check_egmap_readers.py` compares it with the
Rust reader and opens the file with the official PMTiles readers for Python and
JavaScript. The decisions it implements are DESIGN D18 with ADR 0018 (data tiles in
PMTiles v3), ADR 0017 (nested discs), and the rows of `design/DECISIONS.md` on the
viewer, the connections panel (#20) and WebGL 2 (#26).

A map file is a **PMTiles v3 archive** of data tiles. A tile holds the regions and nodes
that are drawn at its zoom, with exact counts, and the labels chosen for it at build
time. Besides the tiles, the file holds **arrays** — node positions, per-node adjacency,
the sibling matrices of the connections panel — listed in a **section table** that
standard PMTiles readers do not see. A viewer opens the file with one request for its
first 16 KiB, then fetches one tile, or one chunk of an array, per request.

Contents: 1 Conventions · 2 The container · 3 What a tile shows · 4 The tile payload ·
5 Arrays · 6 Zoom consistency · 7 Summary maps · 8 Versions · 9 Hostile input ·
10 Decided here.

```
offset 0       PMTiles v3 header          127 bytes
       127     root directory             gzip
               section table              48 + 64 × n bytes; ends at or before 16384
               zero bytes up to 16384
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
- **Gzip** is RFC 1952: one member, nothing after it. It is the only compression in the
  file.
- **The cap** is 1 MiB (1,048,576 bytes): no tile payload, directory, metadata or chunk
  decodes to more. A reader stops inflating at the cap and refuses the item.
- `pad8(n)` is `n` rounded up to a multiple of 8. Padding and reserved bytes are zero;
  readers check that they are.
- "Refuse" means the reader returns an error for the item and does not use any of it.

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
| min zoom, max zoom | the shallowest and deepest zoom with a tile (both 0 when there is none); max zoom ≤ 20 |
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
| `version` | `"1.0"`: the version of this specification the writer followed |
| `name` | the graph's name |
| `description` | one line on what the map shows |
| `attribution` | where the graph comes from; in a summary map, a label only (§ 7) |
| `zoom` | the thresholds of § 3.1: `region_px`, `open_px`, `node_px`, `label_off_px` |

The metadata describes; it does not locate anything. Everything a reader needs to find
tiles and arrays is in the header and the section table, so a reader that does not parse
JSON reads the whole map. Exact counts are in the section table and in the tiles, not in
the JSON.

### 2.3 Directories

Directories are PMTiles v3 directories: a varint count (unsigned LEB128), then the
tile-id deltas (the first one is the first id), the run lengths, the lengths and the
offsets, each as varints. An offset is stored as `offset + 1`, or as 0 for an entry
that starts where the previous one ends; the first entry's is never 0. A varint takes
at most 10 bytes and fits in 64 bits, and the directory ends with its last offset. The **tile id** of `(z, x, y)` is the number of
tiles of zooms `0 … z − 1`, `(4^z − 1) / 3`, plus the position of `(x, y)` on the
Hilbert curve of order `z` (PMTiles specification, "Tile IDs"). A map file narrows the
rules:

- Ids increase strictly within a directory, and each entry has length ≥ 1.
- The root holds tile entries (run length 1) or leaf entries (run length 0). A leaf
  holds tile entries only: one level of leaves, no runs of repeated tiles.
- A leaf entry's id is the first id of its leaf, and a leaf holds only ids from its own
  id up to, not including, the next root entry's id.
- Leaves fill the leaf directories in id order, each byte once, as tiles fill the tile
  data.
- Tiles fill the tile data exactly once, in tile-id order: the first tile starts at
  offset 0 of the tile data, each next one where the one before ended, and the last ends
  at the end of the tile data. No two entries share bytes.
- A tile's payload names its own `(z, x, y)` (§ 4.1); it must be the tile its id stands
  for.

A writer puts all entries in the root when they fit; otherwise it cuts them into leaves
of 4,096 entries, doubling the leaf size until the root fits, up to 65,536 entries a
leaf (at most 15 bytes an entry, so a leaf decodes within the cap).

### 2.4 Reading

**Open.** Read the first `min(16384, file length)` bytes. Check the header (§ 2.1).
Inflate and parse the root directory (§ 2.3); check that each entry lies inside the
tile data (tile entries) or the leaf directories (leaf entries). The section table
starts at `127 + root length` (§ 5.1); check it and its arrays. Check that no two
regions overlap (§ 2.1).

**A tile.** Find the last root entry whose id is at most the tile's id. A tile entry
with that id is the tile; a leaf entry: fetch, inflate and check the leaf, then search
it. Fetch the tile's bytes, inflate within the cap, decode and check the payload (§ 4),
and check that it names the tile asked for, that its zoom is within the header's zooms,
that no label is drawn past `max zoom + 1`, that its region ids and partners are below
the table's `regions` and its node ids below `nodes`, that only a summary map has
regions of kind other, and, in a summary map, that it holds no node.

**The whole file** (`MapReader::verify`): the metadata; every tile in tile-id order,
checking the tiling of the tile data (§ 2.3) and that the number of tiles is the
header's; every chunk of every array (§ 5.2) and the rules of § 5.3; and the census of
§ 6.

## 3. What a tile shows

The map draws the hierarchy of the graph's communities as **nested discs**: each region
a disc inside its parent's disc, each node a small disc inside its region's (ADR 0017).
The **entities** of the map are its regions and its nodes. A tile at zoom `z` holds the
entities drawn at zoom `z` whose disc meets the tile.

The rules of this section are the builder's (#69). A reader checks those of § 2.4 and
§ 4.3, which one tile shows; the census of § 6 and the builder's tests check the others
(the values of `zmin` and `z_off`, that every tile a disc meets holds it).

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
- a region `R` of radius `r` with parent `P` of radius `r_P`:
  `zmin(R) = min(max(size(r, region_px), zmin(P), size(r_P, open_px)), zmax)` —
  it is large enough, and its parent is drawn and **open**;
- a node of radius `r` in region `R` of radius `r_R`:
  `zmin = min(max(size(r, node_px), zmin(R), size(r_R, open_px)), zmax)`.

Since a child's disc is inside its parent's, a child is never drawn before its parent.
`zmax` is the map's deepest zoom: at `zmax` every entity is drawn. The builder (#69)
chooses the thresholds; the file records them, and readers do not recompute `zmin`.

### 3.2 Regions

A region is a part of the map hierarchy: a **community** of the hierarchy, a
**subdivision** (a part of a community that the map hierarchy split so that regions stay
small), an **aggregate** (small communities of one parent drawn as one region), or, in a
summary map only, **other** (§ 7). The map numbers its regions from 0, breadth first:
region 0 is the root, which is never drawn; a parent's id is below its children's. A
region record holds:

| Field | Meaning |
|---|---|
| `id`, `parent` | its id; its parent's id (0 at the top) |
| `x`, `y`, `r` | its disc |
| `nodes` | its nodes, exactly |
| `internal` | the weight of the pairs inside it, a kept loop once |
| `volume` | the sum of its nodes' degrees, a kept loop's twice |
| `communities` | for an aggregate or other, the communities it holds (≥ 1); 0 otherwise |
| `rank` | its place among its siblings by node count, from 1: the key `#3` of the approved design |
| `partner` | the sibling it shares the most weight with (the glyph's tick); 0 for none |
| `kind` | 0 community, 1 subdivision, 2 aggregate, 3 other |
| `depth` | 1 for a child of the root, its parent's depth + 1 below |
| `colour` | the index (0–4) of its top region's colour in the five-colour assignment, computed once at build time |
| `zmin` | § 3.1 |

The weight of the pairs between a region and the rest of the graph is
`volume − 2 · internal`, as in `eg-communities.md` § 1; it is not stored. On an
unweighted graph every weight is a count of pairs.

### 3.3 Nodes

A node record holds its node id in the graph (`id`), the deepest region that holds it
(`region`), its disc, its original identifier as text (`name`), its degree in the
undirected projection with a kept loop counted twice (`degree`, `graph-semantics.md`),
and its `zmin`. Its tree path is its region, then that region's ancestors, all of which
are in the same tile (§ 3.5).

### 3.4 Labels

Labels are chosen at build time, so that what a tile shows does not depend on the
frame, and each label has a **zoom interval** `[z_on, z_off)`: it is drawn at every
zoom from `z_on` up to, not including, `z_off`.

- A label is anchored to its entity: `centre` (the text centred on the disc's centre) or
  `below` (centred under the disc, its top on the disc's lowest point). Its box is the
  text's box in the viewer's font metrics (#121, `m5-14-design-system`), in pixels.
- The builder visits zooms from 0 to `zmax`. At zoom `z` the candidates are the entities
  drawn at `z` that have a text, in a fixed order: depth (regions before nodes, shallow
  before deep), then `nodes` or `degree` (larger first), then id. A candidate already on
  stays on. A new candidate is accepted if its box overlaps no box accepted at `z`; it
  then has `z_on = z`.
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

An entity is in tile `(z, x, y)` when `zmin ≤ z` and its closed disc meets the tile's
closed square (§ 1). In binary64, from the stored binary32 values, with
`w = 1 / 2^z`, `dx = max(x·w − cx, cx − (x + 1)·w, 0)` for the disc's centre `cx`, and
`dy` the same in `y`: the disc meets the tile when `dx·dx + dy·dy ≤ r·r`. Each
operation is one IEEE 754 operation rounded to nearest, so every reader gets the same
answer. So a region's
ancestors are in every tile the region is in: their discs hold its disc, and their
`zmin` is not above its own.

The builder keeps a **budget** per tile as a guard. When a tile would pass it, the
builder leaves entities out, last in this order first: regions before nodes, then
`zmin` (smaller first), then depth, then `nodes` or `degree` (larger first), then id; an
entity is kept only with its ancestors. What it leaves out is counted in the tile's
`omitted` figures, never dropped silently: `regions` and `nodes` left out, and
`region_nodes`, the nodes inside the regions left out, each node once. A budget that
leaves an entity out of a tile at `z` and keeps it at `z − 1` makes it pop (§ 6); if the
deepest tiles pass 32 KB under the rule above, the budget becomes nested: an entity kept
in a tile is kept in every tile below it that its disc meets.

### 3.6 Edges

A tile holds no edges. The approved viewer draws links for the selection only, from the
per-node adjacency (§ 5.3); the connections panel draws the sibling matrices of § 5.3
(`design/DECISIONS.md`, 2026-11-26, #20). The figures the caption shows are exact: the
weight inside and between regions is in the region records, and a selection's links are
its whole adjacency list.

## 4. The tile payload

A tile is stored gzip. Its payload — what the gzip decodes to — is at most the cap.

### 4.1 Header

```
offset  size  field
0       4     magic                 "EGMT"
4       2     major version         1
6       2     minor version         0
8       1     z                     0 … 20
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
0       4     tag      bit 31: required; bits 0–30: the block number
4       4     rows     ≥ 1
8       8     length   of the body, a multiple of 8
16            body
```

Block numbers increase strictly through the payload. A reader that does not know a
block's number skips the block if bit 31 is clear and refuses the tile if it is set.
The blocks of this version carry bit 31; one of their numbers without it is refused. A
block of zero rows is not written; a missing block holds zero rows.

A body is a sequence of **columns**, one value per row each, every column padded with
zero bytes to `pad8` of its length. The length of a known block is exactly the sum of
its padded columns.

| Tag | Block | Columns, in order |
|---|---|---|
| `0x80000001` | regions | `id` u32, `parent` u32, `x` f32, `y` f32, `r` f32, `nodes` u64, `internal` u64, `volume` u64, `communities` u32, `rank` u32, `partner` u32, `kind` u8, `depth` u8, `colour` u8, `zmin` u8 |
| `0x80000002` | nodes | `id` u32, `region` u32, `x` f32, `y` f32, `r` f32, `name` u32, `degree` u64, `zmin` u8 |
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
   refused as such), zero reserved bytes, `z ≤ 20`, `x` and `y` below `2^z`.
2. The framing of § 4.2: block numbers increasing, known lengths exact, padding zero,
   rows ≥ 1, string ends not decreasing and the last equal to the bytes' length, every
   string UTF-8.
3. Regions: ids increasing strictly; `parent < id`; a region with `parent = 0` has
   depth 1; any other region's parent is in the tile, and the region has the parent's
   depth + 1, the parent's colour and a `zmin` not below the parent's. `kind` ≤ 3,
   `colour` ≤ 4, `zmin ≤ z`, `nodes ≥ 1`, `rank ≥ 1`, `partner ≠ id`,
   `2 · internal ≤ volume`, and `communities ≥ 1` exactly for aggregate and other.
4. Nodes: ids increasing strictly; the node's region is in the tile; `zmin` is at
   least its region's and at most `z`; `name` is a string of the tile.
5. Labels: entities increasing strictly, each an entity of the tile; `text` a string
   of the tile; entity `zmin ≤ z_on ≤ z < z_off ≤ 21`; `anchor` ≤ 1.
6. Geometry: `x` and `y` in `[0, 1]`, `0 < r ≤ 1`, and the disc meets the tile (§ 3.5).

The bytes of a tile are a function of its content: the only freedom is the minor
version, and the optional blocks a later writer may add.

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
24      8     regions          of the map's region tree, the root included
32      8     edges            pairs of the graph, a kept loop once
40      4     summary k        0 for a full map; k ≥ 1 for a summary map (§ 7)
44      4     reserved
48      64·n  entries
```

An entry:

```
offset  size  field
0       32    name            1–32 bytes of a–z, 0–9, ".", "_", "-", then zero bytes
32      1     element         1 u8, 2 u16, 3 u32, 4 u64, 5 f32
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

| Name | Element | Count | Values |
|---|---|---|---|
| `positions` | f32 | `2 · nodes` | `x` then `y` of each node, by node id, in `[0, 1]` |
| `adjacency.offsets` | u64 | `nodes + 1` | where each node's neighbours start in `adjacency.targets`; from 0, not decreasing, ending at its count |
| `adjacency.targets` | u32 | any | each node's neighbours in the undirected projection, increasing strictly, below `nodes`, never the node itself |
| `matrix.offsets` | u64 | `regions + 1` | where each region's pairs start in `matrix.pairs`; from 0, not decreasing, ending at its count |
| `matrix.pairs` | u32 | any | the pairs of a parent's children with weight ≥ 1: `(i << 16) | j`, the children's ranks, `1 ≤ i < j`, increasing strictly within a parent |
| `matrix.weights` | u64 | that of `matrix.pairs` | the weight of the pairs between children `i` and `j` |

Every array is optional: a map without `adjacency.*` draws no links, one without
`matrix.*` has no matrix in its panel. The adjacency arrays come together, and so do
the three matrix arrays. The adjacency is symmetric: `v` is a neighbour of `u` exactly
when `u` is one of `v`; `verify` checks it in one pass, by comparing the sums of
`h₁(u)·h₂(v)` and of `h₁(v)·h₂(u)` over the entries, for two fixed 64-bit mixes of the
ids. The matrices are
those of the connections panel (#20): the lab measured them at 11.2 MB of gzip on lj
(1.3% of its map; 8.5 MB on orkut), with the largest parent's block at 11.9 KB
(`lab/m1-06-edge-rendering/results/overview_lj.json`). A writer refuses a parent with
more than 65,535 children: the builder refuses to write such a parent's matrix. Search arrays (`search.*`) are #70's; attribute summaries
(`attributes.*`) are optional and come with #79.

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
  unread (deflate stores incompressible data at 5 bytes per 65,535).
- **Nothing decodes past the cap.** The 32 KB budget of #69 bounds a tile's gzip size,
  not what it decodes to. One DEFLATE stream cannot exceed a ratio of 1,032; the danger
  is pointers that let entries share bytes: overlapping entries in a zip container
  turned 42 kB into 5.5 GB (Fifield, WOOT'19). Hence one level of leaves, run length 1,
  and tiles that fill the tile data each byte once (§ 2.3): reading every tile inflates
  each byte of the file once, at most 1,032 times its size.
- **The fuzz target** `egmap` (`crates/everygraph-format/fuzz`) decodes tile payloads and
  whole files, raw and damaged, and requires that a payload that decodes encodes back to
  the same bytes.

## 10. Decided here

- **PMTiles v3, no MVT.** The container is what PMTiles readers, CDNs and range servers
  already serve. MVT has no columns and no exact-integer guarantee for the counts
  (decided 2026-11-26, `docs/research/sota-review-2026-11.md` S12).
- **The custom payload, not MapLibre Tile (MLT) or Arrow IPC, for 1.0.** The three
  payloads were compared on the content of every lab tile of lj (203,941 tiles) and
  orkut (142,710), each converted to the records of § 3 (the lab's entities with their
  ancestors; `lab/m5-02-tile-format/README.md`). Exploratory, load-independent byte
  counts, gzip level 6:

  | Graph | Payload | Gzip total | Median tile | Largest tile |
  |---|---|---|---|---|
  | lj | this payload | 453.0 MB | 1,419 B | 18,203 B |
  | lj | MLT (`mlt-core` 0.24, profile Web 4G) | 461.9 MB (+2.0%) | 1,555 B (+9.6%) | 16,769 B |
  | lj | Arrow IPC (arrow-rs 58.4, two streams) | 558.0 MB (+23%) | 2,002 B | 17,982 B |
  | orkut | this payload | 324.7 MB | 1,477 B | 17,998 B |
  | orkut | MLT (profile Web 4G) | 330.5 MB (+1.8%) | 1,613 B (+9.2%) | 17,024 B |
  | orkut | Arrow IPC | 396.7 MB (+22%) | 2,056 B | 17,675 B |

  MLT is lossless here only because `x`, `y` and `r` are kept as f32 property columns:
  its geometry is integer coordinates within the tile's extent. Its u64 and u8 columns
  hold the counts and the kinds. On size, MLT is within the 10% of the adoption rule.
  Its decode time in JavaScript was not measured, so the rule is not met and the custom
  payload stays: it needs no decoder library in a viewer with a 150 KB budget (#77), and
  its columns are typed-array views of the inflated bytes. The decode-time comparison is
  open in `plan/issues/m5-02-tile-format.md`.
- **No edges in tiles** (§ 3.6): they were most of the lab's bytes (830.6 MB of gzip
  tiles on lj with edges, against 453.0 MB for the same entities without).
- **Arrays in a table after the root directory**, not in the JSON metadata: offsets and
  counts stay exact integers, a reader needs no JSON parser, and the first 16 KiB a
  PMTiles client fetches already holds the table. Standard readers never look between
  the root directory and the regions the header names.
- **Chunked arrays with the index at the end**, so that a writer streams an array
  without knowing its length first, and a viewer reads one chunk with two small reads.
- **The cut is derived, not stored**, as in `eg-communities.md`.
