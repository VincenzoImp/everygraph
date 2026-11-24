---
title: Tutorials and notebooks for the ten user stories
milestone: M6 · Python and interop
labels: type:docs, area:docs, P1, risk:low, size:M
---
**Why.** DuckDB and Polars grew through documentation that starts from tasks. The old prototype's documentation read defensively; this one should read like a tool that wants to be used, while keeping the honesty about limits.

**Scope**
- [ ] One executable notebook per user story from {{#m0-11-api-sketch}}, each on a public dataset small enough for a laptop, with a "scale it up" section
- [ ] "Coming from NetworkX", "Coming from Gephi", "Coming from Neo4j" migration pages
- [ ] A page on memory budgets and what the planner does, with a real `explain()` output
- [ ] Notebooks executed in CI so they cannot rot

**Acceptance**
- A new user reaches a first map in under five minutes following the quickstart (timed with three people)
