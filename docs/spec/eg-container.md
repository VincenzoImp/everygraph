# The `.eg` container: byte layout, version 1.0

Status: specification for 0.1, written by #28 (`plan/issues/m2-01-container-spec.md`;
"v0" of the specification, format version 1.0). Implemented by
`crates/everygraph-format/src/container/`; an independent reader written from this text
alone is `tools/eg_container.py`. The decision it implements is DESIGN D7 with ADR 0007,
and ADR 0027 for single file against directory bundle.

A graph file holds named, content-hashed **sections**: the parts of the graph
(adjacency, identifier dictionary, columns) and everything derived from it later
(in-adjacency, results, hierarchy, layout, indexes). Nothing in a section refers to the
container: each section's own format is specified separately (`eg-adjacency.md`,
`eg-columns.md`, the dictionary's sections in `everygraph_ingest::ids`; results in
§ 6). The container only says where each section is, what it is, how to check it and
how it was made, and how sections are added without risk to what the file already
held.

Contents: 1 Conventions · 2 Header blocks · 3 Sections · 4 Table of contents ·
5 Kinds and codecs · 6 Provenance records · 7 Reading · 8 Updating · 9 Compaction ·
10 Versions · 11 Memory maps and concurrent use · 12 Decided here · 13 Segments
(reserved).

```
offset 0      header block 0   (4096 bytes; 128 used)
       4096   header block 1   (4096 bytes; 128 used)
       8192   section          at a multiple of 4096, then ≥ 16 zero bytes to a multiple of 4096
              section …
              table of contents (aligned and padded the same way): the last committed item
              … bytes left by a write that was never committed (none in a clean file)
```

---

## 1. Conventions

- All integers are little-endian. Offsets and lengths are `u64` and count bytes from
  the start of the file.
- Floating-point values are IEEE 754 binary32 or binary64, little-endian. Result and
  derived sections store NaN only as the canonical quiet NaN (`0x7FC00000`,
  `0x7FF8000000000000`) and never −0.0, which is written as +0.0: the same result is
  then the same bytes, and the same hash, on every platform
  (`docs/research/sota-numerics.md`). This is a rule for the writers of those sections;
  the container does not inspect them.
- **BLAKE3** is the BLAKE3 hash function (O'Connor, Aumasson, Neves, Wilcox-O'Hearn,
  2020) in its default mode (not keyed, not derive-key), with the standard 32-byte
  output.
- An **item** is a section or a table of contents. The **padded end** of an item of
  length `len` at `offset` is `ceil((offset + len + 16) / 4096) × 4096`: the first
  multiple of 4096 at least 16 bytes after its last byte.
- "Reserved" fields are written as zero by writers of this version and **ignored** by
  readers, so that a later minor version may use them for information an older reader
  can do without (§ 10).

## 2. Header blocks

The file starts with two **header blocks**, block 0 at offset 0 and block 1 at offset
4096, each 4096 bytes. Each block is a complete header on its own. A commit writes one
of them; the other holds the commit before (§ 8). Bytes 128 to 4095 of each block are
zero and are not read.

```
offset  size  field
0       8     magic              89 45 47 52 41 50 48 0A  ("\x89EGRAPH\n")
8       4     byte order         0x0A0B0C0D (bytes 0D 0C 0B 0A)
12      2     major version      1
14      2     minor version      0
16      4     block              0 or 1: which block this is; must match its position
20      4     reserved
24      8     generation         1 … 2^64 − 1: the number of the commit this block records
32      8     required features  bit set (§ 10); bit 0 reserved (`segments`)
40      8     optional features  bit set (§ 10); none defined in 1.0
48      8     TOC offset         where the table of contents starts
56      8     TOC length         its length in bytes
64      32    TOC hash           BLAKE3 of the TOC's TOC-length bytes
96      32    block hash         BLAKE3 of bytes 0 … 95 of this block
```

A block is **valid** when its magic is right, its byte order is `0x0A0B0C0D`, its major
version is 1, its block hash matches, its generation is not 0 and its block field is
its position (0 at offset 0, 1 at offset 4096; a block copied to the other place is not
valid). Bytes 0 to 15 (magic, byte order, versions) keep their meaning in every
version, so that a reader can tell a foreign file from a damaged one. A block is
**foreign** — a file this reader does not read (§ 7) — only when it says so exactly:
the magic followed by the byte order written big-endian (bytes `0A 0B 0C 0D`), or by
the right byte order and a major version above 1. Any other content of bytes 8 to 15
after the magic is a **torn** block: a first commit cut short inside those bytes leaves
part of the byte order or a major version of 0, and must read as unfinished, not as a
file of another version.

