---
title: Professional trademark and freedom-to-operate opinions, advisable before 0.1
milestone: M8 · CLI, docs and v0.1 launch
labels: type:decision, area:launch, P1, risk:high, size:S
---
**Why.** The project's own look-ups (`docs/legal/`) are fact-gathering, not advice, and they found two things only a professional can settle: a pending US trademark application for the identical name and goods (see `docs/legal/name-check.md`), and two granted patents with elements near the tile pyramid (`docs/legal/patent-check.md`). The repository is already public: the owner accepted both risks for that (`design/DECISIONS.md`, "When the repository becomes public", 2026-11-25). The answers are advisable before the first release ({{#m8-08-release}}), but they do not block it: the owner accepts the trademark and patent risk (2026-11-25).

**Scope**
- [ ] Trademark: once the final name is chosen, a clearance search in the US, EU and UK (classes 9 and 42) by a professional, and a filing if advised
- [ ] Patents: the questions listed at the end of `docs/legal/patent-check.md`, put to a patent attorney with the tile-format specification in hand
- [ ] Record the answers and any resulting design change as an ADR

**Acceptance**
- Written answers on file, or the opinions skipped with the reason written down; the release checklist ({{#m8-08-release}}) has the box "legal opinions recorded or skipped with the reason" ticked

**Depends on** {{#m0-09-name-trademark}}, {{#m5-02-tile-format}}

**Needs the owner's hands.** Engaging and paying a professional is the owner's action; Claude prepares the brief and the documents.
