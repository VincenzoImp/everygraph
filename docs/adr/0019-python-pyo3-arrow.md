# ADR 0019 — Python through PyO3 abi3 and the Arrow PyCapsule interface

- **Status:** accepted
- **Decision:** D19 in [DESIGN.md](../DESIGN.md#4-decisions)
- **Date:** 2026-11-25

## Context

The first users are data scientists in Python (DESIGN § 2). Results must reach NumPy,
pandas, Polars and DuckDB without copies, one wheel should cover many Python versions, and
per-call overhead must not dominate.

## Decision

The Python package binds the core with PyO3 against the stable ABI (abi3) and exchanges
results through the Arrow PyCapsule interface. The API is bulk and columnar.

## Evidence

From the Python experiment (FINDINGS § F2; shared machine; lj):
- **Zero copy**: verified by buffer address for NumPy views into the mmap and for Arrow
  PyCapsule export into pyarrow, Polars and pandas; a 4 M-row result became a table in
  0.5–4.4 ms. The GIL is released during kernels.
- **Per-call cost**: an empty method call costs 28 ns (dev cores) and 40 ns (32 threads)
  (`lab/f2-python/results/goalB_lj_dev.json`, `lab/f2-python/results/goalB_lj_t32.json`).
  FINDINGS gave "28–62 ns" until on 2026-11-25; the 62 ns is **not traced** to a result
  file. A neighbour
  list as a zero-copy NumPy view costs 238 ns per call, against 171 ns per node in a
  batch call (`goalB_lj_t32.json`).
- **Packaging**: one 525 KB abi3 wheel for CPython 3.10–3.14 (FINDINGS § F2; the wheel
  size is **not traced** to a result file and is labelled an estimate since on 2026-11-25). Free-threaded CPython needs separate wheels.

## Consequences

- Every per-node operation has a bulk form; per-call methods exist for convenience only.
- #81 builds the API, #82 the wheels, #83 the Arrow interop.
- Free-threaded builds are an extra wheel per version when they are supported.

## What would change our mind

- The abi3 restriction blocks a feature needed for v0.1, or free-threaded CPython becomes
  the default interpreter so that per-version wheels are needed anyway.
- A supported consumer (pyarrow, Polars, pandas, DuckDB) cannot import the PyCapsule
  without a copy (#83's tests).
