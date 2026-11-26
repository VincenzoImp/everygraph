---
title: A stored result keeps the facts of its run (iterations, converged)
milestone: M8 · CLI, docs and v0.1 launch
labels: type:feature, area:cli, P1, risk:low, size:S
---
**Why.** A result served from the graph file (`cached: true`) has no `iterations` and no `converged` in its `meta`, while `docs/api-sketch.md` § 5.3 shows both with `cached: true`. The record of a result (`docs/spec/eg-container.md` § 6) already holds free key/value parameters, so this needs no change of the format: only a place for facts that are not part of the key.

**Scope**
- [ ] `Writer::add_result` accepts facts of the run beside the key, under a prefix that no key parameter can take (for example `run.iterations`, `run.converged`), and `ResultKey` matching ignores them; the spec's § 6 says which names are facts and that they never enter the key
- [ ] `everygraph_kernels::run::store` writes `iterations` and `converged` for `pagerank`; `stored` reads them back into the output
- [ ] `everygraph run pagerank` with `cached: true` has the same `meta.iterations` and `meta.converged` as the run that computed it; the Python package (#81) reads them the same way
- [ ] `everygraph info` shows them with the result (`pagerank  38 iterations · …`)

**Acceptance**
- A test computes `pagerank`, runs it again from the file, and finds equal `meta` but for `cached`
- A record with a fact under the prefix is found by the same key as one without it; a key parameter under the prefix is refused

**Depends on** {{#m8-01-cli}}
