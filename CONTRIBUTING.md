# Contributing to Everygraph

This file holds the coding rules of the Rust workspace. How work is planned, reviewed and
merged is in [CLAUDE.md](CLAUDE.md); why the engine is shaped this way is in
[docs/DESIGN.md](docs/DESIGN.md). Licensing, sign-off, review, the code of conduct and the
security policy are in the last section, "Licensing, sign-off and review".

## Checks

Every pull request passes these on current stable Rust. Continuous integration runs
all of them (see "CI" below); run them yourself before pushing, because a red run costs
a place in the shared queue and a round trip.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p everygraph-core --no-default-features --all-targets -- -D warnings
cargo clippy -p everygraph-core --target i686-unknown-linux-gnu --all-targets -- -D warnings
cargo clippy -p everygraph-core --target i686-unknown-linux-gnu --no-default-features --all-targets -- -D warnings
python3 tools/check_core_portability.py
python3 tools/numerics_lint.py --check
cargo test --workspace                      # or: cargo nextest run --workspace
cargo test -p everygraph-core --no-default-features
cargo build -p everygraph-core --target wasm32-unknown-unknown --no-default-features
cargo build -p everygraph-ingest --target wasm32-unknown-unknown
cargo build -p everygraph-wasm --target wasm32-unknown-unknown
cargo clippy -p everygraph-format --target i686-unknown-linux-gnu --all-targets -- -D warnings
cargo clippy -p everygraph-format --target wasm32-unknown-unknown -- -D warnings
cargo clippy -p everygraph-kernels --target wasm32-unknown-unknown -- -D warnings
cargo clippy -p everygraph-community --target wasm32-unknown-unknown -- -D warnings
cargo clippy -p everygraph-map --target wasm32-unknown-unknown -- -D warnings
RUSTFLAGS="-C target-feature=+simd128" CARGO_TARGET_DIR=target/wasm-simd128 \
  cargo clippy -p everygraph-format --target wasm32-unknown-unknown -- -D warnings
cargo deny check                            # offline: cargo deny check bans licenses sources
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo build --workspace --release
```

And the checks of the checks, each one command (what they do: "CI" below):

```sh
python3 tools/check_portability_canary.py   # the portability checks still catch each probe
python3 tools/check_wasm_toy.py             # toy kernel and the edge-map kernels: native and wasm32 in Node agree
python3 tools/check_wasm_codec.py            # adjacency codec: simd128 in Node agrees with scalar
python3 tools/check_container_python.py     # .eg container: the Python reader reads what Rust wrote
uv run --script tools/check_columns_python.py  # columns: pyarrow and Polars read what Rust wrote
uv run --script tools/check_egmap_readers.py  # map file: the Python reader and the official PMTiles readers read what Rust wrote
tools/check_python_package.sh               # the Python package: maturin build, pytest against NetworkX
python3 tools/check_tsan.py                 # ThreadSanitizer; needs the pinned nightly
```

`tools/ci_local.sh` runs both lists (not the release build) and prints which steps failed (`--miri`
adds the Miri and ThreadSanitizer runs, `--oracle` the oracle). Run it under a hard memory
limit: `systemd-run --user --scope -p MemoryMax=16G -p MemorySwapMax=0 tools/ci_local.sh`.
It covers Linux x86_64 only.

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
cargo +1.100 test --workspace --locked
```

And these on nightly, because Miri is the way to *run* tests with a 32-bit `usize` on a
machine without the 32-bit C runtime, and the required check for any module that
contains `unsafe`:

```sh
cargo +nightly miri test -p everygraph-core --no-default-features -- --skip full_size
cargo +nightly miri test -p everygraph-core --no-default-features --target i686-unknown-linux-gnu -- --skip full_size
MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-ignore-leaks" cargo +nightly miri test -p everygraph-core -- --skip full_size
cargo +nightly miri test -p everygraph-format --test adjacency
cargo +nightly miri test -p everygraph-format --lib adjacency::
RUSTFLAGS="-C target-feature=+ssse3" cargo +nightly miri test -p everygraph-format --test adjacency
cargo +nightly miri test -p everygraph-io --test allocation_failure
cargo +nightly miri test -p everygraph-io --lib pages::
MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks" cargo +nightly miri test -p everygraph-ingest --test heap_peak
MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-ignore-leaks" cargo +nightly miri test -p everygraph-kernels --lib edge_map::
MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-ignore-leaks" cargo +nightly miri test -p everygraph-memtest --test counting --test readers
cargo +nightly test -p everygraph-kernels --doc   # checks the error codes of compile_fail tests
```

