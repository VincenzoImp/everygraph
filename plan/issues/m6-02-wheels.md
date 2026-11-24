---
title: Wheels and packaging
milestone: M6 · Python and interop
labels: type:infra, area:python, P0, risk:low, size:M, lab-evidence
---
**Why.** Installation friction decides adoption: graph-tool still has no PyPI wheels and pays for it; Polars, DuckDB and Ruff won partly on `pip install` just working.

**Scope**
- [ ] maturin + cibuildwheel: manylinux2014 and musllinux (x86_64, aarch64), macOS (arm64, x86_64), Windows (x86_64)
- [ ] One abi3 wheel per platform for CPython ≥ 3.10; separate free-threaded wheels (3.13t/3.14t are not covered by abi3)
- [ ] Runtime CPU-feature dispatch (AVX2/AVX-512/NEON) instead of `target-cpu=native`
- [ ] Wheel size budget ≤ 5 MB; no bundled system libraries beyond what the kernels need
- [ ] sdist that builds with a stock Rust toolchain; conda-forge recipe (after v0.1)
- [ ] Release automation with trusted publishing to PyPI; the viewer assets ship inside the wheel for the widget and `eg open`

**Acceptance**
- `pip install everygraph` in clean containers for every target, followed by a smoke test (ingest + PageRank + map export)
- `uv run --with everygraph` works as a plain script (verified in the lab for 3.10, 3.12, 3.14)
