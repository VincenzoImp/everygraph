---
title: Tutorials and notebooks for the ten user stories
milestone: M6 · Python and interop
labels: type:docs, area:docs, P1, risk:low, size:M
---
**Why.** DuckDB and Polars grew through documentation that starts from tasks. Ours reads like a tool that wants to be used, and stays honest about limits.

**Scope**
- [ ] One executable notebook per user story from {{#m0-11-api-sketch}}, each on a public dataset small enough for a laptop, with a "scale it up" section
- [ ] "Coming from NetworkX", "Coming from Gephi", "Coming from Neo4j" migration pages
- [ ] A page on memory budgets and what the planner does, with a real `explain()` output
- [ ] Notebooks executed in CI so they cannot rot

**Acceptance**
- A new user reaches a first map in under five minutes following the quickstart, timed in three agent-driven sessions (`design/DECISIONS.md`, "Usability, accessibility and hardware checks"): each starts from a fresh environment and follows the quickstart as written, with the time counted as in {{#m5-06-viewer-ux}}; steps and errors recorded. Formative, not a claim (`docs/research/README.md` H4): 3 of 3 has an exact 95% lower bound of 0.29. A published rate needs a study with people, n ≥ 20, with its exact interval; such a study is advisable after 0.1, not a gate

**Depends on** {{#m6-02-wheels}}, {{#m6-04-in-memory-constructors}}, {{#m6-07-subgraph-bridge}}, {{#m7-02-shortest-paths}}, {{#m7-05-temporal}}, {{#m2-13-preflight-resume}}, {{#m3-03-semi-external}}
