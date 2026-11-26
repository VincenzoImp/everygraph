# Property columns: byte layout, version 1

Status: specification for 0.1, written by #37 (`plan/issues/m2-10-property-columns.md`).
Implemented by `crates/everygraph-format/src/columns/`. Checked from outside by
`tools/check_columns_python.py`: pyarrow and Polars read the column sections of a file
written by Rust, without Everygraph, and a reader of the layout and statistics records
written from this text finds the same buffers and the same statistics. The decisions it
implements are DESIGN D9 with ADR 0009 and ADR 0028, and research decision B17
(`docs/research/README.md`; `docs/research/sota-ingest-query.md` § 3, challenge C4).

A **column** is a sequence of `rows` values of one type, some of them null. It is stored
as up to three sections of the `.eg` container (`eg-container.md`):

| Kind | Section | Holds |
|---|---|---|
| 7 | `<scope>/arrow/<column>` | the values: a complete Arrow IPC file (§ 3) |
| 8 | `<scope>/layout/<column>` | the column's type and shape, and where each buffer of the Arrow file lies (§ 4) |
| 9 | `<scope>/stats/<column>` | the statistics of a numeric column (§ 5) |

Contents: 1 Conventions · 2 Types · 3 The column section · 4 The layout section ·
5 The statistics section · 6 Names, and what a graph stores · 7 Reading · 8 Versions ·
9 Decided here.

---

## 1. Conventions

- Integers are little-endian. Offsets inside a column section count bytes from the
  start of that section. A section starts at a multiple of 4096 in the file
  (`eg-container.md` § 3), so an offset that is a multiple of 4096 in the section is one
  in the file too.
- Floating-point values are stored as the container specifies for derived sections
  (`eg-container.md` § 1): NaN only as the quiet NaN (`0x7FC00000`, `0x7FF8000000000000`),
  never −0.0 (written as +0.0). This holds for input columns too, so that equal values
  are equal bytes.
- **Arrow** is the Apache Arrow columnar format, version 1.5 of its IPC format
  (`arrow.apache.org/docs/format/Columnar.html`), metadata version V5. Names of Arrow
  tables and fields (`RecordBatch`, `bodyLength`) are those of its `Schema.fbs`,
  `Message.fbs` and `File.fbs`.

## 2. Types

| Type | Arrow type | Bytes per row | Layout code, parameter 1, parameter 2 | Statistics |
|---|---|---|---|---|
| `int8` … `int64` | `Int(bits, signed)` | 1, 2, 4, 8 | 1, bits, 1 | yes |
| `uint8` … `uint64` | `Int(bits, unsigned)` | 1, 2, 4, 8 | 1, bits, 0 | yes |
| `float32`, `float64` | `FloatingPoint(SINGLE / DOUBLE)` | 4, 8 | 2, 32 or 64, 0 | yes |
| `bool` | `Bool` | 1 bit | 3, 0, 0 | no |
| `timestamp[unit]` | `Timestamp(unit, "UTC")`, an `i64` | 8 | 4, unit (0 s, 1 ms, 2 µs, 3 ns), 0 | yes |
| `fixed_size_binary[w]` | `FixedSizeBinary(w)`, w from 1 to 2^20 | w | 5, w, 0 | no |
| `decimal128(p, s)` | `Decimal(p, s, 128)`, p from 1 to 38 | 16 | 6, p, s | yes |
| `decimal256(p, s)` | `Decimal(p, s, 256)`, p from 1 to 76 | 32 | 7, p, s | yes |
| `dictionary[codes]` | `Utf8` values, `DictionaryEncoding` with `Int(8, 16 or 32, unsigned)` indices | 1, 2, 4 | 8, 8, 16 or 32, 0 | no |

- The scale `s` of a decimal is from −76 to 76; parameter 2 holds it as an `i32` in
  two's complement. A decimal's value is its stored integer times 10^−s; a writer
  refuses an integer with more than `p` digits.
