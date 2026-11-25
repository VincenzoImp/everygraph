# Contributing to Everygraph

This file holds the coding rules of the Rust workspace. How work is planned, reviewed and
merged is in [CLAUDE.md](CLAUDE.md); why the engine is shaped this way is in
[docs/DESIGN.md](docs/DESIGN.md). Licence sign-off, the code of conduct and the security
policy arrive with issue #9.

## Checks

Every pull request passes these on current stable Rust. Until continuous integration
exists (issue #3), run them yourself and paste the output in the pull request.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p everygraph-core --no-default-features --all-targets -- -D warnings
cargo test --workspace                      # or: cargo nextest run --workspace
cargo test -p everygraph-core --no-default-features
cargo build -p everygraph-core --target wasm32-unknown-unknown --no-default-features
cargo check -p everygraph-core --target i686-unknown-linux-gnu --all-targets
cargo deny check
```

Lint levels live in `[workspace.lints]` in the root `Cargo.toml`, each group with its
reason, and every crate inherits them with `[lints] workspace = true`.

And these on nightly, because Miri is the only way here to *run* tests with a 32-bit
`usize`, and the required check for any module that contains `unsafe`:

```sh
cargo +nightly miri test -p everygraph-core --no-default-features
cargo +nightly miri test -p everygraph-core --no-default-features --target i686-unknown-linux-gnu
MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-ignore-leaks" cargo +nightly miri test -p everygraph-core
```

The third command runs the tests on rayon's thread pool. It needs the two flags because
of rayon's dependencies, not because of this code: `crossbeam-epoch` keeps an intrusive
list that Miri's default aliasing model (Stacked Borrows) rejects, and rayon's global
pool is still alive when the test process ends. The first two commands run our own code
under the default, stricter settings.

## Crates

| Crate | What lives there | Filled by | Must build for wasm32 |
|---|---|---|---|
| `everygraph-core` | id types, `ByteSource`, the parallel facade, the memory accountant interface; later segmentable arrays | this skeleton, #31 | yes, enforced |
| `everygraph-io` | native `ByteSource` backends (file, mmap, direct I/O) and sinks | #29 | no — the one crate for OS I/O |
| `everygraph-format` | the `.eg` container and the adjacency codec | #28, #30 | yes |
| `everygraph-ingest` | text and Parquet readers, id dictionary, build pipeline | #32–#35 | yes (browser ingest, D22) |
| `everygraph-kernels` | the edge-map framework and the kernels | #44, #47–#52 | yes |
| `everygraph-community` | Leiden and the map hierarchy | #57 | yes |
| `everygraph-map` | layout, tile format, tile builder | #67–#69 | yes |
| `everygraph-query` | point queries, shortest paths, views | #90, #91, #93 | yes |
| `everygraph-cli` | the `everygraph` command | #98 | no |
| `everygraph-py` | Python bindings | #81, #82 | no |
| `everygraph-wasm` | browser bindings | #108 | yes |
| `everygraph-bench` | the `everygraph-bench` harness | #4 | no |

A crate that has no content yet has no dependencies either. When a crate starts to use
the core it adds the dependency with `default-features = false`: a library does not
decide whether threads exist. The artifacts that ship with threads (the `everygraph`
command, the Python extension, the threaded browser build) enable
`everygraph-core/parallel`; Cargo's feature unification then turns it on for everything
they link. `parallel` is a default feature of the core, so that someone who depends on
`everygraph-core` alone gets threads without asking.

"Must build for wasm32" is enforced for `everygraph-core` today. The other rows state the
intent; each crate joins the wasm32 build, and gets a `clippy.toml` like the core's, in
the pull request that gives it content.

## The portability rule

`everygraph-core` does not open files, map memory, start threads, read the clock or the
environment, or call the operating system (DESIGN D2; the browser experiment in
FINDINGS § G). Code above it reaches bytes through `ByteSource` and threads through
`everygraph_core::par`. Three mechanisms enforce this, because none is enough alone:

1. **Clippy.** `crates/everygraph-core/clippy.toml` lists the `std` items that compile
   for `wasm32-unknown-unknown` and then fail or panic when called (`std::fs`,
   `std::thread`, `std::time::Instant::now`, `std::env`, `std::process`, sockets, the
   standard streams). Using one is an error from `disallowed_types` or
   `disallowed_methods`, which the workspace sets to `deny`. Clippy reads the
   `clippy.toml` nearest to the crate it checks, so the list does not apply to
   `everygraph-io`. Clippy can disallow items but not whole modules: a function added to
   `std::fs` by a future Rust release must be added to the list by hand.
2. **The wasm32 build** without default features. It rejects what does not exist on that
   target at all: everything under `std::os`, and any dependency that needs an OS.
3. **No `unsafe` and no stray dependencies.** With `unsafe` forbidden the crate cannot
   make a system call of its own, and `deny.toml` bans memory-mapping crates everywhere
   and allows rayon as a direct dependency of `everygraph-core` only.

## Integers

Node ids are `u32`, edge offsets are `u64`, and `usize` is only an index into memory
(DESIGN D3). Use `NodeId` and `EdgeOffset` and their conversions; widening is infallible
and narrowing returns an error. A bare `as` that can truncate or change sign is a clippy
error (`cast_possible_truncation`, `cast_possible_wrap`, `cast_sign_loss`): on wasm32 and
i686 it is a silent bug that tests on x86-64 never see. Sizes that describe data rather
than memory (file lengths, byte offsets, memory budgets) are `u64` on every target.

## `unsafe`

`unsafe` is allowed only in modules named in the table below. Today there are none, and
every crate root carries `#![forbid(unsafe_code)]`.

| Module | Why safe code is not enough | Miri test |
|---|---|---|
| *(none yet)* | | |

To add one, in a single pull request:

1. Remove `#![forbid(unsafe_code)]` from that crate's root. The workspace lint
   `unsafe_code = "deny"` still applies to the rest of the crate.
2. Put the `unsafe` code in one module with a safe public interface, and start that
   module with `#![allow(unsafe_code)]` and a module comment that says why safe code is
   not enough, with the measurement if the reason is speed.
3. Give every `unsafe` block a `// SAFETY:` comment that states the invariant it relies
   on, and one unsafe operation per block (`undocumented_unsafe_blocks` and
   `multiple_unsafe_ops_per_block` are `deny`).
4. Add tests that reach every `unsafe` block and pass under `cargo +nightly miri test`.
5. Add the module to the table above. The pull request gets the independent review
   that CLAUDE.md § 4 requires for `unsafe` code.

Where we expect to need it, from the lab prototypes: SIMD in the adjacency codec (#30),
disjoint parallel writes into shared arrays in the build pipeline and in Leiden (#35,
#57), memory maps in `everygraph-io` (#29), and the generated glue of the Python and
browser bindings (#81, #108). `everygraph-core` is expected to stay free of it.

## Rust version

The minimum supported Rust version is **1.85**, recorded as `rust-version` in the
workspace manifest: it is the first release with edition 2024 and the MSRV-aware
dependency resolver, and the toolchain Debian 13 ships. Everything builds on stable. The
one planned exception is the threaded browser build (#108), which needs a nightly
toolchain pinned by that build's own script, not by a `rust-toolchain.toml` at the root,
which would move the whole workspace to nightly.

Raising the minimum is done in its own pull request, which says what needs the newer
release. Lints are run with current stable only: an older clippy is not expected to pass.

## Builds

The release profile uses fat LTO and one codegen unit. Nothing in the repository sets
`target-cpu=native`: a shipped binary must run on the user's CPU, so code that wants
AVX2 or NEON detects it at run time. The lab prototypes under `lab/` are excluded from
the workspace and keep their own flags.

## Dependencies and licences

Everygraph is licensed `Apache-2.0 OR MIT` (DESIGN D23). Every source file starts with
`// SPDX-License-Identifier: Apache-2.0 OR MIT`. `deny.toml` lists the licences a
dependency may carry; adding one to the list is a licensing decision, not a build fix.
The core takes no C or C++ dependency (DESIGN D1).
