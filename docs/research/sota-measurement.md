# State of the art: measurement — statistics, noise, drives and public comparisons

Second wave, area W3 of `sota-coverage.md` § (d) (topics 3 and 7 of § (c)). Written
2026-11-25/26 by an Opus research agent. Nothing was built, run or loaded. The only
measurements of the machine taken were cheap counter reads: `/proc/diskstats`, `/sys`,
hwmon and the fstrim timer. Two committed official result files were re-analysed offline
(§ 3.1). Papers are in `/home/tux/graphlab/papers/measurement/` (PDFs) and its `txt/`
(pdftotext output).

**Read this first: two findings touch work in flight.**

- **ADR 0026 / #35 (promise 1).** The lab drive's "~10 GB burst, then 1.4–1.6 GB/s" is
  probably a property of how this host uses the drive, not of the drive model. A review of
  the same model reports ~240 GB of fast-write region when the drive is empty. On this
  host, ext4 is mounted without `discard` and trimmed weekly, while ~0.6 TB/day is written
  (§ 3.2, challenge 1).
- **The `.eg` spec (PR #170) and the edge-map framework (#44).** Both are cheap places now
  to record two things every honest graph benchmark must report: vertex ordering and
  isolated-vertex counts, and work counters (edges traversed, bytes decoded) (§ 4,
  challenges 6 and 7).

## 1. Scope and reading log

**Scope**
- Repetition and CI methodology, measurement bias, ordering and interleaving.
- CPU frequency, SMT and thermal effects; SSD burst vs steady state; write amplification
  and wear.
- Faults in graph benchmarks, LDBC/GAP rules, and generator validity.
- Our side: `crates/everygraph-bench` (gate, disturbance, warm-up, report, compare),
  `bench/specs/headline.toml`, `bench/README.md`, CLAUDE.md § 3 and § 6, ADR 0026,
  FINDINGS § B1 and § Idle-machine re-runs, and issues m0-03, m3-12, m3-14, m8-03, m3-02,
  m2-08, m4-04, m1-10, m0-04 and m3-13.
- Not redone: the earlier survey's LDBC Graphalytics algorithm definitions
  (`memory-bounded-algorithms.md` § D.17), or anything in the first-wave reading logs.

**Who read what**
- Read levels marked (me) are my own reads.
- (helper) means read in full by a helper agent whose notes I checked: I grepped a
  sample of 12 quotes and numbers against the text, and all matched.
- (helper²) means read by a helper's sub-helper; that agent re-checked 21 quotes.

| # | Work | Venue, year | Verified link | Read level | PDF (`papers/measurement/`) |
|---|---|---|---|---|---|
| 1 | Hoefler, Belli: Scientific benchmarking of parallel computing systems | SC 2015 | doi.org/10.1145/2807591.2807644 | full text (me) | 2015-hoefler-scientific-benchmarking.pdf |
| 2 | Kalibera, Jones: Rigorous benchmarking in reasonable time | ISMM 2013 | kar.kent.ac.uk/33611 | full text (me) | 2013-kalibera-rigorous-benchmarking.pdf |
| 3 | Laaber, Scheuner, Leitner: Software microbenchmarking in the cloud | EMSE 24(4) 2019 | doi.org/10.1007/s10664-019-09681-1 | full text except online appendix (me) | 2019-laaber-microbenchmarking-cloud.pdf |
| 4 | Abedi, Brecht: Repeatable experiments in highly variable clouds (RMIT) | ICPE 2017 | doi.org/10.1145/3030207.3030229 | full text (me) | 2017-abedi-rmit.pdf |
| 5 | Maricq et al.: Taming performance variability | OSDI 2018 | usenix.org/conference/osdi18/presentation/maricq | §§ 2, 4–5, 7 (me); § 6 skimmed | 2018-maricq-taming-variability.pdf |
| 6 | Duplyakin et al.: Avoiding the ordering trap | USENIX ATC 2023 | usenix.org/system/files/atc23-duplyakin.pdf | full main text (me) | 2023-duplyakin-ordering-trap.pdf |
| 7 | Mytkowicz et al.: Producing wrong data without doing anything obviously wrong | ASPLOS 2009 | doi.org/10.1145/1508244.1508275 | full text (helper) | 2009-mytkowicz-wrong-data.pdf |
| 8 | Curtsinger, Berger: Stabilizer | ASPLOS 2013 | people.cs.umass.edu/~emery/pubs/stabilizer-asplos13.pdf | full text (helper) | 2013-curtsinger-stabilizer.pdf |
| 9 | Georges, Buytaert, Eeckhout: Statistically rigorous Java performance evaluation | OOPSLA 2007 | buytaert.net/files/oopsla07-georges.pdf | prose full, figures garbled (helper) | 2007-georges-statistically-rigorous.pdf |
| 10 | Barrett et al.: Virtual machine warmup blows hot and cold | OOPSLA 2017 | arxiv.org/abs/1602.00602 | main text full, appendix skimmed (helper) | 2017-barrett-vm-warmup.pdf |
| 11 | Chen, Revels: Robust benchmarking in noisy environments | arXiv 2016 | arxiv.org/abs/1608.04295 | full text (helper) | 2016-chen-robust-benchmarking.pdf |
| 12 | van der Kouwe et al.: Benchmarking crimes | arXiv 2018 | arxiv.org/abs/1801.02381 | full text (helper) | 2018-vanderkouwe-benchmarking-crimes.pdf |
| 13 | Bulej et al.: Duet benchmarking | ICPE 2020 | arxiv.org/abs/2001.05811 | full text (helper) | 2020-bulej-duet.pdf |
| 14 | Raasveldt et al.: Fair benchmarking considered difficult | DBTest 2018 | doi.org/10.1145/3209950.3209955 | full text (helper) | 2018-raasveldt-fair-benchmarking.pdf |
| 15 | McSherry, Isard, Murray: Scalability! But at what COST? | HotOS 2015 | usenix.org (hotos15-paper-mcsherry.pdf) | full text (helper) | 2015-mcsherry-cost.pdf |
| 16 | Schöne et al.: Energy efficiency aspects of AMD Zen 2 | IEEE Cluster 2021 | arxiv.org/abs/2108.00808 | full text (helper) | 2021-schone-zen2-energy.pdf |
| 17 | Acun, Miller, Kale: Variation among processors under Turbo Boost | ICS 2016 | charm.cs.illinois.edu/newPapers/16-08/paper.pdf | abstract/results only (me) | 2016-acun-turbo-variation.pdf |
| 18 | Mehrotra, Anand et al.: SoK: the faults in our graph benchmarks | arXiv 2024 | arxiv.org/abs/2404.00766 | full text (me) | 2024-mehrotra-sok-graph-benchmarks.pdf |
| 19 | Meng et al.: Revisiting graph analytics benchmark | SIGMOD 2025 (PACMMOD 3(3)) | doi.org/10.1145/3725345 ; arxiv.org/abs/2506.21811 | main text full; appendix online only, not read (helper) | 2025-meng-revisiting-graph-analytics-benchmark.pdf |
| 20 | LDBC Graphalytics v1.0.5 (rules only) | arXiv 2023 | arxiv.org/abs/2011.15028 | process/rules sections (helper) | 2023-iosup-graphalytics-spec.pdf |
| 21 | Beamer, Asanović, Patterson: The GAP benchmark suite | arXiv 2015/2017 | arxiv.org/abs/1508.03619 | full text (helper) | 2015-beamer-gap.pdf |
| 22 | Bläsius, Fischbeck: External validity of average-case analyses | ESA 2022 | arxiv.org/abs/2205.15066 | main text full (helper) | 2022-blasius-external-validity.pdf |
| 23 | Seshadhri, Pinar, Kolda: In-depth analysis of stochastic Kronecker graphs | arXiv 1102.5046 (journal version not verified) | arxiv.org/abs/1102.5046 | results full, proofs skimmed (helper) | 2013-seshadhri-kronecker.pdf |
| 24 | Haas, Lee, Bonnet, Leis: SSD-iq | PVLDB 18(11) 2025 | vldb.org/pvldb/vol18/p4295-haas.pdf | §§ 1–4 full (me); latency and end-to-end skimmed | 2025-haas-ssd-iq.pdf |
| 25 | Lee, Ziegler, Leis: How to write to SSDs | PVLDB 19(7) 2026 | vldb.org/pvldb/vol19/p1469-lee.pdf ; arxiv.org/abs/2603.09927 | intro, background, death-time grouping, NoWA (me); evaluation skimmed | 2026-lee-how-to-write-ssds.pdf |
| 26 | Maneas et al.: SSDs in enterprise storage, a field study | FAST 2022 | usenix.org/system/files/fast22-maneas.pdf | full text (helper²) | 2022-maneas-ssd-field-study.pdf |
| 27 | He et al.: The unwritten contract of SSDs | EuroSys 2017 | doi.org/10.1145/3064176.3064187 | full text (helper²) | 2017-he-unwritten-contract.pdf |
| 28 | SNIA SSS Performance Test Specification v2.0.2 | SNIA 2020 | snia.org/sites/default/files/2025-02/SNIA-SSS-PTS-2.0.2.pdf | definitions and reporting only (helper) | 2025-snia-pts-2.0.2.pdf |
| 29 | Viglas: Write-limited sorts and joins for persistent memory | PVLDB 7(5) 2014 | vldb.org/pvldb/vol7/p413-viglas.pdf | full text (helper²) | 2014-viglas-write-limited.pdf |
| 30 | Blelloch et al.: Sorting with asymmetric read and write costs | SPAA 2015 | arxiv.org/abs/1603.03505 | full text (helper²) | 2015-blelloch-asymmetric-sorting.pdf |
| 31 | Jiao, Bhimani, Kim: Wear leveling in SSDs considered harmful | HotStorage 2022 | doi.org/10.1145/3538643.3539750 | slides only (ACM PDF blocked) | 2022-jiao-wear-leveling-harmful-slides.pdf |
| 32 | Balaji, Lucia: When is graph reordering an optimization? | IISWC 2018 | doi.org/10.1109/IISWC.2018.8573478 | abstract only (the downloaded file is the talk slides, not read) | 2018-balaji-reordering-slides.pdf |
| 33 | Samsung 990 PRO datasheet Rev. 1.0 | vendor, 2022 | download.semiconductor.samsung.com (990 PRO datasheet) | TBW rows opened | 2022-samsung-990pro-datasheet.pdf |
| 34 | Tom's Hardware, Samsung 990 Pro review, p. 2 | review, not peer-reviewed | tomshardware.com/reviews/samsung-990-pro-ssd-review/2 | cache sentences extracted | — (web page) |
| 35 | Beyer et al.: Reliable benchmarking (BenchExec) | STTT 2019 (by name) | not verified | **not accessible**: the publisher returned HTML and the search budget was exhausted | — |

**Budgets.** No OpenAlex requests were used. About 25 web searches were used, and the
session's shared search budget ran out near the end. That is why item 35 is unverified.

## 2. The state of the art, as a short map

**Statistics and repetition**

| Question | Best-supported answer | Who, with numbers |
|---|---|---|
| Which centre and interval? | Nonparametric: the median, with a rank-based CI. Performance data is rarely normal. | Hoefler–Belli (rule 5–7): Le Boudec ranks; "n > 5 measurements are needed". Maricq: normality rejected for 710 of 713 configurations (CloudLab, ~900k points). Raasveldt: median plus a quantile CI. Dissent: Chen–Revels prefer the minimum (additive-noise model, no CPU or platform stated). |
| How many repetitions? | Enough for a CI of a set width. Dimension once per platform. **Repeat at the highest level that varies** (build → process → iteration). | Kalibera–Jones: no counts below 5 at the top level. Barrett: diminishing returns from 6 process executions, "all but identical" at 10. Maricq: CoV 1% needs 12 runs for a 1%/95% CI; CoV 5% needs 121; CoV 8.1% needs 670. |
| How to compare A and B? | A CI of the ratio or effect size, not a significance test or fixed threshold. | Kalibera–Jones: Fieller interval for the ratio. Georges: fixed thresholds mislead in up to 16% of comparisons (startup). Laaber: Wilcoxon or bootstrap CIs; ~40% false positives on bare metal with 1 instance × 1 trial (bleve-3). |
| In which order? | **Randomized multiple interleaved trials (RMIT)**: rounds, with variants shuffled in each round. | Abedi–Brecht: consecutive trials "falsely report" 37.8% between identical systems (EC2 CPU traces). Duplyakin: on 1,700 bare-metal servers, fixed vs random order differ by 50% or more; 72% of configurations could change a conclusion; 94% of artifacts use one fixed order. Laaber: same-instance randomized trials detect slowdowns ≤ 10%. |
| Bias from the setup | Layout bias (environment size, link order) of a few % flips 5–10% conclusions. Randomize the setup. | Mytkowicz: perlbench O3/O2 0.92–1.10 over 33 link orders (Core 2). Curtsinger: up to 57% from link order. Kalibera: small at reference sizes (≤ 1.7% CoV, 3.3–6.8% shifts). |
| Warm-up | A fixed discard count is no guarantee. Inspect run-sequence and independence once per system. | Barrett: only 30–43.5% of ⟨VM, benchmark⟩ pairs reach steady state consistently, and C (GCC) rows are also inconsistent. Kalibera: heuristics err by tens of %. |
| Shared hosts | Measure A and B under the same interference: interleave; duet only when interference is real. | Bulej: duet shrinks the 99% CI 5.03× (ScalaBench) and 37.4× (SPEC CPU2017) on cloud VMs, but gives no gain on idle bare metal. Maricq § 7.5: avoid shared hosts if you can; otherwise many more repetitions. |

**Platform effects**

| Effect | Evidence | Note for us |
|---|---|---|
| Boost frequency differs between nominally identical chips | Acun: up to 16% between processors of the same model (Edison/Cab/Stampede); < 1% without dynamic overclocking | A laptop number belongs to one unit, not to the model. |
| SMT and frequency coupling on Zen | Schöne (Zen 2 EPYC): an idle sibling thread set to a higher P-state lifts the active core 1.5 → 2.5 GHz; frequency changes wait for a 1 ms window; RAPL "should be considered inaccurate" | CPU share is a weak disturbance proxy; effective frequency is a direct one. |
| Thermal limits on laptops | **No peer-reviewed laptop study found.** The nearest are edge-device and phone studies (not read). | We have to measure it ourselves (m8-03). |

**Drives**

| Effect | Evidence | Note for us |
|---|---|---|
| Steady state needs a defined drive state | SNIA PTS: purge, then precondition until 5 rounds have range < 20% and slope < 10% (≤ 25 rounds). SSD-iq: erase, then a sequential fill; without it, convergence takes > 4 h (> 6 drive writes). | Our warm-up is a weak form of preconditioning. |
| Earlier runs affect later ones for weeks | Maricq § 7.4: one SSD shows a periodic pattern over months despite `blkdiscard` before every run | Drive state is an uncontrolled variable. |
| WAF | Field (Maneas, ~2 M enterprise SSDs): p10 = 2; 98.8% of drives above 1.3. Lab (SSD-iq): a single sequential write front gives low WAF only with zones > 1 GB; several fronts "worsen significantly". Lee: if all zones in an active group die together, SSD WAF = 1 even when writes are multiplexed. | Spill files deleted together are the friendly case (inference). |
| Endurance | Datasheet: 990 PRO 600 TBW (1 TB), 1,200 TBW (2 TB). SSD-iq: one Intel D7 P5520 ran to read-only after > 4000 drive writes (8 PB host, 26 PB physical, WAF 3.2), double its rating. | § 3.3 |

**Graph benchmarks**

| Fault or rule | Source and number |
|---|---|
| Vertex ordering changes times | SoK: degree order ≈ 40% faster PageRank on twitter-2010 (Galois/Gemini/GraphChi); different orderings must never be compared |
| Zero-degree vertices | SoK: BFS from a zero-degree root is 6–7× (Galois) to ~10× (Gemini) "faster"; removing them gives up to 13% on PageRank/CC and > 30% on triangles at R-MAT scale ≥ 23. Friendster is quoted as 124 M or 65 M nodes. Seshadhri: Graph500 Kronecker graphs are 51% (ℓ = 26) to 74% (ℓ = 42) isolated. |
| PageRank metric undefined | SoK: 55% of uses time a fixed number of iterations, 20% time to convergence, 25% unspecified |
| Single-thread baseline | McSherry: COST of 16 cores for Naiad, 512 for GraphLab, unbounded for GraphX on PageRank (twitter_rv, laptop) |
| What is timed | GAP: load and build excluded; kernel-specific conversion or relabelling included; 64 BFS sources (non-zero degree). Graphalytics: load T_l, makespan T_m and processing T_p reported separately; 3 runs; f64; validation; timeouts. |
| Generators | Bläsius–Fischbeck: GIRG/Chung–Lu predict BFS, cliques, diameter and vertex cover on 2,740 real graphs, but not Louvain iterations. Meng: FFT-DG controls scale, density and diameter, not the degree distribution. SoK: use Smooth Kronecker, not R-MAT. |
| Usability score | Meng: LLM scores against humans ρ = 0.75 / 0.714, 7 platforms, code never executed. Weak. |

## 3. Decision by decision

### 3.1 The harness (#4 / m0-03) against the literature

| Aspect | Ours (`bench/README.md`, `harness/*.rs`) | Best known | Verdict |
|---|---|---|---|
| Isolation and resource accounting | cgroup per repetition, hard `MemoryMax`, `memory.peak`, OOM events, subreaper I/O, left-process kill, spec and binary hashes | BenchExec (by name, unverified) also uses cgroups; none of the papers read accounts others' activity during the run | **ahead** of every paper read |
| Disturbance | others' cores > 0.5 or I/O > 50 MB/s, in settle windows and during the run | Barrett: APERF/MPERF frequency check (1 ± 0.03), plus a temperature return to ±3 °C before each process | level; misses frequency and temperature |
| Repetitions | 3 per case (official), 1 warm-up; lj/orkut quoted over 6 (two runs) | ≥ 5–6 at the top level (Hoefler, Kalibera, Barrett); adaptive until CI width ≤ x (Hoefler, Georges, Maricq) | **behind** |
| Summary | "median (min–max)" | median with a rank CI at a stated level | **behind**, and the fix is small: with *n* = 3, [min, max] is exactly a 75% CI of the median (1 − 2·0.5³); with *n* = 6 it is 96.9%; with *n* = 10, [x₍₂₎, x₍₉₎] is 97.9% (binomial arithmetic). We print the interval but not its confidence. |
| Levels | `glab run` reports the best of 2 in-process iterations; the harness takes the median of those over 3 processes | Kalibera: name the levels and repeat at the top; Georges: best-of misleads | **behind**: a mixed statistic, stated in the spec comment only |
| Order | cases in spec order; repetitions of a case back to back (multiple consecutive trials) | RMIT (Abedi–Brecht, Laaber, Duplyakin) | **behind**. CLAUDE.md § 6 already requires "interleaved A/B runs in the same process or the same harness run", but the harness cannot interleave. |
| Comparison | `--compare`: difference of medians against `--max-diff`; m3-12 plans a 7% regression gate | CI of the ratio (Kalibera: Fieller; Laaber: bootstrap or Wilcoxon), per-benchmark thresholds from A/A runs (Laaber § 6.1.3) | **behind** |
| Warm-up | 1 untimed run; for write-heavy cases, to reach the drive's sustained rate | Barrett, Kalibera: check the run sequence; SNIA: steady state by a 5-round window | level for intent, behind for the check |
| Recorded environment | CPU model, cores, SMT, governor, boost, THP, kernel | SNIA: drive model, **firmware**, purge and preconditioning; Hoefler rule 9: everything that varies | **behind** on the drive. The governor field misleads: this host reads `powersave` under `amd-pstate-epp` with EPP `performance`, and the row records only `powersave`. |

**A/A test on our own data.** I re-analysed the two official idle runs of `headline`
offline:
- Files: `bench/results/headline/20261125T101402Z-…` and `…104322Z-…`, same spec, same
  revision, 68 cases with 3 counted repetitions each, all 204 rows undisturbed.

| \|difference of medians\| between the two runs | Cases |
|---|---|
| ≥ 2% | 13 of 68 |
| ≥ 5% | 4 of 68 |
| ≥ 7% | 3 of 68: k-core 16 T +14.0%, orkut parse-only +12.1%, Friendster 8 GB −7.5% |
| ≥ 10% | 2 of 68 |

- Within-run spread (max−min)/median was ≥ 5% in 12 of 68 cases.
- The Friendster cases of those two runs had no warm-up.
- Consequence: with no code change, a 7% gate over this suite would fire on about three
  cases per comparison. This is the m3-12 design flaw that Laaber's A/A tests predict.

**Minimal change that meets the standard.** About 300–500 lines, harness only, no product
risk; this is challenge 2. Five changes:
1. The report prints the median with an exact rank CI and its confidence level.
2. Official runs default to *n* = 6.
3. A `--interleave` mode runs seeded random rounds.
4. `compare` reports a ratio CI against a per-case δ taken from an A/A run.
5. The fingerprint records drive state, temperatures, EPP and effective frequency.

### 3.2 Promise 1 / ADR 0026 (drive-bound ingest)

**What the ADR's numbers say, by arithmetic.**
- Bytes written per edge are *b* = 45.1 GB / 1.806 B = **25.0 B/edge** at a 32 GB budget,
  and 55.4 / 1.806 = **30.7 B/edge** at 8 GB.
- At 1.5 GB/s that predicts 45.1/1.5 = 30.1 s and 55.4/1.5 = 36.9 s. The ADR measured
  29.06 s and 36.97 s.
- So **sustained ingest time ≈ bytes written ÷ sustained write rate** holds within 4% on
  both budgets.
- That is the rule ADR 0026's "what would change our mind" asks for: "a rule in bytes
  written per second".

**Why the "sustained" figure may belong to the host, not the drive.** The pieces:
- *Same model, empty:* a review (not peer-reviewed; same model, 2 TB) measured "6.2 GBps
  for 39 seconds, suggesting a pSLC cache size of about 240GB", then "around 1.4 GBps".
  It adds that the slow state is "more likely … when the drive is fuller".
- *What our probe saw:* the ADR's probe saw ~10 GB fast after 5 idle minutes, then
  1.4–1.6 GB/s. 10 GB is the size other sources give for this model's static region (not
  verified in the datasheet, which says only "operates only within a specific data
  transfer size").
- *What the drive is told:* this host mounts ext4 **without `discard`**, and `fstrim.timer`
  runs weekly (last Mon 23 Nov 01:11, next Mon 30 Nov 01:08). Since boot (14.9 days),
  `/proc/diskstats` shows 9.06 TB written and only 1.85 TB discarded. That is about
  0.61 TB/day of writes on a 2 TB drive whose file system is 24% full.
- *Inference, not measured:* between trims the FTL sees most LBAs as live, so the dynamic
  fast-write region is near zero.

**Consequences, if it holds.**
- After a trim, or on a user's emptier drive of the same model, a Friendster ingest
  (45–55 GB) would run inside the fast region.
- On a fuller or QLC laptop drive it would run below 1.4 GB/s.
- Either way, "about 60 M edges/s for Friendster" describes this host's write regime. It
  does not describe "a graph whose ingest writes more" in general.
- Maricq § 7.4, an SSD's periodic behaviour lasting weeks despite `blkdiscard`, is the
  peer-reviewed precedent for drive state as an uncontrolled variable.

| Promise 1 part | Literature | Verdict |
|---|---|---|
| "about 100 M edges/s while writes fit in memory or the burst" | no counter-evidence | level |
| "bounded by the drive's sustained write rate: about 60 M edges/s" | SNIA: report drive state and firmware; SSD-iq: devices of similar specification differ widely; Maricq: drive history persists | **behind**: one drive in one undocumented state |
| warm-up repetition for write-heavy cases | SNIA preconditioning with a steady-state check | level in intent; not verified per run |

**How to state it for other drives and laptops** (proposal for the ADR 0026 follow-up):

> Ingest writes *b* bytes per edge (lab prototype: 25 B at a 32 GB budget, 31 B at 8 GB;
> the product pipeline targets fewer, #35). While the bytes written fit in free memory
> or in the drive's fast-write region, ingest runs at about 100 M edges/s. Beyond that,
> it runs at about *W*/*b* edges/s, where *W* is the drive's sustained write rate. The
> fast region's size depends on the drive model, its fill level and when it was last
> trimmed. On the lab drive as this host uses it, the region is ~10 GB and *W* is
> 1.4–1.6 GB/s, giving 49–62 M edges/s.

The preflight's time estimate (P-9) should span both rates, *b*·*m*/*W*_burst to
*b*·*m*/*W*_sustained, rather than pick one.

### 3.3 Wear, and whether the planner should prefer recompute over spill

**Volume per Friendster pipeline.** These are four measured parts from different runs;
their total is an estimate:
- ingest 45–55 GB;
- out-of-core Leiden spill 20–27 GB at ≤ 8 GiB (FINDINGS § M1-02);
- `.eg` about 10 GB;
- tiles 6.8 GB.

That totals **≈ 80–100 GB of host writes**.

**Endurance, arithmetic on the datasheet.** TBW counts host bytes, so the drive's WAF is
inside the rating.

| Drive | Rated TBW | Per 100 GB run | Runs to the rating |
|---|---|---|---|
| 990 PRO 2 TB | 1,200 TB | 0.0083% | ≈ 12,000 |
| 990 PRO 1 TB | 600 TB | 0.017% | ≈ 6,000 |

- A researcher who re-ingests a billion-edge graph 10 times a day reaches 600 TB in about
  1.6 years (arithmetic).
- The product's normal pattern is to ingest once and read many times, and reads do not
  wear.
- **This host's own pace:** 0.61 TB/day from all agents means 1,200 TB in about
  5.4 years, close to the 5-year warranty (arithmetic). SMART "Percentage Used" cannot be
  read without root (`/dev/nvme0` is `root:root 0600`), so checking it needs the owner's
  hands.

**WAF.**
- No paper states that large sequential write-once files reach WAF ≈ 1.
- The field floor is ~2 (Maneas p10).
- Lee shows WAF = 1 when the streams of an active group are invalidated together.
- Our spill buckets are written in parallel and deleted together at the end of a phase,
  which is the favourable case (inference).
- On this host the deletes are not trimmed for up to a week. Until then the drive sees
  them as live data (He et al.: ext4 with `discard` trims at once; without it, only
  `fstrim` does).

| Question | Answer | Basis |
|---|---|---|
| Prefer recompute over spill to save wear? | **No.** Wear per run is 10⁻⁴ of rating. | arithmetic above |
| Prefer it to save time? | Sometimes. Weight writes by λ = *R*/*W*: on the lab drive about 3.9 GB/s read (FINDINGS semi-external) ÷ 1.4–1.6 GB/s sustained write ≈ 2.4–2.8; about 0.7 inside the burst. Then use Viglas' rule: materialize an intermediate only if it is re-read more than λ times; otherwise re-read or recompute. | Viglas 2014 (simulated NVM, single thread); Blelloch 2015 (AEM model) |
| What to do instead | Count bytes written in the planner's cost model and in the plan shown (P-9). Delete a phase's spill files together. Keep spill writes as large sequential appends. | Lee, He, SSD-iq |

### 3.4 Other decisions and issues in this area

| D / issue | Our evidence or plan | Literature | Verdict |
|---|---|---|---|
| **D1** (4.4–19× vs NetworKit) | same CSR arrays for both tools; NetworKit at its best (32 T), ours 8–16 T; best-of runs | Hoefler rule 1: state base and thread count; McSherry: give single-thread COST; SoK: same ordering, isolated vertices, BFS source | level on fairness notes (runners record what ran). Behind: no CIs, single BFS source (hub), no COST line. |
| **D3** (u64 +71–87%) | in-process best of 3, one process | layout bias is of order 2–10% (Mytkowicz) | conclusion robust: the effect is ≫ the bias |
| **D4** (compressed ÷ raw 0.73–1.52) | same binary, idle, raw then compressed | ratios of 0.97–1.04 lie inside layout and ordering bias (Mytkowicz, Duplyakin) | the large ratios are robust; "no cost" claims near 1.0 need a ratio CI from interleaved runs |
| **D13** (SMT gives nothing for scans) | 16 vs 32 T, idle | Schöne: SMT and P-state coupling on Zen; k-core 16 T spread 27% in our own A/A | "no gain" is unproven where the spread is above the difference; needs RMIT with a CI |
| **D14 / D15** costs (2.4–3.0×; 1.38–1.63×) | best of N | effects ≫ noise | robust |
| **D11** (OOC PageRank ~10%) | idle, median per iteration | ordering and drive state (Duplyakin, Maricq) | 10% is inside what fixed ordering can produce; restate with interleaved in-RAM vs OOC rounds |
| **m3-12** 7% regression gate | median of 3 vs median of 3 | Laaber: per-benchmark A/A calibration, same-host randomized trials | **behind**: 3 of 68 false alarms in our own A/A |
| **m8-03** public report | pinned versions, maintainer review, laptop plus cloud | Raasveldt checklist; SoK practices; GAP/Graphalytics rules | needs a written protocol (challenge 4) |
| **#100 / LDBC addendum** | adopt LDBC as oracle and workload | Graphalytics: T_l / T_p / T_m separately, 3 runs, f64, validation, no "LDBC result" wording | level once the protocol is written |
| **m1-10, m0-04, m3-13** graph families | real graphs per family | Bläsius: GIRG predicts BFS and k-core-like behaviour but not Louvain iterations; Meng: density and diameter knobs; SoK, Seshadhri: avoid plain Kronecker/R-MAT (51–74% isolated) | add a generator sweep for kernels; communities stay on real graphs |
| **m3-02** planner | cost model "random vs sequential access", bandwidth probe | asymmetric read/write cost (Viglas λ, Blelloch ω) | add a write-cost term; no wear rule (§ 3.3) |
| **#35 / m2-08** build | bytes written reported next to time | § 3.2: time ≈ *b*·*m*/*W* | level; add *b* (bytes per edge) as a reported metric |
| **#44** edge-map | — | GAIL (Beamer, cited in SoK): time = edges × requests/edge × time/request; TEPS alone misleads | opportunity: deterministic work counters now (challenge 6) |
| **PR #170** `.eg` spec | content-hashed sections | SoK: publish dataset hashes; report ordering and zero-degree counts | opportunity: ordering provenance and isolated-node count in metadata (challenge 7) |

## 4. Challenges, ranked

| # | Change | Expected gain (source) | Cost and risk | Cheapest experiment that settles it | Issue |
|---|---|---|---|---|---|
| 1 | **Find out whether ADR 0026's sustained rate is the drive's or the host's.** Record drive state in every row: model, firmware (`/sys/block/*/device/firmware_rev`, readable here: `4B2QJXD7`), file-system fill (`statfs`), discards since boot and time since the last `fstrim` (`/proc/diskstats` field 17, timer log), NVMe temperature (hwmon). Then restate promise 1 as the bytes rule of § 3.2. | If the fast region after a trim is ≫ 10 GB, "60 M edges/s" is a host artefact, and a user with an emptier drive gets ~100 M (inference from the review's 240 GB; Maricq § 7.4 precedent). The bytes rule (*W*/*b*) already fits both measured budgets within 4%. | ~60 lines in `machine.rs`. No product risk. Changes a public sentence (promise 1, ADR). | Re-run PR 136's 64 GB `O_DIRECT` write probe twice: Mon 30 Nov just after the 01:08 `fstrim`, and again on Wed. Run one `ingest-friendster` case after each, recording `/proc/diskstats` discards and `df`. A sudo `fstrim` is faster but needs the owner. | new: "Harness: record drive state; ADR 0026 follow-up: ingest rate as bytes written ÷ drive rate" |
| 2 | **Statistics in the report and the comparison.** (a) Print the median with an exact rank CI and its confidence (n = 3 → 75%, 6 → 96.9%, 10 → 97.9%). (b) Official default *n* = 6, with *n* = 3 allowed but labelled "no 95% interval". (c) `compare` gives a ratio of medians with a CI (bootstrap over rounds, or Fieller) and passes only if the CI lies within ±δ, where δ is per case from an A/A run. (d) m3-12's 7% gate uses (c). | Removes the ~3-per-suite false alarms of our own A/A (§ 3.1). Meets Hoefler rule 5/7, Kalibera, van der Kouwe B4. | ~200 lines in `report.rs`; one official window doubles in length at *n* = 6 (headline ≈ 26 → 50 min). | Offline: recompute both runs' CIs (no machine). Then one official `headline` run at *n* = 6, and an A/A `compare` of two runs to set δ per case. | follow-up to m0-03; amend m3-12 |
| 3 | **RMIT mode (`run --interleave --seed S`).** Rounds over the selected cases, or over two binaries/specs for an A/B; order reshuffled each round (seeded, recorded per row as `round`, `position`); warm-ups stay per case. | Abedi–Brecht: consecutive trials gave 37.8% between identical systems; Duplyakin: 72% of configurations order-sensitive; ADR 0026 itself was an ordering effect. Makes CLAUDE.md § 6's A/B rule executable, and robust to background load on the shared host (Laaber). | ~150–250 lines in `run.rs`. Cold-cache rotation and drive warm-ups need care: interleaving writers with readers changes drive state, so it is opt-in per spec. | Run `kernels-orkut` and `ingest-text` interleaved and consecutive on the same idle window, n = 10 each; compare medians and CI widths. | new: "Harness: randomized interleaved rounds for A/B and suites" |
| 4 | **A written protocol for public comparisons (#100, m8-03) that survives the SoK** (list below). | Pre-empts the SoK's faults (40% from ordering, 6–10× from BFS roots, 13–30% from isolated vertices) and Raasveldt's pitfalls; required for "survive hostile review". | Docs and spec changes; runners gain a source list and an ordering option; ~1 day. | Run BFS on orkut from the hub vs 64 seeded non-zero-degree sources, ours and NetworKit, interleaved: does the 4.4–19× range move? | amend m8-03 and m3-12 |
| 5 | **CPU-side environment and laptop protocol.** Record `scaling_driver` and EPP (not only the governor), effective frequency per repetition (cycles ÷ busy time via `perf_event_open` on the scope; `perf_event_paranoid` is 2 here, which allows counting one's own processes in user mode; else `scaling_cur_freq` samples), k10temp/coretemp and NVMe temperatures at start and end, and AC/battery (`/sys/class/power_supply`). For the m8-03 laptop: report "cool start" and "back-to-back steady" like rested vs sustained drives, with a cooldown to a recorded temperature between rounds. | Barrett's APERF/MPERF check flagged 225 iterations (0.004%), 98% of which their outlier filter also caught; Acun: 16% chip-to-chip under boost; Schöne: Zen frequency coupling. Laptop literature is thin, so we would be first to publish such numbers (none found). | ~100 lines; perf counters may be unavailable without privileges, so fall back. No product risk. | On the lab machine: kernels-orkut 16 T with one 0.3-core co-runner on another CCD vs none, interleaved. Does effective frequency move while "others' cores" stays below 0.5? | fold into challenge 1's issue; amend m8-03 |
| 6 | **Work counters in the edge-map framework (#44, in development).** Per kernel call: edges traversed, lists decoded, bytes read from storage, iterations. Summed per thread at the end; deterministic for deterministic kernels. | GAIL decomposition (time = work × requests/edge × latency) and honest TEPS (SoK § 5.2). Separates algorithmic work from implementation speed in D4/D11 comparisons and out-of-core claims ("one sequential read per pass" becomes a measured ratio). | Small if designed in now (a per-thread `u64` set); a retrofit later touches every kernel. The overhead must be measured (RMIT, PageRank on orkut). | Prototype on lab PageRank and BFS: counters on vs off, interleaved, n = 10; accept if the ratio CI is within ±1%. | amend #44 (m3-01) |
| 7 | **`.eg` metadata for benchmark honesty (PR #170, spec still free):** vertex-ordering provenance (input order / degree / seeded relabel + seed, #146), original-id range, number of isolated nodes kept and dropped, a content hash of the source file. | SoK practices "report the ordering", "state zero-degree counts", "dataset hashes" become automatic for anything run on a `.eg`. Graphalytics CDLP and BFS need original ids (earlier survey). | A few fields in the spec; no format risk now, a format break later. | None needed; spec review. | comment on PR #170 / m2-01 |
| 8 | **Generator sweep for kernel claims (m1-10, m3-12).** GIRG (locality × heterogeneity) plus FFT-DG-style density and diameter knobs; report *n* after removing isolated vertices; no plain Kronecker/R-MAT for scaling claims. | Bläsius: GIRG predicts BFS, diameter and cliques on 2,740 real graphs; Seshadhri: 51–74% isolated in Graph500. Communities stay on real graphs (Louvain unpredictable). | A pure-Rust GIRG generator (linear-time algorithms exist, not read here) in the bench crate; deterministic by seed. | BFS and WCC on 9 GIRG points (β × T) at orkut size against orkut itself: do our kernels' times track locality as Bläsius predicts? | amend m1-10; new "bench: GIRG generator" |
| 9 | **Spill hygiene, not recompute.** Planner cost term for bytes written (λ ≈ 2.4–2.8 on the lab drive, sustained); spill files of a phase deleted together; large sequential appends; ENOSPC preflight (already in m4-04). No wear-based rule. | Time only (§ 3.3); Lee/He: grouping by death time keeps WAF low. | A cost-model constant; no product risk. | None now; check with #35's bytes-written metric. | amend m3-02 |
| 10 | **Name the measurement level of every number.** For official numbers, record every in-process iteration and let the harness reduce (median over processes, with min shown); drop `--reps 2 → best` inside commands. | Kalibera (levels), Georges (best-of misleads), Chen (min); the A/A k-core case (27% spread) may be partly the best-of-2 inside. | Changes in the product kernels' JSON output; harness `reduce` exists already. | — | fold into challenge 2 |

**Challenge 4: the comparison protocol.** Each rule with its source:
1. Same input file (hash) and the same vertex ordering for every tool. Results in at least
   three orderings: input, degree, reverse degree (SoK).
2. State *n* with and without isolated vertices; feed every tool the packed graph
   (Friendster: 65.6 M, not 124 M).
3. BFS and SSSP from 64 seeded non-zero-degree sources, the same for every tool (GAP).
4. PageRank in f64 with a stated tolerance and iteration count, reported both to
   convergence and per iteration (SoK, Graphalytics). Triangles only on undirected graphs,
   exact.
5. Times reported separately for load and build (T_l), kernel (T_p) and end to end (T_m).
   Any per-kernel relabelling is charged to the kernel (GAP, Graphalytics).
6. A 1-thread line and COST for every tool (McSherry). Each tool at its best thread count
   and at a common one, SMT stated.
7. Outputs validated with Graphalytics' match rules; never call the results "LDBC results".
8. Tools run interleaved in randomized rounds (RMIT), at least 6 rounds; ratios given with
   CIs; geometric mean only across cases (van der Kouwe B5, Hoefler rule 4).
9. Peak memory via the cgroup for every tool.
10. Each tool's recommended configuration, checked with its maintainers (Raasveldt,
    "failure to optimize"). Hot runs labelled hot, and cold runs reported separately.
11. Drive and CPU state recorded (challenges 1 and 5).

## 5. Ideas we have that the literature lacks (novelty, checked honestly)

| Idea | Closest prior work found | Verdict |
|---|---|---|
| Per-repetition accounting of **others'** CPU and I/O (machine minus own cgroup), marking rows disturbed | Laaber, Bulej and Maricq treat noise statistically; Barrett checks frequency and temperature; BenchExec (by name, unverified) uses cgroups for own-resource measurement | probably new as a disturbance criterion; not a substitute for interleaving, which handles what it cannot see (frequency, drive state) |
| Cold cache via private copies, alternated so the same blocks are never read twice in a row | SNIA covers write-state preconditioning only; none of the papers read handle drive read caches | new in what I read; small |
| Warm-up repetition for write-heavy cases to reach the drive's sustained state | **SNIA PTS preconditioning (WDPC) with a steady-state window** is the prior art, and stricter | not new; adopt SNIA's 5-round / range 20% / slope 10% check as the test |
| Result hash in every row as a correctness check | Graphalytics validates outputs (epsilon or exact); GAP verifies | stronger than both (bit-identical), but only because of D14; modest novelty |
| Estimate vs actual (P-9) recorded by the harness for every planned run | none in graph benchmarks read; databases do this for cardinality estimates (not read here) | likely new for graph engines |
| Ingest speed explained by bytes written per edge × drive rate | Lee and SSD-iq reason in flash writes per operation for DBMSs | the rule is ordinary; applying it to a public ingest promise is ours |

## 6. What to ignore, and why

- **Stabilizer:** LLVM 3.1 and C only, 6.7% average overhead. For Rust, use environment
  padding and ASLR instead.
- **Duet benchmarking:** halves the cores per variant and gains nothing on a quiet host
  (Bulej RQ3). Our cases use all 16 cores.
- **Full changepoint analysis (Barrett):** our repetitions are processes with few
  iterations. A run-sequence/trend flag is enough.
- **The LLM API-usability score (Meng):** ρ 0.71–0.75 on 7 platforms, code never
  executed, circular tuning.
- **Wear-leveling algorithms (Jiao):** they live inside the FTL; the host cannot act on
  them.
- **ZNS, FDP and NoWA compensation writes (Lee):** consumer drives lack ZNS and FDP. Only
  the "die together" principle transfers.
- **Graph500 Kronecker as a scaling input:** 51–74% isolated vertices and oscillating
  degrees.
- **RAPL energy on AMD (Schöne):** modelled, not measured.
- **Write-limited algorithms for wear (Viglas, Blelloch):** our wear is 10⁻⁴ of the
  rating per run. Only their time model is useful.
- **Minimum-only statistics (Chen–Revels):** their additive-noise model ignores boost
  and frequency effects and multithreaded scheduling. Show the min; do not decide on it.
