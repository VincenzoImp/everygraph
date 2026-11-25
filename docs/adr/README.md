# Architecture decision records

An ADR here records one design decision of `docs/DESIGN.md` § 4 (D1…D25): what was decided,
what was measured, how far the measurement supports it, and what would make us change it.
ADR `NNNN` ratifies decision `DN`; ADR 0000 records the assumptions the first lab
overturned, so they are not brought back by accident.

A decision changes only through a new or amended ADR, with `docs/DESIGN.md` updated in the
same pull request and the change noted in `docs/STATE.md` (`CLAUDE.md` § 5). Implementing
something different without one is drift.

## Status vocabulary

| Status | Meaning |
|---|---|
| `accepted` | The evidence supports the decision as written. Nothing is pending. |
| `accepted, partly validated` | We build on it, but part of it is unmeasured or was measured narrowly (one graph family, one kernel, a prototype, a shared machine). The ADR names the experiment or issue that closes the gap. |
| `open — needs experiment` | The evidence does not support the decision as `DESIGN.md` states it. The unsupported part is not built until the named experiment or issue reports. |
| `superseded by ADR NNNN` | Replaced; kept for the record. |

## Evidence rules

Every number names where it comes from: the `docs/FINDINGS.md` section and, where it exists,
the result file under `lab/*/results`. In the same sentence it says the graph, the thread
count, the numeric precision where it matters, and the machine state:

- **idle** — the idle-machine re-runs of 2026-11-24 (FINDINGS "Idle-machine re-runs"):
  `lab/b1-core/results/bench/`; `lab/b2-leiden/results/scaling/`, `…/quality/`,
  `…/hier/` and `…/refs/` with their whole-machine tables in
  `lab/b2-leiden/results/summary.txt`; and `lab/a-baselines/results/`;
- **shared** — every other lab number, taken while up to eight experiments ran at once
  (load average 10–45; ±30% typical). Sizes, hashes and modularity are not affected by
  load; timings are.

A number that could not be traced to a result file is written as "not traced". An
extrapolation is labelled as one every time.

## Template

```markdown
# ADR NNNN — <title>

- **Status:** accepted | accepted, partly validated | open — needs experiment
- **Decision:** DN in [DESIGN.md](../DESIGN.md#4-decisions)
- **Date:** YYYY-MM-DD

## Context
Why the question arises; what forces act on it.

## Decision
The decision as it stands, in one paragraph.

## Evidence
Each number with its source, graph, threads, precision and machine state. What was not
measured, said as plainly as what was.

## Consequences
What follows for the code, the plan and the claims.

## What would change our mind
A measurable condition.

## Closed by
(Required unless the status is `accepted`.) The experiment or issue, as #N, that closes it.
```

## Index

| ADR | Title | Status | Closed by |
|---|---|---|---|
| [0000](0000-overturned-assumptions.md) | Assumptions the first lab overturned | accepted | — |
| [0001](0001-rust-core.md) | Rust core, no C/C++ dependencies in the core | accepted | — |
| [0002](0002-portable-core.md) | A core portable by construction | accepted | — |
| [0003](0003-dense-u32-ids.md) | Dense u32 node ids by default | accepted | — |
| [0004](0004-compressed-adjacency-default.md) | Compressed adjacency is the default storage | accepted, partly validated | #118 |
| [0005](0005-webgraph-optional.md) | webgraph-rs is an optional feature | accepted | — |
| [0006](0006-in-adjacency.md) | Directed graphs store out- and in-adjacency | accepted | — |
| [0007](0007-eg-container.md) | One `.eg` file per graph; maps as PMTiles | accepted, partly validated | #28 |
| [0008](0008-id-dictionary.md) | ID dictionary: typed keys and minimal perfect hash | accepted, partly validated | #34, #25 |
| [0009](0009-property-columns.md) | Properties as Arrow-compatible columns | accepted, partly validated | #37 |
| [0010](0010-views-mask-or-materialize.md) | Views: mask or materialize | open — needs experiment | #93 |
| [0011](0011-execution-regimes.md) | Three execution regimes chosen by O(n) state | accepted, partly validated | #16 |
| [0012](0012-edge-map-framework.md) | One edge-map framework | accepted, partly validated | #44 |
| [0013](0013-threads-per-kernel.md) | Thread count chosen per kernel | accepted, partly validated | #22 |
| [0014](0014-deterministic-by-default.md) | Deterministic by default, across threads, platforms, budgets and regimes | accepted, partly validated | #18, #54, #60 |
| [0015](0015-leiden-visiting-order.md) | Leiden with block-permuted order and a connectivity guarantee | accepted, partly validated | #24, #57 |
| [0016](0016-two-community-outputs.md) | Two community outputs: partition and map hierarchy | accepted, partly validated | #19 |
| [0017](0017-layout-disc-packing.md) | Layout by hierarchy-constrained disc packing | accepted, partly validated | #21 |
| [0018](0018-data-tiles.md) | Data tiles in PMTiles v3 and a WebGL2 viewer | accepted, partly validated | #26, #15 |
| [0019](0019-python-pyo3-arrow.md) | Python through PyO3 abi3 and Arrow PyCapsule | accepted | — |
| [0020](0020-networkx-backend.md) | The NetworkX backend is an adoption channel | accepted | — |
| [0021](0021-queries-traversal-api.md) | Queries in v0.1: a traversal API, no query language | accepted, partly validated | #118, #91 |
| [0022](0022-browser-tier.md) | Browser tier after v0.1 | accepted, partly validated | #108 |
| [0023](0023-licence-and-open-specs.md) | Apache-2.0 OR MIT; published format specs | accepted | — |
| [0024](0024-visual-system.md) | One visual system, Vernier | accepted | — |
| [0025](0025-user-facing-choices.md) | User-facing choices live in `design/DECISIONS.md` | accepted | — |

`docs/tools/check_adrs.py` checks that this index, the ADR files and the decision table of
`docs/DESIGN.md` agree, that each ADR has the required sections and a known status, that
every cited issue exists in `plan/published.json` and every cited `lab/` path exists.