The magic's first byte is not ASCII (it catches transfers that drop the eighth bit) and
its last is a line feed (it catches line-ending conversion), as in PNG.

## 3. Sections

A section is a run of `length` bytes at `offset`, where `offset` is a multiple of 4096
and at least 8192, followed by zero bytes up to its padded end (§ 1). Sections may have
length 0. No item starts before the padded end of another: sections and their padding
never overlap, and every section ends, padding included, at or before the offset of the
table of contents that lists it.

Two things follow. A SIMD decoder may read up to 16 bytes past the end of any section
and see zeros, inside the file. And every section starts on a 4 KiB boundary, so that
direct I/O and memory maps can address it without copying.

The container never needs a section in memory as a whole: hashes are computed in
pieces, sections are read through positional reads. Whether *using* a section needs one
large allocation is up to the section's own format; the rule of the project (no single
allocation above 2 GiB, for wasm32) applies there.

## 4. Table of contents

The table of contents (TOC) lists the sections of one commit. It is stored at a multiple
of 4096 and padded like a section; its length is exactly `32 + n × entry size + names
length`.

```
offset  size  field
0       8     magic          "EGTOC\0\0\0"
8       4     entries        n
12      4     entry size     72 in 1.0; a multiple of 8, at least 72
16      8     names length   bytes of the name area
24      8     reserved
32            n entries of entry-size bytes each, sorted by name
              the name area: names-length bytes
```

An entry (the first 72 bytes; a later minor version may make entries longer, and a
reader skips the bytes it does not know):

```
offset  size  field
0       4     kind         what the section holds (§ 5); never 0
4       2     codec        how its bytes are encoded (§ 5)
6       2     reserved
8       8     offset       a multiple of 4096, at least 8192
16      8     length
24      32    hash         BLAKE3 of the section's length bytes (padding not included)
56      4     name offset  into the name area: the sum of the name lengths of the
                           entries before this one
60      4     name length  1 to 1024
64      4     provenance   the index (0-based, in this TOC) of the entry holding the
                           provenance record of this section, or 0xFFFFFFFF for none
68      4     reserved
```

- A **name** is 1 to 1024 bytes of UTF-8, unique in the TOC. Entries are sorted by name,
  bytewise and strictly increasing, which also proves the names unique. Names are not
  interpreted by the container; the graph layer gives them meaning
  (`adjacency.out.index`, `ids.keys`, `results/…`).
- Names beginning with `seg/` are reserved for segments (§ 13): `seg/` + the segment
  number as 8 lowercase hexadecimal digits + `/` + the name the section would have in an
  unsegmented file (`seg/0000002a/adjacency.out.index`). The base run keeps the names
  of an unsegmented file. Bytewise name order then groups a segment's sections and
  orders segments by number. Such names go with required bit 0, as kind 6 does (§ 10):
  a file with segments sets the bit and a 1.0 reader refuses it first, and a TOC that
  holds a `seg/` name in a file *without* the bit breaks a rule (§ 7). So no 1.0 reader,
  writer or compaction ever meets one in a file it accepts. A 1.0 writer writes no such
  name (nor kind 6).
- The **name area** is exactly the names, concatenated in entry order: entry `i`'s name
  offset is the sum of the lengths of the names before it, and the names length is the
  sum of all of them. A reader checks both. So what a reader holds of the names is at
  most the size of the TOC: names cannot overlap or be read twice.
- A provenance reference names another entry of the same TOC, whose kind is 1
  (provenance record). An entry of kind 1 has no provenance reference (0xFFFFFFFF):
  records do not refer to records, so a copy of the file can always put every record
  before the entries that refer to it. An index is valid only within its TOC: a writer
  that rewrites the TOC renumbers it.

## 5. Kinds and codecs

The **kind** says what a section's bytes are; it is assigned by the specification that
defines them. A reader that does not know a kind lists the section, can verify and copy
it, and otherwise ignores it: that is how later issues add artifacts without a new
version. A section that a reader *must* understand to read the file correctly is
announced with a required feature (§ 10), not by its kind.

