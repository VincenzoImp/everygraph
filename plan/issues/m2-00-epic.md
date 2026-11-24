---
title: M2 epic: format and ingest
milestone: M2 · Format and ingest
labels: type:epic, area:format, P0
---
Tracking issue for milestone M2.

**Goal.** Any CSV or Parquet edge list with integer, hex or string identifiers becomes a verified `.eg` file within the user's memory budget, at about 100 M edges/s in RAM.

**Lab baseline (idle machine).** lj 0.42 s, orkut 0.89 s, Friendster 21 s with a 32 GB budget and 52 s with 8 GB (3.6 GB peak RSS); output byte-identical in every mode. Prototype limits: integer ids < 2^32, undirected, unweighted, plain text.

**Exit criteria**
- [ ] `.eg` container and adjacency codec specified, implemented and fuzzed ({{#m2-01-container-spec}}, {{#m2-03-adjacency-codec}})
- [ ] Portable I/O layer and memory accountant ({{#m2-02-io-layer}}, {{#m2-04-arrays-accountant}})
- [ ] Readers for text and Parquet ({{#m2-05-text-reader}}, {{#m2-06-parquet-reader}}); id mapping for all key types ({{#m2-07-id-mapping}})
- [ ] One build pipeline for in-RAM and out-of-core ({{#m2-08-csr-build}}) with full graph semantics ({{#m2-09-graph-semantics}})
- [ ] Properties, in-adjacency, artifact cache, verification ({{#m2-10-property-columns}}, {{#m2-11-in-adjacency}}, {{#m2-12-artifact-cache}}, {{#m2-14-verify}})
- [ ] Friendster text → `.eg` in < 60 s at an 8 GB budget and < 25 s at 32 GB on the lab machine; twitter-2010 directed; a hex-id input of Friendster size in < 4 min at 16 GiB
