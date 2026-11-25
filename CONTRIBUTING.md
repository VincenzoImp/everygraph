# Contributing to Everygraph

This file holds the coding rules of the Rust workspace. How work is planned, reviewed and
merged is in [CLAUDE.md](CLAUDE.md); why the engine is shaped this way is in
[docs/DESIGN.md](docs/DESIGN.md). Licence sign-off, the code of conduct and the security
policy arrive with issue #9.

## Checks

Every pull request passes these on current stable Rust. Continuous integration runs
all of them (see "CI" below); run them yourself before pushing, because a red run costs
Actions minutes and a round trip.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p everygraph-core --no-default-features --all-targets -- -D warnings
cargo clippy -p everygraph-core --target i686-unknown-linux-gnu --all-targets -- -D warnings
cargo clippy -p everygraph-core --target i686-unknown-linux-gnu --no-default-features --all-targets -- -D warnings
python3 tools/check_core_portability.py
cargo test --workspace                      # or: cargo nextest run --workspace
cargo test -p everygraph-core --no-default-features
cargo build -p everygraph-core --target wasm32-unknown-unknown --no-default-features
cargo build -p everygraph-ingest --target wasm32-unknown-unknown
cargo build -p everygraph-wasm --target wasm32-unknown-unknown
cargo clippy -p everygraph-format --target i686-unknown-linux-gnu --all-targets -- -D warnings
cargo clippy -p everygraph-format --target wasm32-unknown-unknown -- -D warnings
RUSTFLAGS="-C target-feature=+simd128" CARGO_TARGET_DIR=target/wasm-simd128 \
  cargo clippy -p everygraph-format --target wasm32-unknown-unknown -- -D warnings
