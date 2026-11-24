---
title: Cargo workspace, crate layout and coding rules
milestone: M0 · Foundations and decisions
labels: type:infra, area:foundations, P0, risk:low, size:M
---
**Why.** The crate boundaries encode the portability rules that keep the browser tier and the out-of-core modes possible (DESIGN D1, D2). Getting them wrong later means rewriting kernels.

**Scope**
- [ ] Workspace with `eg-core`, `eg-io`, `eg-format`, `eg-ingest`, `eg-kernels`, `eg-community`, `eg-map`, `eg-query`, `eg-cli`, `eg-py`, `eg-wasm`, `eg-bench`
- [ ] `eg-core` is `#![forbid]`-clean of `std::fs`, `std::thread` and mmap; it holds `NodeId`, `EdgeOffset`, the `ByteSource` trait, the parallel facade (rayon optional feature), the memory accountant interface and segmentable arrays
- [ ] MSRV policy (stable Rust for everything except the threaded wasm build, which needs a pinned nightly)
- [ ] `unsafe` policy: allowed only in named modules, each with a safety comment and a Miri test
- [ ] Release profile (`lto = "fat"`, `codegen-units = 1`); **no** `target-cpu=native` in shipped builds — runtime CPU-feature dispatch instead (the lab used `native`)
- [ ] Lints: clippy pedantic subset, rustfmt, `cargo deny` for licences

**Acceptance**
- `cargo build --workspace` and `cargo test --workspace` pass on a clean checkout
- `cargo build -p eg-core --target wasm32-unknown-unknown` passes with default features off
- A deliberately added `std::fs` call in `eg-core` fails CI

**Pointers.** Lab constraints list in FINDINGS § G; lab crates `lab/b1-core`, `lab/g-wasm/core`.
