---
title: Release v0.1.0
milestone: M8 · CLI, docs and v0.1 launch
labels: type:infra, area:launch, P0, risk:low, size:M
---
**Scope**
- [ ] Versioning policy: semver for the API; separate version numbers for the `.eg` and `.egmap` formats with a compatibility promise (files written by 0.1 stay readable)
- [ ] Release automation: crates.io, PyPI (wheels from {{#m6-02-wheels}}), npm (viewer), GitHub release with CLI binaries and checksums
- [ ] Changelog, upgrade notes, security policy contact
- [ ] README with a 30-second demo (animated map), quickstart, honest comparison table, links to benchmarks
- [ ] Launch material: blog post with the method behind each claim, short demo video, posts for Hacker News / relevant subreddits / network-science and graph-database communities
- [ ] Pre-launch review by 3–5 outside practitioners; fix what confuses them. The public names listed in `docs/api-sketch.md` § 2.10 are frozen only after this review (finding the reviewers needs the owner: messages from the owner's accounts)
- [x] Repository made public during development by the owner's decision of 2026-11-25 (`design/DECISIONS.md`, "When the repository becomes public"); the legal opinions ({{#m8-10-legal-opinions}}) remain advisable before the first announced release

**Acceptance**
- `pip install everygraph==0.1.0` and `cargo install everygraph-cli` work on all supported platforms on release day
- Every claim in the launch post links to a reproducible result

**Design reference (decided 2026-11-25, `design/DECISIONS.md`).** Approved (`choices.md` D1, D4, D7): the repository becomes public at 0.1 with benchmarks and one showcase map; Apache-2.0 or MIT at the user's choice; no telemetry, stated in the README. The visual identity to ship is `design/preview/style/`.
