---
title: Showcase 3: a blockchain transaction graph with public labels
milestone: M8 · CLI, docs and v0.1 launch
labels: type:feature, area:launch, P1, risk:medium, size:L
---
**Why.** Follow-the-money analysis is the founder's domain and a field where commercial tools cost tens of thousands per year and keep data on their servers. It exercises everything the old prototype could not handle: directed multigraph, weights (amounts), timestamps, 42-byte address ids.

**Scope**
- [ ] Ingest a public Ethereum transaction dataset (a bounded period to start) with address ids, value and block time
- [ ] Import public address labels (e.g. GraphSense TagPacks, which are open) as node properties; show them on the map
- [ ] Investigation walkthrough in live mode: find an address, its community, strongest counterparties, paths to labelled exchanges with hub avoidance, activity over time
- [ ] Position relative to GraphSense (MIT-licensed, strong on address-level investigation, heavy infrastructure): complementary macro view; evaluate reading their clusters ({{#m9-09-domain-integrations}})
- [ ] Privacy and ethics note: public-chain data only; no attribution claims beyond imported public labels

**Acceptance**
- A reproducible notebook plus a recorded walkthrough; all stages within a 16 GiB budget

**Depends on** {{#m5-13-attribute-overlays}}, {{#m5-10-jupyter-widget}}