Kinds 2–5 define the graph and are not optional for a reader that reads the graph;
"unknown kinds are skipped" applies only to kinds a reader does not know, and a section
added after 1.0 that a reader must understand is announced with a required feature.
(Within the adjacency format, whether an adjacency has `.events` is announced by its
index's own flags, `eg-adjacency.md`; the container needs no bit for it.)
Kinds 7–9 hold property columns (`eg-columns.md`): a reader that reads a column reads
its sections of all three kinds, and one that uses no column may leave them unread.

| Kind | Meaning | Specified in |
|---|---|---|
| 0 | not allowed | — |
| 1 | provenance record | § 6 |
| 2 | compressed adjacency, index | `eg-adjacency.md` § 3 |
| 3 | compressed adjacency, data | `eg-adjacency.md` § 1 |
| 4 | one section of an identifier dictionary (the name, `ids.*`, says which) | `everygraph_ingest::ids` (#34) |
| 5 | event offsets of an adjacency (`<name>.events`): where each pair's events start, for graphs that keep repeated pairs as events | `eg-adjacency.md` |
| 6 | segment manifest (`segments`, one per file); reserved, not written by 1.0 | § 13 |
| 7 | a property column: a complete Arrow IPC file (`<scope>/arrow/<column>`) | `eg-columns.md` § 3 |
| 8 | the layout of a property column (`<scope>/layout/<column>`) | `eg-columns.md` § 4 |
| 9 | the statistics of a numeric property column (`<scope>/stats/<column>`) | `eg-columns.md` § 5 |
| 10 | a result: typed values, one per node or per item, with its own provenance record (#39); also a part of a result that holds typed values | § 6 |
| 11, 12, 13 | an adjacency that is part of a result: its index, data and events, laid out as kinds 2, 3 and 5; it does not define the graph | § 6, `eg-adjacency.md` |
| 14 … 0x7FFFFFFF | assigned by later specifications (layout, indexes) | — |
| 0x80000000 … 0xFFFFFFFF | private and experimental use; never assigned | — |

The **codec** says how the stored bytes encode the content. Version 1.0 defines only 0:
the stored bytes are the content. The hash covers the stored bytes, so a section with a
codec the reader does not know can still be listed, verified and copied, but not read.

## 6. Provenance records

A provenance record (kind 1, codec 0) says how the sections that refer to it were made:
the inputs, the parameters, the engine, the times. One record may serve many sections
(the fourteen sections of a dictionary and the two of an adjacency, written by one
ingest). A string is a `u32` length followed by that many bytes of UTF-8; a time is an
`i64` count of nanoseconds since 1970-01-01T00:00:00 UTC, with `i64::MIN` meaning "not
known".

```
offset  size  field
0       8     magic        "EGPROV\0\0"
8       4     version      1
12      4     reserved
16      8     started      time
24      8     finished     time
32            engine       string, such as "everygraph 0.1.0"
              operation    string, such as "ingest" or "pagerank"
              inputs       u32 count, then for each:
                             name       string (a path as given, a URL, a label)
                             bytes      u64
                             modified   time
                             algorithm  string: what the digest is ("blake3", or
                                        "blake3-first-last-1MiB" for the sample of
                                        ux-contract R-6); empty for none
                             digest     u32 length, then that many bytes
              parameters   u32 count, then for each: key string, value string;
                           keys unique and in increasing byte order
```

The record ends exactly at the section's end. A reader refuses a record longer than
16 MiB before reading it. The values are text chosen by the operation that wrote them;
`docs/api-sketch.md` § 2.8 lists what a result's record must hold to serve as its cache
key.

**Graph identity.** A result's provenance record holds the parameter `graph`: the
**graph identity**, as 64 lowercase hexadecimal digits — BLAKE3 of the concatenation, in
name order, over every section that defines the graph, of `kind` (`u32`), the name's
length in bytes (`u32`), the name, and the section's 32-byte hash. So two files whose
graph sections differ in kind, name or content never share an identity. The kinds that
define the graph are those 1.0 defines as such — 2 to 5 (adjacency index and data,
`.events`, dictionary) and 7 (property columns, `eg-columns.md` § 8; their layouts
and statistics, kinds 8 and 9, follow from kind 7 and are left out). Two more
exclusions: sections whose names begin with `adjacency.in.` are left out by their name,
since they hold the in-adjacency of a directed graph, a function of `adjacency.out`
(`eg-adjacency.md` § 3.2), so a file has the same identity with or without it and
results stored before it was added are still served after; and the adjacencies that
results hold (kinds 11 to 13, "Parts of a result" below) are left out by their kind, so
a result never changes the identity it is keyed by. A kind added
later that defines the graph is announced with a required feature bit (§ 10), so that a
reader that does not know it refuses the file instead of computing an identity without
it. For a segmented file (§ 13) the identity is BLAKE3 of
the manifest's `(kind, name length, name, hash)` alone, since the manifest lists the
hashes of every run's sections. A reader serves a stored result only for a graph with
that identity: after an append the graph has a new identity, and results stored before
it remain what they are, results as of the earlier graph, never served as results of
the new one.

**Results.** A result on the whole graph (`docs/api-sketch.md` § 2.8) is a section of
kind 10 named `results/<operation>/<key>`, with a provenance record of its own named
`results/<operation>/<key>.provenance`. The record's operation is the operation (such as
`pagerank`), and its parameters are the result's **cache key**: `graph` (the identity
above), `version` (the version of the operation's algorithm, in decimal; not the engine's
version, which is the record's `engine`), `deterministic` (`true` or `false`), and every
parameter of the call that can change the result (`seed` and `precision` included; never
the memory budget, the thread count or the scratch directory). `<key>` is 32 lowercase
hexadecimal digits: the first 16 bytes of BLAKE3 of the record encoded as above with only
its operation and parameters set (engine empty, times unknown, no inputs). A reader looks
a result up by that name and serves it only when the record's operation and parameters
are exactly the key it asks for and the file's identity is the key's `graph`; the name
alone is never trusted. A look-up reads the record (checked against its hash) and the
section's first 16 bytes, not the values: like any section, a result is checked against
its hash when it is verified (§ 7, § 12), not on every read. Storing a result under a key
already stored removes the old section and record in the same commit. An operation name
is not empty and holds no `/`. The section holds:

```
offset  size  field
0       8     magic        "EGRES\0\0\0"
8       4     value type   1 u32, 2 u64, 3 f32, 4 f64
12      4     reserved (§ 1)
16            the values, little-endian; (length − 16) is a whole number of values
```

The values start 16 bytes into a section that starts at a multiple of 4096, so a memory
map serves them aligned for their type. Floating-point values follow § 1: canonical NaN,
no −0.0.

**Parts of a result.** A result that needs more than one array (the levels of a
community hierarchy, `eg-communities.md`) keeps the others as **parts**: sections named
`<result>/<part>`, where `<result>` is the result's section name and `<part>` is not
empty, each referring to the result's own record. A part holds typed values (kind 10,
with the layout above) or an adjacency (kinds 11, 12 and 13, named `<part>.index`,
`<part>.data` and `<part>.events`, with the layouts of kinds 2, 3 and 5; the events give
each entry's weight). A reader finds a part only through its result, and serves it only
when it refers to the result's record. Storing a result under a key already stored
removes the old parts too, in the same commit. A part is not itself a result: a listing
of results counts its bytes with its result's. The adjacency of a part does not define
the graph, so it does not enter the graph identity; `verify --deep` checks its structure
as for kinds 2, 3 and 5, and that its lists are symmetric, loops allowed.

**Other parameters.** No field beyond the key/value parameters is needed for what the
measurement review asks a build to record (`docs/research/sota-measurement.md`, § 4,
challenge 7). These keys are conventional, so that every operation writes them the same
way: `order` (the vertex-ordering method: `input`, `first-appearance`, `sorted-key`,
`degree`, `seeded-relabel`), `order_seed` (its seed, when it has one), `directed`
(`true` or `false`) and `self_loops` (`drop` or `keep`) on the record of an adjacency's
sections, which `everygraph verify --deep` checks the lists against (symmetric lists,
no loop), `id_min` and
`id_max` (the range of the original identifiers, when they are integers),
`isolated_kept` and `isolated_dropped` (nodes with no edge kept in the graph, and
dropped by the build). The content hash of the source file is an input's fingerprint
(`algorithm` = `blake3`, `digest`), not a parameter. From the numerics review
(`docs/research/sota-numerics.md`, challenge 8): `seed` (the seed of a randomized
operation), `numerics` (the version of the engine's rules for bit-identical floating
point), `libm` (the version of the pinned maths library, rule R14), `weight_scale` (the
exponent of a fixed-point weight). Counts and seeds are
written in decimal; floating-point values as the shortest decimal that reads back to
the same bits (Rust's `{}`), never with a fixed number of digits.

## 7. Reading

Opening a file costs **two positional reads**, whatever its size:

1. Read the first 4,224 bytes (block 0 and the used part of block 1; fewer if the file
   is shorter, and a block cut short by the end of the file is absent). Classify each
   block: *valid*, *foreign* or *torn* (§ 2), or *absent* (no magic). If the source
   fails this read, read the 128 bytes of each block on their own, so that one
   unreadable sector does not hide the other block; a block that still cannot be read
   is *unreadable*.
2. If either block is foreign, refuse the file, even when the other block is valid: a
   foreign block cannot come from a tear (§ 2), it may hold the newest commit, and a
   reader that opened the other block would let a writer destroy it.
3. Take the valid block with the highest generation; on a tie, block 0. Refuse the file
   if *that block's* required features contain a bit the reader does not know (the
   other block's bits do not matter unless it is used). Read its TOC (TOC offset, TOC
   length), check the TOC hash, then check the TOC (below).

**Falling back.** If the TOC of the newest valid block cannot be used — its hash does not
match, it breaks a rule below, it lies outside the file, or the source fails to read it
— and the other block is valid, a reader opens that older commit instead and says that
it did. The update protocol never produces such a file; a disk that acknowledged a sync
it had not performed, a bad sector, or a failing network can. A read error is treated
like damage here because a reader cannot tell a bad sector from a passing failure; a
caller that would rather retry than see the older commit checks what the reader said.
If one block was unreadable and the other is used, the reader says so too: the
unreadable block may have held a newer commit. A reader says it fell back whenever the
block it used is not the first in the order of step 3 — on a tie whose block 0 is
unusable as well — so that a writer needs consent before overwriting the other (§ 8).

If no block is valid (and none is foreign), the file is refused: with the read error
when a block was unreadable; as "never completed" when a block has the magic (a first
commit that did not finish); as "not an `.eg` file" otherwise.

A reader checks, before trusting a TOC: the magic; the entry size; the exact length;
for every entry, kind ≠ 0, name length 1–1024, name offset equal to the sum of the
name lengths before it, valid UTF-8, offset a multiple of 4096 and at least 8192,
padded end at or before the TOC offset, no provenance reference on an entry of kind 1;
the names length equal to the sum of all name lengths; names strictly increasing; no two
sections overlapping (padding included); every provenance index naming another entry of
kind 1; and, in a file without required bit 0, no entry of kind 6 and no name beginning
with `seg/`. The TOC offset must be a multiple of 4096, at least 8192, and its length at
least 32 and at most 16 MiB.

**Length and padding.** A valid file is at least as long as its committed end (the TOC's
padded end, § 8), and every padding byte, after sections and after the TOC, is zero.
Opening checks neither (it reads only the header and the TOC); verification checks
both. Verification also checks bytes 128 to 4095 of both header blocks. The other
block — not the one the file reads from — holds a valid header or is empty, unless a
commit was cut short while it was being written: such a block, which no reader can
tell from a damaged one, is reported as a note, not as damage. When the file reads as
an older commit than one it holds — the newest table of contents is unusable, or the
newest header block is not valid while the table it names is whole — verification
names the lost generation; that too is not damage of what the file reads as.

A section is checked against its hash when it is used for something that must be right
(`everygraph verify`, compaction, a provenance record) — not when the file is opened,
which would read the whole file.

**No amplification.** No field of a header block or of the TOC may make a reader
allocate or read more than a bound linear in the bytes the field refers to, and every
bound is checked before the allocation or the read: the header read is fixed (4,224
bytes); the TOC is read only once a valid header points to it and is at most 16 MiB;
entries and names are counted against the TOC's own length (§ 4), so what a reader
holds after opening is linear in the TOC; a provenance record is at most 16 MiB and at
most its section; sections are hashed and copied in bounded pieces, never read whole
for the container's sake; and no pointer is followed from a section to another section,
so nothing is read twice or recursively. A later minor version that adds a pointer
states its bound in the same terms.

## 8. Updating

Sections are added to a file, never changed in place. A writer works on one file at a
time, with no other writer on it (§ 11), and its storage offers positional writes, a
length, and a **sync** after which everything written before it survives a crash of the
machine (in Rust, `everygraph_core::ByteSink`; on a native file, `fsync`).

**Appending.** The **committed end** of a file is the padded end of the TOC of its
newest valid commit. Sections are written from the committed end on, one after
another, each followed by explicit zero padding (the bytes there may be left over from
an earlier write that was never committed, so they are written, not assumed). Nothing
before the committed end is written, except the header block of step 3 below.

**Committing** makes the new set of sections the file's state. With `g` the generation
of the newest valid header block (0 for a new file) and `B` the *other* block (block 0
for a new file):

1. Write the new TOC — every section of the commit, those carried over from the
   previous TOC included — at the next multiple of 4096 after the last section, with
   its zero padding; set the file's length to the TOC's padded end (which also cuts
   off any older uncommitted bytes past it).
2. **Sync.**
3. Write header block `B`: generation `g + 1`, pointing to the new TOC.
4. **Sync.** The commit is durable when this returns.

**A failed write or sync.** If any write, length change or sync fails, the commit is not
acknowledged and the writer is **poisoned**: it does nothing more, and the file must be
opened again to go on. In particular a sync that returned an error is never retried and
then trusted: after a failed `fsync`, Linux may mark the dirty pages it could not write
clean, report success on the next call, and keep serving from the page cache bytes that
never reached the disk (the "fsyncgate" behaviour; Rebello et al., *Can Applications
Recover from fsync Failures?*, USENIX ATC 2020). So reopening is not "reading what is on
the disk". The protocol is safe anyway: a commit rewrites, whole, every byte after the
committed end it relies on (sections, padding, TOC and its padding, header block), and
relies on bytes before the committed end only through syncs that succeeded. Whatever the
page cache holds of a failed commit lies past the committed end of the commit the file
reads as, and the next writer writes over it.

When `g` is 2^64 − 1 there is no next generation: the writer refuses to commit and
writes nothing (generations never wrap; a wrapped one would be a commit that is not the
file's state). At one commit per microsecond that takes 584,000 years; the rule exists
so that a damaged or hostile header cannot make a commit look successful.

**After a fallback.** A file opened at an older commit (§ 7: the newest TOC unusable, or
a header block unreadable) is not updated unless the caller asks for it explicitly
(`Writer::update_recovered`): its committed end is the older commit's, so the first
commit writes over the newer commit's sections and TOC, and step 3 over the header
block that may have held it. That newer commit may be intact but for one bad sector; a
writer must not destroy the only copy without being told to.

A new file is created the same way: length set to 0, then to 8192 (two zero header
blocks), sections, then steps 1–4. It is written under a temporary name
(`NAME.eg.partial`, `docs/ux-contract.md` C-1) and renamed to `NAME.eg` only after
step 4, and the directory that holds it is then synced so that the rename survives a
power cut (`everygraph_io::persist` does both; on Windows, where a directory cannot be
synced, the rename is durable once the file system commits its journal). Compaction
ends the same way.

**What a crash leaves.** Whatever happens at any instant — the process killed, the
machine losing power, any write before a sync lost, reordered or torn at any byte:

- Before step 3 reaches the disk whole, both header blocks are as they were: the file
  reads as the previous commit, byte for byte, because no byte of it was written. A
  torn step 3 fails block `B`'s hash and is ignored.
- Once step 3 is on the disk whole, the file reads as the new commit, whose sections
  and TOC were made durable by step 2 before step 3 was written.
- So at every instant the file reads as one of the two commits, every section of that
  commit matches its hash, and no section any earlier commit held has changed.
- What remains of an interrupted write is bytes past the committed end, which no commit
  refers to; the next writer writes over them and step 1 cuts off any excess.
- After step 4 returns, the new commit survives any crash.

Removing a section is a commit whose TOC no longer lists it; its bytes stay until
compaction. Replacing a section is a removal and an addition under the same name. A
writer may commit several times in one session (the three cached phases of `map`,
ux-contract § 5.1): each commit is a generation, and a crash leaves the file at the
last one that reached the disk.

These guarantees are tested step by step in `crates/everygraph-format/tests/container.rs`:
a sink records every write, length change and sync, and at every step every image the
sync model allows is checked — everything, nothing, every prefix, every suffix (the
writes issued first lost, later ones kept), each write alone, each write torn after
1 byte, half way and all but the last byte (a header write after every one of its 128
bytes), each write replaced by garbage rather than zeros, random subsets torn by 512-byte
sector or turned to garbage. Two sequences are checked the same way: a crash, then a
second update from the crashed image, which crashes at every step; and a commit to a
file opened at an older commit (a damaged newest table), which crashes at every step and
must leave the older or the new commit. The writes of one
unsynced window never overlap, so applying any subset of them in program order covers
every order in which they could reach the disk. That model is argued, not measured on
hardware. A real `kill -9` of a writing process (`crates/everygraph-io/tests/container.rs`)
tests process death only: the page cache survives it, so it produces the "everything
written" image, never a lost or torn write.

## 9. Compaction

Compaction writes a new file holding exactly the sections of the newest commit (same
names, kinds, codecs, hashes and provenance references; provenance records first, then
in the order of the original offsets), as one commit of generation 1. Every section is
checked against its hash while it is copied: a damaged section stops the compaction,
which never makes damage look whole. The new file is written under a temporary name and
persisted over the original (§ 8); the original is not modified, so a crash during
compaction leaves it as it was.

What compaction reclaims is the file's length minus the bytes the newest commit uses
(the header blocks, every section and the TOC, each with its padding): sections removed
or replaced, older TOCs, and bytes of writes never committed. A reader can compute it
from the TOC alone (`Container::reclaimable_bytes`).

## 10. Versions

- **Major version.** A reader refuses a major version it does not know, in either
  header block (§ 7). A new major version is needed when a reader of the previous one
  would misread the file.
- **Minor version.** A reader reads every minor version of its major. A new minor
  version may only add what an older reader can ignore: kinds, codecs, optional feature
  bits, longer TOC entries, meanings for reserved fields. A writer does not update a
  file whose minor version is newer than its own (it could not keep what the newer minor
  added); it may update an older one, and writes its own minor version.
- **Required features.** A bit set in the header: a reader refuses a file in which a
  bit it does not know is set, without falling back to an older commit. This is how a
  later version adds something that must be understood (a section kind without which
  the graph would read wrong). None are defined in 1.0. Bit 0 of the required
  features, `segments`, is reserved: it is set exactly when the graph of the file is the
  union of more than one adjacency run (§ 13). A file without it is read as today; a
  1.0 reader refuses a file with it, rather than read the base run alone and return an
  older graph. Setting it only when segments exist keeps every unsegmented file
  readable by 1.0 readers. Bit 0 goes with kind 6: a file holds the segment manifest
  (kind 6, `segments`) exactly when it sets bit 0, and a reader that finds one without
  the other treats the file as damaged.
- **Optional features.** A bit set a reader may ignore; writers keep the bits they find.
  None are defined in 1.0.
- **Unknown kinds** are listed and skipped — only kinds the reader does not know; the
  kinds 1.0 defines, 2–5 above all, are read by every reader that reads the graph (§ 5).
  A kind added after 1.0 that defines the graph (and so enters its identity, § 6) is
  announced with a required feature bit.
  **Unknown codecs** make a section unreadable but not unverifiable (§ 5).

## 11. Memory maps and concurrent use

A writer only ever writes the header block of step 3 and bytes at or after the
committed end, and only ever shortens the file to the TOC's padded end, which is at or
after the committed end of every earlier commit. So for a reader of any earlier commit,
the bytes from 8192 to that commit's committed end never change while it reads.

That is what makes reading during an update safe, under the invariant of
`design/DECISIONS.md` ("Memory-mapped files are read through a safe constructor"): a
reader that maps a graph file maps only the range from 8192 to the committed end of the
commit it opened (`Container::stable_range`,
`everygraph_io::MmapSource::from_file_range`), and keeps the header and the table it read
before mapping (`Container::with_source`): reading the header blocks again could find a
newer commit, whose sections lie past the map. A map of the whole file is not safe
against a commit by someone else: step 3 changes 128 bytes it covers, and step 1 may cut
off an uncommitted tail it covers (touching those pages then ends the reader with
`SIGBUS`). Within one process, `everygraph_io::FileSink` refuses each write or length
change that would reach a mapped range (`ResourceBusy`): a commit goes through beside a
map of that range, and on Unix fails beside a whole-file map (on Windows there is no such
guard). A map reads the file's length and registers its range in one step, so no sink of
the process can shorten the file between the two. Windows refuses to shorten a file that
another process maps: there step 1 leaves the bytes past the new committed end in place,
which § 7 allows and a later commit or `compact` removes.

One writer at a time: two writers appending to one file at once would write over each
other's sections. The container itself does not lock. `everygraph_io::FileSink` holds an
exclusive lock on its file while it lives (`File::try_lock` on Unix; on Windows the share
mode, since `File::try_lock` there would stop readers too), and a second sink, in this
process or another, is refused with `ResourceBusy` without waiting. The lock is on the
open file, not on its name: a compaction in place holds a `FileSink` on the original
until the rename over it has returned, and a sink that finds the name pointing to another
file once it holds the lock opens it again. On Unix the lock is advisory: a program that
does not take it is not stopped. The operating system releases it when the writer dies,
so a crash leaves no stale lock.

## 12. Decided here

- **Two self-contained header blocks, alternated**, rather than one header rewritten
  in place: a single 4 KiB block rewritten during a power cut could be torn, and with it
  the only pointer to the TOC. Each block is in its own 4 KiB page, so that even a
  drive that rewrites a whole 4 KiB sector to change 128 bytes cannot damage the other.
  The cost is 4 KiB per file and one more check on open.
- **The TOC is the last committed item**, so the committed end follows from the header
  alone and a writer never needs to search for free space.
- **Hashes are checked on use, not on open.** Opening a 15 GB file reads 4 KiB and the
  TOC; `verify` reads everything.
- **Fall back to the older commit** when the newest TOC cannot be used — read errors
  included — and say so, rather than refuse the file: a lying disk or a bad sector then
  costs the last commit, not the graph. Writers do not follow a fallback without
  consent (§ 8), so reading the older commit never destroys the newer one.
- **Names sorted in the TOC**: look-up by binary search and uniqueness checked in one
  pass, with no hash table in the format.
- **The name area is the names concatenated, nothing else**: a TOC of 16 MiB then holds
  at most 16 MiB of names. Letting names overlap (as a first draft did) let a 9 MB TOC
  hold 258 MB of distinct names (review of PR #170).
- **No provenance on provenance records**: references then always point from sections
  to records, and a copy can write every record first; a chain or a cycle of records
  is not expressible.
- **Generations do not wrap**, and 0 is never valid.
- **Provenance records are sections** referred to by entry index, not fields of each
  entry: one record serves many sections, records are hashed like everything else, and
  the TOC stays small.
- **Provenance in a fixed binary layout**, not JSON: the format crate takes no JSON
  dependency, and a reader in any language parses it with length-prefixed reads.
- **BLAKE3 from the `blake3` crate with its `pure` feature**: no C or assembly to build
  on any target (wasm32 and i686 included). That forgoes its AVX-512 and NEON
  assembly; hashing runs on its Rust SSE4.1/AVX2 code on x86 and portable code
  elsewhere. Hashing is on the path of writing sections and of `verify`, not of
  reading.
- **Single file for every state of a graph, in-progress builds included** — ADR 0027.
- **Results are found by name, then checked against their record.** The name holds a
  hash of the key, so a look-up is one binary search in the TOC and one record read,
  however many results the file holds; comparing the record with the key means a name
  written by another program is never trusted alone.
- **A result of many arrays is one result with parts**, not many results: one key, one
  record and one look-up find all of it, and replacing it cannot leave a stale array.
  The adjacencies of parts have kinds of their own (11–13) rather than kinds 2, 3 and
  5, so that a result never changes the graph identity it is keyed by.
- **The cache key holds the algorithm's version, not the engine's.** An upgrade that
  does not change an algorithm keeps its stored results; one that does changes the
  version and so the key (`docs/api-sketch.md` § 2.10).
- **One writer at a time is a lock that refuses, not one that waits**
  (`design/DECISIONS.md`, "A second writer on a graph file is refused"). On Windows it
  is the share mode of the writer's open (no other handle may write), not
  `File::try_lock`: that is `LockFileEx`, a mandatory lock, under which readers in
  other processes could not read the file either.
- **Segments are reserved now, built later** (§ 13): required bit 0, the `seg/` names,
  kind 6 and the graph identity cost nothing in an unsegmented file, and let
  appends (m9-04) arrive without a new major version
  (`docs/research/sota-dynamic.md` § 0.2). One commit may add a segment and drop the
  runs a merge replaced; a reader of an earlier commit keeps reading those runs (§ 11).

## 13. Segments (reserved; not implemented in 1.0)

A segmented file holds a base run and segments `1 … k`, each a complete set of graph
sections (adjacency out and in, `.events`, columns, dictionary extension) over the node
ids `0 … n_i − 1` known after it. The manifest (kind 6, name `segments`) lists, per
segment: number, level (compaction tier), node count after it `n_i`, pairs, events,
`t_min` and `t_max` of its events in the time column's unit (absent without a time
column), the input fingerprints, and the hashes of its sections. The graph of the file
is the union of the events of all runs; a pair's aggregates are computed over its
events in every run. Appending writes one segment and commits; merging writes a merged
run and commits a TOC without the runs it replaces; `compact` reclaims their bytes.

What 1.0 fixes of this, and nothing more: a file with segments sets required bit 0
(§ 10), names its segments' sections `seg/NNNNNNNN/…` (§ 4), stores the manifest as
kind 6, and gives results the graph identity of § 6. The manifest's byte layout, the
merge policy and the reading rules are specified with the issue that builds segments.
