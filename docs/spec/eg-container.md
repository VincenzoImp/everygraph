# The `.eg` container: byte layout, version 1.0

Status: specification for 0.1, written by #28 (`plan/issues/m2-01-container-spec.md`;
"v0" of the specification, format version 1.0). Implemented by
`crates/everygraph-format/src/container/`; an independent reader written from this text
alone is `tools/eg_container.py`. The decision it implements is DESIGN D7 with ADR 0007,
and ADR 0027 for single file against directory bundle.

A graph file holds named, content-hashed **sections**: the parts of the graph
(adjacency, identifier dictionary, columns) and everything derived from it later
(in-adjacency, results, hierarchy, layout, indexes). Nothing in a section refers to the
container: each section's own format is specified separately (`eg-adjacency.md`, the
dictionary's sections in `everygraph_ingest::ids`). The container only says where each
section is, what it is, how to check it and how it was made, and how sections are added
without risk to what the file already held.

Contents: 1 Conventions · 2 Header blocks · 3 Sections · 4 Table of contents ·
5 Kinds and codecs · 6 Provenance records · 7 Reading · 8 Updating · 9 Compaction ·
10 Versions · 11 Memory maps and concurrent use · 12 Decided here.

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
16      4     block              0 or 1: which block this is
20      4     reserved
24      8     generation         ≥ 1: the number of the commit this block records
32      8     required features  bit set (§ 10); none defined in 1.0
40      8     optional features  bit set (§ 10); none defined in 1.0
48      8     TOC offset         where the table of contents starts
56      8     TOC length         its length in bytes
64      32    TOC hash           BLAKE3 of the TOC's TOC-length bytes
96      32    block hash         BLAKE3 of bytes 0 … 95 of this block
```

A block is **valid** when its magic is right, its byte order is `0x0A0B0C0D`, its major
version is 1 and its block hash matches. Bytes 0 to 15 (magic, byte order, versions)
keep their meaning in every version, so that a reader can tell a foreign file from a
damaged one: a block with the magic and another byte order or major version is a file
this reader does not read (§ 7), not a torn block.

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
56      4     name offset  into the name area
60      4     name length  1 to 1024
64      4     provenance   the index (0-based, in this TOC) of the entry holding the
                           provenance record of this section, or 0xFFFFFFFF for none
68      4     reserved
```

- A **name** is 1 to 1024 bytes of UTF-8, unique in the TOC. Entries are sorted by name,
  bytewise and strictly increasing, which also proves the names unique. Names are not
  interpreted by the container; the graph layer gives them meaning
  (`adjacency.out.index`, `ids.keys`, `results/…`).
- A provenance reference names another entry of the same TOC, whose kind is 1
  (provenance record). An index is valid only within its TOC: a writer that rewrites the
  TOC renumbers it.
- The name area holds the names in entry order in what this implementation writes; a
  reader only uses each entry's offset and length into it.

## 5. Kinds and codecs

The **kind** says what a section's bytes are; it is assigned by the specification that
defines them. A reader that does not know a kind lists the section, can verify and copy
it, and otherwise ignores it: that is how later issues add artifacts without a new
version. A section that a reader *must* understand to read the file correctly is
announced with a required feature (§ 10), not by its kind.

