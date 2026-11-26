---
title: Documentation site and published specifications
milestone: M8 · CLI, docs and v0.1 launch
labels: type:docs, area:docs, P0, risk:low, size:M
---
**Scope**
- [ ] Site with: quickstart (first map in five minutes), task guides, concepts (memory budget and regimes, determinism, how to read a map), API reference (Python, Rust, CLI), format specifications (`.eg`, adjacency codec, `.egmap`, graph model)
- [ ] "How it compares" page with fair, sourced comparisons and an explicit "use X instead when…" section for NetworkX, igraph, NetworKit, Gephi, Neo4j, DuckDB/DuckPGQ, LadybugDB, cosmos.gl, Graphistry
- [ ] Limits page: what is not supported, known weak cases (dense-graph edges on the map, out-of-core path queries, browser tier ceiling)
- [ ] Versioned docs; search; examples executed in CI
- [ ] Styles, fonts, icons, the mark and the favicon come from `@everygraph/style` (`design/system/`); no literal colour in the site's stylesheets (`design/system/tools/lint_colours.py` passes)
- [ ] Tone: direct and concrete; every performance statement links to its benchmark
- [ ] An LLM-in-the-loop check of the docs as a regression test (added 2026-11-25, `docs/research/README.md` H4): an agent given only the docs writes code for each of the ten user stories, the code is run, and wrong calls are counted per release. Never published as evidence of usability: LLM scores agreed with human reviewers only at Spearman 0.75 / 0.71 over 7 platforms, with the code never executed (Meng et al., SIGMOD'25)

**Acceptance**
- A reader can find how to do each of the ten user stories within two clicks
- Format specs are sufficient to write an independent reader (validated by the Python readers from {{#m2-01-container-spec}} and {{#m5-02-tile-format}})

**Depends on** {{#m8-01-cli}}, {{#m5-14-design-system}}

**Design reference (decided 2026-11-25, `design/DECISIONS.md`).** The landing page, documentation page, API reference entry, search, 404, changelog and README header are designed in `design/preview/style/web.html`: headline "Every graph, every size, every machine." in ink, no performance numbers until the released software is measured, and a "When to use something else" section that names the other tools.
