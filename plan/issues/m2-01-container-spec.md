---
title: The .eg container: specification v0 and reader/writer
milestone: M2 · Format and ingest
labels: type:spec, area:format, P0, risk:medium, size:L
---
**Why.** One file per graph is the "a graph is a file" story (DESIGN D7). The container was not prototyped in the lab — only the map side (PMTiles) was — so this is new design.

**Design constraints from the lab**
- Sections 4 KiB-aligned, little-endian, padded ≥ 16 bytes so SIMD decoders may over-read (C)
- Readable through positional reads only: local file, mmap, HTTP range, browser `File` (G)
- No section may require a single > 2 GiB allocation to use (wasm32)
- Derived artifacts are appended later (in-adjacency, hierarchy, layout, indexes), so the table of contents must be rewritable without moving data

**Scope**
- [ ] Layout: fixed header (magic, version, endianness, feature flags), sections, table of contents at the end plus a pointer in the header; each TOC entry has kind, name, offset, length, codec, content hash (BLAKE3), and a provenance record id
- [ ] Atomic update protocol: append sections, write new TOC, fsync, flip the header pointer; a crash leaves the previous TOC valid
- [ ] `compact` to reclaim replaced sections
- [ ] Versioning and forward-compatibility rules (unknown section kinds are skipped; feature flags that readers must understand)
- [ ] Provenance record: input fingerprints, parameters, engine version, timestamps
- [ ] Reader/writer in `eg-format` over `ByteSource`; a minimal independent reader in Python (≈ 100 lines) to prove the spec is sufficient
- [ ] Written spec in `docs/spec/eg-container.md`

**Acceptance**
- Round-trip and crash-injection tests (kill between every write step)
- The independent Python reader lists sections and verifies hashes of a file written by Rust
- Opening a 15 GB file costs one header read plus one TOC read

**Open question.** Single file vs directory bundle for in-progress builds: decide here and record the ADR.

**Depends on** {{#m2-02-io-layer}}
