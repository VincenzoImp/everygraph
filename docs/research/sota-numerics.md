# State of the art: bit-identical numerics across threads, budgets, regimes and platforms

Second-wave review W1 (`docs/research/sota-coverage.md` § (c) topic 1, § (d) W1), 2026-11-25,
Opus. Scope: reproducible summation and its cost; FMA and contraction in Rust/LLVM; libm and
correct rounding; counter-based RNG and seeding; wasm NaN bits, relaxed SIMD and the Wasm
3.0 deterministic profile; tools that reveal accumulation order. Nothing was built or
measured except one tiny compile-only check (§ 3.2, under `MemoryMax=4G`); no graph was
loaded. Paper numbers are given with the paper's machine, data and thread count; none of
them is comparable to ours as-is.

> **Touches work in development now.**
> - **`.eg` container spec (PR #170, branch `m2-01-container-spec`).** Free to change. Add
>   to § 2 conventions one sentence on floats (IEEE 754 binary32/64, little-endian; result
>   and derived sections store NaN only as the canonical quiet NaN and never −0.0), and to
>   § 6 a convention for provenance parameters: `seed`, `numerics` (version of the rule
>   list below), `weight_scale` (the fixed-point exponent of challenge 1), floats written
>   shortest-round-trip (`{}`), never `{:.N}`. No layout change is needed: parameters are
>   already free key/value text. See challenge 8.
> - **Edge-map framework (#44, branch `m3-01-edge-map`, no commits yet).** Four rules must
>   be in its contract: a per-node accumulator continues across decode-batch and I/O-buffer
>   boundaries (the 8,192-entry batches of m3-01 l. 9 and the budget-sized buffers of the
>   streaming regime must never start a new partial sum); push and pull variants are
>   interchangeable by the planner only if they give the same bits; SIMD lane count is a
>   constant of the source; global scalars go through an exact accumulator (challenge 6).
>   See challenge 7.

## 1. Scope and reading log

Read levels: **full** = method and evaluation read; **partial** = the sections named;
**abstract/results**; **not accessible** (reason given). HAL (`hal.science`) returned an
anti-bot page to every request, which cost three papers.

| # | Work | Venue, year | Verified link | Read level | PDF (`/home/tux/graphlab/papers/numerics/`) |
|---|---|---|---|---|---|
| 1 | Demmel, Nguyen: Parallel reproducible summation | IEEE TC 64(7), 2015 | doi.org/10.1109/TC.2014.2345391 | full (§ 3–5) | 2015-demmel-parallel-reproducible-sum.pdf |
| 2 | Demmel, Nguyen: Fast reproducible floating-point summation | ARITH 2013 | doi.org/10.1109/ARITH.2013.9 | slides only (paper's Berkeley URL dead) | 2013-demmel-fast-reproducible-sum-slides.pdf |
| 3 | Ahrens, Demmel, Nguyen: Algorithms for efficient reproducible FP summation | ACM TOMS 46(3), 2020 | doi.org/10.1145/3389360 (PDF: people.eecs.berkeley.edu/~demmel/ma221_Fall23/J115_…pdf) | partial: § 1, 6.2, 7 read; § 3–5 skimmed; appendix not read | 2020-ahrens-efficient-reproducible-sum-toms.pdf |
| 4 | Müller, Arteaga, Hoefler, Alonso: Reproducible FP aggregation in RDBMSs (extended) | ICDE 2018 | arxiv.org/abs/1802.09883 | full (§ II–VI) | 2018-muller-repro-aggregation-rdbms.pdf |
| 5 | Hallberg, Adcroft: An order-invariant real-to-integer conversion sum | Parallel Computing 40, 2014 | doi.org/10.1016/j.parco.2014.04.007 (PDF: adcroft.github.io) | full | 2014-hallberg-order-invariant-integer-sum.pdf |
| 6 | Arteaga, Hoefler, Fuhrer: Designing bit-reproducible portable HPC applications | IPDPS 2014 | doi.org/10.1109/IPDPS.2014.127 (PDF: htor.inf.ethz.ch/sec/bitrep-ipdps.pdf) | full | 2014-arteaga-bit-reproducible-portable.pdf |
| 7 | Xie, Gao, Wang, Xue: FPRev, revealing FP accumulation orders | USENIX ATC 2025 | usenix.org/conference/atc25/presentation/xie | full (§ 1–8) | 2025-xie-fprev.pdf |
| 8 | Salmon, Moraes, Dror, Shaw: Parallel random numbers, as easy as 1, 2, 3 | SC 2011 | doi.org/10.1145/2063384.2063405 | full (§ 1–2, 4–6; § 3 skimmed) | 2011-salmon-philox.pdf |
| 9 | Gladman, Innocente, Mather, Ozaki, Zimmermann: Accuracy of mathematical functions… | report, Aug. 2026 version | members.loria.fr/PZimmermann/papers/accuracy.pdf | partial: § 1–3 (binary32, binary64) | 2026-gladman-libm-accuracy.pdf |
| 10 | Sibidanov, Zimmermann, Glondu: The CORE-MATH project | ARITH 2022 | doi.org/10.1109/ARITH54963.2022.00014 | paper not accessible (HAL); conference slides read in full | 2022-sibidanov-core-math-slides.pdf |
| 11 | Sibidanov, Zimmermann: CORE-MATH progress report | FPTalks 2023 (slides) | members.loria.fr/PZimmermann/talks/core-math-fpbench2023.pdf | full (slides) | 2023-sibidanov-core-math-progress-slides.pdf |
| 12 | Anderson, Cornea, Stepin, Panu: Correctly rounded functions for vector applications | arXiv 2026 | arxiv.org/abs/2605.15547 | full | 2026-anderson-cr-vector-functions.pdf |
| 13 | Boldo, Jeannerod, Melquiond, Muller: Floating-point arithmetic | Acta Numerica 32, 2023 | doi.org/10.1017/S0962492922000101 | partial: § 2.10.3, 3.2–3.4 | 2023-boldo-floating-point-arithmetic.pdf |
| 14 | Neal: Fast exact summation using small and large superaccumulators | arXiv 2015 | arxiv.org/abs/1505.05571 | abstract/results (§ 1 summary of timings; figures not read) | 2015-neal-superaccumulators.pdf |
| 15 | Small, Kalia, Nakano, Vashishta: Order-invariant real number summation (HP method) | IPDPS 2016 | sites.usc.edu/cacs/files/2020/12/Small-OrderInvariantSum-IPDPS16.pdf | abstract/results | 2016-small-hp-order-invariant-sum.pdf |
| 16 | Widynski: Squares, a fast counter-based RNG | arXiv 2020 (v7 2022) | arxiv.org/abs/2004.06278 | abstract + algorithm (§ 1–3) | 2020-widynski-squares.pdf |
| 17 | Goodrich, Eldawy: Parallel algorithms for summing FP numbers | arXiv 2016 (SPAA) | arxiv.org/abs/1605.05436 | abstract only | 2016-goodrich-parallel-fp-summation.pdf |
| 18 | Brisebarre, Hanrot, Muller, Zimmermann: Correctly rounded evaluation of a function: why, how, at what cost? | ACM CSUR 58(1), 2025 | doi.org/10.1145/3747840 | not accessible (HAL anti-bot) | — |
| 19 | Collange, Defour, Graillat, Iakymchuk: Numerical reproducibility for the parallel reduction (ExBLAS) | Parallel Computing 49, 2015 | doi.org/10.1016/j.parco.2015.09.001 | not accessible (HAL anti-bot; publisher paywall) | — |
| 20 | L'Ecuyer et al.: Random numbers for parallel computers | MATCOM 2017 | (author page) | not accessible (returned HTML) | — |
| 21 | "Bitwise reproducibility as an incomplete correctness signal" | EarthArXiv 2026 | eartharxiv.org/repository/view/13757/ | not accessible (empty page) | — |

Specifications, documentation and code read (no PDF):

| Source | What was read | Level |
|---|---|---|
| Rust RFC 3514, float semantics | rust-lang.github.io/rfcs/3514-float-semantics.html | read (through a page summary; quotes below) |
| Rust std/core source, nightly 2026-11-24 (`library/{std,core}/src/num/f64.rs`, `iter/traits/accum.rs`, `portable-simd`) | "Unspecified precision" notes, `mul_add`, `mul_add_relaxed`, `algebraic_*`, `max`, float `Sum` | full for those items |
| `libm` 0.2.16 source (local cargo registry) | origins of `exp`, `sin`, `pow`, `cbrt`; `arch/` dispatch | relevant files |
| WebAssembly 3.0 announcement; relaxed-SIMD Overview.md; core spec appendix "Profiles" | webassembly.org/news/2025-09-17-wasm-3.0/ ; github.com/WebAssembly/relaxed-simd ; webassembly.github.io/spec/core/appendix/profiles.html | read |
| rand book, "Reproducibility" | rust-random.github.io/book/crate-reprod.html | read |
| metallic-rs README | github.com/jdh8/metallic-rs | read |
| yFoil assessment of pure-Rust CR libms (third party) | github.com/thclark/yfoil PR #23, issue #26 | search snippet only, not verified |
| Dolan-Gavitt, "Someone's been messing with my subnormals!" (2022); LLVM #57589 | moyix.blogspot.com/2022/09/… | search summary only |
| WGSL § 15.7 (reassociation, fusion, accuracy) | w3.org/TR/WGSL | not read (fetch truncated) |
| xsum, bitrep crates | crates.io/crates/xsum ; github.com/KyleClouthier/bitrep | search summary only (their own claims) |

First wave already holds Demmel–Nguyen TC'15 and Philox (kernels, items 25–26, "3–7×",
"keep fixed blocks"); this review reads them again for the cases kernels did not cover
(aggregation, atomics, budgets).

## 2. The state of the art, as a map

| Approach | Representative work | What it guarantees | Cost, as reported (machine, data, threads) |
|---|---|---|---|
| **Fixed order / fixed tree** | MKL CNR, cuBLAS (per Ahrens § 1), NumPy (FPRev § 6), our `core::par` | same bits only if partition, thread count and data order are fixed | MKL CNR "at most a factor of 2" (Ahrens § 1, vendor claim); sorting input to fix order in MonetDB: TPC-H Q1 total CPU 727% of baseline (Müller Table IV, 2× Xeon E5-2630 v3, 16 cores, HT off) |
| **Pre-rounding / binned accumulators** | Demmel–Nguyen 2013/2015, ReproBLAS (Ahrens 2020), Arteaga 2014 single sweep, Müller's `repro<T,L>` | same bits for any order, partition, reduction tree; to-nearest only; 6 words (48 B) for double, K = 3, W = 40 | ~7 effective FAdds/element in cache vs ~1 for MKL `dasum`, ~8 vs ~3 out of L3 (Demmel–Nguyen Fig. 4, one Sandy Bridge core, 2 GHz; read off the figure); 20% slower than MKL + MPI at 1,024 processes, ≥ 3× at few processes (Fig. 5, Cray XC30, 10^6 doubles); ~9n flops + 3n bitwise (Ahrens abstract); 1.2–1.6× recursive summation for the [15] implementation (Ahrens § 6.2, reported); Arteaga: < 10% overall for large arrays, 40% (k = 2) / 80% (k = 3) in cache (128 MPI ranks, Xeon E5-2670); Müller: hash GROUP BY 4–12× slower with repro types, **1.9–2.4×** with summation buffers (geomean 2.12–2.41 for double types), **+2.7%** end to end on TPC-H Q1 in MonetDB (2^30 pairs, same machine as above) |
| **Exact long accumulator** | Kulisch (via Boldo § 3.4), Neal 2015 (xsum), ExBLAS (not read), bitrep | exact sum, correctly rounded once: reproducible by definition | Neal: large superaccumulator (4,096 × 64-bit chunks, 32 KiB) "less than twice" simple ordered summation, serial, ≥ tens of thousands of terms, 16 systems 2000–2013 (abstract/§ 1); small one = 67 chunks |
| **Fixed-point / integer conversion** | Hallberg–Adcroft 2014 (GFDL ocean model), Small 2016 (HP), SQL DECIMAL | exact, order- and decomposition-invariant; range set by the scale | Hallberg: 2N real mult + N adds per value, "2N to 4N" compute-bound, "much smaller" when memory-bound; 10.5× (N = 3) and 21× (N = 6) on 72 cores, 2.2× and 3.5× on 648 cores, global sums of a 64,800-element array (Cray XT6, Table 3). Müller Fig. 4: repro types 4–12× slower "than with integers or IEEE floats" — **integers cost what floats cost** |
| **Correctly rounded libm** | CORE-MATH (MIT, C), LLVM libc, glibc ≥ 2.4x (partly CORE-MATH), Intel CR SIMD (2026) | one right answer per input: identical on every platform and version | CORE-MATH binary64 `exp` 18 cycles vs glibc 2.36 13 (EPYC 7282, reciprocal throughput); `pow` 66 vs 43 (i7-8700) (2023 slides); Intel SIMD CR binary32: 0.42× the throughput of their 1-ulp SIMD versions, geomean over 19 functions (Xeon Platinum 8592+, AVX-512) |
| **Counter-based RNG** | Philox, Threefry, ARS (Salmon 2011); Squares (Widynski) | the n-th draw is f_key(n): no state, no sequence, any thread order | Philox4×32-10: 3.6 cycles/byte on one Xeon X5667 core, 145 GB/s on a GTX580; Philox4×32-7 2.4 cpB; Mersenne Twister 2.0 cpB (Table 2); all counter-based ones pass BigCrush |
| **Platform semantics** | Rust RFC 3514; Wasm 3.0; relaxed SIMD | Rust: + − × ÷ `%` sqrt `mul_add` casts "exactly match IEEE 754-2008"; NaN sign non-deterministic; x87 and 32-bit ARM NEON non-compliant. Wasm: NaN bits and relaxed ops implementation-dependent; the **deterministic profile** makes NaNs "canonical and positive" and relaxed ops fixed — browsers implement the full profile | — |
| **Order-revealing tests** | FPRev 2025 | reconstructs the summation tree of a black-box sum with n(n−1)/2 masked inputs (±M among ones) | ~1 s at n = 8,192 for NumPy sum (BasicFPRev > 100 s), Xeon E5-2690 v4; NumPy sum: identical tree on three CPUs; NumPy dot/matvec: trees differ by CPU (2-way vs sequential) |

Who is best on what: **cheapest reproducible group sums** — integers/fixed point (Müller's
baseline, Hallberg); **best general float accumulator** — ReproBLAS binned (one pass, one
reduction, 48 B); **exact global sums** — Neal's large superaccumulator; **libm identity** —
correct rounding (CORE-MATH, LLVM libc), at 1.4–1.5× glibc for double `exp`/`pow`
(2023 slides); **RNG** — counter-based, Philox the established one (MATLAB, cuRAND, MKL per
Widynski § 1); **verification** — FPRev is the only tool found that names the order rather
than detecting a difference.

What libraries actually return (Gladman et al. Table 3, binary64, largest known error in
ulps): glibc 2.44 `exp` 0.511, `log` 0.520, `sin`/`cos` 0.516, `pow` 0.523, `atan2` 0.524;
musl 1.2.6 `sin` 0.831, `cos` 0.834, `atan2` 1.55; FreeBSD 15.1 `exp` 0.949, `pow` 636;
LLVM libc 22.1.8: everything 0.500 except `atan`, `atan2`, `pow`. Different libraries give
different bits; glibc and the Intel library also **dispatch at run time by CPU** (FMA,
AVX2, AVX-512), so one binary can differ between two x86-64 machines (§ 1 of the report);
glibc 2.27 and 2.36 return different `sin` for the same input (CORE-MATH slide 3).

## 3. Decision by decision

### 3.1 Table

| Ours | Best known work | Behind / level / ahead | Evidence |
|---|---|---|---|
| **D14 / ADR 0014** any threads, platform, budget, regime | Hallberg (invariance to processor count *and* decomposition), ReproBLAS (any order, partition, tree), Müller ("data independence": physical order must not change a query) | **Ahead in scope** for a graph engine (budget- and regime-invariance not found elsewhere); **behind in mechanism**: we rely on fixed order everywhere, which cannot survive schedule-dependent sums (atomics, hash/bucket aggregation, push vs pull). Evidence is **weak**: platforms compared "by PageRank sum and maximum … not by a hash" (ADR 0014); Leiden on x86-64 only; nothing on ARM | ADR 0014 § Evidence; Hallberg § 2; Müller § II-C |
| **m4-04** "exactness assumes integer weights… need a fixed summation order (or fixed-point weights)" | Hallberg / DECIMAL: integer sums are exact in any order at integer cost; binned per group costs 1.9–2.4× (Müller) and 48 B per accumulator | The integer-weight limit **can be lifted** — by fixed-point, not by fixed order. Fixed order constrains m4-03 parallel aggregation and the atomic commit; fixed point does not | § 3.3; challenge 1 |
| **m4-02** "fixed-point community totals (designed in the lab, not implemented)"; acceptance "modularity equal across aggregation" | same | **Right idea, unspecified**: scale, width, overflow, semantics not written. Its acceptance "equal across aggregation" is achievable *only* with exact sums (a float sum over members ≠ the same float sum over a quotient in general) | lab `seqagg.rs` l. 46–51 (`w: u32`), `b2-leiden` l. 378 (`gain * 2^50 as i64`) |
| **m4-01** deterministic Leiden: "commutative commit, fixed-point gain sums" | Hallberg N = 1 | **Level** for gain totals; σ (community volume) is still an f64 `compare_exchange_weak` loop (`fadd`, `b2-leiden/src/util.rs` l. 51–56, called at `main.rs` l. 345–346) — exact only for integer degrees | lab code |
| **m3-04 / #131** fixed reduction blocks by node range for the dangling mass | Neal / ReproBLAS: an exact or binned accumulator makes the block shape irrelevant | **Level** (works), but a class of bugs (PR #131: chunk 64 vs 65,536) stays possible for every future global scalar; an exact accumulator removes it at ≪ 1% of an iteration (challenge 6) | PR #131 note in m3-04 |
| **m3-11** "no fused multiply-add or platform maths routines in result-bearing arithmetic" | Arteaga § II-B (FMA ambiguity is a C/C++/Fortran compiler freedom; own portable libm, geomean 0.83× the time of platform libms on 5 systems); RFC 3514 | **Rule right, partly mis-aimed and already violated.** rustc never contracts (§ 3.2), so FMA is a non-issue unless written; the real hazards are std transcendentals, `powi`, runtime-dispatched SIMD, `HashMap` order, NaN/−0 bits, FTZ. Violations in code that reaches results: lab layout `cos`/`sin` (`e-map/gmap/src/layout.rs` l. 139–140), tile zoom `log2` (`tiles.rs` l. 134), Leiden tolerance `0.1f64.powi(iter)` (`b2-leiden` l. 846, `m1-13` l. 843, `m1-02` l. 1105), oracle graph generator `powf` (`crates/everygraph-oracle/src/generators.rs` l. 282) | grep of `lab/` and `crates/` (§ 3.4) |
| **m3-11** acceptance "a PR that makes a reduction order depend on the thread count fails CI" | FPRev: masked inputs reveal the tree | **Behind**: hashes on unweighted graphs cannot see order changes in sums of integers (exact in any order) — Leiden on tier-1 graphs would pass with any order. Needs weighted adversarial inputs and order probes (challenge 3) | FPRev § 4 |
| **D17 / m5-01** layout "byte-identical across thread counts" | Arteaga (portable transcendentals) | **Threads only.** Native ↔ wasm32 and x86 ↔ ARM identity of the layout untested and at risk: `f64::sin`/`cos` go to glibc on Linux, to the `libm` crate (FreeBSD/musl port) on `wasm32-unknown-unknown`, to Apple libm on macOS, to musl on `*-linux-musl` | Rust std docs ("varies by platform, Rust version"); Gladman Table 3 |
| **D18 / m5-03** tiles byte-identical | — | Same exposure through `log2` (zoom choice) and `powi(2)` in a tuple that is then sorted (`tiles.rs` l. 322) | grep |
| **D15 / m1-14** seeded order, seeded relabelling | Philox (key = seed, counter = id); Mt-KaHyPar 256 fixed chunks (kernels report) | **Level**: lab keys draws by (seed, node) via SplitMix64 (`m1-02 main.rs` l. 111–125) and layout by (seed, parent, tile) (`layout.rs` l. 428) — counter-based in practice. **Behind** in policy: not written; Leiden's sub-round priority hash is unseeded (`x.wrapping_mul(0x85EB_CA6B)`), so seeds do not vary it — matters for consensus across seeds (m9-10) | lab code; Salmon § 5 |
| **ADR 0008** (`fastrand` seeded from a thread-local RNG in `ptr_hash`) | rand book: `StdRng`, `SmallRng` non-portable; never sample `usize` | **Fixed for that crate; no general guard.** No `deny.toml` or clippy rule bans entropy-seeded RNGs or `RandomState` hash maps | `deny.toml`, `crates/*/clippy.toml` |
| **D2 / D22** portable core, wasm32 | Wasm 3.0 deterministic profile; relaxed SIMD | **Level if two flags are kept**: never build with `+relaxed-simd` (FMA "fused or not", min/max on NaN/±0 "implementation-defined"); canonicalize NaN before hashing/writing (full profile, which browsers run, leaves NaN bits open) | relaxed-SIMD overview; profiles appendix |
| **D1** no C/C++ in the core | CORE-MATH is C (MIT); pure-Rust CR libms immature (metallic: CR claimed, "falls back to a slower path with double rounding" without FMA → target-dependent; yFoil found an `atan2` bug, unverified) | **Constraint holds**: identity does not need correct rounding, only *one* implementation compiled for every target. Pin the pure-Rust `libm` crate (bits identical wherever SSE2/NEON/wasm, inference from its source: no target-dependent float paths except i586) | `libm` 0.2.16 `src/math/arch/mod.rs`; metallic README |
| **D19** Python wheel | Dolan-Gavitt 2022: a shared library linked with `-ffast-math` sets FTZ/DAZ for the whole process on load; 2,514 PyPI packages depend on one that did | **Unguarded**: a user's other import can change our subnormal handling, and — MXCSR being per thread — make the calling thread (sequential path) differ from rayon workers | search summary; challenge 9 |
| **D3 / D9** f64 by default; weights per entry | Ahrens § 6.2 (binned capacity 2^64 summands for double) | **Level**; f32 weights summed in f64 are exact only while the fixed-point scheme or integers hold | — |
| **m3-09** HyperBall, sampled betweenness | — | HLL max is integer (fine); the estimator's small-range correction and the sample-size formula use `ln`/`log2` — must be the pinned math or integer logs | issue text |
| Novelty of "deterministic for any budget" | Hallberg (decomposition-invariant sums), Müller (data independence) | **Concept not new** in HPC and databases; **new for graph engines** as far as read | § 5 |

### 3.2 Does rustc ever contract or reassociate? (checked here)

`rustc 1.100.0` (LLVM 23.1.5), `-O`, a five-function file (axpy loop `a*x[i]+y[i]`, dot
product loop, explicit `mul_add`, `iter().sum()`, `sin`), assembly inspected
(`/home/tux/graphlab/scratch`, deleted):

| Target | Result |
|---|---|
| x86-64, `-C target-cpu=znver4` | **one** `vfmadd213sd`, in `mul_add` only; the loops use `vmulpd`/`vaddpd`/`vaddsd`; the sum is a sequential `vaddsd` chain (no reassociation) |
| aarch64, `-C target-cpu=neoverse-v2` | **one** `fmadd`, in `mul_add` only |
| wasm32 | `mul_add` → `call fma` (software, correctly rounded); `sin` → `call sin` (the `libm` crate) |
| i686 | SSE2 `mulsd`/`addsd`; `fma` and `sin` are calls into the C library whose results come back through x87 (`fstpl`) |

So: target-cpu flags are safe; FMA appears only where written. This agrees with RFC 3514
(strict IEEE rules out turning `a*b + c` into FMA; fast variants are opt-in) and with the
std docs: `mul_add` is "guaranteed" fused; the new `mul_add_relaxed` (unstable) "is either
the result of `mul_add` … or of `self * a + b`"; `algebraic_*` allow reassociation. The
residual risks are code that chooses float paths by `cfg(target_feature)` or run-time
detection (hand SIMD, crates such as metallic), and i686's C-library calls.

### 3.3 Lifting m4-04's integer-weight restriction: options and costs

Where order enters Leiden with real weights: σ_c updates in the commit (atomic f64 add,
order = schedule); quotient weights in aggregation (order within a key = bucket arrival,
which depends on bucket boundaries; the lab sorts with `sort_unstable`, `seqagg.rs`
l. 578–646); totals 2m and Q (fixed blocks today). k_in per candidate community is summed
in adjacency order and is already deterministic.

| Option | Identity across threads, budgets, regimes | Cost on Friendster level 0–1 (paper numbers + arithmetic, extrapolated) | Risk |
|---|---|---|---|
| A. Canonical order everywhere (stable sort by (key, source position); sorted commits) | yes, if every path keeps it | position field +4 B per bucket record; every future aggregation (m4-03) and the commit must preserve order; Müller: order-by-sort 7× CPU in MonetDB | fragile: one optimization breaks it silently |
| B. **Fixed-point i64**: w → round(w·2^s), s chosen once from the exact total T of all entry weights so that T·2^s ≤ 2^62 | yes, by construction (integer sums exact in any order) | conversion once per entry at level 0; i64 `fetch_add` instead of an f64 CAS loop (x86 has no atomic float add — cheaper, inference); 64-bit weight in a bucket record 16 B vs the lab's 12 B (`w: u32`): +33% bucket bytes, the same as any f64 weight (lab spill 20.2–26.8 GB → ~27–36 GB, arithmetic) | quantization: every weight is a multiple of 2^-s ≤ 2T·2^-62; worst-case total error ≤ (entries)·2^-(s+1) ≈ 3.6·10^9 · T·2^-62 ≈ 2^-30 T, against the f64 recursive-summation bound n·u·T ≈ 2^-21 T — a ~500× tighter bound (arithmetic). Weights below 2^-s round to 0 (only if the dynamic range relative to T exceeds 2^62) |
| C. Binned (ReproBLAS) σ_c and per-key sums | yes | 48 B per accumulator: σ for 65.6 M Friendster nodes = 3.15 GB vs 0.52 GB (arithmetic) — against O(n) state of 21–53 B/node (FINDINGS § B2); 1.9–2.4× for buffered group sums (Müller), 4–12× unbuffered | doubles O(n) state; breaks the 8 GiB case |
| D. i128 or Hallberg two-limb i64 with deferred carry | yes, range ~2^126 | 16 B per σ (1.05 GB); limbs make atomics portable (wasm32 has i64 atomics, not 128-bit) | only needed if a real input needs > 2^62 dynamic range |

**Recommendation: B**, with D as the fallback the planner chooses when T and the smallest
non-zero weight do not fit 62 bits (or refuses). For integer weights with T < 2^62,
s ≥ 0 and every weight is exact: the unweighted and integer cases keep today's bits
(scaling by 2^s is exact in every product of the gain, so comparisons are unchanged). The
gain stays an f64 expression of exactly converted integers (`as f64` rounds to nearest,
deterministic). It also makes m4-02's "modularity equal across aggregation" and the map's
"exact numbers at every level" true for weights: parent total = sum of children, exactly.

## 4. Challenges, ranked

| # | Change | Expected gain (source) | Cost and risk vs promises | Cheapest experiment | Issue |
|---|---|---|---|---|---|
| 1 | **Fixed-point i64 weights for every schedule-dependent sum** in community detection and map aggregates (σ, quotient weights, L(c), S(c), level totals); scale from the exact total T (§ 3.3, option B); `weight_scale` recorded in provenance and in the result | Lifts m4-04's integer-only identity; makes m4-02's aggregation-equality acceptance attainable; integer sums cost what float sums cost (Müller Fig. 4; Hallberg § 3); binned alternative 1.9–2.4× and +2.6 GB on Friendster | ~150 lines (scale choice, conversion, i64 atomics, docs); no format change; promise *strengthened*. Risk: quantization semantics must be documented; inputs with > 2^62 dynamic range need option D or a refusal | Lab `m1-02-ooc-leiden` with random real weights (log-uniform over 2^-20…2^20) on orkut: hash at 1/4/16 threads and under 100%/50% of the edge array; Q and time vs the integer-weight path, interleaved A/B; then Friendster under 16 and 8 GiB | m4-02 (specify scheme), m4-04 (drop restriction), m4-01, m4-03 |
| 2 | **Write the rule list** (below) into m3-11 and enforce R2, R3, R8, R9 by clippy `disallowed-methods`/`-types` in the shared portable list (m2-18) and `deny.toml` | Turns "no platform maths" into checked rules; the four lab violations of § 3.1 would have failed CI | ~1 day; zero runtime cost | Add the lists; confirm they flag `layout.rs` l. 139 and `generators.rs` l. 282 | m3-11, m2-18 |
| 3 | **Order probes + adversarial weights in the determinism CI**: an FPRev-style probe (all ones, one +M and one −M) run through every reduction primitive and per-node accumulator at threads {1, 2, 3, 8, 32 on 4}, chunk sizes {64, 65,536}, every regime and two budgets; the revealed tree must equal the specified one. Plus weighted tier-1 graphs with wide-range weights for Leiden/PageRank hashes | Detects order changes that hashes on unweighted graphs cannot (integer sums exact in any order); FPRev: n = 8,192 in ~1 s | Test code only; O(n²) probe calls, keep n ≤ 1,024 | Probe `core::par::map_ranges` fold and the PageRank pull accumulator first | m3-11, m0-05 (#5), #44 |
| 4 | **One pinned math module** (`everygraph_core::math`, wrapping an exact-pinned `libm` version, or own code for the few functions needed) for every transcendental in result-bearing code; `x*x` for `powi(2)`; precomputed constants for `0.1^iter`; `ilog2`/`leading_zeros` for integer decisions; golden vectors compared bit-for-bit on x86-64, aarch64 (qemu-user on the self-hosted runner), i686, wasm32 (Node) | Native↔wasm↔ARM identity of layout, tiles, Leiden tolerance, HyperBall estimator, sample sizes; also identity across glibc versions and musl/gnu targets (CORE-MATH slide 3; Gladman § 1 on run-time dispatch). Arteaga: portable versions geomean 0.83× platform time | Small; slower than glibc for some functions, irrelevant at our call counts (layout: per node, not per edge). Accuracy: `libm` is < 1 ulp but not CR, and its `pow` comes from FreeBSD `e_pow.c`, whose library shows 636 ulp (Gladman Table 3) — inference that ours shares it: avoid `pow` | Golden-vector test: 10^5 inputs per function, four targets | m3-11, m5-01, m5-03, m3-09, m4-01, m0-05 |
| 5 | **Counter-based RNG policy**: one `core::rng(seed, purpose, counter) -> u64` (Philox4×32-10, ~40 lines, integer-only), key schema per kernel (purpose tag, entity ids, level, iteration, draw index — never thread id, chunk index, budget, or a stream advanced in schedule order); frozen reference vectors; floats as `(x >> 11) as f64 · 2^-53`; bounded integers by 64-bit multiply-shift on `u64`, never `usize`; ban `thread_rng`, `rand::rng`, `StdRng`, `SmallRng`, `fastrand`, `getrandom` in result crates | Every randomized step (relabel, visiting order, refinement θ, betweenness samples, LPA ties, layout jitter) independent of threads and regime by construction (Salmon § 5); seeds become meaningful for consensus (m9-10) | Tiny; Philox 3.6 cpB (Salmon Table 2) | Reference-vector test on four targets; Leiden hash at two seeds differs, at one seed equal across threads | m3-11, m4-01, m1-14, m3-09, m4-09, m5-01 |
| 6 | **Exact accumulator for global scalars** (2m/T, Q, dangling mass, residuals, totals in map tiles): Neal's small superaccumulator or a 3-limb Hallberg fixed point, mergeable per chunk in any order | Removes the PR #131 bug class; reduction blocks stop mattering for scalars. Cost ≪ 1% of a PageRank iteration: O(n) values per iteration at "< 2×" a plain sum (Neal) vs 3.2 s/iteration on Friendster (arithmetic estimate) | ~100 lines, test against `xsum` as a dev-dependency oracle | Dangling-mass sum: bits at chunk 64 vs 65,536 vs 1 thread (today needs fixed blocks) | m3-04, #44 |
| 7 | **Edge-map accumulation contract (#44, in development)**: per-node accumulator spans decode batches and budget-sized I/O buffers; SIMD lanes fixed in source (no `cfg(target_feature)` choice of width); push/pull/propagation-blocking variants interchangeable by the planner only if probe-equal (kernels challenge 5 says PB can reproduce pull's order); `Iterator::sum` banned for floats in result code (it starts from −0.0: an empty list gives −0.0, a manual loop +0.0) | Keeps D11's three regimes bit-identical for PageRank-like kernels without exact accumulators | Spec text + probe tests | Probe a hub whose list spans two batches, in-RAM vs streamed at two budgets | #44 (m3-01), m3-04 |
| 8 | **Canonical floats in results and `.eg` (#170, in development)**: NaN → 0x7FF8…0, −0.0 → +0.0 in every result/derived section and in the result hash; `total_cmp` instead of `max/min` where ±0 can tie (`f64::max` "may return either input non-deterministically"); provenance keys `seed`, `numerics`, `weight_scale`; shortest-round-trip float text | NaN sign is non-deterministic in Rust (RFC 3514), x87 returns alter payloads, wasm full profile leaves NaN bits open; content hashes would differ for equal results | One paragraph in the spec now (free before v1.0); a helper in core | Hash a result with −0.0/NaN produced on x86 and wasm | #170 / m2-01, #37, #39, m3-11 |
| 9 | **FTZ/DAZ guard in the Python module**: at kernel entry test `black_box(f64::MIN_POSITIVE) / 2.0 != 0.0` on the calling thread and once per worker; refuse (or reset in scope) and say why | A user's `-ffast-math` import cannot change bits, or make the sequential path differ from the parallel one (MXCSR is per thread) | ~20 lines, no `unsafe` needed for the probe | Load a test `.so` linked with `-ffast-math` in CI, run PageRank on a graph with subnormal weights | m6-01, D19 |
| 10 | **Supported-platform list in the promise**: x86-64 (any `target-cpu`), aarch64, i686 with SSE2, wasm32 without `relaxed-simd` (compile error if enabled); i586/x87 and 32-bit ARM NEON float SIMD excluded (RFC 3514 non-compliance) | Makes "any platform the release supports" (api-sketch § 2.11) a checkable list | Docs + one `compile_error!` | — | m3-11, D22, m8-08 |
| 11 | (Low) **Fixed-point PageRank** only if propagation blocking cannot reproduce pull's association | Order-free ranks for push, pull, PB, GPU | Precision: 2^-62 absolute vs f64's relative; ranks ~0.15/n keep ~33 bits (arithmetic) — below NetworkX's f64; unmeasured cost | Lab A/B on Friendster, s/iteration, interleaved | m3-04 (only if needed) |

### The rule list for m3-11 (R1–R15)

| Rule | Statement | Enforced by |
|---|---|---|
| R1 | Result-bearing arithmetic uses only + − × ÷ `%` `sqrt`, explicit `mul_add`, `as` casts, comparisons (IEEE-exact in Rust, RFC 3514) | review; R2–R3 lints |
| R2 | No std transcendental (`sin cos tan asin acos atan atan2 sin_cos exp exp2 exp_m1 ln log log2 log10 ln_1p powf powi cbrt hypot sinh cosh tanh asinh acosh atanh gamma ln_gamma erf erfc`, f32 and f64) outside `everygraph_core::math`; `powi(2)` → `x*x` | clippy `disallowed-methods` (shared list, m2-18) |
| R3 | No `mul_add_relaxed`, `algebraic_*`, fast-math intrinsics, `std::arch` FMA/approximate (`rsqrt`, `rcp`) intrinsics, or run-time CPU dispatch (`is_x86_feature_detected!`) in float code; no `cfg(target_feature)` choice of float algorithm or lane count | clippy `disallowed-methods`/`-macros`; review |
| R4 | Every reduction tree is a function of the data and source constants only — never of thread count, budget, regime, I/O buffer, decode batch or SIMD width | FPRev probes (R15b) |
| R5 | Sums whose order depends on the schedule (atomics, hash or bucket aggregation, push contributions) are integer or fixed-point, never float | review; weighted hashes (R15a) |
| R6 | Global scalars use the exact accumulator | review; probe |
| R7 | No `Iterator::sum`/`product` on floats in result code (−0.0 start); `total_cmp` where ±0 or NaN can tie | a `core` float-fold helper; review (clippy cannot ban `Iterator::sum` for floats only) |
| R8 | No `HashMap`/`HashSet` with a randomly seeded hasher (`std` `RandomState`, hashbrown/foldhash, ahash defaults) whose iteration order reaches a result; fixed-seed hashers or sorted output | clippy `disallowed-types`; `deny.toml` |
| R9 | All randomness from `everygraph_core::rng` with a documented key schema; no entropy-seeded or stateful-stream RNG; no `usize` sampling; no `rand_distr` float distributions | `deny.toml` bans (`fastrand`, `getrandom`, `rand` outside dev-deps); clippy |
| R10 | NaN and −0.0 canonicalized in results, derived sections and result hashes; results never NaN by spec (graph-semantics) | helper + test |
| R11 | Supported targets as challenge 10; `relaxed-simd` → `compile_error!` | build matrix |
| R12 | FTZ/DAZ probe at entry from foreign hosts (Python) | test with a `-ffast-math` library |
| R13 | Text ↔ float: `str::parse` (correctly rounded) and `{}` (shortest round-trip); never `{:.N}` in cache keys or provenance | review |
| R14 | Pinned versions: `libm` exact version and rule-list version (`numerics=N`) in provenance; other versions are not promised (api-sketch § 2.11) | `Cargo.toml` `=x.y.z`; provenance test |
| R15 | Tests: (a) result hashes at 1, 2, 3, 8, 32-on-4 threads × regimes × two budgets × mask/materialized, on **weighted** graphs with wide-range weights as well as unweighted; (b) order probes; (c) math golden vectors on four targets; (d) RNG reference vectors; (e) native vs wasm32 by full-output hash, not by sum and maximum | CI (self-hosted runner; qemu-user for aarch64; no paid minutes) |

## 5. Ideas we have that the literature does not — checked

| Claim | Verdict | Who did it before |
|---|---|---|
| Same bits for any **memory budget and regime** (in RAM, mmap, streamed; view masked or materialized) | **New for graph engines** as far as read. The concept equals decomposition-invariance in HPC and data independence in databases | Hallberg–Adcroft (processor count and decomposition), Müller (physical order), ReproBLAS (any partition) |
| Fixed-point gain totals in deterministic Leiden (lab) | **Not new as technique** (Hallberg N = 1); new in Leiden as far as read | Hallberg 2014 |
| Fixed-point weights making modularity equal across aggregation levels (challenge 1) | **New application**, not found | — |
| Bit-identical native ↔ wasm32 results | **Not new as concept** (Wasm deterministic profile, for blockchains and replay); our case is harder: browsers run the full profile, so NaN canonicalization and no relaxed SIMD are on us | Wasm 3.0 |
| Seeded per-entity draws (relabel, layout) | **Not new** (counter-based RNG, Salmon 2011) | Salmon; Mt-KaHyPar per kernels report |
| Order-revealing CI for a graph engine's reductions | **New use** of FPRev's method; FPRev targets numerical libraries | Xie et al. 2025 |

## 6. What to ignore, and why

- Kulisch hardware accumulators, IEEE 754-2019 augmented operations, Tensor-Core multi-term adders — no portable access from Rust/wasm.
- Binned (ReproBLAS) accumulators for per-community state — 48 B each; fixed point does the job at 8 B (§ 3.3).
- MPFR, double-double, posits/unums, decimal FP — accuracy or formats we do not need for identity.
- CORE-MATH in the core — C (D1); revisit only through a pure-Rust port that is CR on every target without FMA.
- Correct rounding as a goal in itself — identity needs one implementation, not the correct one; CR matters only for agreement with other tools.
- Stochastic-rounding tools (Verrou, Verificarlo) and Varity/pLiner — sensitivity and diff tools, not identity; maybe for m9-10 later.
- Ozaki scheme, reproducible BLAS3 and Krylov solvers — no dense linear algebra in the plan.
- GPU layout identity (m9-05) — WGSL float rules not read here; treat GPU output as outside the promise or make it integer.
- Squares RNG — faster than Philox by its author's claim, but newer and less established; RNG speed is not our bottleneck.
- Goodrich–Eldawy exact parallel summation (PRAM/MapReduce) — superseded for us by Neal / fixed point.