cargo deny check                            # offline: cargo deny check bans licenses sources
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo build --workspace --release
```

And the checks of the checks, each one command (what they do: "CI" below):

```sh
python3 tools/check_portability_canary.py   # the portability checks still catch each probe
python3 tools/check_wasm_toy.py             # toy kernel: native and wasm32 in Node agree
python3 tools/check_wasm_codec.py            # adjacency codec: simd128 in Node agrees with scalar
python3 tools/check_container_python.py     # .eg container: the Python reader reads what Rust wrote
python3 tools/check_tsan.py                 # ThreadSanitizer; needs the pinned nightly
```

The four clippy runs are not redundant: the core is linted once for each configuration
its code can be compiled under (with and without `parallel`, 64-bit and 32-bit). The
i686 runs also do what `cargo check --target i686-unknown-linux-gnu` would.

On a Linux x86_64 machine with the 32-bit C runtime (`gcc-multilib` on Debian and
Ubuntu), the core's tests also run natively with a 32-bit `usize`, with and without
rayon. CI does this; without `gcc-multilib` the link fails (`cannot open crtn.o`), and
the Miri run on i686 below is the local substitute:

```sh
cargo test -p everygraph-core --target i686-unknown-linux-gnu
cargo test -p everygraph-core --target i686-unknown-linux-gnu --no-default-features
```

Lint levels live in `[workspace.lints]` in the root `Cargo.toml`, each group with its
reason, and every crate inherits them with `[lints] workspace = true`.

On the minimum supported toolchain (see "Rust version"):

```sh
cargo +1.87 test --workspace --locked
```

And these on nightly, because Miri is the way to *run* tests with a 32-bit `usize` on a
machine without the 32-bit C runtime, and the required check for any module that
contains `unsafe`:

```sh
cargo +nightly miri test -p everygraph-core --no-default-features
cargo +nightly miri test -p everygraph-core --no-default-features --target i686-unknown-linux-gnu
MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-ignore-leaks" cargo +nightly miri test -p everygraph-core
cargo +nightly miri test -p everygraph-format
RUSTFLAGS="-C target-feature=+ssse3" cargo +nightly miri test -p everygraph-format --test adjacency
cargo +nightly miri test -p everygraph-io --test allocation_failure
```

The last two run the adjacency codec's tests on the scalar decoder, then on the SSSE3 one
(compiled in, so that run-time detection finds it under Miri). Its tests run a sample under
Miri (`cfg!(miri)`): the full corpus takes about 15 minutes there.

The third command runs the tests on rayon's thread pool. It needs the two flags because
of rayon's dependencies, not because of this code: `crossbeam-epoch` keeps an intrusive
list that Miri's default aliasing model (Stacked Borrows) rejects, and rayon's global
pool is still alive when the test process ends. The first two commands run our own code
under the default, stricter settings.

CI runs them on the nightly named in `tools/nightly-toolchain` (with the `miri` and
`rust-src` components), not on whatever `nightly` is today, so that a Miri regression
upstream does not turn every pull request red. To run exactly what CI runs, replace
`+nightly` by `+$(cat tools/nightly-toolchain)`. Moving the pin is a pull request of its
own.

## CI

GitHub Actions, two workflows under `.github/workflows/`, on GitHub-hosted runners.
While the repository is private, Actions minutes are counted against the account's
monthly quota, each job rounded up to a whole minute (macOS ten-fold, Windows two-fold);
once it is public, standard runners are free. Either way the queue is shared by every
agent's pull request, so on a pull request the expensive jobs run only when it changed
what they check.

| Workflow | When | Jobs (each step is a command from "Checks") |
|---|---|---|
| `ci.yml` | every pull request and every push to `main`, unless only documentation or results changed (`*.md`, `docs/`, `plan/`, `design/`, `lab/`, `bench/results/`); nightly at 03:41 UTC; by hand | **lint** (always): which jobs the run needs (below), fmt, the clippy runs (the codec's on i686 included), `check_core_portability.py`, `cargo deny check`, docs. **test** (always): nextest, doc tests, the core without default features, the three wasm32 builds (core, ingest, everygraph-wasm) and the codec's wasm32 clippy runs (with and without `simd128`), the wasm toy, the wasm codec check (`simd128` in Node against scalar), the container check (the independent Python reader against the Rust writer), the portability canary; plus the release build when a manifest changed, the core's and the codec's tests natively on i686 when the core or the codec did, the oracle crate's own tests when the oracle job runs. **oracle**: `oracle/run-ci.sh` (the correctness oracle: Python references through `uv`, toy kernel, seeded bugs, golden files; `oracle/README.md`). **msrv**: `cargo +1.87 test`. **sanitizers**: the Miri commands (the codec's two included), then `check_tsan.py`. All on Linux x86_64 |
| `other-platforms.yml` | pull requests that touch `everygraph-core`, `-io`, `-format` or `-ingest`; Mondays 04:17 UTC; by hand (`gh workflow run other-platforms.yml --ref <branch>`, optionally `-f platform=macos\|windows\|linux-arm -f packages="-p <crate>"`) | clippy, tests and the wasm32 build of the core on macOS arm64, Windows x86_64 and Linux aarch64; `cargo deny check advisories` on `main` (weekly) |

Which jobs of `ci.yml` a pull request runs is decided by `.github/scripts/ci_changes.py`
in the lint job, from `git diff` of GitHub's merge commit against its base (run it
locally with `--base main` to see what your branch would run):

| Job | Runs on a pull request when it changes |
|---|---|
| lint, test | always |
| sanitizers (Miri, ThreadSanitizer), and the i686 tests in test | `crates/everygraph-core/`, a module of the `unsafe` table below, `tools/nightly-toolchain`, `tools/check_tsan.py`, `tools/canary/tsan/` |
| oracle, and the oracle crate's tests in test | `crates/everygraph-oracle/`, `oracle/`, the kernel crates (`everygraph-kernels`, `-community`, `-map`), `datasets/`, `crates/everygraph-bench/src/datasets/` |
| msrv, and the release build in test | any `Cargo.toml` (`rust-version`, dependencies, profiles), `Cargo.lock`, a `rust-toolchain` file |
| everything | anything under `.github/`, or the pull request has the label **`full-ci`** |

Everything also runs on every push to `main`, and nightly unless `main`'s last commit
already passed a full run (then only lint, which hears of new advisories and of a new
stable toolchain). So nothing merged goes unchecked by any job for more than a day. What
a pull request may therefore leave to `main`: a newer *language* feature below the MSRV
in a change that touched no manifest (newer *std* APIs are caught on every run by
clippy's `incompatible_msrv`), a dependency bump in `Cargo.lock` that changes the core's
behaviour under Miri, and anything a kernel change breaks in the oracle's Python suite
through a crate not listed above. The coordinator adds `full-ci` to every Tier 2 pull
request (CLAUDE.md § 4) before merging it. The test job runs even when lint fails, so
that a red pull request still gets its test results; the conditional jobs wait for lint
to pass.

A new push to a pull request cancels the run it replaces. Caches (`Swatinem/rust-cache`)
(and `astral-sh/setup-uv`'s cache of the oracle's Python packages) are written by runs on `main` and read by every branch. Third-party actions are pinned
to a commit, with the version in a comment; workflows get `contents: read` and nothing
else (the lint job also reads Actions runs, for the nightly check).

**Push when the branch is ready for CI, not after every commit.** Every push to an open
pull request starts a run, which costs minutes while the repository is private and
takes a place in the queue every agent shares, even when the next push cancels it. Run
the checks of the crates you touched locally ("Checks"), commit as often as you like,
and push once the branch is ready to be checked — or push to a branch without a pull
request, which starts nothing.

What the scripts at the end of "Checks" are for (`tools/check_wasm_codec.py` is the wasm toy's pattern applied to the codec; `tools/check_container_python.py` has `everygraph-io`'s example `container_sample` write a container and requires `tools/eg_container.py`, a reader in Python written from `docs/spec/eg-container.md` alone, to list the same sections, verify their hashes, report a damaged one and fall back past a damaged table of contents):

- **`tools/check_portability_canary.py`** breaks the portability rule on purpose. Each
  file under `tools/canary/portability/` is a probe (direct `std::fs`, file access
  through `Path`, clock reads, a DNS lookup, a thread, an inner `allow`, a
  `cfg(not(clippy))` block, a platform `cfg`, `std::os`, an I/O dependency); the
  script adds it to a scratch copy of the core under `target/portability-canary/`,
  runs the check the probe names, and fails unless that check fails with the message
  the probe expects. A check that has quietly stopped checking — `clippy.toml` moved,
  a list entry deleted, an allow-list widened — is caught this way. A new way around the
  rule gets a probe in the pull request that closes it. The probes under
  `tools/canary/allocation/` do the same for the allocation rule (below) in each kernel
  crate.
- **`tools/check_wasm_toy.py`** runs a toy kernel (degrees of a generated graph,
  through `ByteSource`, `NodeId` and `par`; `tools/wasm-toy/`) natively with rayon and
  as wasm32 in Node, and compares the hashes. Real kernels replace it with the
  determinism CI (m3-11).
- **`tools/check_tsan.py`** runs the core's tests with default features (rayon) under
  ThreadSanitizer on the pinned nightly (`-Zsanitizer=thread -Zbuild-std`), and then
  `tools/canary/tsan/`, a toy kernel on `par` with a deliberate data race, which must be
  reported. Parallel kernels join it by having tests in their crate; the script then
  needs their package added next to the core's.

Not in CI yet: the threaded wasm32 build (issue #108; `ci.yml` holds a placeholder
describing it).

## Crates

| Crate | What lives there | Filled by | Must build for wasm32 |
|---|---|---|---|
| `everygraph-core` | id types, `ByteSource` and its writable counterpart `ByteSink`, the parallel facade, the memory accountant (`Budget`, `Memory`, `Reservation`), segmented arrays (`SegVec`), per-thread scratch (`ScratchPool`), the `MemoryProbe` interface | this skeleton, #31 | yes, enforced |
| `everygraph-io` | native `ByteSource` backends (file, mmap, direct I/O) and sinks (`FileSink` is the native `ByteSink`); the Linux `MemoryProbe` (`/proc/meminfo` and cgroup limits) | #29, #31, #28 | no — the one crate for OS I/O |
| `everygraph-format` | the `.eg` container and the adjacency codec | #28, #30 | yes, enforced (with and without `simd128`) |
| `everygraph-ingest` | text and Parquet readers, id dictionary, build pipeline | #32–#35 | yes (browser ingest, D22) |
| `everygraph-kernels` | the edge-map framework and the kernels | #44, #47–#52 | yes |
| `everygraph-community` | Leiden and the map hierarchy | #57 | yes |
| `everygraph-map` | layout, tile format, tile builder | #67–#69 | yes |
| `everygraph-query` | point queries, shortest paths, views | #90, #91, #93 | yes |
| `everygraph-cli` | the `everygraph` command | #98 | no |
| `everygraph-py` | Python bindings | #81, #82 | no |
| `everygraph-wasm` | browser bindings | #108 | yes |
| `everygraph-bench` | the `everygraph-bench` harness; the dataset registry and fetcher (`datasets/`) | #4, #5 | no |
| `everygraph-oracle` | the correctness oracle: generators, references, comparators, invariants, the `KernelUnderTest` trait (a development crate, `publish = false`, used by other crates' tests only; see `oracle/README.md`) | #6 | no |

The oracle's reference implementations are Python (NetworkX, python-igraph, scipy) in
`oracle/`, a `uv` project with a lock file. Python lives there and nowhere else: no engine
crate may depend on it. `oracle/run-ci.sh` is the oracle's part of the checks below.

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
`everygraph_core::par`.

No single tool can check that rule, so it is checked in pieces. Each piece covers what
is written next to it and nothing more:

| What is checked | By | What it does not cover |
|---|---|---|
| Named items of `std` that reach the file system (`std::fs`, the querying methods of `Path`), threads, the clock (`now` and `elapsed`), the environment, the process, the network (sockets, name resolution) and the standard streams (including `println!` and friends) | `crates/everygraph-core/clippy.toml`, through `disallowed_types`, `disallowed_methods` and `disallowed_macros` | An item that is not on the list. Clippy cannot disallow a module, so the list is kept by hand |
| That those lints cannot be switched off | `#![forbid(…)]` at the crate root: an inner `#[allow]` or `#[expect]`, also of a group such as `clippy::all`, is error E0453 under clippy | Removing the `forbid` line — next row |
| That the `forbid` line is present in every crate root of the core, that every entry of `clippy.toml` names something (clippy only warns about an entry it cannot resolve, and `-D warnings` does not make that an error), that code is compiled only under conditions the lint runs cover (`test`, `feature = "parallel"`, `target_pointer_width`), and that every dependency, direct or transitive, normal or build, is on an allow-list | `tools/check_core_portability.py` | Code pulled in from outside the crate directory with `include!` or `#[path]` |
| Anything under `std::os`, and dependencies that do not compile for the target | `cargo build -p everygraph-core --target wasm32-unknown-unknown --no-default-features` | Everything that compiles for wasm32 and fails when called: all of the rows above |
| System calls and foreign functions declared in the crate itself | `#![forbid(unsafe_code)]` (an `unsafe extern` block is rejected too) | — |

