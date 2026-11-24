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
- [ ] Pre-launch review by 3–5 outside practitioners; fix what confuses them
- [ ] Make the repository public only after {{#m0-09-name-trademark}} and {{#m0-10-patent-check}} are closed

**Acceptance**
- `pip install everygraph==0.1.0` and `cargo install eg-cli` work on all supported platforms on release day
- Every claim in the launch post links to a reproducible result
