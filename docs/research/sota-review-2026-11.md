# State-of-the-art review, 2026-11-26: synthesis

A condensed English version of the review's synthesis: conclusions and sources only. The
full synthesis (in Italian) and the eight area reports (`ooc.md`, `kernels.md`,
`compression.md`, `communities.md`, `ingest-query.md`, `maps.md`, `gpu-resources.md`,
`benchmarks.md`) live outside the repository, on the owner's machine, in
`/home/tux/graphlab/sota-2026-11-26/`. The synthesis checked them against `main` at
fa0226b and the open pull requests #249, #251 and #254. Nothing was built or run, except
the checks marked as measured.

**Marks.** [C] confirmed by the synthesis against code, ADRs, issues or result files;
[R] reported by an area report, usually from a paper; [I] a hypothesis. A paper's number
holds on the paper's machine only. "Exploratory" means taken on a loaded machine
(CLAUDE.md § 3). Proposals are S1–S14 and experiments F1–F8, apart from the decisions
D1–D25.

## 0. Summary

1. **"Never out of memory" has three small, certain holes** [C]: the CLI accepts a budget
   equal to the cgroup limit, while the no-OOM suite tests `budget + 96 MiB`; outside Linux
   the default budget is a fixed 8 GB; the page cache is neither measured nor reserved.