Two things follow from the table.

**Adding a dependency to `everygraph-core` is a portability decision.** A crate that
walks directories or starts a runtime builds for wasm32 and passes clippy and
`cargo deny`; only the allow-list in `tools/check_core_portability.py` stops it. To add
one: read what it does on a target without an OS, add it and its own dependencies to
`ALLOWED_DEPENDENCIES` in the same pull request, and say why in the description. The
same list is what keeps C and C++ out of the core (DESIGN D1): everything on it is pure
Rust. `deny.toml` adds two workspace-wide rules: rayon may be a direct dependency of
`everygraph-core` only, and memory maps come from memmap2 and only into `everygraph-io`.

**Adding a `cfg` condition to `everygraph-core` needs a lint run that compiles it.**
`#[cfg(not(clippy))]` hides code from every lint, and `#[cfg(windows)]` hides it from a
lint run on Linux. A new condition goes into `ALLOWED_CFG` together with a clippy
command, in "Checks" above, that selects it.

Not checked at all: **blocking**. `Mutex::lock`, `Condvar::wait`, `Barrier::wait` and a
channel's `recv` compile everywhere and can block a browser's main thread, which is not
allowed to wait. Code in the core must not block; today nothing verifies it.

The rule is enforced for `everygraph-core` only. Clippy reads the `clippy.toml` nearest
to the crate it checks, so the lists do not apply to `everygraph-io` or to any other
crate (see "Crates").

