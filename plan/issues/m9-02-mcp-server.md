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
- [ ] Added 2026-11-25 (`docs/research/README.md` H8; `docs/research/sota-human-factors.md` challenge 8): node labels and properties are returned marked as untrusted data, since retrieved text is an injection channel and labels are attacker-controlled (Greshake et al., AISec'23); the reveal level of a shared map (`full`/`summary`, {{#m5-15-summary-export}}) applies to agent sessions too
- [ ] A "region card" tool: exact counts, top nodes by a metric, strongest links and children of a region, deterministic and citable by its stable region id ({{#m4-06-hierarchy-artifact}}) — a GraphRAG-style community summary without an LLM; and a top-down "drill" tool over the capped hierarchy (CheapRAG's relevance filtering on our tree). LLMs reason poorly over graphs given as text (accuracy 4.8% to 61.8% by encoding on 5–20-node graphs, Fatemi et al., ICLR'24), so tools return exact numbers
- [ ] The evaluation set gains two-level drill-down questions and one question over an injected label

**Acceptance**
- An agent answers ≥ 80% of the evaluation questions with correct numbers
- No tool call can exceed its declared budget

**Depends on** M7
