# State of the art: compressed graph storage and vertex ordering

Status: research note, 2026-11-25 (web research and reading; no measurement was run). Area:
DESIGN D3, D4, D5, D6; issues m1-09 (#hierarchy order), m1-14 (#146 relabel at ingest),
m2-03 (#30 codec), m2-07 (id mapping), m2-11 (in-adjacency), m2-15 (webgraph feature),
m2-17 (zstd), m2-20 (archival codec), m9-07 (#114 u64 ids). It builds on
`docs/research/memory-bounded-algorithms.md` § 9 (reordering at ingest) and does not repeat it.

Every number below is the paper's own, with its machine, data and threads; none is
comparable to ours as-is. Our numbers are from FINDINGS § C and § M1-13. Arithmetic and
extrapolations are labelled.

---

## 1. Scope and reading log

PDFs are in `/home/tux/graphlab/papers/storage/`, text in `…/storage/txt/`.

| Work | Venue / year | Link | Read level | PDF |
|---|---|---|---|---|
| Lemire, Kurz, Rupp — Stream VByte | IPL 2018 | https://arxiv.org/abs/1709.08990 | full text | `2018-lemire-streamvbyte.pdf` |
| Pibiri, Venturini — Techniques for inverted index compression | ACM CSUR 2020 | https://arxiv.org/abs/1908.10598 | full text of the evaluation (§ 6); codec sections skimmed | `2020-pibiri-invertedsurvey.pdf` |
| Shun, Dhulipala, Blelloch — Ligra+ | DCC 2015 | https://people.csail.mit.edu/jshun/ligra+.pdf | full text | `2015-shun-ligraplus.pdf` |
| Versari et al. — Zuckerli | arXiv 2020 (2021) | https://arxiv.org/abs/2009.01353 | full text | `2021-versari-zuckerli.pdf` |
| Fontana, Vigna, Zacchiroli — WebGraph: the next generation (is in Rust) | WWW'24 companion | https://upsilon.cc/~zack/research/publications/www-2024-webgraph-rs.pdf | full text | `2024-fontana-webgraphrs.pdf` |
| Boldi, Rosa, Santini, Vigna — Layered Label Propagation | WWW 2011 | https://arxiv.org/abs/1011.5425 | full text (results; method skimmed) | `2011-boldi-llp.pdf` |
| Dhulipala et al. — Recursive graph bisection (BP) | KDD 2016 | https://arxiv.org/abs/1602.08820 | full text | `2016-dhulipala-bp.pdf` |
| Mackenzie, Petri, Moffat — Faster index reordering with BP | SIGIR 2021 | https://jmmackenzie.io/pdf/mpm21-sigir.pdf | full text | `2021-mackenzie-fasterbp.pdf` |
| Lécuyer, Danisch, Tabourier — [Re] Gorder | ReScience C 2021 | https://github.com/lecfab/rescience-gorder | full text | `2021-lecuyer-gorderreplication.pdf` |
| Arai et al. — Rabbit Order | IPDPS 2016 | https://www.rd.ntt/_assets/pdf/sic/team_researchers/other/araij2016ipdps.pdf | full text | `2016-arai-rabbitorder.pdf` |
| Dhulipala, Blelloch, Shun — GBBS | SPAA'18 / TOPC (arXiv v4 2019) | https://arxiv.org/abs/1805.05208 | partial: § 6.4 (compressed primitives), § 7 setup and inputs | `2018-dhulipala-gbbs.pdf` |
| Faldu, Diamond, Grot — DBG, a closer look at lightweight reordering | IISWC 2019 | https://arxiv.org/abs/2001.08448 | partial: abstract, § III–IV, § VI-A | `2020-faldu-dbg.pdf` |
| Koohi Esfahani, Kilpatrick, Vandierendonck — Locality analysis of graph reordering | IISWC 2021 | https://blogs.qub.ac.uk/DIPSA/locality-analysis-of-graph-reordering-algorithms/ | partial: abstract, environment, § VIII, conclusion | `2021-esfahani-localityanalysis.pdf` |
| Wei, Yu, Lu, Lin — Gorder | SIGMOD 2016 | https://github.com/datourat/Gorder | method skimmed (numbers taken from the replication) | `2016-wei-gorder.pdf` |
| Lemire, Boytsov — SIMD-BP128 (decoding billions of integers per second) | SPE 2015 | https://arxiv.org/abs/1209.2137 | abstract + § 3 (differential coding) | `2015-lemire-simdbp128.pdf` |
| Ottaviano, Venturini — Partitioned Elias–Fano | SIGIR 2014 | http://groups.di.unipi.it/~ottavian/files/elias_fano_sigir14.pdf | abstract/results only | `2014-ottaviano-pef.pdf` |
| Besta et al. — Log(Graph) | PACT 2018 | https://arxiv.org/abs/2010.15879 | abstract/results only | `2018-besta-loggraph.pdf` |
| Besta, Hoefler — Survey of lossless graph compression | arXiv 2018 | https://arxiv.org/abs/1806.01799 | downloaded, not read (taxonomy only used via other works) | `2018-besta-compressionsurvey.pdf` |
| Drescher et al. — BOBA | arXiv 2023 (no peer-reviewed venue found) | https://arxiv.org/abs/2306.10410 | abstract/results only | `2023-drescher-boba.pdf` |
| Balaji, Lucia — When is graph reordering an optimization? | IISWC 2018 | https://bvignesh.github.io/papers/Reordering-IISWC18.pdf | slides only (paper itself not accessible) | `2018-balaji-reordering-slides.pdf` |
| Dhulipala, Blelloch, Shun — Aspen (C-trees) | PLDI 2019 | https://arxiv.org/abs/1904.08380 | abstract/results only | `2019-dhulipala-aspen.pdf` |
| Dhulipala et al. — PaC-trees / CPAM | PLDI 2022 | https://arxiv.org/abs/2204.06077 | abstract/results only | `2022-dhulipala-cpam.pdf` |
| Pandey et al. — Terrace | SIGMOD 2021 | https://users.cs.utah.edu/~pandey/courses/cs6968/spring23/papers/terrace.pdf | abstract only | `2021-pandey-terrace.pdf` |
| Wheatman et al. — BYO graph-container benchmark | VLDB 2024 | https://www.vldb.org/pvldb/vol17/p2307-wheatman.pdf | abstract only | `2024-wheatman-byo.pdf` |
| Brisaboa, Ladra, Navarro — k²-trees | SPIRE 2009 | https://users.dcc.uchile.cl/~gnavarro/ps/spire09.1.pdf | abstract only | `2009-brisaboa-k2trees.pdf` |
| Carmona et al. — Depth-first k²-trees | arXiv 2026 | https://arxiv.org/abs/2607.28136 | abstract only | `2026-carmona-k2treedfs.pdf` |
| Lim, Kang, Faloutsos — SlashBurn | TKDE 2014 | https://datalab.snu.ac.kr/~ukang/papers/SlashBurnTKDE.pdf | abstract only (numbers via BP's Table 2) | `2014-lim-slashburn.pdf` |
| Mackenzie, Petri, Moffat — Tradeoff options for BP | TKDE 2022 | https://jmmackenzie.io/pdf/mpm22-tkde.pdf | abstract only | `2022-mackenzie-bptradeoffs.pdf` |
| Mackenzie et al. — BP reproducibility study | ECIR 2019 | https://jmmackenzie.io/pdf/mm+19-ecir.pdf | abstract only | `2019-mackenzie-bpreproducibility.pdf` |
| Dubuisson — Copy, predict, code the rest | arXiv 2026 | https://arxiv.org/abs/2605.21510 | abstract/method excerpt | `2026-dubuisson-copypredict.pdf` |
| Koohi Esfahani et al. — ParaGrapher | arXiv 2024 | https://arxiv.org/abs/2404.19735 | abstract only | `2024-esfahani-paragrapher.pdf` |
| Koohi Esfahani, Vandierendonck — PoTra, locality of graph transposition | arXiv 2025 | https://arxiv.org/abs/2501.06872 | abstract only | `2025-esfahani-transposition.pdf` |
| Xu et al. — Laconic (rule-based compression) | VLDB 2024 | https://www.vldb.org/pvldb/vol17/p2212-xu.pdf | abstract only | `2024-xu-laconic.pdf` |
| Zhang et al. — Cagra / frequency-based clustering | BigData 2017 | https://arxiv.org/abs/1608.01362 | downloaded, not read | `2017-zhang-cagra.pdf` |
| Lemire et al. — Roaring (2016), Roaring software (2018) | SPE | https://arxiv.org/abs/1603.06549, https://arxiv.org/abs/1709.07821 | not read (numbers via Pibiri–Venturini) | `2016-lemire-roaring.pdf`, `2018-lemire-roaringsoftware.pdf` |
| Plaisance et al. — Masked VByte; Lemire et al. — SIMD intersection | iSWAG 2015; SPE 2016 | https://arxiv.org/abs/1503.07387, https://arxiv.org/abs/1401.6399 | not read (Stream VByte paper compares them) | `2015-plaisance-maskedvbyte.pdf`, `2016-lemire-simdintersect.pdf` |
| Boldi, Vigna — WebGraph I | WWW 2004 | https://vigna.di.unimi.it/ftp/papers/WebGraphI.pdf | downloaded, not read (BV known from later papers) | `2004-boldi-webgraph.pdf` |
| Cologni, Vigna — Fast, parallel, scalable differential graph compression | WAW 2026 | https://link.springer.com/chapter/10.1007/978-3-032-27193-8_7 | **not accessible** (Ca' Foscari copy embargoed to 2027-06-30); abstract only: up to 15% smaller than BV, random access up to 54% faster than Zuckerli | – |
| Chen et al. — CompressGraph | SIGMOD 2023 | https://dl.acm.org/doi/10.1145/3588684 | **not accessible** (author host unreachable, ACM returned HTML) | – |
| Chen, Chung — Corder / workload balancing via reordering | PPoPP'21 / TPDS 2022 | https://www.cs.nthu.edu.tw/~ychung/Journal/2022-IEEE-TPDS.pdf | **not accessible** (returned HTML); search abstract only: up to 2.59×, mean 1.47× over original order | – |
| Li, Kuo — Amortized cost in graph reordering (BFS order) | SSDBM 2025 | https://doi.org/10.1145/3733723.3733730 | **not accessible** (ACM returned HTML); search abstract only: BFS order lowest amortized cost in 47 of 60 cases | – |
| Barik et al. — Vertex reordering, empirical evaluation | IISWC 2020 | DOI 10.1109/IISWC50251.2020.00031 | **not accessible** (no open copy found) | – |
| Hoag et al. — Balanced partitioning in LLVM (function layout) | arXiv 2211.09285 / TOPLAS | https://arxiv.org/abs/2211.09285 | search result only | – |

Software checked (docs, not papers): `lemire/streamvbyte` README (has `streamvbyte_decode_0124`
and delta functions; SSE4.1 and NEON paths, no AVX-512); `bitpacking` crate (pure-Rust port of
simdcomp, BitPacker1x/4x/8x, scalar fallback, MIT); `webgraph` 0.7.0 (2026-10-12, Apache-2.0
OR MIT, depends on `mmap-rs` and on the `zstd` binding crate).

---

## 2. The map of the field

**Three families of adjacency codecs**, ordered by decode speed:

| Family | Representatives | Size (paper's data) | Speed (paper's machine) | Fit for us |
|---|---|---|---|---|
| Byte-aligned gap codes, SIMD or run-headers | Stream VByte (ours), Ligra+ byte / byte-RLE / nibble, GBBS parallel-byte, Masked VByte, Varint-G8IU | Ligra+: 53% / 56% / 49% of the uncompressed graph on average (10 graphs, best ordering per graph) | Stream VByte 1.1–4.0 G ints/s (i7-4770 Haswell, 1 thread, ClueWeb09 posting lists), 2.5–5× VByte, up to 2× Varint-G8IU, 0.5–2 bits/int more than VByte | ours; pure Rust; wasm `simd128` done |
| Bit-aligned codes with partitioning | PEF, Opt-VByte, BIC, SIMD-BP128 / FastPFor / QMX, Roaring, Log(Graph) | Pibiri–Venturini, i9-9900K, 1 thread, lists > 4096 postings: VByte 8.81–9.29 bits at 0.96–1.09 ns/int; Opt-VByte 3.89–6.42 at 0.72–0.92; PEF 3.12–5.45 at 0.76–1.31; QMX 5.12–7.40 at 0.80–0.87; Roaring 6.63–9.78 at 0.50–0.71; prefix sum alone 0.5 ns/int | fast only on long, dense lists (blocks of 128); not built for lists of ~20 entries | partial: EF already in our index |
| Reference / entropy codes | BV (WebGraph), Zuckerli, Cologni–Vigna 2026, k²-trees, CB (Dubuisson) | twitter-2010: Zuckerli 12.12 bits/edge vs BV 14.46 (list access); web graphs 0.9–2.8 | Zuckerli random list 2.74 µs vs 0.27 raw (tw-2010, AMD 3970X); webgraph-rs random access 17–47 ns per arc (i7-12700KF; Xeon Gold 6342); k²-tree 2–15 µs per neighbour (SPIRE'09) | archival / interchange only |

Who is best on what:
- **Decode speed among byte codes:** Stream VByte (Lemire 2018), the format we use. Ligra+'s
  byte-RLE (header per run of ≤ 64 same-width gaps) is its scalar ancestor; GBBS keeps Ligra+'s
  format and adds chunked lists.
- **Size on social graphs:** Zuckerli (−10–15% vs BV, Table 7); Cologni–Vigna claim −15% vs BV
  with faster random access (not read).
- **Size on web graphs:** BV/Zuckerli with LLP or BP order (≈1–3 bits/edge). Byte codes stay
  ≥ 9.5 (our FINDINGS § C).
- **Largest graphs processed compressed in RAM:** GBBS, Hyperlink2012-Sym (225.8 B edges) in
  330 GB, "less than 1.5 bytes per edge", 72-core 4-socket Xeon E7-8867 v4, 1 TB.

**Ordering, by what it buys per second spent:**

| Order | Compression effect (paper) | Locality effect (paper) | Cost (paper's machine) | Deterministic? |
|---|---|---|---|---|
| BP (recursive bisection) | best on all but one graph, 5–20% better than the best alternative; BV bits: LiveJournal 14.61 → 10.73, Twitter (sym) 21.56 → 11.62 | not measured in the paper | LiveJournal 3 min, Twitter 2.4 B edges 149 min (32-core Xeon E5-2660); 3.7× faster with Mackenzie's median-finding and approximations (IR index, 32 threads) | yes if each split is seeded and ties broken by id (inference: subproblems are independent) |
| LLP | runner-up to BP; LiveJournal 10.90 vs 15.1 for BFS and 14.35 for the natural order (BV, LLP Table 6); orkut 15.25 → 11.32 BV, −14% for our Stream VByte (our lab) | raw PageRank −32% on orkut (our lab) | ≈ 800 k arcs/s overall, Java, 24-core Xeon X5660; 23 min on LJ (BP paper); ≈ 70 min on webbase (Rabbit paper); ~9 min on orkut (our lab, webgraph-rs) | not shown in what we read |
| Rabbit Order (Louvain-style incremental aggregation + DFS of the dendrogram) | not reported | PageRank 3.37× faster than **random** ids on average (2× 12-core Xeon E5-2697v2, 48 threads); end-to-end 2.21× | webbase 1.0 B edges in 13.8 s (48 threads); Friendster 139 s (Xeon Gold 6130 machine, Esfahani 2021) | **no**: CAS-based; "produces different permutations in different executions" (Esfahani 2021) |
| Gorder | not designed for it | best cache locality; first in half of 81 series in the replication | twitter 1.47 B edges: 25,475 s (Xeon E5-4650L; sequential code) | yes, but sequential and hours |
| RCM / BFS / DFS | BFS ≈ natural on social graphs (LJ 14.69 vs 14.61), large gain only when ids are random (web-Google 20.08 → 7.69) | RCM and DFS match Gorder in half of the replication's experiments | twitter: DFS 54 s, RCM 158 s (replication) | BFS/DFS yes if sequential or level-synchronous with sorted frontiers |
| DBG / HubSort / HubCluster / degree sort | degree order hurts BV (orkut 17.75 vs 15.25, our lab) | DBG +16.8% mean over 40 app×graph points (Gorder +18.6%); on already-structured graphs (lj, wl, fr, mp) only +6.5% (Gorder +6.9%); Sort and HubSort slow down up to 38.4% | O(n), seconds | yes (stable sort) |
| BOBA (order of first appearance in the edge list) | not reported | SpMV 1.36–3.22× vs **random** labels (GPU) | about the cost of computing degrees; ~16 ms for 60 M edges on a GPU | yes |

Two cross-cutting findings: speed-ups reported against **random** ids (Rabbit, BOBA) are
inflated relative to real inputs, whose original ids already carry locality (Gorder
replication; DBG's "structured" datasets; Rabbit's own note that published datasets are
pre-ordered). And the quality metric that predicts compressed size is the **average log gap**
(LLP: correlation 0.968 with BV bits/link, 0.174 for average log distance), which a codec can
compute for free while encoding.

---

## 3. Decision by decision

### D3 — dense u32 ids (and #114 u64 ids, m9-07)

- **Best known practice:** GBBS and Ligra use 32-bit vertex ids and 64-bit edge offsets by
  default, and GBBS ran Hyperlink2012 (3.56 B vertices, under 2^32) that way. webgraph-rs uses
  `usize` because Software Heritage has 34 B nodes.
- **Verdict: level.** u32 covers the largest public graph that the in-RAM literature
  processes (Hyperlink2012). Gap codecs are width-agnostic (BV, Ligra+, ours: FINDINGS § C).
  Nothing in the literature changes #114. m9-07's "u32 local ids within partitions" matches
  what grid systems do; it is not needed for the adjacency codec.

### D4 — own Stream VByte + delta codec, Elias–Fano offsets (spec `docs/spec/eg-adjacency.md`)

| Aspect | Literature | Us | Verdict |
|---|---|---|---|
| Word codec | Stream VByte is the fastest byte-oriented codec published (Lemire 2018); Ligra+'s byte-RLE and GBBS's parallel-byte are the same family | Stream VByte, SSSE3 / NEON / `simd128`, shuffle and prefix sum fused | **level** on speed |
| First word relative to the node | Ligra+ and GBBS: first edge as a signed difference from the source; BV likewise | `zigzag32(x0 − v)` | **level** |
| Gaps − 1 | BV codes successors as gaps − 1 | same | **level**; it only pays off together with a code that stores 0 in fewer than 8 bits (see Challenge 2) |
| No degree in the record | Ligra+ and GBBS keep degrees in a separate uncompressed array; webgraph-rs builds a "degree cumulative function" (`build_dcf`, used to balance parallel work by arcs; its storage form not checked) | degree from the Elias–Fano entry index | **level** |
| Per-node offsets | Ligra+ / GBBS: uncompressed offsets; webgraph-rs: Elias–Fano offsets (`.ef`) | Elias–Fano byte and entry offsets, 0.25–0.76 bits/entry together | **level with webgraph-rs, ahead of Ligra+ / GBBS** in size |
| Delta form | SIMD-BP128 uses D4 deltas (x_i − x_{i−4}) for speed at a cost of up to 2 bits/int (§ 3) | D1 with an in-register prefix sum | **right for a byte codec**: D4 would push more words past a byte boundary |
| Decode width | Lemire's library: SSE4.1 and NEON only, no AVX2/AVX-512 decoder | SSSE3, 4 lanes | **level**; a wider decoder is unexplored by everyone, us included |
| Hub lists | Ligra+: lists over T = 1,000 split into chunks, each chunk's first edge relative to the source, offsets per chunk, decoded in parallel ("up to 2 times slower" without it on some application/graph pairs; same performance for T from 100 to 10,000). GBBS builds map, filter, pack and intersection on these chunks ("block starts") | one record per list; a list over 65,536 entries is a block of its own; never split | **behind** |
| Size on social graphs | BV 8–14% smaller (orkut 15.4 vs 16.7, twitter-sym 13.9 vs 16.1, our lab); Zuckerli 12.12 vs our 16.54 on twitter-2010 (−27%, if the LAW id order is the one we stored); bit codes cost 9–22× in decode speed (our lab) | 16.1–18.1, Friendster 23.2 | **behind by 8–27% at 10×+ the speed** — the right trade for a default |
| Size on web graphs | BV 2.1 (uk-2005), Zuckerli 0.9–2.8 | 11.4 | **far behind**; two levers: order + a zero-byte code (Challenges 1–2), or the archival tier |
| Order | everyone who reports sizes reports them after an ordering (Ligra+ "best ordering per graph"; LAW files are LLP-ordered; BP, LLP) | input order, nothing else | **behind** — the largest gap (§ 4, Challenge 1) |

Is there a codec that beats ours on decode speed at a similar size, or on size at a similar
speed, in pure Rust and wasm? **Not for the short, high-entropy lists of social graphs.** The
faster codecs in Pibiri–Venturini (Roaring, Slicing, QMX, Opt-VByte) are faster only on long
dense lists and get their size from dense partitions; at our list lengths (mean degree 17–76)
they degenerate to the byte case or worse. The smaller codecs (BV, Zuckerli, BIC, PEF) are
2–20× slower to decode. One arithmetic check, not a measurement: a per-list Elias–Fano costs
about 2 + log2(n/d) bits per entry when neighbours are spread uniformly (n and mean
degree d from the datasets' published counts); that is 20.1 (lj),
17.3 (orkut), 21.5 (twitter-sym) and 22.2 (Friendster) against our 17.2, 16.4, 16.0 and 23.0.
Only on Friendster, whose ids carry no locality, would Elias–Fano lists be smaller (by about
0.8 bits/entry), and they decode slower. The size lever is the **order**, not the codec.

Is the layout "first word `zigzag(x0 − v)`, then gaps − 1, no degree prefix" the best for
SIMD decode? **It is the layout the literature converged on.** The measured cost that remains
— per-list overhead on short lists (lj 1.24–1.40× at 1 thread) — was tested against block
framing in #30 (2–5% gain on lj, losses elsewhere). Two layout changes have evidence behind
them: a zero-length word (Challenge 2) and chunked hub records (Challenge 3). A third is
inference only: a random list costs two Elias–Fano selects (byte offset, entry offset) before
the data, so three cache misses against raw's two (Challenge 9).

### D5 — webgraph optional

- **Best known work:** webgraph-rs 0.7.0 (2026-10-12) is the reference BV/LLP implementation.
  It reaches random access at 17–47 ns per arc (Fontana et al. 2024, i7-12700KF / Xeon Gold
  6342) — about 10× slower per arc than our 87–217 ns per list at mean degrees 17–76
  (arithmetic, different machines). It still depends on `mmap-rs` and on the `zstd` binding
  crate (lib.rs dependency list), so the wasm verdict stands (inferred, not built).
  Cologni–Vigna (WAW 2026) add a Huffman/π-code variant: up to −15% vs BV, up to 54% faster
  random access than Zuckerli (abstract only).
- **Verdict: level, decision confirmed.** One addition: m2-15 should export LLP and BP orders
  (both exist in the webgraph ecosystem or as small Rust code) as permutations, so m1-09 can
  compare them on our codec.

### D6 — in-adjacency as a derived artifact

- **Best known work:** Ligra, Ligra+ and GBBS store compressed in- and out-adjacency for
  directed graphs; WebGraph stores the transpose as a second BV graph. k²-trees answer both
  directions from one structure, at 2–15 µs per neighbour (SPIRE'09). PoTra (2025) transposes
  up to 8.7× faster than prior parallel transposition by keeping buffers cache-sized and
  treating high- and low-degree vertices apart (abstract; 20 graphs up to 128 B edges).
- **Verdict: level.** For m2-11's "≤ 15 s on twitter-2010 at 16 threads", PoTra's design is
  the reference to read in full before building the transposition.

### m1-14 (#146) — seeded relabelling at ingest

- **What the literature says:** a random relabelling is the standard *worst case* for both
  compression and locality. LLP paper, Table 6 (BV bits/link): random ids against the
  natural order cost 23.50 vs 14.35 on LiveJournal (+64%), 23.62 vs 16.92 on Amazon (+40%),
  14.49 vs 13.87 on Flickr (+4%), and 10× on URL-ordered web graphs (uk 27.55 vs 2.75); LLP
  itself recovers the same size from either start (Table 7). DBG paper: random reordering slows
  applications by up to 28.5% on its datasets. Rabbit and BOBA use random ids as the baseline
  they beat by 2–3×. Our own measurement agrees: +26% bits on lj, +16% orkut, +5% Friendster.
- **Verdict: the "relabel always at ingest with a random bijection" option is behind the
  state of the art before it is measured.** It trades every kernel's locality and 5–26% of
  storage for the out-of-core visiting order of one algorithm. Two alternatives fit the
  literature better: (a) a Leiden-only relabelled working copy (already in the issue);
  (b) relabel to a **seeded locality order** (BP is seeded by its initial split) and test
  whether Leiden, visiting in that storage order, keeps its quality. Ordering by
  communities places neighbours together, which is the opposite of the decorrelation #146
  found useful; that is exactly what (b) must measure. The issue should add a locality
  ordering as a fifth arm (Challenge 5).

### m2-20 — archival codec (varint + zstd per block)

- **What the literature and our lab say:** on uk-2005 BV is both smaller (2.1 vs 2.4 bits) and
  57× faster at random access (768 ns vs 44 µs; FINDINGS § C, shared machine) than varint +
  zstd blocks. Zuckerli is another −20% to −30% vs BV on web graphs. Our byte tier loses on web
  graphs mainly because one byte plus two control bits (10 bits) is the floor for every gap,
  including the many gaps of 1 that BV codes as intervals.
- **Verdict: behind as specified.** Before building codec value 1, measure what ordering and
  a zero-length word do to uk-2005 on the byte tier (Challenges 1–2). Keep zstd blocks only if
  they stay the best wasm-portable archival option, and require a random-list target in
  microseconds, not tens of microseconds.

### m2-17 — zstd fast path

Out of this area's literature: a single zstd frame decodes sequentially by format, so the
options are what the issue lists (multi-frame or seekable input, or native libzstd behind a
feature). Nothing read changes the issue.

### m2-07 — id assignment (touches D4 directly)

- The issue leaves the policy open ("by first appearance, by sorted key, or by a later
  renumbering") and describes a "hash-partitioned parallel dedupe that emits dense ids".
- **What the literature says:** whatever order the ids get at ingest *is* the storage order.
  web-Google in its natural (random-like) order costs 20.08 BV bits/edge against 7.69 after
  a BFS order (BP, Table 2). Our own web-google is 21.9 bits/entry (PR #157). BOBA's whole
  contribution is that first-appearance order recovers much of what heavyweight orders give
  over random ids, at the cost of a degree pass.
- **Verdict: a risk, not yet a mistake.** If dense ids follow hash partitions, every
  string-keyed graph (URLs, addresses, user names) is stored at its worst size and locality.
  The default must be first appearance (deterministic: smallest input position wins, at any
  thread count) or sorted key. Never hash order (Challenge 4).

---

## 4. Challenges, ranked

| # | Change | Expected gain (source, case) | Cost and risk to our promises | Cheapest experiment that settles it | Issue |
|---|---|---|---|---|---|
| 1 | **A deterministic locality ordering as an optional rewrite of the topology** (`compact --order`), outside the ingest path. Two candidates: BP with a fixed seed, fixed depth (log n − 5) and an iteration cap; and the order of the product's own Leiden hierarchy (DFS of the dendrogram, i.e. Rabbit Order's ordering step on a deterministic community algorithm). | Size: BP −27% BV on LiveJournal, −46% on Twitter-sym vs its source order (BP paper, 32-core Xeon); LLP −26% BV and −14% Stream VByte on orkut (our lab). Extrapolation, labelled: a BP order might take lj from 18.1 to about 15.5–16 bits and Friendster, whose ids carry no locality, well below 23. Kernels: raw PageRank −32% on orkut with LLP (our lab). Gains are largest exactly on our headline graphs: Friendster sits above the random-list bound and orkut close to it (§ 3, arithmetic). | BP: about 500–800 lines, parallel by subtree; deterministic if each split is seeded and ties break by id; time bounded by depth × iterations × m; needs the graph in RAM. At the paper's speed (149 min on twitter-sym, ÷ 3.7 for Mackenzie's tricks) it is minutes, not seconds: never inside promise 1's ingest; must be opt-in or deferred, and budgeted. A renumbering also permutes the dictionary (D8's rank bitmap for integer keys becomes a 4-byte permutation) and changes Leiden's visiting order (D15). | m1-09 widened: lj, orkut, twitter-sym, Friendster, uk-2005 × {original, random, BFS, DBG, Leiden-hierarchy key, BP (Petri–Mackenzie code in `lab/`), LLP (webgraph)} → our bits/entry, log-gap cost, reordering time, and PageRank / BFS / WCC time as interleaved A/B in one process at 16 threads, under the 8 GB scope. | m1-09 (widen); new: "Deterministic BP ordering as a compaction step" |
| 2 | **A zero-length word: Stream VByte 0124 as block codec 2** (lengths 0, 1, 2, 4 bytes; word 0 = consecutive neighbour costs 2 bits instead of 10), chosen per block by the encoder when smaller. | Lemire's library ships this variant for data with many zeros (README). Gain depends on the share of gaps equal to 1: large on web graphs and after any locality ordering, zero or negative on unordered social graphs (3-byte words become 4). Not measured by anyone on graphs. | ~150 lines, a second shuffle table, the decoders and the fuzz target; the codec byte already exists, so version 1 readers refuse it cleanly. No effect on determinism. | Offline, no timing: histogram of word byte-lengths per block on lj, orkut, uk-2005, web-google, before and after the best order of Challenge 1 → exact bits for codec 0 vs 2 per block. Decode speed by interleaved A/B in one process. | new: "Block codec 2: Stream VByte 0124" (feeds m2-20) |
| 3 | **Chunked hub records** (Ligra+ / GBBS): lists above T ≈ 1,024 entries stored as chunks, each chunk's first word relative to v, with chunk offsets in the record. | Ligra+: without it, "up to 2 times slower" on some application/graph pairs (40 cores); GBBS needs it for parallel map, filter and intersection on 225 B-edge graphs. Ours: BFS 1.20–1.52× raw at 16 threads, budgeted 3-hop 2.1× on twitter-sym, Friendster WCC 1.36–1.57× — part of each is whole-hub decode (FINDINGS § M1-13). | Format: codec value or version 2 (spec § 4 says "no list is ever split"). About 0.03 bits/entry at T = 1,024 (arithmetic: one 4-byte offset per 1,024 entries). Intersections (triangles, common neighbours) can skip by chunk starts. | twitter-sym and Friendster: BFS, 3-hop with a 10k budget, common neighbours, triangle forward lists, interleaved A/B chunked vs unchunked in one binary. | new: "Chunked records for hub lists" |
| 4 | **Id assignment by first appearance (or sorted key), never by hash partition**, as m2-07's default. | Natural vs random-like order on web-Google: 20.08 vs 7.69 BV bits/edge after BFS (BP Table 2); BOBA's first-appearance order gets near-heavyweight locality on edge lists in input order. Our web-google: 21.9 bits/entry. | Small: the dedupe keeps the minimum input position per key and ranks by it (deterministic at any thread count). Costs one u64 per distinct key during dedupe. | lj and web-google ingested with string ids under hash order, first appearance and sorted key → bits/entry and PageRank time (interleaved). Minutes. | m2-07 (acceptance criterion) |
| 5 | **m1-14: add a seeded locality order as an arm, and default against random relabelling of the stored graph.** | Random ids against natural order: +4% (Flickr) to +64% (LiveJournal) BV bits on social graphs, 10× on web graphs (LLP Table 6); up to 28.5% application slowdown (DBG); ours +5–26% bits. | None to build; it changes what the issue measures. | As in #146, plus "BP order (seeded), Leiden visiting in storage order" at 16, 8 and 4 GiB on Friendster and orkut. | m1-14 |
| 6 | **m2-20 judged against BV, after 1–2.** | uk-2005: BV 2.1 bits at 768 ns per random list; varint + zstd 2.4 bits at 44 µs (our lab). | Avoids building a tier that loses on both axes. | Re-run uk-2005 with the best order + codec 2 on the byte tier; if it is within 2× of BV, the archival tier becomes "BV through the webgraph feature" for native builds and is dropped for wasm. | m2-20 |
| 7 | **A locality score from the encoder**: mean log2 gap per entry and the ratio of bits/entry to the random-list bound 2 + log2(n/d), stored in the stats, to tell the user and the planner when an ordering is worth offering. | LLP: log-gap cost correlates 0.968 with BV bits/link. Balaji–Lucia (slides): a packing factor predicts HubSort speed-up with Pearson 0.92. Arithmetic on our stores: the ratio is about 0.86 (lj), 0.95 (orkut), 0.74 (twitter-sym), 1.04 (Friendster). | A few lines in the encoder; no format change (a stats field). | Compute on the existing stores; check the ratio ranks graphs by the gain Challenge 1 measures. | m2-03 follow-up or m1-09 |
| 8 | **Transposition in the style of PoTra** for D6. | Up to 8.7× faster transposition than prior work (abstract; 3 CPU architectures, 20 graphs). | Inside m2-11's scope; determinism is unaffected if each in-list is sorted. | Read PoTra in full; twitter-2010 transposition time against the 15 s target. | m2-11 |
| 9 | **Fewer cache misses per random list**: the two Elias–Fano selects (byte offset, entry offset) touch separate memory; co-locating their select samples, or reading the degree from the byte-offset sequence plus the control bytes, may save one miss. | Inference only: three dependent misses against raw's two; neighbour lookup +0.02–0.11 µs (FINDINGS § M1-13). | Reader-side only if samples are not in the format. | perf cache-miss counts on 100 k random lists, orkut and Friendster, one thread. | m7-01 |
| 10 | **A wider decoder** (two 128-bit groups per step on AVX2 / AVX-512). | Nobody has published one for Stream VByte (Lemire's library has none). Matters only at 1–4 threads, where we decode 2.2–2.5× slower than a raw scan. | Another `unsafe` path to keep identical to scalar; low priority. | Microbenchmark decode-only, one thread, interleaved. | none yet |

---

## 5. What we have that the literature does not — novelty checked

| Our idea | Already done? | Honest status |
|---|---|---|
| Compressed adjacency as the default because it is faster than raw at high thread counts | Ligra+ (2015) showed compressed faster than uncompressed at 40 cores on Twitter, uk-union, Yahoo for most applications; GBBS made it the only way to fit 225 B edges | **not new**; our contribution is the measurement on our kernels |
| Elias–Fano per-node offsets plus an Elias–Fano cumulative-degree index | webgraph-rs stores Elias–Fano offsets (`BASENAME.ef`) and computes a degree cumulative function; cumulative sums in Elias–Fano are textbook succinct practice | **not new** |
| One codec in a portable core that runs in the browser with a `simd128` decoder | No graph codec in what we read targets wasm | **new as a combination**, not as a codec |
| Raw CSR as a planner decode-cache tier with a break-even rule | Not found in the works read; the "amortized cost" metric for reordering (SSDBM'25, abstract) and Ligra+'s remark on amortized encoding are the same reasoning applied elsewhere | **probably new as a planner policy**; the reasoning is standard |
| Storage order = Leiden-hierarchy order (m1-09), shared with the map layout | Rabbit Order (2016) orders by DFS of a Louvain-style dendrogram; LLP (2011) combines clusterings at several resolutions; Dubuisson (2026) runs LLP inside community groups | **not new as an ordering**; new only in reusing the product's deterministic hierarchy so that storage, layout and visiting order are one seeded permutation |
| A deterministic ordering independent of thread count and budget | BP is deterministic by construction once seeded (inference from its structure; LLVM ships a BP for function layout); Rabbit is not | **not new**, but rare among the fast orderings |
| Per-block codec byte to mix encodings | PEF and Opt-VByte choose an encoding per partition in inverted indexes | **not new** in IR; uncommon in graph stores (Ligra+ uses one code per graph) |

---

## 6. What to ignore, and why

- **Gorder:** best locality but 25,475 s on twitter (sequential code, replication); DFS and RCM tie it in half the experiments.
- **Rabbit Order code:** nondeterministic by design (CAS merges) and NTT's evaluation-only licence (earlier survey); keep only its idea, which Challenge 1 already covers.
- **k²-trees (incl. 2026 depth-first variants):** 3.3–5.3 bits/link on web graphs at 2–15 µs per neighbour; built for edge queries, not scans.
- **Copy, Predict, Code the Rest (CB):** smallest files, sequential access only, slower than WebGraph and Zuckerli.
- **Zuckerli as a dependency:** C++, compresses at 0.4–1.6 M edges/s; its ideas reach us through webgraph-rs.
- **Aspen, CPAM, Terrace, BYO's dynamic containers:** for streams of updates; the `.eg` store is immutable. Revisit for live mode or temporal segments (m5-08, m9-04).
- **CompressGraph / Laconic (rule-based):** CompressGraph's peak memory was 65.38 GB according to Laconic, and its compression is slow; a poor fit for a hard budget.
- **Roaring and bitmaps for adjacency:** the largest of all codecs in Pibiri–Venturini; use them for frontiers and view masks, not lists.
- **D4 (vectorized) deltas, SIMD-BP128 / FastPFor per list:** + up to 2 bits/int, and blocks of 128 do not fit lists of 17–76 entries.
- **Log(Graph):** 20–35% smaller than uncompressed GAPBS arrays; we are already at about 50%.
- **SlashBurn:** worse than the natural order for BV on social graphs (BP Table 2: FB-NewOrlean 12.65 vs 10.79 for BFS).
- **Masked VByte, Varint-G8IU:** slower than Stream VByte on the same data (Lemire 2018).
- **Hub sorting / degree sorting as a storage order:** helps vertex-state locality a little on structured graphs (+6.5% for DBG) but costs compression (orkut BV 17.75 vs 15.25); at most an in-RAM working order for one kernel.
