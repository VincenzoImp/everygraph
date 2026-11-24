---
title: Name check and namespace reservation
milestone: M0 · Foundations and decisions
labels: type:decision, area:foundations, P0, risk:medium, size:S
---
**Why.** "Everygraph" was free on PyPI, crates.io and npm on 2026-11-24, but trademark registries were not searched, and the names are not reserved.

**Scope**
- [ ] Search EUIPO, USPTO and WIPO Global Brand Database for "Everygraph" and close variants in classes 9 and 42
- [ ] Web and GitHub search for existing software or companies using the name
- [ ] Reserve `everygraph` on PyPI, crates.io and npm (placeholder 0.0.0 packages), the GitHub organization if available, and a domain
- [ ] Decide short names. Known conflict: **`eg` is already taken on PyPI ("Examples at the command line", which installs an `eg` command), crates.io and npm.** The plan therefore uses `everygraph` as the command and `everygraph-*` as crate names; choose a free short alias if one is wanted. The Python alias `import everygraph as eg` is local to user code and unaffected. Also confirm the file extensions `.eg` / `.egmap`

**Acceptance**
- Written result of the searches; names reserved or an alternative chosen before any public announcement
