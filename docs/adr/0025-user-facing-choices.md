# ADR 0025 — User-facing choices live in `design/DECISIONS.md`

- **Status:** accepted
- **Decision:** D25 in [DESIGN.md](../DESIGN.md#4-decisions)
- **Date:** 2026-11-25

## Context

Many choices a user sees or types — API names and shapes, CLI messages, defaults of graph
semantics, map style — are made in previews and specifications, not by measurement. They
need one place of record so that a later session can tell a considered choice from an
accident.

## Decision

User-facing choices are fixed in `design/DECISIONS.md`: API shape and names, CLI
messages, graph-semantics defaults, map style (bubbles on a dark canvas, links for the
selection only, five reused region colours, artefact regions drawn as such). A choice is
changed by adding a row there, not by building something different.

## Evidence

- The owner approved the previews in `design/preview/` as a whole (#119, closed) and on
  2026-11-25 put the project under Claude's management; where the approved design is
  silent, the choice and its reason are recorded as a row (`design/DECISIONS.md`, and
  `CLAUDE.md` § 8).
- Several rows have been decided since then, each with its reason (for example "What
  'repeatable' promises", default weights, time windows, exit status 6 for an unknown
  node).
- No measurement applies; these are product choices.

## Consequences

- Each choice stays open to change when it is built; changing one is a new row.
- A row that changes a decision of `docs/DESIGN.md` also needs an ADR here. Two have
  already: "What 'repeatable' promises" widened D14 (ADR 0014), and the default number of
  Leiden iterations is to be confirmed by #57 with an ADR (ADR 0015).
- The API and command-line specifications (#12, #13) cite the rows they implement.

## What would change our mind

- The pre-launch review by outside practitioners (#105) asks to rename public names or
  change defaults; those changes are rows in `design/DECISIONS.md`, frozen at 0.1.
