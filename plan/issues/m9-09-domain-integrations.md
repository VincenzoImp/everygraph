---
title: Domain integrations: GraphSense, OpenAleph, research datasets
milestone: M9 · After v0.1
labels: type:feature, area:interop, P2, risk:medium, size:M
---
**Why.** Specialists already have tools for their domain objects; they lack a macro view at scale. Integrating gives real users on day one instead of competing with mature domain platforms.

**Scope**
- [ ] GraphSense (MIT-licensed cryptoasset analytics): import address clusters and TagPacks as node properties; link from a map region to their dashboard; talk to the maintainers about a "map view"
- [ ] OpenAleph (MIT fork of OCCRP Aleph): import FollowTheMoney entity graphs
- [ ] Research datasets used by the founder's field: Telegram forward/mention networks, Nostr follow graphs, Bluesky firehose snapshots — loaders and worked examples
- [ ] OpenAlex and Software Heritage loaders maintained as examples

**Acceptance**
- One external group uses Everygraph on its own data and reports back

**Depends on** {{#m8-08-release}}
