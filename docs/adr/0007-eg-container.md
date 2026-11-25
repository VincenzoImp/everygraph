# ADR 0007 — One `.eg` file per graph; maps exported as PMTiles

- **Status:** accepted, partly validated
- **Decision:** D7 in [DESIGN.md](../DESIGN.md#4-decisions)
- **Date:** 2026-11-25

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
- **The `.eg` container, unvalidated**: it was not prototyped. The lab stored graphs as
  loose files (`glab-csr-v0`, `lab/LAB.md`). Its constraints come from other experiments:
  4 KiB-aligned sections and over-read padding for SIMD decoders (§ C), positional reads
  only (§ G), no section needing one allocation above 2 GiB (§ G).

## Consequences

- #28 writes the specification and the reader/writer, with crash-injection tests and an
  independent Python reader; it also decides single file vs directory bundle for builds in
  progress, recorded as an ADR.
- Nothing else depends on container details until #28 merges; formats inside sections
  (codec #30, columns #37) are specified separately.

## What would change our mind

- #28's acceptance fails: a crash between write steps leaves no valid table of contents,
  or opening a 15 GB file costs more than one header read plus one table-of-contents read.
- Appending derived artifacts makes `compact` necessary so often that a directory bundle
  is simpler for users than one file.

## Closed by

#28 (container specification v0 and reader/writer).