## The allocation rule

Every large buffer is reserved against the user's memory budget before it is allocated
(DESIGN D11; issue #31). Code that allocates takes a `Memory` handle and holds memory
through the types of `everygraph-core` that own both the buffer and its reservation:
`SegVec` for arrays (growth reserves first; segments of at most 1 GiB, for wasm32),
`ScratchPool` for per-thread scratch (one buffer per chunk that can run at once, the
plan's thread count), and a `Reservation` kept next to anything else (a hash table) —
sized by the caller, so it is the last resort. A refusal is a typed error
(`MemoryError::Budget` carrying `InsufficientBudget { needed, available }`), never an
abort. A plan adds up the sizes first (`SegVec::<T>::bytes_for`, in `u64`) so that a
kernel is refused before it allocates anything.

In the kernel crates (`everygraph-kernels`, `everygraph-community`, `everygraph-map`,
`everygraph-query`) a `clippy.toml` disallows the ways of sizing a buffer directly —
`Vec::with_capacity`, `reserve`, `resize`, `vec![x; n]`, `HashMap::with_capacity` and
the like — and each crate root forbids the lint, so it cannot be allowed locally. The
four lists are identical; `tools/check_portability_canary.py` checks that each still
catches every entry. What no lint sees: growth through `push`, `extend`, `collect` and
`clone`. Small vectors use those; a large one is a review finding, and what catches it
is a counting global allocator in the kernel tests that asserts heap bytes ≤
`Budget::used()` plus a fixed slack (#53). `everygraph-ingest` and `everygraph-format` join the
rule once their first pull requests (#30, #32–#35) are in.

## Integers

Node ids are `u32`, edge offsets are `u64`, and `usize` is only an index into memory
(DESIGN D3). Use `NodeId` and `EdgeOffset` and their conversions; widening is infallible
and narrowing returns an error. A bare `as` that can truncate or change sign is a clippy
error (`cast_possible_truncation`, `cast_possible_wrap`, `cast_sign_loss`): on wasm32 and
i686 it is a silent bug that tests on x86-64 never see. Sizes that describe data rather
than memory (file lengths, byte offsets, memory budgets) are `u64` on every target.

## `unsafe`

`unsafe` is allowed only in modules named in the table below. Every other crate root
forbids `unsafe_code`: each `lib.rs`, each `main.rs`, and each file under `tests/` and
`examples/`, which Cargo compiles as crates of their own.

| Module | Why safe code is not enough | Miri test |
|---|---|---|
| `everygraph-io`, `src/sys.rs` | std has no memory maps, no file preallocation, no `posix_fadvise` and no `F_NOCACHE`. Four blocks: memmap2's `Mmap::map`, sound under the invariant that no writer modifies or truncates a mapped file (`design/DECISIONS.md`, "Memory-mapped files are read through a safe constructor"; the sinks refuse mapped files in-process, `src/claim.rs`), and `fallocate`, `posix_fadvise` and `fcntl(F_NOCACHE)`, each taking only a borrowed file descriptor and integers | none possible: Miri cannot execute foreign calls or map files, nor model the map's real hazard, outside writers. Native tests instead (accepted in the review of PR #151): `tests/conformance.rs` reaches every block and checks that `fallocate` allocated the blocks on Linux; `sinks_refuse_a_mapped_file_and_maps_refuse_a_file_being_written` checks the guard; the macOS block runs on the macOS runner of `other-platforms.yml` |
| `crates/everygraph-io/tests/allocation_failure.rs`, module `refusing` | A test that makes the allocator refuse, to reach every `MemoryError::AllocationFailed` path of `SegVec` and `ScratchPool`: replacing the global allocator is an `unsafe impl GlobalAlloc`. It is not in `everygraph-core` because the core forbids `unsafe` in every crate root, its tests included | `cargo +nightly miri test -p everygraph-io --test allocation_failure` |
| `everygraph-format`, `src/adjacency/` (the `unsafe` code is in `simd.rs`) | Stream VByte decodes four words with one SIMD byte shuffle and a fused prefix sum (SSSE3, NEON, wasm `simd128`); loads, stores and calls of feature-enabled functions are `unsafe`. The lab's scalar prefix-sum pass was 1.17–1.25× slower on single-thread `PageRank` (FINDINGS § M1-13) | `crates/everygraph-format/tests/adjacency.rs` under both Miri commands above (scalar, then SSSE3 compiled in); NEON is run by the weekly arm64 jobs (`cargo test (format)`); `simd128` by `tools/check_wasm_codec.py` in Node |

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
5. Add the module to the table above, written in backticks as its path
   (`` `crates/everygraph-io/src/sys.rs` ``, a directory ending in `/`) or as the crate
   followed by the path inside it (`` `everygraph-io`, `src/sys.rs` ``): CI reads that
   column to run the sanitizers job when the module changes. If the module is outside
   the core, add its Miri run to that job in `ci.yml`. The pull request gets the independent review that CLAUDE.md § 4
   requires for `unsafe` code.

Where we expect to need it, from the lab prototypes: SIMD in the adjacency codec (#30),
disjoint parallel writes into shared arrays in the build pipeline and in Leiden (#35,
#57), and the generated glue of the Python and
browser bindings (#81, #108). `everygraph-core` is expected to stay free of it.

## Rust version

The minimum supported Rust version is **1.87**, recorded as `rust-version` in the
workspace manifest. It was 1.85 (the first release with edition 2024 and the MSRV-aware
resolver, and Debian 13's toolchain) until 2026-11-25, when it was raised to 1.87 because
the fix for RUSTSEC-2026-0344 in `ruzstd` (0.9.1, needed by the text reader, #32) requires
it. Everything builds on stable. The
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

`Cargo.lock` is committed, and it is what the checks run against: continuous integration
passes `--locked`, so a manifest change without the matching lock change fails. Updating
dependencies (`cargo update`) is a pull request of its own. The resolver is aware of
`rust-version`, so an update does not select a release that needs a newer Rust than the
minimum.

The core takes no C or C++ dependency (DESIGN D1); the dependency allow-list described
under "The portability rule" is what checks it. For the other crates nothing checks it
yet.

## Licensing, sign-off and review

### Under which licence a contribution comes in

Inbound equals outbound: you license your contribution under the same terms the project
uses for that kind of file, and you keep your copyright. There is no contributor licence
agreement.

- Code — source, scripts, build files, workflows, code samples anywhere — is
  `Apache-2.0 OR MIT`: everyone who receives it may choose either licence. Your
  contribution must therefore be offered under both. Apache-2.0 adds an explicit patent
  licence from each contributor for their contribution (its section 3); MIT is shorter
  and compatible with GPL-2.0. Section 5 of Apache-2.0 says the same about inbound terms.
- Documentation and format specifications (`docs/`, `design/`, and Markdown outside
  `crates/` and `lab/`) are CC BY 4.0. [LICENSE-DOCS](LICENSE-DOCS) says exactly which
  paths it covers; code inside those files stays `Apache-2.0 OR MIT`.
- Code written by someone else is accepted only under a licence that `deny.toml` allows,
  with its copyright notice kept and an entry in
  [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md). Say in the pull request where it
  came from. Copyleft code is not accepted.

### Sign-off

Every commit carries a `Signed-off-by:` line with your name and the same e-mail address
as the commit's author. By adding it you certify the
[Developer Certificate of Origin 1.1](https://developercertificate.org/): in short, that
you wrote the change or otherwise have the right to submit it under the project's
licences, and that you understand the contribution and your sign-off are public and kept
for good.

```sh
git commit -s                       # adds the line from user.name and user.email
git commit --amend -s               # adds it to the last commit
git rebase --signoff origin/main    # adds it to every commit of the branch
```

`.github/workflows/dco.yml` checks every commit of a pull request. While the repository
is private it runs only on pull requests from forks; it runs on all pull requests once
the repository variable `DCO_ALL_PULL_REQUESTS` is `true`. Commits made before this rule
are unsigned and stay as they are.

### AI-assisted contributions

Welcome, on the same terms as any other. The person who submits the pull request is its
author for every purpose here: they have read and understood the change, they answer for
it in review, and their sign-off certifies the Developer Certificate of Origin for it —
including that the output does not reproduce code under a licence the project cannot
accept. Say in the pull request which tools were used. (The project itself is developed
by AI agents under [CLAUDE.md](CLAUDE.md); [GOVERNANCE.md](GOVERNANCE.md) explains who
is responsible for that work.)

### Review

Every change reaches `main` through a pull request that follows
`.github/pull_request_template.md`: it names the issue it closes and copies the issue's
acceptance criteria, each ticked only with its evidence (a test, a result file, or a
command and its output). It passes the checks under "Checks", and changes to kernels,
on-disk formats, `unsafe` code, the planner or memory accounting get an independent
review whose findings are fixed or answered before merge ([CLAUDE.md § 4](CLAUDE.md)).
Pull requests are merged by squash. Who decides, and how that changes as more people
join, is in [GOVERNANCE.md](GOVERNANCE.md). Behaviour in the project follows the
[code of conduct](CODE_OF_CONDUCT.md); security problems are reported privately as
described in [SECURITY.md](SECURITY.md).

### What the project commits to

- The format specifications are public and versioned: every format the engine writes
  is specified in this repository under CC BY 4.0, with a version in the format itself.
- No feature is held back for a paid edition.
- No telemetry: the software does not send anything anywhere unless the user asks it to.
