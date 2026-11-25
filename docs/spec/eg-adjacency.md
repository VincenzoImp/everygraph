# Compressed adjacency: byte layout, version 1

Status: specification for 0.1, written by #30 (`plan/issues/m2-03-adjacency-codec.md`).
**Revision 2, 2026-11-25 (ADR 0028):** an adjacency stores pairs, and the events of a pair
are found through a third byte string, the `.events` section (§ 3.1), announced by index
flag bit 1 (§ 3); flag bit 0 is reserved (§ 3); the block rule no longer fixes the order of
reductions (§ 4); merged runs are byte-identical to encoded ones (§ 4). For a graph without
repeated pairs the index and the data are byte for byte those of revision 1; their version
field stays 1 (§ 6). Implemented by
`crates/everygraph-format/src/adjacency/` (the `.events` section by #35). The decision it
implements is DESIGN D4 with the conditions of ADR 0004; the measurements behind it are
FINDINGS § C and § M1-13 and the pull request of #30.

An **adjacency** is `n` sorted lists of `u32` (the neighbours of nodes `0 … n − 1`). It is
stored as two byte strings, the **index** and the **data**, and, when some pair of the
graph has more than one event, a third, the **events** (§ 3.1). The `.eg` container (#28)
keeps each as a section; nothing in them refers to the container, and each can be read
from any positional byte source. All integers are little-endian.

Contents: 1 Records (the data) · 2 Elias–Fano sequences · 3 The index (3.1 Pairs and
events) · 4 Blocks · 5 What a reader checks · 6 Versions · 7 Sizes measured · 8 Decided
here.

---

## 1. Records (the data)

The data is the concatenation, in node order, of one **record** per node whose list is
not empty. A node with an empty list has no record. Nothing precedes, separates or
follows the records.

The list of node `v` with `d ≥ 1` neighbours `x_0 < x_1 < … < x_{d−1}` (strictly
increasing `u32`: each neighbour once, because an adjacency stores pairs, § 3.1) becomes
`d` **words**, in 32-bit wrapping arithmetic:

- `w_0 = zigzag32(x_0 − v)`, where the difference is taken modulo 2^32 and read as a
  signed 32-bit integer `t`, and `zigzag32(t) = (t << 1) xor (t >> 31)` (arithmetic
  shift). So `x_0 = v + unzigzag32(w_0) mod 2^32` with `unzigzag32(w) = (w >> 1) xor
  −(w and 1)`.
- `w_j = x_j − x_{j−1} − 1` for `j ≥ 1` (no wrap: the list increases).

The words are coded with **Stream VByte** (Lemire, Kurz and Rupp, 2018):

```
record = control[0 .. ⌈d/4⌉)  data bytes
control[i] bits 2k, 2k+1  = (byte length of word 4i + k) − 1,  k = 0..3
word j: its byte length L_j ∈ {1, 2, 3, 4} is the fewest bytes that hold it;
        its L_j low bytes, least significant first, follow the words before it
```

The data bytes of word `j` start after all control bytes and after the data bytes of
words `0 … j − 1`. In the last control byte, the fields of words `≥ d` are **written as
zero and ignored by readers**. The record's length is `⌈d/4⌉ + Σ L_j`.

Example: node 5 lists 3, 4, 300. Words: `zigzag32(3 − 5) = 3`, `4 − 3 − 1 = 0`,
`300 − 4 − 1 = 295`. Lengths 1, 1, 2 → control byte `0b00_01_00_00` = `0x10`. Record:
`10 03 00 27 01` (5 bytes).

For any list whose first neighbour is within 2^31 − 1 of `v`, `w_0` equals the 64-bit
zigzag of `x_0 − v`, so the records are byte-identical to the Stream VByte records of the
lab (lab C's `svb` codec and `lab/m1-13-compressed/src/store.rs`), without the lab's
degree prefix (§ 8).

## 2. Elias–Fano sequences

A non-decreasing sequence of `len ≥ 1` values `y_0 ≤ … ≤ y_{len−1} ≤ U` (the
**universe**) is stored with `l` low bits per value:

- `l = ⌊log2(⌊U / len⌋)⌋` when `⌊U / len⌋ ≥ 2`, else `l = 0`. It is not stored.
- **Low part:** `y_i mod 2^l` at bit positions `i·l … i·l + l − 1` of a bit string, bit
  `p` being bit `p mod 64` of 64-bit word `⌊p/64⌋`; stored as `⌈len·l / 64⌉` words
  (`u64`, little-endian).
- **High part:** a bit string of `len + (U >> l)` bits in which bit `(y_i >> l) + i` is
  set for every `i` and no other bit is; stored as `⌈(len + (U >> l)) / 64⌉` words.
- Every bit past the end of either string, in its last word, is zero.

`y_i = ((select_1(i) − i) << l) | low_i`, where `select_1(i)` is the position of the
`i`-th set bit (from 0) of the high part. A reader may keep select samples in memory;
they are not part of the format.

## 3. The index

```
offset  size  field
0       8     magic          "EGADJIDX"
8       4     version        1
12      4     flags          bit 1 `events` (§ 3.1); bit 0 reserved (below); every
                             other bit 0
16      8     nodes          n, at most 2^32 − 1
24      8     entries        Σ d_v
32      8     data_bytes     length of the data
40      8     blocks         b: 0 if n = 0, else 1 ≤ b ≤ n
48            block starts   b + 1 × u32: first node of each block, then n;
                             zero bytes up to a multiple of 8
              block codecs   b × u8: 0 for every block (§ 4); zero bytes up to a
                             multiple of 8
              byte offsets   Elias–Fano, len = n + 1, U = data_bytes: the offset in
                             the data of the record of each node (of the next record
                             for an empty list), then data_bytes
              entry offsets  Elias–Fano, len = n + 1, U = entries: Σ_{u<v} d_u for
                             each v, then entries
```

The low part of each sequence is followed directly by its high part; the entry offsets
follow the byte offsets directly. The index has no other content: its length is
determined by the header, and must be exactly that.

The degree of `v` is `entry[v + 1] − entry[v]` (distinct neighbours: the number of
pairs); its record is `data[byte[v] .. byte[v + 1])`. The entry offset of `v` also indexes
the per-pair edge columns (§ 3.1, #37).

**Flag bit 1, `events`,** is set exactly when the adjacency has a `.events` section
(§ 3.1). It makes the presence of the section part of the index, so that a lost section is
an error and not another valid graph, and it is the signal that a reader must understand
the section: a reader of revision 1 refuses the bit, as it refuses every flag (§ 6).

**Flag bit 0, `sparse`, is reserved for segments** (ADR 0028; issue #111): the index then
holds, after the block codecs, an Elias–Fano sequence of the node ids that have a list
(`len = present`, `U = nodes`), and the offset sequences run over present nodes only, so
that a small segment of a large graph does not carry a dense index of about `2(n + 1)`
bits. Its header fields are specified with segments; until then no writer sets the bit,
and a reader refuses it as it refuses every flag (§ 6).

### 3.1 Pairs and events

An adjacency stores **pairs**: the list of `v` holds each neighbour once, whatever the
number of events (input rows stored as edges, `docs/spec/graph-semantics.md` § 1) of the
pair, in increasing internal id of the neighbour (§ 1) — which is graph order until a
renumbering (`plan/issues/m2-07-id-mapping.md`, `m2-24-bp-ordering.md`). When the graph
has more than one event for some pair, the index sets flag bit 1 and the section
`<name>.events`
(container kind 5; `<name>` is the adjacency's name, so `adjacency.out.events` beside
`adjacency.out.index` and `adjacency.out.data`) holds, for each entry in entry order, the
number of events of the entries before it, then `events`:

```
offset  size  field
0       8     magic          "EGADJEVT"
8       4     version        1
12      4     flags          0 (no flag is defined in version 1)
16      8     entries        the entries of the adjacency's index
24      8     events         Σ over entries of the events of each; events ≥ entries
32            event offsets  Elias–Fano, len = entries + 1, U = events: ev[e] for
                             each entry e, then events
```

The low part is followed directly by the high part; the section has no other content,
and its length is determined by the header and must be exactly that. Entry `e` has
`ev[e + 1] − ev[e] ≥ 1` **stored events**, at the **event offsets** `ev[e] … ev[e + 1] − 1`,
in the storage order of `docs/spec/graph-semantics.md` § 2.5. **Per-event columns** (a
time, the weight of each row under `duplicates="events"`, the edge id that reaches a wide
or cold column, DESIGN D9) are indexed by event offset; **per-pair columns** (aggregates
over a pair's events, the `edges` column of `duplicates="sum"`) by entry offset.

A graph whose every pair has one stored event — any graph read with
`duplicates="collapse"` or `"sum"`, and any input without a repeated pair — has **no
`.events` section** and flag bit 1 clear: its event offsets are its entry offsets, and a
per-event column is then also a per-pair one. A writer never writes a `.events` section
in which every entry has one event. The number of stored events is the multiplicity of
the pair (`graph-semantics.md` § 1) except under `"sum"`, where the multiplicity is the
`edges` column (`graph-semantics.md` § 2.4); a reader of this format alone cannot tell
the two apart, and the graph layer must.

`events` counts the events *of this adjacency*: on an undirected graph each event of a
pair `{u, v}` with `u ≠ v` lies in the ranges of both entries (in the list of `u` and in
that of `v`), in the same order, so `events` of the out-adjacency is `2·num_edges` minus
the events of kept loops. Each adjacency of a graph (out, and in on a directed graph,
ADR 0006) has its own `.events` section when it has repeated pairs, and its own event
offsets.

## 4. Blocks

Blocks are node-aligned runs of lists: block `i` holds the lists of nodes
`start[i] … start[i+1] − 1`. Blocks are the unit of I/O and of encoding. The order of
result-bearing reductions is not taken from them: it is fixed by reduction blocks of node
ranges that are a function of the graph, never of the file's blocks, the scan's chunk
size, the thread count, the memory budget or the storage format (raw or compressed) — the
reduction blocks of the edge-map framework (#44) and of PageRank (#47). So re-encoding,
merging segments or compacting never changes a result (DESIGN D14).

The codec byte of a block says how its records are coded. Version 1 defines only 0, the
records of § 1; a reader refuses any other value. (The issue reserves the field for an
archival block coding, varint + zstd, measured at 2.4 bits per entry on a web graph in
FINDINGS § C; it is not specified yet.)

An encoder is a deterministic function of its input. The codec byte it writes for a block,
and every parameter of that codec, depend only on the lists of the block and on the
encoder's fixed settings, never on random sampling, timing, the thread count or the memory
budget. Equal input therefore gives equal bytes (DESIGN D7, D14). Adaptive formats that
choose an encoding by sampling with a random generator, as BtrBlocks does
(`docs/research/sota-ingest-query.md` § 3), are excluded; any sampling a future codec needs
is seeded by position.

Any valid directory may be written. The encoder of `everygraph-format` writes blocks
greedily: a block closes before a list that would take it past 65,536 entries, or when it
holds 65,536 nodes; a list longer than 65,536 entries therefore has a block of its own,
and no list is ever split.

**A merged run is byte-identical to the run an encoder writes for the same pairs** (and
the same events): records, Elias–Fano sequences, the `.events` section and the greedy
block rule are pure functions of the lists and their multiplicities. Every writer of
Everygraph — the in-memory encoder of #30, the streaming encoder of #35, a merge of
segments (#111) — follows the greedy rule, so "compacted" and "built in one ingest" are
the same bytes for the same history, and a merge can be checked by re-encoding.

## 5. What a reader checks

Opening (the index): the magic, version and flags; `nodes < 2^32`; the block count against
`nodes`; the exact length; strictly increasing block starts from 0 to `nodes`; codec bytes
and every padding byte or bit zero; the number of set bits of each high part; both
sequences start at 0 and end at their universe; for every node, offsets that do not
decrease, a degree below 2^32, and a record length `r` that fits the degree `d`: `r = 0`
when `d = 0`, else `⌈d/4⌉ + d ≤ r ≤ ⌈d/4⌉ + 4d`. Then the data length must equal
`data_bytes`.

The events, when flag bit 1 is set (and a `.events` section without the flag is an
error). On opening: the magic, version and flags; `entries` equal to the index's;
`events > entries` (a section in which every entry has one event is refused, so that a
graph has one stored form); the exact length, computed with checked arithmetic (`events`
is any `u64` and `l` can reach 63, so `len·l` and `U >> l` are computed in 128 bits or
refused on overflow). At the first use of the section, before any value read from it is
returned, not at opening: the number of set bits of the high part, every padding bit
zero, and a sequence that starts at 0, ends at `events` and **strictly increases** (every
entry has at least one event). These read the whole section, `O(entries)` bits; a kernel
that takes multiplicities from a per-pair column never pays them.

Across sections (the graph layer, when it opens a graph): a section named `<adj>.events`
has container kind 5, and a section of kind 5 is named `<adj>.events` for an adjacency
`<adj>` of the file; on a directed graph, the out- and in-adjacency both have a `.events`
section or neither has, and their `events` are equal (each event lies once in each);
every per-event column has `events` rows, or `entries` rows when there is no `.events`
section.

Decoding: every read stays inside the data, and a record whose words do not end exactly
where the next byte offset says is an error. A reader is not required to detect a list
that decodes out of order (a gap that wraps past 2^32 in a damaged file): the container's
section hash (#28) covers what structure cannot.

The implementation decodes the same way with every decoder (scalar, SSSE3, NEON, wasm
`simd128`): for any bytes, valid or not, the same lists and the same errors; tested with
the correctness oracle's corpus, property tests and a fuzz target
(`crates/everygraph-format/fuzz`).

## 6. Versions

The version field changes when a reader of the previous version would misread the
bytes. A new block codec or a new flag needs a new version or a defined flag; a reader
refuses what it does not know (unknown version, any flag, any non-zero codec byte), it
never guesses. Node ids wider than 32 bits (DESIGN D3, a later build) are a new version:
words are 32-bit here. The `.events` section has its own version field, under the same
rule.

**Revision 2 keeps version 1** for the index and the data. Revision 1's own text already
required every list to be strictly increasing (§ 1: `x_0 < x_1 < …`,
`w_j = x_j − x_{j−1} − 1`), so an equal neighbour was never a valid version-1 list,
whoever wrote it; a crafted word that wraps to an equal neighbour was invalid under
revision 1 too, and is left to the section hash (§ 5). Revision 2 therefore reads every
valid revision-1 file as revision 1 did, which is the test of the rule above. What it adds
is a defined flag — the path this section allows — and a section: bit 1 is set only on an
adjacency with a `.events` section, and a revision-1 reader refuses it, which is right,
since it would take every multiplicity as 1. For a graph without repeated pairs the
index and the data are byte for byte those of revision 1. Reserving flag bit 0 needs no
version either: every reader refuses it already.

## 7. Sizes measured

Bits per entry, index included (exploratory, `lab/m2-03-codec`, `encode`; the counts do
not depend on machine load): lj 17.99 (data 17.22, index 0.76), orkut 16.64 (16.41 +
0.23). The issue's targets: ≤ 18.5 (lj), ≤ 17 (orkut, twitter-sym), ≤ 24 (Friendster).
twitter-sym and Friendster are measured in the official run of #30. These graphs have no
repeated pair, so they have no `.events` section.

The `.events` section, on an adjacency with `r = events / entries > 1`, takes `1 + r` bits
per entry when `r < 2`, and between `2 + ⌊log2 r⌋` and `3 + ⌊log2 r⌋` bits per entry
otherwise (arithmetic from § 2, not measured; #35 measures it on a temporal graph). That
is the structure alone: the per-pair columns a graph with repeated pairs also stores (a
`float64` pair sum is 64 bits per entry) come on top (ADR 0028 compares both).

## 8. Decided here

- **No degree before each record.** The lab's records began with the degree as a LEB128;
  the entry offsets (required by ADR 0004, condition 4) give it without touching the
  data, so the prefix would store it twice. Dropping it saves 0.47 bits per entry on lj
  and 0.12 on orkut (data 17.69 → 17.22 and 16.54 → 16.41).
- **The first neighbour stays a Stream VByte word.** The issue's text had it as its own
  LEB128 of the 64-bit zigzag; that costs 0.10 bits per entry on lj and 0.04 on orkut,
  and was not what the lab measured. The 32-bit wrapping zigzag makes the word fit for
  every pair of `u32` ids, which the lab's 64-bit zigzag did only for `|x_0 − v| < 2^31`.
- **Per-list records, not one stream per block.** The A/B of #30 (single thread,
  interleaved): block framing ran `PageRank` 2–5% faster on lj and no faster on orkut, scanned
  orkut 30% slower and random lists 12–24% slower, for 0.2–1.2% fewer bytes; per-list
  records keep random access to one list a single read.
- **Readers ignore unused control fields** instead of refusing them: every decoder then
  counts record lengths the same way without a check per record.
- **Pairs in the adjacency, events beside it** (ADR 0028), not equal neighbours in the
  lists: every kernel reads pairs at the bits per entry of § 7 and never deduplicates
  events, and an append can add events to an old pair without touching the base's lists.
- Targets need not be below `n`: the codec stores any sorted `u32` sets; that they are
  node ids of the same graph is the graph's invariant (oracle `targets_in_range`).