- Timestamps are always UTC (`graph-semantics.md` § 2.6).
- A dictionary-encoded column stores one code per row and its strings once (§ 3.2). Its
  codes are below the number of strings: at most 256 strings with `uint8` codes, 65,536
  with `uint16`, 2^31 − 1 with `uint32` (an Arrow array's limit).
- A row is null or holds a value. A column that is not **nullable** has no null.

## 3. The column section

### 3.1 Batches

The rows are cut into **batches** of `R` rows, `R` a power of two from 1 to 2^30
fixed for the column: batch `i` holds rows `i·R` to `min((i + 1)·R, rows) − 1`, so
row `r` is row `r mod R` of batch `r / R`. A column of 0 rows has no batch. A writer
uses `R = 2^26` unless it is told otherwise, and fewer rows for values wider than 16
bytes (`R` = the largest power of two, at most 2^26, with `R × width ≤ 2^30`), so that a
batch holds at most 1 GiB of values.

Limits, checked by writers and readers: every batch body (§ 3.3), a full batch with
nulls included, is below 2^31 bytes; so is the dictionary's body; a column name is 1 to
512 bytes of UTF-8; a layout section (§ 4) is at most 16 MiB, about 400,000 batches.

### 3.2 The bytes

The column section is a complete Arrow IPC file, which ends with the section:

```
offset 0     "ARROW1" 00 00
       8     the schema message, with no body; zero bytes to a multiple of 8
             the dictionary batch message (dictionary-encoded columns only)
             one record batch message per batch, in row order
             FF FF FF FF 00 00 00 00          end of stream
             the footer (a FlatBuffer)
             its length, i32
             "ARROW1"                         the end of the section
```

A **message** at position `p` (a multiple of 8) is `FF FF FF FF`, an `i32` length `L`,
the message's FlatBuffer, then zero bytes up to `p + 8 + L`. For the schema message,
`p + 8 + L` is the first multiple of 8 after the FlatBuffer. For a message with a body,
it is the **body start** `b`: the first multiple of 4096 at or after the end of the
FlatBuffer. The body follows, and the next message starts at its end.

A **body** holds the message's buffers. An empty buffer has offset 0. A non-empty buffer
starts at the first multiple of 4096 at or after the end of the previous non-empty
buffer (0 for the first). The body ends at the first multiple of 8 at or after the end of
its last non-empty buffer. Every byte of a body outside its buffers is zero. Buffer
offsets in a message count from its body start; since `b` is a multiple of 4096, every
non-empty buffer starts at a multiple of 4096 in the section, and so in the file.

The buffers of a **record batch** of `n` rows of which `k` are null are: the validity
bitmap, `⌈n/8⌉` bytes when `k > 0`, empty when `k = 0`; then the values, `n × width`
bytes, or `⌈n/8⌉` for `bool`. The **dictionary batch** has `values` rows and no null; its
buffers are an empty validity bitmap, `values + 1` offsets (`i32`: where each string
starts in the data, then the end of the last), and the strings' UTF-8 bytes,
concatenated.

The FlatBuffers (Arrow's metadata), field by field, slot numbers as in Arrow's schema
files, every field listed here written (defaults included) and no other:

| Table | Fields (slot: value) |
|---|---|
| `Message` | 0 `version`: `i16` 4 (V5) · 1 `header_type`: `u8` 1 schema, 2 dictionary batch, 3 record batch · 2 `header`: the table · 3 `bodyLength`: `i64` (0 for the schema) |
| `Schema` | 0 `endianness`: `i16` 0 (little) · 1 `fields`: a vector of one `Field` |
| `Field` | 0 `name`: the column's name · 1 `nullable`: `bool` · 2 `type_type`: `u8`, the Arrow type code (2 `Int`, 3 `FloatingPoint`, 5 `Utf8`, 6 `Bool`, 7 `Decimal`, 10 `Timestamp`, 15 `FixedSizeBinary`) · 3 `type`: the type's table · 4 `dictionary`: a `DictionaryEncoding`, for dictionary-encoded columns only · 5 `children`: an empty vector |
| `Int` | 0 `bitWidth`: `i32` · 1 `is_signed`: `bool` |
| `FloatingPoint` | 0 `precision`: `i16` 1 (single) or 2 (double) |
| `Bool`, `Utf8` | no field |
| `Decimal` | 0 `precision`: `i32` · 1 `scale`: `i32` · 2 `bitWidth`: `i32` 128 or 256 |
| `Timestamp` | 0 `unit`: `i16` · 1 `timezone`: `"UTC"` |
| `FixedSizeBinary` | 0 `byteWidth`: `i32` |
| `DictionaryEncoding` | 0 `id`: `i64` 0 · 1 `indexType`: `Int(bits, false)` · 2 `isOrdered`: `false` · 3 `dictionaryKind`: `i16` 0 |
| `RecordBatch` | 0 `length`: `i64` rows · 1 `nodes`: one `FieldNode` (`i64` length, `i64` null count) · 2 `buffers`: one `Buffer` (`i64` offset, `i64` length) per buffer, in the order above |
| `DictionaryBatch` | 0 `id`: `i64` 0 · 1 `data`: the `RecordBatch` · 2 `isDelta`: `false` |
| `Footer` | 0 `version`: `i16` 4 · 1 `schema`: the `Schema` · 2 `dictionaries`, 3 `recordBatches`: one `Block` per message, in file order |

A `Block` is `i64` the message's position, `i32` its metadata length `b − p`, four zero
bytes, `i64` its body length.

Each FlatBuffer is laid out by one rule, so that its bytes follow from its content.
Positions count from the FlatBuffer's first byte, which is at a multiple of 8 in the
section.

- Byte 0 holds the `u32` offset of the root table.
- A **table** is: zero bytes to a multiple of 4; its vtable (`u16` 4 + 2m, `u16` the
  table's length, then `m` `u16` field positions relative to the table, 0 for a slot not
  written; `m` = its highest slot + 1); zero bytes to a multiple of 4; the table itself:
  an `i32` equal to the table's position minus the vtable's, then its fields in slot
  order, each after zero bytes up to a multiple of its size (1, 2, 4 or 8 bytes; 4 for a
  reference to a child). The table's length runs from the `i32` to the end of its last
  field. Then its children, in slot order, each written in full before the next. A
  reference holds the child's position minus the reference's own.
- A **string**: zero bytes to a multiple of 4, `u32` length, the bytes, one zero byte.
- A **vector of tables**: zero bytes to a multiple of 4, `u32` count, one `u32`
  reference per table, then the tables in order.
- A **vector of structs**: zero bytes up to a position 4 more than a multiple of 8,
  `u32` count, the structs.

### 3.3 Values

Values are stored little-endian in their type's width: integers in two's complement,
decimal256 as 32 bytes of two's complement, floats as § 1 says. A null row stores zero
bytes (a zero bit for `bool`). Bit `j % 8` of byte `j / 8` of a bitmap is row `j` of its
batch: set when the row is valid (validity), or true (`bool` values). Bits past the last
row of a batch are written as zero, and ignored by readers.

## 4. The layout section

The layout (kind 8) tells a reader where each buffer is without parsing FlatBuffers.

```
offset  size  field
0       8     magic            "EGCOLLAY"
8       4     version          1
12      4     flags            0
16      8     rows
24      8     batch rows       R
32      8     batches          ⌈rows / R⌉
40      8     column bytes     the length of the column section
48      1     type code        § 2
49      1     nullable         0 or 1
50      2     reserved         0
52      4     parameter 1      § 2
56      4     parameter 2      § 2
60      4     name length      1 to 512
64      8     strings          the dictionary's number of strings; 0 for other types
72      8     string bytes     their total length; 0 for other types
80            name             UTF-8, then zero bytes to a multiple of 8
              dictionary       for dictionary-encoded columns only, 32 bytes: offset and
                               length (u64 each) of the offsets buffer, then of the
                               strings buffer
              batches          40 bytes per batch: null count, validity offset, validity
                               length, values offset, values length (u64 each)
```

Offsets are relative to the start of the column section, not of the file: compaction
moves sections, never rewrites them (`eg-container.md` § 9). An empty buffer's offset is
its body start.

Everything in a layout but the name, the type, the counts and the null counts follows
from § 3. **A layout is valid only if it is byte for byte the layout a writer writes for
its rows, batch rows, type, nullability, name, dictionary counts and null counts**: a
reader recomputes every offset, length and the column's length from those, and compares.
A layout section is at most 16 MiB.

## 5. The statistics section

Written for every numeric column (§ 2) by the writer of the column, over its stored
values (`graph-semantics.md` § 2.7). Exactly 112 bytes:

```
offset  size  field
0       8     magic       "EGCOLSTA"
8       4     version     1
12      4     flags       bit 0: min and max hold values; every other bit 0
16      8     count       rows that are not null (NaN included)
24      8     null count
32      8     NaN count   0 for every type but floats
40      8     sum         f64: the exact sum of the values neither null nor NaN,
                          rounded once (to nearest, ties to even); +0.0 for none
48      32    min         the smallest of those values, stored as in the column
                          (its width), zero bytes to 32
80      32    max         the largest, the same way
```

- `count + null count = rows`, and `NaN count ≤ count`. Bit 0 is set exactly when
  `count > NaN count`; without it `min` and `max` are zero.
- Floats are ordered as numbers, ±∞ included; `min` and `max` are canonical (§ 1).
  Integers, timestamps and decimals are ordered as the integers they store.
- The sum is exact before its one rounding: of the values as numbers (a decimal's stored
  integer times 10^−s), whatever their order, with no intermediate overflow. An infinity
  makes it infinite; infinities of both signs make it NaN (stored as the quiet NaN). A
  sum that rounds to zero, or of no value, is +0.0, never −0.0. A float column is **clean** (`graph-semantics.md`
  § 2.7) when `NaN count = 0`, `min ≥ 0` (or bit 0 is clear) and the sum is finite.

## 6. Names, and what a graph stores

**Scopes.** A column's three sections are named `<scope>/arrow/<column>`,
`<scope>/layout/<column>` and `<scope>/stats/<column>`, where `<column>` is the column's
name (also its Arrow field name; any UTF-8, `/` included) and `<scope>`, which is not
empty and holds no `/`, says what its rows are:

| Scope | Rows | Indexed by |
|---|---|---|
| `nodes` | one per node | the dense node id (`graph-semantics.md` § 2.3) |
| `<adj>.pair` | one per entry of adjacency `<adj>` (`adjacency.out`, `adjacency.in`) | the entry offset (`eg-adjacency.md` § 3) |
| `<adj>.event` | one per stored event of `<adj>` | the event offset (`eg-adjacency.md` § 3.1; the entry offset when there is no `.events` section) |
| `edges` | one per stored edge (`num_edges`), in input order | the edge id |

The role comes after a scope without `/` and before the column's name, so that no
column name can make one section's name equal another's.

**What the build (#35) stores.**

- **Hot columns** — the weight, the time and the edge type declared at `eg.read` — are
  stored aligned with each adjacency: in the out-adjacency, and in the in-adjacency of a
  directed graph (ADR 0006), each in its own order (ADR 0009: an indirection through an
  edge id made weighted PageRank 1.7–2.1× slower). Every other edge column is **cold**:
  stored once, in `edges`, in input order. A column becomes hot only by being declared;
  promoting a column on its first use by a kernel needs a later commit that adds its
  aligned copies, and is not done in 0.1.
- **Per-event columns** (`<adj>.event`): each row's time, each row's weight under
  `duplicates="events"`, and the **edge id** — the index in `edges` of each stored event,
  `uint32` below 2^32 events and `uint64` from there (ADR 0028) — when the graph has a
  cold column. The in-adjacency carries the edge id too.
- **Per-pair columns** (`<adj>.pair`), one row per entry. The pair sum of each weight
  column `w` is stored as `w`, in the type `graph-semantics.md` § 4.3 gives a reported
  sum: `float64` for a float input (the exact sum rounded once), `decimal128(38, 0)` for
  an integer input, `decimal128(38, s)` for `decimal128(p ≤ 26, s)`, otherwise
  `decimal256(76, s)`. It is what weighted kernels read (rounded to `f64` as they load
  it, when it is not `float64`) and what reported sums show; the same stored value
  serves both, so they cannot disagree. Under `duplicates="sum"` it is the weight column
  itself (`graph-semantics.md` § 2.4), and the per-pair `edges` column (`uint64`) holds
  each pair's multiplicity, with or without a weight; `first` and `last` are per-pair
  too, under these names. Under `duplicates="collapse"` the kept row's weight and time
  are per-pair columns under their own names.
- **The columns of a text input.** The text reader gives a `float64` weight and a time
  in microseconds, never null: the build stores `float64` and `timestamp[us]` columns,
  named as the input's header names them (`column N` without a header). So a directed
  graph read with `--weight value --time at` stores, under `"events"`,
  `adjacency.out.event/…/value`, `adjacency.out.event/…/at` and
  `adjacency.out.pair/…/value`; under `"sum"`, `adjacency.out.pair/…/value`, `edges`,
  `first` and `last`; under `"collapse"`, `adjacency.out.pair/…/value` and `…/at`.
- **Computed from the events when asked, not stored**: everything else over a pair's
  events — the smallest event cost of `path()` (`graph-semantics.md` § 4.2), counts and
  sums in a time window (§ 6 there), and the multiplicity under `duplicates="events"`,
  which is `ev[e + 1] − ev[e]` of the `.events` section (1 without one). Whether a
  per-pair multiplicity column would make kernels faster than decoding `.events` is to
  be measured with the first weighted kernel (#47, `m3-04-pagerank`); kernels never
  read per-event columns (ADR 0028).
- **Edge types** are dictionary-encoded: `uint8` codes up to 256 values, `uint16` up to
  65,536, `uint32` beyond (`graph-semantics.md` § 2.6): this format stores text only
  dictionary-encoded (§ 9).
- **Nodes**: a node property table is the columns of scope `nodes`, one row per node.
- **Nulls**: a null weight is skipped by sums, a null time is in no window
  (`graph-semantics.md` § 2.6); in storage, a null is a cleared validity bit (§ 3.3).
- **Statistics** are written for every numeric column of every scope. The largest
  multiplicity of a pair (`graph-semantics.md` § 2.7) is a property of the graph, not
  of a column: the build records it in the parameters of its provenance record
  (`provenance/ingest`) as `multiplicity.max`, a decimal count, 0 for a graph without
  a pair and 1 under `"collapse"`. It is a parameter because it is a fact of the
  commit, like the counts of the ingest report beside it, and because a statistics
  section belongs to a column.
- **No degree histogram.** The degree of a node is `entry[v + 1] − entry[v]` of its
  index (`eg-adjacency.md` § 3), so the exact histogram is one pass over the index, made
  on read (`m3-07-kcore-degrees`); a stored one would cost six sections per graph and a
  check of its own.

## 7. Reading

A reader of a column:

1. reads the layout section (refusing one over 16 MiB before reading it) and checks it
   (§ 4): magic, version and flags (an unknown version, flag or type is refused as
   unsupported), the length its header implies, the name's UTF-8, the limits of § 3.1,
   null counts no larger than their batch (and 0 in a column that is not nullable), and
   that the layout is the one its contents imply;
2. checks the column section against it: its length is the layout's `column bytes`, and
   every byte that is not inside a buffer — the file magic, every message's metadata and
   padding, every body's padding, the end of stream, the footer — is the byte § 3
   implies. That reads about 4 KiB per batch, not the values. The values are then where
   the layout says, for this reader and for an Arrow reader alike;
3. reads the statistics section, if it uses it, and checks it (§ 5): its length, magic,
   version and flags, that the counts fit the column's rows and type, that `min` and
   `max` are canonical values of the type with `min ≤ max`, that the sum is not −0.0
   and is +0.0 when there is no value, and that the record is stored canonically.

What a reader does not check, because it would read every value: that floats are
canonical, that the values of null rows are zero, that dictionary codes name a string
and decimals fit their precision, that the statistics are those of the values. The
container's section hash covers them (`eg-container.md` § 7). A dictionary string is
checked when it is read: its offsets in order and inside the strings, its bytes UTF-8.

**No amplification**: a reader allocates nothing larger than the layout section (whose
length it bounds first), its own copy of the layout, and one batch's metadata at a time.

Across sections (the graph layer): the three sections of a column have kinds 7, 8, 9 and
the names of § 6; a column of scope `<adj>.pair` has the adjacency's `entries` rows, one
of `<adj>.event` its `events` (or `entries` without a `.events` section), one of `nodes`
the node count, one of `edges` `num_edges`.

`everygraph verify --deep` checks all of this for every column of the file but the rows
of `nodes` and `edges`, and also what a reader leaves to the hash: it reads every value,
refuses a float that is not canonical, a null row that is not zero, a code that names no
string and a decimal past its precision, and recomputes the statistics from the values
and compares them byte for byte with the stored ones. A column of scope `<adj>.pair` or
`<adj>.event` whose adjacency is not in the file is an error. And it checks what the
build derives: `multiplicity.max` against the largest `ev[e + 1] − ev[e]` of the
adjacency, or under `duplicates="sum"` the largest `edges`; there, `edges ≥ 1` and
`first ≤ last` on every row (`everygraph_format::verify`).

## 8. Versions

Each of the three records has its version (the Arrow file is version 1 of this
specification as a whole, through its layout). A reader refuses a version, a flag or a
type code it does not know: a new type, a new flag or a different framing is a new
version of the layout. A change that changes the bytes written for the same values (a
different default `R` included) changes the golden bytes of
`crates/everygraph-format/tests/columns.rs`, and needs a new version.

The container kinds are 7 (column), 8 (layout) and 9 (statistics), all defined in
container version 1.0. Kind 7 defines the graph and enters its identity
(`eg-container.md` § 6). Kinds 8 and 9 follow from kind 7 and do not: the layout is what
§ 3 and § 4 compute from the Arrow file (the batch row count of a column of one batch,
the one choice the Arrow file does not record, changes no value), and the statistics are
computed from the values.

## 9. Decided here

- **Each column a complete Arrow IPC file**, not one file for all columns: a column can
  be added, replaced or dropped by one commit, and any one can be opened alone with
  `pa.ipc.open_file` on a slice of the map (B17).
- **Our own framing**, about 500 lines with their documentation (`flatbuf.rs`,
  `arrow.rs`), not arrow-rs's writer: a change in an Arrow library cannot change our
  files, the bytes are pinned by golden tests, and the format crate keeps its two
  dependencies (BLAKE3 and the core). pyarrow and Polars check what it writes.
- **Every buffer at 4 KiB, not only the values.** Validity bitmaps too, so that each can
  be mapped or read with direct I/O on its own; it costs a page of padding per non-empty
  buffer, and each batch's metadata takes a page before its body. A small column takes
  16 KiB of file: a 5-row `int64` column is a 4,330-byte column section, its layout and
  its statistics, each padded to 4 KiB by the container (the sample of
  `tools/check_columns_python.py`). By the same arithmetic, a column of one full batch of
  2^26 `float32` values takes 16 KiB more than its 256 MiB of values.
- **The side table is a section**, not Arrow custom metadata (which needs FlatBuffers to
  read) and not the container's table of contents (which has no field for it). Its
  offsets are relative to the column section, so that compaction keeps them true.
- **The layout is canonical**, recomputed and compared by the reader, and the column's
  framing is compared byte for byte: so a file in which Everygraph and pyarrow would find
  different values cannot be opened, and fuzzing has one stored form to check.
- **Unsigned dictionary codes** (`uint8`, `uint16`, `uint32`): Arrow allows them,
  `graph-semantics.md` § 2.6 asks for `uint8` and `uint16`, and pyarrow and Polars read
  them. Text is stored only dictionary-encoded, with `Utf8` (32-bit offsets) strings,
  below 2 GiB of strings per column. A column of more distinct text (above 2^31 − 1
  values or 2 GiB) is refused by this version; `LargeUtf8` is the way to add it.
- **An empty validity bitmap for a batch without null**, as Arrow allows: one stored form
  per batch, and no bitmap to read for columns without nulls.
- **The exact sums.** Floats are summed by `everygraph_core::ExactSum`, the one exact
  accumulator of the engine (rule R6 of `docs/research/sota-numerics.md`). Integers and
  decimals are summed as integers, exactly, in 640 bits (2^64 values below 2^255), and
  a decimal's sum is scaled by 10^−s, exactly, before its one rounding. Both are
  independent of the order of the values.
- **Not built here**: per-block minimum and maximum of time and weight (zone maps,
  B18), which `m7-04-views` specifies and builds for the views of #93 and #94; and the
  reading of columns from text and Parquet (#32, #33, #35).
