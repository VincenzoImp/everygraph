# ADR 0007 — One `.eg` file per graph; maps exported as PMTiles

- **Status:** accepted
- **Decision:** D7 in [DESIGN.md](../DESIGN.md#4-decisions)
- **Date:** 2026-11-25
- **Amended:** 2026-11-25, by #28: the container is specified (`docs/spec/eg-container.md`)
  and validated by its tests; single file in every state, builds in progress included
  ([ADR 0027](0027-single-file-builds.md)).

## Context

"A graph is a file": the topology, the id dictionary, property columns and every derived
artifact (in-adjacency, hierarchy, layout, indexes) should travel together, be readable
through positional reads only (file, mmap, HTTP range, browser `File`), and survive a
crash while results are appended. The map must be servable as a static file.

## Decision

One `.eg` file per graph: a section table, content-hashed sections, derived artifacts
appended later, and `compact` to reclaim space. Maps are exported as a separate standard
PMTiles v3 file.

## Evidence

- **Map side, validated** (FINDINGS § E and R5; shared machine): a real PMTiles v3 file
  with a custom payload (tile type 0x00) was written and opened by the official Python
  reader; lj produced 173,017 tiles in one 851 MB file, orkut 63,846 tiles in 460 MB
  (`lab/e-map/results/tiles_lj_t32.json`, `lab/e-map/results/tiles_orkut_t32.json`).
- **The `.eg` container, validated by #28** (it was not prototyped: the lab stored graphs
  as loose files, `glab-csr-v0`, `lab/LAB.md`; its constraints come from other
  experiments: 4 KiB-aligned sections and over-read padding for SIMD decoders (§ C),
  positional reads only (§ G), no section needing one allocation above 2 GiB (§ G)).
  Specification `docs/spec/eg-container.md`. Tests, not measurements (no timing is
  claimed): opening costs two reads — the header blocks and the table of contents — on
  a synthetic 15 GB file (`everygraph-format`, `container::read` unit test); a crash at
  every step of creating a file, of an update and of three commits by one writer, with
  every image the sync model allows (lost, reordered and torn writes after the last
  sync, header writes torn at every byte), leaves the previous or the new commit, every
  section matching its hash, and no section the file held changed (`tests/container.rs`
  of `everygraph-format`; the sync model is argued in the specification, § 8, not
  measured on hardware); a writer killed with `SIGKILL` at drifting moments, twenty
  times, left a valid file holding every acknowledged commit (`tests/container.rs` of
  `everygraph-io`) — evidence about process death only, since the page cache survives
  `SIGKILL`; an independent
  reader in Python, written from the specification alone, lists the sections of a file
  written by Rust and verifies their BLAKE3 hashes (`tools/check_container_python.py`).

## Consequences

- #28 wrote the specification and the reader/writer, with crash-injection tests and an
  independent Python reader, and decided single file against directory bundle for builds
  in progress: one file in every state (ADR 0027).
- A process commits to a graph file only when it holds no memory map of it (the sinks
  refuse, `design/DECISIONS.md`); the protocol never writes bytes a reader of an earlier
  commit can see between offset 8192 and its committed end, so mapping that range only
  would lift the restriction (issue `m2-21-range-maps`).
- Formats inside sections are specified separately (codec #30, `docs/spec/eg-adjacency.md`;
  columns #37); they do not refer to the container.

## What would change our mind

- #28's acceptance fails: a crash between write steps leaves no valid table of contents,
  or opening a 15 GB file costs more than one header read plus one table-of-contents read.
- Appending derived artifacts makes `compact` necessary so often that a directory bundle
  is simpler for users than one file.

## Closed by

#28 (container specification v0 and reader/writer).