| Kind | Meaning | Specified in |
|---|---|---|
| 0 | not allowed | — |
| 1 | provenance record | § 6 |
| 2 | compressed adjacency, index | `eg-adjacency.md` § 3 |
| 3 | compressed adjacency, data | `eg-adjacency.md` § 1 |
| 4 | one section of an identifier dictionary (the name, `ids.*`, says which) | `everygraph_ingest::ids` (#34) |
| 5 … 0x7FFFFFFF | assigned by later specifications (columns #37, results #39, hierarchy, layout, indexes) | — |
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

## 7. Reading

Opening a file costs **two positional reads**, whatever its size:

1. Read the first 4,224 bytes (block 0 and the used part of block 1; fewer if the file
   is shorter). Classify each block: *valid* (§ 2); *foreign* (the magic, but another
   byte order or major version); *torn* (the magic but a wrong block hash); *absent*
   (no magic).
2. Take the valid block with the highest generation (block 0 on a tie). Refuse the file
   if its required features contain a bit the reader does not know. Read its TOC
   (TOC offset, TOC length) and check the TOC hash. Check the TOC (below).

If the TOC of the newest block cannot be used — its hash does not match, it lies outside
the file, or it breaks a rule below — and the other block is valid, a reader should open
that older commit instead and say so. The update protocol never produces such a file;
a disk that acknowledged a sync it had not performed can. If no block is valid, the
file is refused: as "not an `.eg` file" when no block has the magic, as "never
completed" when one has the magic (a first commit that did not finish), as unreadable
by this version when a block is foreign.

A reader checks, before trusting a TOC: the magic; the entry size; the exact length;
for every entry, kind ≠ 0, name length 1–1024 and within the name area, valid UTF-8,
offset a multiple of 4096 and at least 8192, padded end at or before the TOC offset;
names strictly increasing; no two sections overlapping (padding included); every
provenance index naming another entry of kind 1. The TOC offset must be a multiple of
4096, at least 8192, and its length at least 32 and at most 16 MiB.

A section is checked against its hash when it is used for something that must be right
(`everygraph verify`, compaction, a provenance record) — not when the file is opened,
which would read the whole file. Verification also checks that the padding is zero.

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

A new file is created the same way: length set to 0, then to 8192 (two zero header
blocks), sections, then steps 1–4. It is written under a temporary name
(`NAME.eg.partial`, `docs/ux-contract.md` C-1) and renamed to `NAME.eg` only after
step 4; the rename and the sync of the directory that makes it durable are the
caller's.

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

These guarantees are tested step by step in `crates/everygraph-format/tests/container.rs`
(a sink that records every write, length change and sync; at every step, every image
the sync model allows: everything, nothing, every prefix, each write alone, each write
torn at the first byte, half way and the last byte, random subsets torn by 512-byte
sector) and with a real `kill -9` of a writing process in
`crates/everygraph-io/tests/container.rs`.

## 9. Compaction

Compaction writes a new file holding exactly the sections of the newest commit (same
names, kinds, codecs, hashes and provenance references; provenance records first, then
in the order of the original offsets), as one commit of generation 1. Every section is
checked against its hash while it is copied: a damaged section stops the compaction,
which never makes damage look whole. The new file is written under a temporary name and
renamed over the original by the caller; the original is not modified, so a crash
during compaction leaves it as it was.

What compaction reclaims is the file's length minus the bytes the newest commit uses
(the header blocks, every section and the TOC, each with its padding): sections removed
or replaced, older TOCs, and bytes of writes never committed. A reader can compute it
from the TOC alone (`Container::reclaimable_bytes`).

## 10. Versions

- **Major version.** A reader refuses a major version it does not know. A new major
  version is needed when a reader of the previous one would misread the file.
- **Minor version.** A reader reads every minor version of its major. A new minor
  version may only add what an older reader can ignore: kinds, codecs, optional feature
  bits, longer TOC entries, meanings for reserved fields. A writer does not update a
  file whose minor version is newer than its own (it could not keep what the newer minor
  added); it may update an older one, and writes its own minor version.
- **Required features.** A bit set in the header: a reader refuses a file in which a
  bit it does not know is set, without falling back to an older commit. This is how a
  later version adds something that must be understood (a section kind without which
  the graph would read wrong). None are defined in 1.0.
- **Optional features.** A bit set a reader may ignore; writers keep the bits they find.
  None are defined in 1.0.
- **Unknown kinds** are listed and skipped; **unknown codecs** make a section
  unreadable but not unverifiable (§ 5).

## 11. Memory maps and concurrent use

A writer only ever writes the header block of step 3 and bytes at or after the
committed end, and only ever shortens the file to the TOC's padded end, which is at or
after the committed end of every earlier commit. So for a reader of any earlier commit,
the bytes from 8192 to that commit's committed end never change while it reads.

That is what makes reading during an update safe, under the invariant of
`design/DECISIONS.md` ("Memory-mapped files are read through a safe constructor"): a
reader that maps a graph file must map only the range from 8192 to the committed end
of the commit it opened, and read the header blocks with positional reads. Today
`everygraph_io::MmapSource` maps whole files, so within one process the sinks refuse to
open a file that is mapped (`ResourceBusy`): a process commits to a graph file only
after dropping its maps of it. Mapping by range, which would let a process commit to a
graph it holds mapped, is a follow-up.

One writer at a time: two writers appending to one file at once would write over each
other's sections. The container does not lock; the caller holds a lock that keeps
writers apart (the command line and the Python layer, which own the file).

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
- **Fall back to the older commit** when the newest TOC cannot be used, and say so,
  rather than refuse the file: a lying disk then costs the last commit, not the graph.
- **Names sorted in the TOC**: look-up by binary search and uniqueness checked in one
  pass, with no hash table in the format.
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
