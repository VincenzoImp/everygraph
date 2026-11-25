# Compressed adjacency: byte layout, version 1

Status: specification for 0.1, written by #30 (`plan/issues/m2-03-adjacency-codec.md`).
Implemented by `crates/everygraph-format/src/adjacency/`. The decision it implements is
DESIGN D4 with the conditions of ADR 0004; the measurements behind it are FINDINGS § C and
§ M1-13 and the pull request of #30.

An **adjacency** is `n` sorted lists of `u32` (the neighbours of nodes `0 … n − 1`). It is
stored as two byte strings, the **index** and the **data**. The `.eg` container (#28)
keeps them as two sections; nothing in them refers to the container, and either can be
read from any positional byte source. All integers are little-endian.

Contents: 1 Records (the data) · 2 Elias–Fano sequences · 3 The index · 4 Blocks ·
5 What a reader checks · 6 Versions · 7 Sizes measured · 8 Decided here.

---

## 1. Records (the data)

The data is the concatenation, in node order, of one **record** per node whose list is
not empty. A node with an empty list has no record. Nothing precedes, separates or
follows the records.

The list of node `v` with `d ≥ 1` neighbours `x_0 < x_1 < … < x_{d−1}` (strictly
increasing `u32`) becomes `d` **words**, in 32-bit wrapping arithmetic:

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
12      4     flags          0 (no flag is defined in version 1)
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

The degree of `v` is `entry[v + 1] − entry[v]`; its record is `data[byte[v] ..
byte[v + 1])`. The entry offset of `v` also indexes edge columns (#37).

## 4. Blocks

Blocks are node-aligned runs of lists: block `i` holds the lists of nodes
`start[i] … start[i+1] − 1`. They are the unit of parallel work and of I/O, and they make
the chunking of scans a property of the file, not of the machine (DESIGN D14).

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

## 5. What a reader checks

Opening (the index): the magic, version and flags; `nodes < 2^32`; the block count against
`nodes`; the exact length; strictly increasing block starts from 0 to `nodes`; codec bytes
and every padding byte or bit zero; the number of set bits of each high part; both
sequences start at 0 and end at their universe; for every node, offsets that do not
decrease, a degree below 2^32, and a record length `r` that fits the degree `d`: `r = 0`
when `d = 0`, else `⌈d/4⌉ + d ≤ r ≤ ⌈d/4⌉ + 4d`. Then the data length must equal
`data_bytes`.

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
words are 32-bit here.

## 7. Sizes measured

Bits per entry, index included (exploratory, `lab/m2-03-codec`, `encode`; the counts do
not depend on machine load): lj 17.99 (data 17.22, index 0.76), orkut 16.64 (16.41 +
0.23). The issue's targets: ≤ 18.5 (lj), ≤ 17 (orkut, twitter-sym), ≤ 24 (Friendster).
twitter-sym and Friendster are measured in the official run of #30.

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
- Targets need not be below `n`: the codec stores any sorted `u32` sets; that they are
  node ids of the same graph is the graph's invariant (oracle `targets_in_range`).
