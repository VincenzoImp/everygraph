# ADR 0024 — One visual system, Vernier

- **Status:** accepted
- **Decision:** D24 in [DESIGN.md](../DESIGN.md#4-decisions)
- **Date:** 2026-11-25

## Context

Everygraph shows itself in a map viewer, documentation, notebooks, a terminal and
exported figures. Without one system each surface drifts, and data colours chosen by eye
fail colour-blind readers.

## Decision

One visual system, "Vernier", for viewer, docs, notebook, terminal and figures: warm paper
and soft black, one ink-blue accent, every number in monospace, the two-ring mark. Values
live in `design/system/tokens.css`, rules in `design/preview/style/SPEC.md`.

## Evidence

This is a design decision approved by the owner, not a lab measurement:
- Three rounds of previews; Vernier approved by the owner on 2026-11-25
  (`design/DECISIONS.md`, row "Visual identity, round 2").
- Contrast and palette were measured, not assumed (`design/preview/style/SPEC.md`, last
  section): text tokens reach 4.5:1 on every surface they are allowed on, control borders
  3:1, computed live by `foundations.html` from the tokens; the ten data colours keep
  ΔE ≥ 15 for normal vision and about 7.3 for protanopia and deuteranopia, which is why
  they are always used with labels. These figures are computed by the preview pages, not
  stored as result files.

## Consequences

- #121 turns the tokens into the shared design-system package used by the viewer, docs
  and notebook widget.
- Any surface that departs from `SPEC.md` is a decision change recorded in
  `design/DECISIONS.md`, not a local choice.

## What would change our mind

- A contrast or colour-vision check run on the built viewer (#77) fails for a token, or
  user testing shows the five reused region colours are confused on real maps; the
  palette is then re-measured and amended in `SPEC.md` and `design/DECISIONS.md`.
