# lab — prototypes from the feasibility study (2026-11-24)

Research code written in a few hours each to answer specific questions. It is the evidence
behind `docs/FINDINGS.md`, not the product: it contains absolute paths
(`/home/tux/graphlab`), lab-only build flags (`target-cpu=native`), lightly tested options
and `unsafe` shared-memory code that has not been through Miri or ThreadSanitizer.

| Directory | Question | Summary in FINDINGS |
|---|---|---|
| `a-baselines` | What do NetworKit, igraph, GVE and DuckDB do on this machine? | § A |
| `b1-core` | Ingest, kernels, larger-than-RAM execution | § B1 |
| `b2-leiden` | Parallel and deterministic Louvain/Leiden, hierarchy | § B2 |
| `c-compression` | Adjacency codecs, ordering, cold scans | § C |
| `e-map` | Layout, tile pyramid, PMTiles container, WebGL2 viewer | § E |
| `f1-query` | Id dictionary, point queries, properties, views | § F1 |
| `f2-python` | Query-engine comparison, PyO3 binding, NetworkX backend | § F2 |
| `g-wasm` | The same core in the browser | § G |

`LAB.md` is the brief every experiment worked from (machine, shared data, rules).
Datasets, build outputs and result files are not included; issue "Make the first lab
reproducible with one command" tracks turning these scripts into benchmark specs.

One experiment (`e-map`) contains its own `REPORT.md`; the others reported through the
study coordinator and are summarized in `docs/FINDINGS.md`.
