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

**Design reference (decided 2026-11-25, `design/DECISIONS.md`).** The spelling is settled: "Everygraph" in text, `everygraph` lowercase in the wordmark and in everything typed, `eg` only as the Python alias, `.eg` / `.egmap` extensions (`design/preview/style/SPEC.md`, "The name"). What remains here is the registry research and reserving the names.

**Merged into {{#m8-10-legal-opinions}} (2026-11-26).** The research and the package names are done; the three open boxes below are that issue's now, and this one is closed.

**Update 2026-11-25.** Research done (`docs/legal/name-check.md`): a US application for EVERYGRAPH covering graph database analysis software was filed on 2026-11-17; TMview shows no other record under the name. The owner keeps the name. What remains in this issue:
- [ ] Reserve with the owner (accounts and payment are the owner's): domains `everygraph.dev` and `everygraph.org` (both appeared unregistered; `.com` is parked by someone else, `.io` and `.ai` unknown); a GitHub organization (`everygraph` is taken — candidates `everygraph-dev`, `everygraph-project`); the npm scope `@everygraph`
- [x] Reserve package names on PyPI, crates.io and npm: done 2026-11-26 by the owner, as 0.0.0 placeholders with no code (crates `everygraph` and `everygraph-cli`, PyPI `everygraph`, npm `everygraph`), checked on the registries (`docs/STATE.md`). The first real pre-release replaces them
- [ ] Re-check the status of US serial 50223142 at every milestone and record it in `docs/legal/name-check.md`
- [ ] Hand the name question to the professional in {{#m8-10-legal-opinions}}