The three codec commands run the adjacency codec's tests on the scalar decoder, then on
the SSSE3 one (compiled in, so that run-time detection finds it under Miri). Its tests run
a sample under Miri (`cfg!(miri)`): the full corpus takes about 15 minutes there. Miri
runs only the tests that reach `unsafe` code (the `unsafe` table below): the other tests
of the codec's crate and of the kernels are safe code, which the native runs check.

The core's runs skip the tests named `*full_size` (the numerics golden vectors and the
exact sum over chunks of 65,536), which take minutes under Miri; the native test runs on
every target cover them.

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

### Fuzzing

Code that parses bytes has a fuzz target (CLAUDE.md § 6, issue #191), in a `fuzz/`
package of its crate that is not part of the workspace: `everygraph-format` has
`adjacency`, `container`, `columns`, `hierarchy` and `egmap` (the map file of
`everygraph-map`), `everygraph-ingest` has `text`. Each target's corpus is
committed under `fuzz/corpus/TARGET`. With `cargo-fuzz` and the pinned nightly:

```sh
tools/fuzz.sh replay                 # every corpus, once: what every pull request runs
tools/fuzz.sh run container 300      # fuzz one target for 300 seconds
tools/fuzz.sh cmin container         # then shrink its corpus before committing it
```

Every run refuses one allocation above 64 MiB, so that a size an input declares (a zstd
window) cannot become memory. A crash leaves its input under `fuzz/artifacts/TARGET`;
the fix commits it to the corpus, so that the replay keeps it fixed. Code that writes
has a fault-injection sweep: `everygraph_io::faults` (feature `faults`) fails the k-th
operation of any source or sink, and a test runs the work for every k
(`crates/everygraph-format/tests/faults.rs`).

### Mutation testing

cargo-mutants changes the code one small step at a time (a `+` becomes a `-`, a function
returns a default) and runs the tests: a mutant that no test fails on is a test gap. It is
pinned to version 27.1.0 (`cargo install cargo-mutants --version 27.1.0 --locked`), with
cargo-nextest as the test runner. `.cargo/mutants.toml` holds the options every run
shares; `tools/mutants.sh` runs what CI runs:

```sh
tools/mutants.sh diff origin/main      # the product-code diff of your branch, and its mutant count
tools/mutants.sh pr 0/1                # test those mutants (k/n: one shard of n)
tools/mutants.sh crate everygraph-format  # every mutant of a crate: the milestone audit
tools/mutants.sh report                # the last run as Markdown
```

Run them under a hard memory limit, like the tests. The results are in
`target/mutants/mutants.out/` (`missed.txt`, `caught.txt`, a log per mutant).

**A mutation score is never presented as a correctness argument.** It measures how
strongly the tests react to small changes. Most real faults are coupled to some mutant,
but about one in six is coupled to none, mostly changed algorithms and numerical overflow
or NaN (Just et al., FSE'14; `docs/research/sota-testing.md`, challenge 8). Those are the
kernel bugs that the oracle and its metamorphic relations exist to catch.

What is mutated:

- **On a pull request** (`mutants.yml`), the lines it changed in the product code of
  `tools/mutants.sh` (`everygraph-kernels`, `-format`, `-ingest`, `-community`, and the
  core's `memory` module). Diff-only mutants keep a change to a handful of mutants and
  most of them useful (Petrović et al., TSE'21).
- **At each milestone cleanup**, every mutant of each product crate (CLAUDE.md § 6), on
  the lab machine or with `gh workflow run mutants.yml -f package=CRATE -f shards=16`.
  The cleanup report lists the missed mutants per crate, each killed by a new test or
  explained. A whole run is thousands of mutants, too many for a nightly job in the
  shared queue.
- **Capacity arithmetic is kept.** cargo-mutants skips the arguments of `with_capacity`
  by default, as Google treats capacity arithmetic as arid. Here a wrong capacity breaks
  the memory promise, so `skip_calls_defaults = false`. Only a test that compares the
  budget's ledger with what a counting allocator saw can kill them
  (`everygraph-ingest/tests/heap_peak.rs`, and the no-OOM suite in `everygraph-memtest`,
  which a mutant of another crate does not run); most of them are still missed, and each
  milestone audit lists them.
- **Arid code is not mutated**: `Display` and `Debug` implementations (message text) and
  the build's progress output (`exclude_re` in `.cargo/mutants.toml`). Add a pattern
  there, with its reason, rather than `#[mutants::skip]` in product code.
- Each mutant runs only the tests of its own crate, so the oracle's seeded-bug suite
  (about 90 s) never runs per mutant. nextest does not run doc tests: a mutant that only a
  doc test would catch is reported as missed.

## CI

GitHub Actions, seven workflows under `.github/workflows/`, on GitHub-hosted runners only.
A pull request's CI takes under 15 minutes (CLAUDE.md § 6). The workflows follow these
rules:

1. One cheap gate, `lint` (at most 3 minutes), that every other job needs, the other
   platforms included.
2. One job per concern. Its name says the concern in a few words. A rerun repeats only
   that job.
3. Build once (`cargo nextest archive`), test in partitions (`--partition hash:k/4`).
4. Time limits come from measurement: the 99th percentile × 1.5, rounded up. A job that
   reaches its limit is a defect to fix.
5. Tiers. A pull request runs what its diff can break. A push to `main` runs everything:
   all platforms, the release build, the MSRV. Nightly runs the exhaustive checks.
6. A test runs where its property can differ. Platform-neutral computation runs on Linux
   only. Miri runs only the tests that reach `unsafe` code (the `unsafe` table below).
7. Steps that use the network are separate steps, retried once.
8. A flaky test is quarantined: a nextest filter and an open issue, and it runs nightly.
   It is never retried silently.
9. Nothing but Rust build output goes under `target/`.
10. One aggregate job, `ci-ok`.

| Workflow | When | Jobs (each step is a command from "Checks") |
|---|---|---|
| `ci.yml` | every pull request and every push to `main`, unless only documentation or results changed (`*.md`, `docs/`, `plan/`, `design/`, `lab/`, `bench/results/`); nightly at 03:41 UTC; by hand | **lint** (always): which jobs the run needs (below), fmt, the clippy runs (the codec's on i686 included), `check_core_portability.py`, `numerics_lint.py --check`, `cargo deny check`, docs. **build** (always): `cargo nextest archive` of the workspace (the oracle crate's tests only when the oracle job runs or a manifest changed), doc tests, the core without default features. **test 1/4 … 4/4**: the archive, `--partition hash:k/4`. **wasm** (always): the three wasm32 builds (core, ingest, everygraph-wasm), the wasm32 clippy runs (the codec with and without `simd128`, kernels, community, map), the wasm toy (with the edge-map kernels), the wasm codec check (`simd128` in Node against scalar). **readers** (always): the container, columns and map-file checks against the Rust writers, the Python package. **canary** (always): the portability canary. **i686**: the core's and the codec's tests natively on i686. **release** (full runs only): the release build. **ci-ok**: passes when every other job passed or was not needed. **oracle**: `oracle/run-ci.sh` (the correctness oracle: Python references through `uv`, toy kernel, the edge-map kernels against the Python references, seeded bugs, golden files; `oracle/README.md`). **msrv**: the tests on 1.100, skipped while 1.100 is the current stable (the test job ran them on the same compiler, the oracle crate's included). **sanitizers**, two parallel parts: the Miri commands of the modules outside the core through `cargo miri nextest run`, the kernels' doc tests on nightly, then `check_tsan.py`. **miri-core** (full runs only), two parallel parts: the core's three Miri commands. **no-oom** (nightly and by hand only): `cargo test --release -p everygraph-memtest -- --ignored`, the no-OOM suite at the nightly sizes, the long search and the cgroup matrix, its records kept as an artifact. All on Linux x86_64 |
| `fuzz.yml` | as `ci.yml`; nightly at 02:17 UTC; by hand | **replay** (always, merge only when green): `tools/fuzz.sh replay`. **campaign** (advisory), when a pull request changes `everygraph-format`, `-ingest`, the core or `tools/fuzz.sh` (`ci_changes.py`'s `fuzz_*` flags): 5 minutes per target of that crate. **nightly**: 1 hour per target, then `cmin`; the corpus is uploaded as an artifact, and its new entries come back by pull request ("Fuzzing" above) |
| `mutants.yml` | pull requests that change the product code of `tools/mutants.sh` (or the mutation tooling); by hand for one crate | **plan**: `tools/mutants.sh diff HEAD^1`, and one shard per three mutants, at most four. **shard** (advisory): `tools/mutants.sh pr k/n` in a 10-minute box; missed mutants become annotations on the changed lines and a list in the run's summary; `mutants.out` is uploaded. Red only when cargo-mutants itself fails (bad diff, bad option, internal error). By hand: `tools/mutants.sh crate CRATE k/n` over the shards asked for ("Mutation testing" above) |
| `other-platforms.yml` | pull requests that touch `everygraph-core`, `-io`, `-format` or `-ingest`; every push to `main`, unless only documentation or results changed; Mondays 04:17 UTC; by hand (`gh workflow run other-platforms.yml --ref <branch>`, optionally `-f platform=macos\|windows\|linux-arm -f packages="-p <crate>"`) | clippy, nextest and doc tests, and the wasm32 build of the core, on macOS arm64, Windows x86_64 and Linux aarch64: of those four crates on a pull request, of the workspace otherwise (without `everygraph-memtest`) |
| `docs.yml` | every pull request and every push to `main` | the document checks: `docs/tools/check_adrs.py`, `check_api_sketch.py`, `check_graph_semantics.py`, `check_transcripts.py docs/ux-contract.md`, `check_schemas.py docs/ux-contract.md docs/api-sketch.md` (with `jsonschema`, through `uv`), `plan/publish.py VincenzoImp/everygraph --dry-run`, and the design system: `design/system/tools/build.py --check`, `measure.mjs` (Node), `lint_colours.py`. Standard library only, except the schema check |
| `dco.yml` | every pull request | the `Signed-off-by` check ("Sign-off" below) |
| `determinism.yml` | nightly at 01:53 UTC; by hand; pull requests that change it | **grid**: `everygraph-determinism` with `EVERYGRAPH_DETERMINISM=full` (every combination of 1, 2, 3, 8 and 32 threads, two chunkings and four sources, over a third of the CI corpus) and the determinism tests of the kernels, Leiden and the build, optimized. **schedules**: Miri with `-Zmiri-many-seeds` over seeds 0 to 15, in four jobs, on the tests of the core's `par`. A failed scheduled run opens an issue. The sampled grid runs in `ci.yml`'s test job |

Which jobs of `ci.yml` a pull request runs is decided by `.github/scripts/ci_changes.py`
in the lint job, from `git diff` of GitHub's merge commit against its base (run it
locally with `--base main` to see what your branch would run):

| Job | Runs on a pull request when it changes |
|---|---|
| lint, build, test, wasm, readers, canary, ci-ok | always |
| sanitizers (Miri, ThreadSanitizer), i686 | `crates/everygraph-core/`, a module of the `unsafe` table below, `tools/nightly-toolchain`, `tools/check_tsan.py`, `tools/canary/tsan/` |
| oracle, and the oracle crate's tests in build | `crates/everygraph-oracle/`, `oracle/`, the kernel crates (`everygraph-kernels`, `-community`, `-map`), `datasets/`, `crates/everygraph-bench/src/datasets/` |
| msrv | any `Cargo.toml` (`rust-version`, dependencies, profiles), `Cargo.lock`, a `rust-toolchain` file |
| everything, miri-core and release included | anything under `.github/`, or the pull request has the label **`full-ci`** |

Everything also runs on every push to `main`, and nightly unless `main`'s last commit
already passed a full run (then only lint, which hears of new advisories and of a new
stable toolchain). So nothing merged goes unchecked by any job for more than a day. What
a pull request may therefore leave to `main`: a newer *language* feature below the MSRV
in a change that touched no manifest (newer *std* APIs are caught on every run by
clippy's `incompatible_msrv`), a dependency bump in `Cargo.lock` that changes the core's
behaviour under Miri, and anything a kernel change breaks in the oracle's Python suite
through a crate not listed above. The coordinator adds `full-ci` to every Tier 2 pull
request (CLAUDE.md § 4) before merging it. The jobs that run always start even when
lint fails, so that a red pull request still gets its test results; the conditional jobs
wait for lint to pass.

A new push to a pull request cancels the run it replaces. The caches (`Swatinem/rust-cache`,
and `astral-sh/setup-uv`'s cache of the oracle's Python packages) are written by runs on
`main` and read by every branch. Third-party actions are pinned
to a commit, with the version in a comment; workflows get `contents: read` and nothing
else (the lint job also reads Actions runs, for the nightly check).

**Push when the branch is ready for CI, not after every commit.** Every push to an open
pull request starts a run, which takes a place in the queue every agent shares, even when
the next push cancels it. Run
the checks of the crates you touched locally ("Checks"), commit as often as you like,
and push once the branch is ready to be checked — or push to a branch without a pull
request, which starts nothing.

What the scripts at the end of "Checks" are for:

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
  through `ByteSource`, `NodeId` and `par`; `tools/wasm-toy/`) and the edge-map
  framework's kernels (PageRank, the proof WCC, BFS over the raw and the compressed source)
  natively with rayon and as wasm32 in Node, and compares the hashes. The determinism
  CI (m3-11) replaces it.
- **`tools/check_wasm_codec.py`** applies the wasm toy's pattern to the adjacency codec:
  the `simd128` decoder in Node must agree with the scalar one.
- **`tools/check_container_python.py`** has `everygraph-io`'s example `container_sample`
  write a container. `tools/eg_container.py`, a reader in Python written from
  `docs/spec/eg-container.md` alone, must then agree with the Rust reader, on the sample
  and on files patched to exercise the reading rules (fallback past a damaged or
  rule-breaking table, the tie between header blocks, required features of the older
  block). It must also verify the hashes and report a damaged section.
- **`tools/check_columns_python.py`** has the example `columns_sample` write property
  columns of every type (`docs/spec/eg-columns.md`). pyarrow, on a slice of a memory map
  of the file, must read the values given to the writer, with every buffer in the map at
  a multiple of 4096 where the layout section says; the statistics must equal those it
  recomputes exactly (`fractions.Fraction`); Polars must read the same values; and the
  sections pinned by the golden test of `crates/everygraph-format/tests/columns.rs` must
  be among those read. Then the same reading of every column of three graphs built by
  `everygraph ingest --weight --time` (one per `duplicates` option), which must also
  agree with the input as the script reads it. `--big` runs the 2^27-row check of the
  research decision B17
  (about 3 GB of memory, two minutes).
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
| `everygraph-core` | id types, `ByteSource` and its writable counterpart `ByteSink`, the parallel facade, the memory accountant (`Budget`, `Memory`, `Reservation`), segmented arrays (`SegVec`), per-thread scratch (`ScratchPool`), the `MemoryProbe` interface, the `Storage` a file is on | this skeleton, #31 | yes, enforced |
| `everygraph-io` | native `ByteSource` backends (file, mmap, direct I/O) and sinks (`FileSink` is the native `ByteSink`); the Linux `MemoryProbe` (`/proc/meminfo` and cgroup limits), physical cores and the storage of a directory; with the feature `faults`, the fault-injecting wrapper of tests | #29, #31, #28, #190 | no — the one crate for OS I/O |
| `everygraph-format` | the `.eg` container, the adjacency codec, property columns, and `verify` | #28, #30, #37, #41 | yes, enforced (with and without `simd128`) |
| `everygraph-ingest` | text and Parquet readers, id dictionary, build pipeline | #32–#35 | yes, built in CI (browser ingest, D22) |
| `everygraph-kernels` | the edge-map framework, the kernels, their operations by name (`run`) and the planner that chooses how each runs within its budget | #44, #47–#52, #45 | yes, built and linted in CI |
| `everygraph-community` | Leiden, the community table and modularity; the map hierarchy | #57, #62, #63 | yes, built and linted in CI |
| `everygraph-map` | layout, the `.egmap` file format, tile builder | #67–#69 | yes, built and linted in CI |
| `everygraph-query` | point queries, shortest paths, views | #90, #91, #93 | yes |
| `everygraph-cli` | the `everygraph` command | #41 (`verify`), #98 | no |
| `everygraph-py` | Python bindings | #81, #82 | no |
| `everygraph-wasm` | browser bindings | #108 | yes, built in CI |
| `everygraph-bench` | the `everygraph-bench` harness; the dataset registry and fetcher (`datasets/`) | #4, #5 | no |
| `everygraph-oracle` | the correctness oracle: generators, references, comparators, invariants, the `KernelUnderTest` trait (a development crate, `publish = false`, used by other crates' tests only; see `oracle/README.md`) | #6 | no |
| `everygraph-memtest` | the no-OOM suite: a counting global allocator, an accountant that refuses its k-th reservation, generated graph families, every operation run in process and in a `systemd-run` scope (a development crate, `publish = false`; its tests are the suite) | #53 | no |
| `everygraph-determinism` | the determinism suite of DESIGN D14: the build, the kernels and Leiden at every thread count, chunking, source and repetition; order probes; Leiden on views and renumbered ids (a development crate, `publish = false`) | #54 | no |

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

"Must build for wasm32" is enforced today for the crates CI builds for wasm32: the core,
`everygraph-format` and `everygraph-ingest` (and the empty `everygraph-wasm`). The other
rows state the intent; each crate joins the wasm32 build, and gets a `clippy.toml` like the
core's, in the pull request that gives it content.

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

The rule is enforced in full for `everygraph-core` only. Clippy reads the `clippy.toml`
nearest to the crate it checks, so the core's lists apply to no other crate.
`everygraph-format` keeps a copy of them in its own `clippy.toml` and forbids the lints
at its root; the script and the canary do not check that copy, and its dependencies are
checked only by the wasm32 builds and by review. The lists do not apply to `everygraph-io`
(see "Crates").

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
`Budget::used()` plus a fixed slack (#53). `everygraph-ingest` and `everygraph-format` have
code but not yet this `clippy.toml`; the M2 cleanup (#200) adds it.

## The numerics rule

A result has the same bits on every supported platform (DESIGN D14). The rules are
R1–R15 of `docs/research/sota-numerics.md` § 4; the module docs of
`everygraph_core::math` and `everygraph_core::rng` say how to follow them. What is
checked:

| What is checked | By |
|---|---|
| No std transcendental (`sin`, `exp`, `ln`, `powf`, `powi`, … on `f64` and `f32`), no `libm::pow`, and no `rand`, `getrandom` or `fastrand` generator in a crate that produces results; use `everygraph_core::math` and `everygraph_core::rng` | The same list in the `clippy.toml` of the core, the format and the four kernel crates, and in the root `clippy.toml`, which every other crate without a `clippy.toml` of its own reads. `tools/numerics_lint.py` writes the seven copies, and its `--check` (in the lint job) fails if one differs. The canary probes every product crate. The harness has its own empty `clippy.toml`: its statistics are not results |
| No `rand` except through proptest, no `getrandom` except through `rand_core` and TLS, no `fastrand` | `deny.toml` |
| No float reduced by the edge-map framework's blocks: no `impl Reduce for` a type that holds `f32` or `f64` in a product crate; a float total goes through `ExactSum` | `tools/numerics_lint.py --check`, by a scan of the sources (the rule is in `everygraph_kernels::edge_map`, "Determinism") |
| A result does not depend on the thread count, chunking, source or repetition; the edge-map reduction and `par::map_ranges` add in their specified order | `crates/everygraph-determinism` (weighted graphs with weights over 2^-20 to 2^21; order probes, R15b), sampled on every pull request, in full in `determinism.yml` |
| The pinned functions, `sqrt`, `mul_add`, `ExactSum` and the generator give the same bits on x86-64, i686, aarch64, macOS, Windows and wasm32 | `crates/everygraph-core/tests/numerics_golden.rs` in every test run of the core, and in Node through `tools/check_wasm_toy.py` |

To change the numerics list, edit `tools/numerics_lint.py` and run it. Not checked by
any lint: `Iterator::sum` on floats (use `math::sum`), `f64::max` where signed zeros or
NaN can tie (use `total_cmp`), and float sums whose order depends on the schedule (use
`ExactSum`, or integers).

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
| `everygraph-io`, `src/sys.rs` | std has no memory maps, no file preallocation, no `posix_fadvise` and no `F_NOCACHE`. Four blocks: memmap2's `MmapOptions::map` of a range of a file, sound under the invariant that no writer modifies the mapped bytes or truncates the file into them (`design/DECISIONS.md`, "Memory-mapped files are read through a safe constructor" and "Range maps of a graph file"; in-process the sinks refuse every write that would reach a mapped range, `src/claim.rs`), and `fallocate`, `posix_fadvise` and `fcntl(F_NOCACHE)`, each taking only a borrowed file descriptor and integers | none possible: Miri cannot execute foreign calls or map files, nor model the map's real hazard, outside writers. Native tests instead (accepted in the review of PR #151): `tests/conformance.rs` reaches every block and checks that `fallocate` allocated the blocks on Linux; `sinks_refuse_a_mapped_file_and_maps_refuse_a_file_being_written` and `a_file_sink_writes_beside_a_range_map_and_never_into_it` check the guard; `tests/container.rs` checks the lock across processes (`a_second_writer_is_refused_while_the_first_lives_and_not_after_kill_9`) and a range map against another process's commits; the macOS block runs on the macOS runner of `other-platforms.yml` |
| `everygraph-io`, `src/pages.rs` | The page backend `PAGES` (#167): an allocator is an `unsafe impl Allocator`, and std has no anonymous maps. Blocks of 128 KiB or more are mapped on their own (at a 2 MiB boundary from 2 MiB on), resized with `mremap` on Linux and unmapped when freed; smaller blocks go to `Global`. Blocks: `mmap`, `munmap` (of a block, and of the head and tail trimmed for alignment), `mremap`, `sysconf`, `Global.deallocate`, `Global.grow` and `Global.shrink` for blocks that stay below the threshold, the copy of a resize between the two kinds, and the `unsafe impl` itself | `cargo +nightly miri test -p everygraph-io --lib pages::` (Miri has `mmap` and `munmap` of whole regions; the 2 MiB trim and `mremap` are left out under Miri and reached by the native tests and by `everygraph-ingest`'s `tests/retention.rs`) |
| `crates/everygraph-io/tests/allocation_failure.rs`, module `refusing` | A test that makes the allocator refuse, to reach every `MemoryError::AllocationFailed` path of `SegVec` and `ScratchPool`: replacing the global allocator is an `unsafe impl GlobalAlloc`. It is not in `everygraph-core` because the core forbids `unsafe` in every crate root, its tests included | `cargo +nightly miri test -p everygraph-io --test allocation_failure` |
| `crates/everygraph-ingest/tests/heap_peak.rs`, module `counting` | A test that counts the bytes the program holds from the allocator, to check that the identifier dictionary's heap stays within the budget its accountant was given (review of PR #158): replacing the global allocator is an `unsafe impl GlobalAlloc` | `MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks" cargo +nightly miri test -p everygraph-ingest --test heap_peak` (Tree Borrows because rayon's dependencies need it; isolation off because the test's scratch is files) |
| `everygraph-kernels`, `src/edge_map/` (the brand and the unchecked indexing are in `node.rs`; the drivers turn checked lists into nodes; `Csr` reads its checked arrays without bounds checks, and `RawCopy` the arrays it checked once and owns; the sources state their guarantee in `unsafe impl Source`) | Kernels index per-node state with the targets of lists many times per entry, and the compiler cannot see that they are in range. Each source checks its lists once, in the memory it hands them out from, and a `Node<'g>` carries the proof (a generative lifetime brand, as in `GhostCell`). The bounds checks cost up to a third of the time of WCC on orkut at one thread (interleaved A/B against `lab/b1-core`, PR #202). `Csr` over a memory map relies also on the invariant of `everygraph-io`, `src/sys.rs` (no writer changes a mapped file) | `MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-ignore-leaks" cargo +nightly miri test -p everygraph-kernels --lib edge_map::`, and `everygraph-memtest`'s `--test readers` (the readers' heap against their reservations, on threads): the unit tests drive `scan`, `scan_until`, `frontier`, `push_with`, `vertices` and random access over `Csr`, `Compressed`, `Stream`, `RawCopy` and `CompressedStream` built in memory, with a corrupt list; the brand's `compile_fail` doc tests run on nightly (`cargo +nightly test -p everygraph-kernels --doc`), which checks their error code |
| `everygraph-memtest`, `src/counting.rs` | The no-OOM suite's counting allocator (#53): the bytes in use, their peak, and the most bytes beyond the ledger by phase, for every test binary of the crate that installs it. Replacing the global allocator is an `unsafe impl GlobalAlloc`; each block is one call to `System` | `MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-ignore-leaks" cargo +nightly miri test -p everygraph-memtest --test counting --test readers` |
| `everygraph-format`, `src/adjacency/` (the `unsafe` code is in `simd.rs`) | Stream VByte decodes four words with one SIMD byte shuffle and a fused prefix sum (SSSE3, NEON, wasm `simd128`); loads, stores and calls of feature-enabled functions are `unsafe`. The lab's scalar prefix-sum pass was 1.17–1.25× slower on single-thread `PageRank` (FINDINGS § M1-13) | `crates/everygraph-format/tests/adjacency.rs` under both Miri commands above (scalar, then SSSE3 compiled in); NEON is run by the arm64 job of `other-platforms.yml`; `simd128` by `tools/check_wasm_codec.py` in Node |

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

Where we expect to need it next, from the lab prototypes: the generated glue of the Python
and browser bindings (#81, #108). The lab's disjoint parallel writes into shared arrays
(the build pipeline and Leiden, #35, #57) are to go through the safe helpers of `par`
instead (#189). `everygraph-core` is expected to stay free of `unsafe`.

## Comments and documents

The rules for language, comments and documents are in the introduction of
[CLAUDE.md](CLAUDE.md).

## Rust version

The minimum supported Rust version is **1.100**, recorded as `rust-version` in the
workspace manifest. 1.100 stabilizes `std::alloc::Allocator` and `Vec::new_in` (release
notes of 1.100.0, "Stabilize custom allocators"): the memory accountant's page backend
(#167) gives `SegVec` segments an allocator, and the core cannot implement a container of
its own over foreign memory without `unsafe`. `Vec::try_with_capacity_in` is not stable in
1.100; code uses `Vec::new_in` and `try_reserve_exact`. Everything builds on stable. The
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

The `dev` profile, which tests use, has `opt-level = 1` and line tables only (no debug
information for dependencies). So run tests without `--release`: the whole workspace
suite takes about a quarter of the CPU time it took at opt-level 0, and a rebuild after a
change does not repeat the link-time optimization. Use `--release` for measurements only.
Benchmark numbers come from the release profile: without LTO, PageRank took 1.6× the
CPU time (exploratory, interleaved, 2026-11-26), so a lighter profile would mislead.

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

`.github/workflows/dco.yml` checks every commit of a pull request. It always runs on
pull requests from forks, and on every pull request while the repository variable
`DCO_ALL_PULL_REQUESTS` is `true` (it is). When you merge `origin/main`
into a branch, sign the merge commit too: `git merge --signoff origin/main`. Commits made
before this rule are unsigned and stay as they are.

### AI-assisted contributions

Welcome, on the same terms as any other. The person who submits the pull request is its
author for every purpose here: they have read and understood the change, they answer for
it in review, and their sign-off certifies the Developer Certificate of Origin for it —
including that the output does not reproduce code under a licence the project cannot
accept. Say in the pull request which tools were used. (The project itself is developed
by AI agents under [CLAUDE.md](CLAUDE.md); [GOVERNANCE.md](GOVERNANCE.md) explains who
is responsible for that work.)

### Review

Every change of code or of a specification reaches `main` through a pull request that
follows `.github/pull_request_template.md` and passes the checks under "Checks". How a
pull request is scoped, reviewed and merged is in [CLAUDE.md](CLAUDE.md) § 2 and § 4.
Pull requests are merged by squash. Who decides, and how that changes as more people
join, is in [GOVERNANCE.md](GOVERNANCE.md). Behaviour in the project follows the
[code of conduct](CODE_OF_CONDUCT.md); security problems are reported privately as
described in [SECURITY.md](SECURITY.md).

### What the project commits to

- The format specifications are public and versioned: every format the engine writes
  is specified in this repository under CC BY 4.0, with a version in the format itself.
- No feature is held back for a paid edition.
- No telemetry: the software does not send anything anywhere unless the user asks it to.
