# ADR 0023 — Licence Apache-2.0 OR MIT; published format specifications

- **Status:** accepted
- **Decision:** D23 in [DESIGN.md](../DESIGN.md#4-decisions)
- **Date:** 2026-11-25

## Context

An embeddable engine is adopted only if its licence lets companies and other projects
depend on it, and a file format outlives its implementation only if it is specified.
Recent graph projects show what happens otherwise.

## Decision

Code is licensed Apache-2.0 OR MIT. The file formats (`.eg`, `.egmap`, the tile payload)
are published as specifications.

## Evidence

This is a decision informed by events, not by a measurement (FINDINGS R7, web research,
as of 2026-11):
- Kùzu was acqui-hired and archived on 2025-10-10, leaving users with a fork (LadybugDB).
- CozoDB, IndraDB and lance-graph are dormant; Memgraph merged MAGE into its BSL core.
- webgraph-rs, the one optional dependency of note, is Apache-2.0 OR MIT since 0.7.0
  (FINDINGS § C), so the optional feature does not change the licence picture.
- Issue #9 (licence, contribution rules, governance) is closed; the licence files are in
  the repository root.

## Consequences

- Dependencies must be compatible with Apache-2.0 OR MIT; the dependency licence check of
  #9 runs in CI.
- Each on-disk format has a written specification in the pull request that introduces it
  (#28 for the container, #68 for tiles).

## What would change our mind

- A dependency needed for v0.1 is only available under a licence incompatible with
  Apache-2.0 OR MIT.
- A foundation the project joins (#115) requires a different licence.
