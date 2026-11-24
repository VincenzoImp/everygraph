---
title: MCP server: let AI agents question a graph
milestone: M9 · After v0.1
labels: type:feature, area:interop, P1, risk:medium, size:M
---
**Why.** Agents are becoming a user class, and "ask a billion-edge graph a question in natural language, get exact numbers and a link to the place on the map" is a capability no local tool offers. Budgets and truncation flags in the query API make it safe to expose.

**Scope**
- [ ] MCP server (stdio and loopback HTTP) exposing: graph summary, schema of properties, search, neighbours, k-hop with budgets, shortest path with constraints, community of a node, community table, top-k by a metric, view definition, "open this on the map" links
- [ ] Every tool has hard limits on time, rows and bytes returned; results are tables with a "truncated" flag
- [ ] Read-only by default; no file-system access beyond the opened graph
- [ ] Evaluation set of 30 investigation questions on the blockchain showcase with expected answers
- [ ] Bind to loopback only; document the data-exposure model

**Acceptance**
- An agent answers ≥ 80% of the evaluation questions with correct numbers
- No tool call can exceed its declared budget

**Depends on** M7