2. **No speed comparison with the best codes holds today.** The reference runs (PR #250)
   do not check outputs, measure tools differently and have no intervals [C]. Ratios change
   sign with load: WCC, GAP over ours on lj, 2.53× at load 46, 0.83× at load 19 [C,
   exploratory].
3. **One cause may explain several gaps:** the frontier driver cuts runs by 256 nodes,
   never splits a list and pays a fixed cost per run [C]. Its weight is [I].
4. **Stable gaps in the rounds read:** SSSP (GAP 1.8–2.6× faster), PageRank (GAP
   1.2–1.4×), k-core on twitter-sym (GBBS 1.5–1.75×) [C, exploratory]. We lead on k-core on
   lj, orkut and Friendster and against GBBS connectivity. ConnectIt, Parallel-KCore and
   PASGAL were not run.
5. **Out of core, the design holds on bytes read; random access is the risk.** Reads are
   synchronous, so queue depth = thread count [C]. Deterministic Leiden, the default, was
   never measured below 16 GiB [C].
6. **D14 promises more than the code keeps** for views, files reordered by
   `compact --order`, the super-level hysteresis, and an undeclared quality cost [C, R].
7. **The most defensible comparison is capacity under a hard memory limit,** and it was
   not planned.
8. **Three format decisions come before 0.1,** because afterwards each costs a format
   version: chunked hub records (m1-17), `zmin` and labels in tiles (#68), the structure
   as Arrow CSR (m6-03).
9. **A query language can come by reuse** (LadybugDB on Arrow CSR, Grafeo over a read-only
   store) [R, documents checked].
10. **Confirmed from outside:** codec 0 as the computing format; no chunk layout; fixed
    reduction blocks, which make asynchronous I/O neutral for results; cache policy
    matters little; no GPU; our own renderer; PMTiles.

## 1. Defects in existing promises

| # | Defect | Evidence |
|---|---|---|
| 1 | Zero margin between budget and cgroup limit (promise 2) | `everygraph-cli/src/machine.rs`; `SLACK` in the cgroup test of PR #251 [C] |
| 2 | Default budget outside Linux: a fixed 8 GB | `machine.rs` [C] |
| 3 | Page cache neither measured nor reserved | `MemoryKind::FileBacked` has no product caller [C] |
| 4 | One `MADV_WILLNEED` reads at most 128 KiB here, yet the plan prints "warmed ahead" | **measured:** a cold 64 MiB file, one call left 131,072 bytes resident; 128 KiB pieces made all of it resident (`synthesis-check/willneed.py`, Linux 6.12.116, ext4) |
| 5 | D14 and views: the sub-round count depends on `n`; draws use stored ids | `everygraph-community/src/level.rs` [C] |
| 6 | D14 and reordered files: `compact --order` changes communities | as above [C] |
| 7 | D14 and map history: super-level hysteresis between 56 and 72 | ADR 0016 [C] |
| 8 | Quality cost of determinism undeclared (about 1 point of modularity on orkut) | `lab/m1-04-det-leiden-scaling/README.md` finding 7 [R] |
| 9 | Reference runs do not check outputs | `bench/specs/references.toml`, PR #250 [C] |
| 10 | A "warm" benchmark under a limit smaller than the graph | `friendster-ooc` in `bench/specs/kernels.toml` [C]; no official result affected |
| 11 | ADR 0029's `MADV_RANDOM` rule never fires; `NVME_BYTES_PER_SECOND = 3.9e9` is a PageRank throughput, not the drive's 7.07 GB/s | planner in PR #249 [C]; `bench/results/io/20261125T205022Z-21498df39590.jsonl` |
| 12 | Cross-tool ratios on a loaded machine are unreliable, even interleaved | #55's rounds: WCC lj 2.53 → 0.83, orkut 2.66 → 0.81 [C] |
| 13 | Plan text: #209 "audit by LDBC" (no Graphalytics audits exist [R]); m8-03 "within 20%" on another machine; m7-06 and FINDINGS § F2 on DuckPGQ and LadybugDB (out of date [R]); m3-20's "deterministic hash table" (not needed: BGSS labels are functions of sets [R]) | issue files [C] |

## 2. Competitive position

- **Ahead:** verified memory accounting and refusal with the exact smallest budget;
  determinism across threads, sources and budgets (no reference code promises it [R]);
  bytes per out-of-core pass (10.32 GB store against 14.45 GB of raw lists [C]);
  out-of-core Leiden (1.17× of RAM at 16 GiB on Friendster, official); random-list decode
  against BV [R]; local layout structure (NP₁ 0.25–0.34 against DRGraph's 0.015 on lj
  [R]); tiled maps at Friendster scale (MSAGL.js stops at 32,768 nodes [R]).
- **Level:** WCC in RAM against GAP, within the noise [C, exploratory].
- **Behind:** out-of-core BFS reads 20.2 GB for a 10.32 GB store (PR #254) [C]; random
  access under a budget (1.9–2.2× of RAM with the prototype cache, ADR 0029); SSSP;
  PageRank; format density (BV 22–27% smaller after the best order, m1-09 [R]); global
  layout structure (Spearman 0.03 against 0.31 on lj, ADR 0017).
- **No data:** BFS on high-diameter graphs; ingest like for like; Graphalytics times.

Splitting hub lists in the driver works on raw lists. On a compressed source a hub's
decode stays serial without restart points in the record, that is, without m1-17 (the
synthesis's inference).

## 3. Proposals

**S1. Repair the memory promise.** Ledger limit = budget − declared headroom; the suite
tests `MemoryMax = budget`; probes for macOS and Windows; `WillNeed` in pieces or
`MADV_POPULATE_READ`. DuckDB assigns 90% of the limit; DataFusion advises about 10% margin
[R].

**S2. Make comparisons valid first.** A digest per output; the same reduction, workload
and flags for every tool; m3-18 and m3-19 before any table is cited; ConnectIt,
Parallel-KCore, PASGAL, and road, web and uniform graphs.

**S3. Frontier driver: runs cut by entries, hub lists split for commuting push bodies.**
Today a frontier of 256 nodes or fewer runs on one thread whatever its hubs [C].
Parallel-KCore splits lists at 128 entries, PASGAL at 1,024 [R]. About 100–150 lines
(estimate). D14 holds: cuts depend on data only. Success: k-core on twitter-sym ≤ 0.7× of
today, no case slower than 1.05×.

**S4. State exactly what D14 promises.** Sub-rounds counted on nodes with non-zero
strength; draws and ties by parent id on materialized views; ADR 0014 lists the
properties and declares its exclusions.

**S5. Reads in flight: a `pread` pool first, io_uring only on evidence.** Scans do not
need it: the drive reads 7.07 GB/s with 4 threads [C]. Random 4–16 KiB reads do [I]. CAVE
finds an optimal concurrency of 60 on a PCIe SSD; ACGraph's I/O pipeline alone gives
2.0–2.3× over Blaze in synchronous mode [R]. io_uring is blocked by Docker's and
containerd's default seccomp profiles [R]. A pool behind `ByteSource` with merge and
amplification caps, a device probe, io_uring as an optional Linux backend. About 300–500
lines (estimate). The internal evidence (m1-15, ADR 0029) is on the async mode. The
deterministic default visits sparse groups of 16 ids, 1,024 sub-rounds per sweep on
Friendster by the formula: many more small reads (arithmetic, not measured).

**S6. Out-of-core BFS: direction chosen by bytes, parents found during the push.**
Parents add a full scan today [C]; ACGraph reads about one pass [R]. Success: bytes ≤ 0.6×
with identical hashes.

**S7. SSSP: one lazy frontier, local bins, pull before push** (Δ*-stepping in PASGAL;
GAP's bins). About 200 lines, after S3. Distances are a unique fixpoint, so D14 holds. Part
of the gap may be representation, f64 column against int32 pairs [I]. Success: ≤ 1.2× GAP
on lj and orkut.

**S8. Deterministic Leiden.** Coarse levels oscillate and end at the 20-sweep cap [R].
First, run small levels and regions sequentially (the mode exists); then the afterburner
filter (Mt-KaHyPar, pLouvain; for Leiden [I], 150–300 lines); then prefix-doubling
sub-rounds (Krause et al.). One A/B on lj and orkut, 7 seeds. Success: deterministic time
≤ 0.8× at 16 threads, modularity not lower.

**S9. The headline comparison: capacity under a hard budget.** Time or OOM per kernel at
`MemoryMax` from 2 to 56 GB, cold, for Everygraph, GBBS on mmap, GAP and NetworKit;
residence proven by `read_bytes`, major faults and refaults; a sequential-read time beside
each out-of-core time; a lint against non-cold runs under a limit; a COST line from
McSherry's code (HotOS'15, Rust, MIT). About 50 lines in the harness and one official
window.

**S10. Hubs and prefixes before the format freezes.** On a compressed source, bottom-up
BFS and WCC sampling decode whole lists [C for BFS]. Entries in lists over 1,024 / over
65,536 entries, from the offsets: twitter-sym 41.6% / 11.2%, Friendster 9.3% / 0, orkut
3.9% / 0, lj 1.2% / 0 [R]. Benefit [I]: much of BFS's 1.20–1.52× and WCC's up to 1.57×
compressed penalty. Experiment: entries decoded against entries examined (F3). Abandon:
surplus under 2×.

**S11. Compression: a census, not a new codec yet.** The review's offline bit count
(Python, input order, data part, bits per entry) [R]:

| coding | lj | web-google | roadnet-ca |
|---|---|---|---|
| codec 0 | 17.22 | 20.72 | 12.89 |
| codec 2 (Stream VByte 0124) | 18.14 | 23.36 | 12.31 |
| Elias–Fano per list | 18.31 | 17.12 | 21.01 |
| bit packing, groups of 32 | 18.98 | 19.28 | 10.39 |
| hybrid-integer tokens, zero-order entropy bound | 13.76 | 17.32 | 8.11 |

The count reproduces published codec 0 totals (lj 17.985 against the spec's 17.99;
web-google 21.917 against PR #157's 21.92). The entropy rows are bounds with no tables and
no decoder. A codec is a permanent duty for every reader, and neither 0.1 nor Graphalytics
needs one. Do the Rust census (about 300 lines, sizes only); do not build m2-20 as written.
Threshold: per-block Huffman ≥ 15% below codec 0 on at least three graphs, tables
included.

**S12. Maps: zoom consistency in the format.** The lab's per-tile selection is top-k by
degree and does not nest; labels by collision in the viewer depend on the frame [C]. Put a
`zmin` per entity and build-time labels with zoom intervals in #68 (Been, Daiches, Yap,
TVCG 2006; Been, Nöllenburg, Poon, Wolff, CGTA 2010; MSAGL.js § 3.1); count popping. Keep
PMTiles v3; compare MLT as a payload; no MVT. The bundle budget rules out deck.gl and
MapLibre as the core: 219 KB and 300 KB min+gzip against 150 KB (bundlephobia,
2026-11-26 [R]).

**S13. Query language by reuse.** The graph lends its structure as Arrow CSR (m6-03); two
spikes later (LadybugDB on Arrow CSR, Grafeo over `.eg`). Risks: LadybugDB's path was in
nightlies; Grafeo's trait is experimental; raw CSR costs 8 B per entry. Results through an
engine are outside D14. A path is dropped if it needs a copy of the graph in the engine.

**S14. Graphalytics: full disclosure** in competition style, and the LAGraph (GraphBLAS)
driver run here on classes S and M (one or two days, estimate).

## 4. Decisions taken

Taken by the coordinator on 2026-11-26, under the owner's delegation. No milestone moves.

| Item | Decision | Where |
|---|---|---|
| S10 | m1-17 is P0 and reports before the `.eg` format freezes; m3-27's counts give its numbers | m1-17 (#174), m2-36 |
| S12 | `zmin`, build-time labels with zoom intervals, MLT in the payload comparison; PMTiles v3, no MVT; if #20 is late, #68 goes ahead on the selection-only design | m5-02 (#68), m5-03, m5-05 |
| S13 | Arrow `indptr`/`indices`, zero-copy where the layout allows; m9-03 chooses an embedded engine and bridges to it; no engine of our own | m6-03 (#83), m9-03 (#110), ADR 0021 |
| S11 | the codec census in Rust (8 graphs × m1-09 orders); m2-20 reduced to the tier the census chooses | m1-16 (#173), m2-20 (#168) |
| S5 | #178 on a `pread` pool; io_uring optional on m2-35's evidence, with fallback on `EPERM`/`ENOSYS`; prefetch in the visiting order; deterministic Leiden at 8 and 4 GiB in the acceptance | m2-23 (#178), m4-04 (#60) |
| S3, S6, S7 | M3 performance pass | m3-22 (#240) |
| S8 | M4 performance pass, one interleaved A/B | m4-12 (#259) |
| S9 | the headline comparison | m3-12 (#55), m8-03 (#100) |
| S14 | full disclosure and LAGraph baseline | m8-11 (#209) |

**Do not implement** (evidence in the area reports): m2-20 as written; a chunk layout
(ChunkGraph); KedaGraph's codec; asynchronous execution for order-sensitive kernels;
FrontOrder; SPDK; an io_uring runtime; our own query engine; MapLibre or deck.gl as the
core renderer; MVT as the payload; deterministic reservations; colouring. Recorded in the
Consequences of ADR 0004, 0014, 0021 and 0029, in m5-02 and in m9-03.

## 5. Deferred, with the arithmetic

**Fully external regime (#114).** The form that keeps D14 is 2-D streaming over a derived
grid: destination ranges in RAM in turn, each destination adding its in-neighbours in
ascending source order. Sums are then bit-identical to the in-RAM pull kernel for any grid
size. Vertex passing through spill buckets (Seraph style) writes about 8 B per entry per
iteration, about 29 GB per PageRank iteration on Friendster (arithmetic).

**GPU (#112)** (arithmetic, not measured). Beyond RAM, the NVMe sets the time. In RAM but
beyond VRAM, PCIe 4 x16 (32 GB/s in theory; EMOGI reached about 12 GB/s on PCIe 3 [R]) is
below this desktop's DRAM bandwidth; integrated GPUs share the CPU's DRAM. The one clear
case is a compressed graph that fits in VRAM: Friendster's store is 10.4 GB, an RTX 4090
has 24 GB; it needs a GPU Stream VByte decoder [I]. WGSL has no f64. CubeCL is the only
Rust route without a second kernel language, and it is pre-1.0.

## 6. Priority experiments

| # | Decides | Accept |
|---|---|---|
| F1 | No-OOM for what the user types: PR #251's matrix at `MemoryMax = budget` | zero `oom_kill` |
| F2 | Cost of `WillNeed` not warming: first cold pass, three variants | within 1.2× of warm |
| F3 | Frontier driver as the common cause; prefix value (m3-27) | decides S3 and S10 |
| F4 | Do cross-tool ratios hold? orkut, validated outputs, idle then loaded | intervals overlap |
| F5 | Random-read rate by size and queue depth, pool against io_uring (m2-35) | io_uring only if the pool stays below 90% of its IOPS |
| F6 | Deterministic Leiden at 8 and 4 GiB, cold | hash identical; bytes per sweep |
| F7 | Out-of-core BFS in about one pass | bytes ≤ 0.6× |
| F8 | Which coding for which graph (m1-16) | decides m1-16 and m2-20 |

## 7. Strategic points

- The real problems are promises to close and a comparison method to make valid, not
  missing technologies.
- New to the project: BYO (VLDB'24: "compressed is faster" holds only for pull scans);
  Mt-KaHyPar's 2025 deterministic techniques; the cost models of LOSC and CAVE; Icebug as
  the nearest neighbour; consistent map labelling; MLT; the COST code; io_uring often
  disabled; the kernel's cap on `WILLNEED`.
- Potentially ahead, with no published rival offering the same guarantees [R]: identical
  results across threads, sources, regimes and budgets; out-of-core Leiden at in-RAM
  quality; a verified hard budget; a map of a billion edges on one machine.
- Largest risks: random access under a budget, above all in the deterministic mode;
  Graphalytics at graph500-30, where out-of-core LCC (MGT, m3-08) and uncompressed
  weights are the weak points; numbers from a loaded machine steering work.
- graph500-30 (estimate): at Friendster's 23.17 bits per entry, 34 billion entries take
  about 98 GB, above 61 GB of RAM; PageRank's per-node state (about 20 B per node [R]) is
  about 9 GB. The semi-external regime is enough; it needs #46, S6, MGT for LCC, streaming
  CDLP and a disk plan.
- Reuse: the `pmtiles` JS reader, arrow-rs `ParquetPushDecoder`, LadybugDB or Grafeo, the
  plain `io-uring` crate. As benchmarks only: GraphBrew, CAVE, COST, LAGraph. Not in the
  core: webgraph-rs, sux-rs, Glommio, MapLibre.

## 8. Benchmarks and tests

The test and harness tools are at the level of the literature or above [R]. Gaps:
schedule independence is sampled at 1, 3 and 8 threads, not searched [C]; certifying
checkers do not run on benchmark outputs; intervals (m3-18) and interleaving (m3-19) are
open [C]. Today only a narrow sentence holds ("on this machine, graph G, N threads, kernel
only, one idle run"), plus capacity and memory claims, which do not depend on load.

Before a public comparison: outputs validated across tools (a row with a different digest
is `invalid`); the same reduction, precision and flags, with 95% intervals; cross-tool
ratios from official runs only; the strongest baselines and graph families beyond social;
the budget lint and proven residence; work and bounds beside times; the exact variant of
each graph (GAP's "twitter" has 61.6 M nodes, 19.9 M isolated; our twitter-2010 41.7 M
[R]); a "never compare" list (historical systems on their hardware, GBBS on
Hyperlink2012, leaderboards, KedaGraph). **m8-03 reproduces ratios and rankings within
their intervals, not absolute times on another machine.** More than one machine needs the
owner's hands.

## 9. Not read or not verified

Not accessible: the CSUR 2026 survey (to read first), Gem, ChunkGraph's TOS version,
KedaGraph's tables, Cologni–Vigna 2026, FrontOrder (ICDE'25), Sahu (HPDC'26), Wasp
(SC'25). Not re-checked by the synthesis: every paper number, the bit count script, the
accounting fixes of PR #251, io_uring in Docker, the absence of Graphalytics audits, the
BGSS reading. #55's rounds were read from a file still being written (1–3 rounds per case,
loads 6 to over 100). No external system was built or run here; no SATA SSD, HDD, laptop
or integrated GPU was measured.
