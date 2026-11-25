# ADR 0027 — A graph is one file in every state; builds in progress use the job's scratch

- **Status:** accepted
- **Decision:** the open question of #28 under D7 in [DESIGN.md](../DESIGN.md#4-decisions)
  (single file or directory bundle for builds in progress); amends
  [ADR 0007](0007-eg-container.md)
- **Date:** 2026-11-25

## Context

ADR 0007 made a graph one `.eg` file and left one question to #28: should a build in
progress (an ingest that takes an hour, a `map` that commits three phases) be a
directory of loose files that becomes one file at the end, or one file from the start?
A directory bundle is what the lab did (`glab-csr-v0`, loose arrays) and what some
engines do (a database directory). Its attractions: no compaction ever, each artifact
replaced by renaming one file, readers that map one array per file.

What the rest of the plan already fixes:

- A **new** graph is written as `NAME.eg.partial` and renamed to `NAME.eg` once durable
  (`docs/ux-contract.md` C-1); what an interrupted ingest keeps lives in the job's
  scratch directory `.everygraph-ID` with its manifest (§ 5.3, R-4–R-9).
- An **existing** graph gets cached results appended — the first three phases of
  `map` are committed one by one (§ 5.1) — and a stop must never change what it held
  (C-2).
- A graph is read through positional reads only: a file, a memory map, an HTTP range,
  a browser `File` (DESIGN D2, FINDINGS § G).

## Decision

**The `.eg` is one file in every state, and nothing about a graph is ever a directory.**
A build in progress keeps its intermediate products in the job's scratch directory (the
one that already exists for resumption, with its manifest), and writes its output as
one container, `NAME.eg.partial`, through the same writer and commit protocol as an
update (`docs/spec/eg-container.md` § 8); the rename to `NAME.eg` is the last step.
Results added to an existing graph are appended to it and committed; replaced ones are
reclaimed by `compact`, which writes a new file and renames it over the old one.

## Evidence

This is a design decision; the evidence is about what each choice costs.

- **The constraints of one file are met** (#28, tests in the pull request): opening
  reads the two header blocks and the table of contents — two reads on a synthetic
  15 GB file (`opening_a_15_gb_file_costs_one_header_read_and_one_table_read`); a crash
  at any step of an update, under every image the sync model allows, leaves the previous
  or the new commit and changes no byte the file held
  (`a_crash_at_any_step_of_an_update_leaves_the_previous_or_the_new_state` and the two
  tests beside it); a writer killed with `SIGKILL` twenty times at drifting moments left
  a valid file with every acknowledged commit each time
  (`a_writer_killed_at_any_moment_leaves_a_valid_file_with_every_acknowledged_commit`).
- **A bundle would break the browser and HTTP paths**: a browser `File` from a drop or
  a file picker is one file, and a static server serves one URL per file; a bundle would
  need an archive format on top (zip, tar) to travel, and then it is one file again with
  a second layout to specify.
- **A bundle would not make builds safer**: what makes an interrupted build resumable is
  the manifest and the phase files of the scratch directory, which exist anyway; the
  output file only ever receives finished phases.
- **Compaction is needed rarely**: appending a result under a new cache key leaves
  nothing to reclaim; only a replaced or removed result does, and the bytes a crash
  leaves past the committed end are overwritten by the next write. How much there is
  to reclaim is known from the table of contents alone
  (`Container::reclaimable_bytes`), so `info` can say when `compact` is worth it.
- Not measured: how often real use replaces results (it depends on #39's cache keys), and
  the cost of `compact` on a large file (it copies every live byte once, hashing it;
  no timing was taken).

## Consequences

- #35 and #40 write `NAME.eg.partial` with `everygraph_format::container::Writer` over
  `everygraph_io::FileSink`, from the products of the scratch directory, commit, then
  rename and sync the directory.
- #39 appends results with their provenance records and commits; a result that replaces
  another removes it in the same commit.
- `compact` is a command over `everygraph_format::container::compact` (#98); `info`
  reports the reclaimable bytes.
- A process that holds a graph mapped cannot commit to it until the map is dropped
  (`everygraph-io`'s in-process guard). Mapping only the section range, which the
  protocol makes safe (spec § 11), is a follow-up.

## What would change our mind

- Results are replaced so often that files grow without bound between compactions and
  users have to run `compact` by hand to keep their disks.
- A use that needs several processes to write one graph at once (the container has one
  writer at a time).
