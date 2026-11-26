---
title: Command line on directed graphs through the in-adjacency
milestone: M8 · CLI, docs and v0.1 launch
labels: type:feature, area:cli, P1, risk:low, size:S
---
**Why.** A directed graph file now holds its in-adjacency ({{#m2-11-in-adjacency}}), but `everygraph run` still refuses what needs it: `pagerank` and `kcore` on a directed graph, and `bfs --direction in|both`, are `Unsupported` with the reason "no graph file holds yet". `everygraph_kernels::run::run` takes the out-lists only. The Python API already answers these (`crates/everygraph-py/src/kernels.rs`), so the two front ends disagree.

**Scope**
- [ ] `everygraph_kernels::run::run` takes the in-lists of a directed graph beside the out-lists (one brand, `Plan::with_pair`), and the undirected projection when an operation needs it
- [ ] `pagerank` pulls along the in-lists, with out-strengths from the out-lists, each weighted by its own `.events` multiplicities when the graph keeps events
- [ ] `bfs`: bottom-up steps along the in-lists for `--direction out`, the in-lists followed for `in`, the undirected projection (`everygraph_format::adjacency::merge`) for `both`
- [ ] `kcore` on the undirected projection; `degrees` of a directed graph gives `in_degree`, `out_degree` and `degree` (`docs/api-sketch.md` § 3.2), `degree` by `everygraph_format::adjacency::union_degrees`
- [ ] `run components --strong`: `everygraph_kernels::scc::scc` on the out- and in-lists, numbered by `components::number` as the Python API does (`crates/everygraph-py/src/kernels.rs`); `Unsupported` on an undirected graph
- [ ] The CLI opens the in-adjacency through `everygraph_ingest::graph::Graph::in_index`, `in_data` and `in_events`; `Unsupported` is left only for a directed file built without its in-adjacency, with a reason that says so
- [ ] Stored results: the keys of these operations on a directed graph do not change when the in-adjacency is added (the graph identity leaves it out, `docs/spec/eg-container.md` § 6)

**Acceptance**
- `crates/everygraph-cli/tests/commands.rs`: on a directed graph, `run pagerank`, `run kcore`, `run degrees` and `run bfs --direction in|both` give the values the Python API gives on the same file (the Python tests compare those with NetworkX)
- A directed file built without its in-adjacency is still refused, with exit status 2

**Depends on** {{#m2-11-in-adjacency}}, {{#m8-01-cli}}
