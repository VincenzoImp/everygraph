# State of the art: human factors — accessibility, API usability, privacy of shared maps, graphs for LLM agents

Second research wave, 2026-11-25. Written by a research subagent and recorded by the
coordinator. PDFs in `/home/tux/graphlab/papers/human-factors/` (text in `txt/`). Nothing was
built or run.

**Gaps in the reading.** The web-search budget ran out late in the work. Several key papers sit
behind bot walls and were read as abstract only: the VIS'22 "Barriers" paper, Torres–Barwaldt,
Becker'19, Pimentel'19, Faulkner'03, Sampath'21. No paper studying NetworkX, igraph or
graph-tool users was found — that gap is itself a finding.

**Work in development.** `.eg` container spec (PR #170): no change needed. Its provenance
records (path as given, modification time, input digest) are fine inside a local `.eg` but
become a leak if the map's "About this map" panel copies them into a shared `.egmap`
(challenge 1); the change belongs in the `.egmap` spec (m5-02) and the static export (#75,
m5-09). Edge-map framework (#44): nothing in this area.

## 1. Scope and reading log

Scope: topics 6, 10 and 11 of `sota-coverage.md` § (c).

| Work | Venue, year | Link | Read | PDF |
|---|---|---|---|---|
| Zong, Lee, Lundgard et al., Rich screen reader experiences for accessible data visualization | EuroVis 2022 (CGF) | arxiv.org/abs/2205.04917 | full text | 2022-zong-rich-screen-reader.pdf |
| Zhao, Nacenta, Sukhai, Somanath, TADA: node-link diagrams for blind and low-vision people | CHI 2024 | arxiv.org/abs/2311.04502 | full text (method, study, limitations) | 2024-zhao-tada.pdf |
| Elavsky, Nadolskis, Moritz, Data Navigator | arXiv 2023 / VIS | arxiv.org/abs/2308.08475 | full text (design, rendering, cases) | 2023-elavsky-data-navigator.pdf |
| Torres, Barwaldt, Approaches for diagrams accessibility for blind people: a systematic review | IEEE FIE 2019 | doi.org/10.1109/FIE43999.2019.9028522 | not accessible (TADA's citation only) | — |
| AlKadi, Serrano, Scott-Brown, Plaisant, Fekete, Hinrichs, Bach, Understanding barriers to network exploration | VIS 2022 (TVCG 29(1)) | doi.org/10.1109/TVCG.2022.3209487 | abstract only (bot walls) | — |
| Li, Appleby, Brumar, Chang, Suh, Knowledge graphs in practice | VIS 2023 (TVCG) | arxiv.org/abs/2304.01311 | full text (§3–6) | 2023-li-kg-in-practice.pdf |
| Sahu et al., The ubiquity of large graphs and surprising challenges of graph processing | VLDB 2017 / VLDBJ | arxiv.org/abs/1709.03188 | results (§2–3.4) | 2017-sahu-ubiquity-large-graphs.pdf |
| Meng et al., Revisiting graph analytics benchmark (LLM-based API usability) | SIGMOD 2025 | arxiv.org/abs/2506.21811 | §5, §8.4 full; rest skimmed | 2025-meng-revisiting-graph-analytics-benchmark.pdf |
| Ledo et al., Evaluation strategies for HCI toolkit research | CHI 2018 | doi.org/10.1145/3173574.3173610 | most of the text | 2018-ledo-toolkit-evaluation.pdf |
| Faulkner, Beyond the five-user assumption | BRMIC 2003 | doi.org/10.3758/BF03195514 | abstract only | — |
| Sampath, Merrick, Macvean, Accessibility of command line interfaces | CHI 2021 | doi.org/10.1145/3411764.3445544 | abstract only | — |
| Becker, Denny et al., Compiler error messages considered unhelpful | ITiCSE-WGR 2019 | doi.org/10.1145/3344429.3372508 | abstract only | — |
| Pimentel, Murta, Braganholo, Freire, Quality and reproducibility of Jupyter notebooks | MSR 2019 | doi.org/10.1109/MSR.2019.00077 | abstract only | — |
| Edge et al., From local to global: a Graph RAG approach | arXiv 2024 | arxiv.org/abs/2404.16130 | full text (method, exp. 1, limitations) | 2024-edge-graphrag.pdf |
| Zhou et al., In-depth analysis of graph-based RAG in a unified framework | PVLDB 2025 | arxiv.org/abs/2503.04338 | lessons, cost tables, CheapRAG | 2025-zhou-graphrag-unified.pdf |
| Sarthi et al., RAPTOR | ICLR 2024 | arxiv.org/abs/2401.18059 | abstract and results | 2024-sarthi-raptor.pdf |
| Guo et al., LightRAG | arXiv 2024 | arxiv.org/abs/2410.05779 | downloaded, not read (covered through Zhou) | 2024-guo-lightrag.pdf |
| Fatemi, Halcrow, Perozzi, Talk like a graph | ICLR 2024 | arxiv.org/abs/2310.04560 | abstract and results | 2024-fatemi-talk-like-a-graph.pdf |
| Greshake et al., Indirect prompt injection | AISec 2023 | arxiv.org/abs/2302.12173 | abstract | 2023-greshake-indirect-prompt-injection.pdf |
| Narayanan, Shmatikov, De-anonymizing social networks | S&P 2009 | arxiv.org/abs/0903.3276 | results (§6) | 2009-narayanan-deanonymizing-social-networks.pdf |
| Hay, Rastogi, Miklau, Suciu, Boosting accuracy of DP histograms through consistency | PVLDB 2010 | arxiv.org/abs/0904.0942 | abstract and method | 2010-hay-dp-histograms-consistency.pdf |
| Kuo, Chiu, Kifer, Hay, Machanavajjhala, DP hierarchical count-of-counts histograms | PVLDB 2018 | arxiv.org/abs/1804.00370 | abstract | 2018-kuo-dp-hierarchical-count-of-counts.pdf |
| Nguyen, Imine, Rusinowitch, Detecting communities under differential privacy | WPES 2016 | arxiv.org/abs/1607.02060 | method and evaluation | 2016-nguyen-dp-communities.pdf |
| Mundra, Papamanthou, Shun, Liu, Local edge-DP graph algorithms | PVLDB 2025 | vldb.org/pvldb/vol18/p4199-mundra.pdf | abstract and introduction | 2025-mundra-ledp-graph-algorithms.pdf |
| Brito et al., Global and local DP release of count-weighted graphs | PACMMOD 2023 | doi.org/10.1145/3589299 | not accessible; EDBT'24 tutorial outline read | 2024-brito-dp-guide-graph-analytics.pdf |
| WCAG 2.2; WAI-ARIA tree view, `aria-setsize`/`aria-posinset` | W3C | w3.org/TR/WCAG22 | from the standards, not re-fetched | — |
| GraphRAG configuration docs (`cluster_graph.max_cluster_size`, `seed`) | software | microsoft.github.io/graphrag/config/yaml | page read; defaults not on the page | — |

Our side, read: `docs/DESIGN.md` § 2, D7, D14, D16, D18, D24, D25; `design/DECISIONS.md`;
`docs/api-sketch.md` § 1 story 5, § 2.7, § 3.8; `docs/ux-contract.md` § 2.2–2.3 and § 9 (share
hint); `design/preview/style/` `foundations.html` § 9, `viewer.html` (keys), `components.html`
(ARIA counts); `design/preview/choices.md` C3; issues m5-02, m5-05, m5-06, m5-09, m5-10,
m5-11, m5-12, m5-13, m6-01, m6-08, m8-01, m8-08, m8-09, m9-02, m1-05, m4-07; provenance in
`docs/spec/eg-container.md` (branch `m2-01-container-spec`).

## 2. The state of the art, as a short map

**Non-visual access to visualizations.**
- Baseline practice (alt text plus a data table) fails: Zong cites Sharif et al. 2021 — 211%
  more time and 61% lower accuracy with screen readers.
- Best published approach: a navigable tree (Zong 2022, Olli) with structural moves (up/down
  levels, sideways between siblings, Home to root), spatial moves (by screen direction) and
  targeted moves (landmark, search hit, list). Descriptions follow rules on content, order and
  verbosity; the level is spoken only when it changes.
- Zong's study (13 BLV participants): a table stays necessary because it is familiar; the
  multi-view tree was significantly harder to learn than the table (Friedman χ²(2, N=13) =
  15.05, p < 0.01, Kendall's W = 0.58); all prototypes had median perceived usefulness 4/5.
- Data Navigator generalizes trees to graphs and renders only the focused element into the
  DOM — suits a canvas with millions of marks. No user study.
- TADA (touch + audio on a tablet, 25 BLV participants): the only BLV study of node-link
  diagrams found; 22/25 answered all questions correctly, 7/25 needed help. Small diagrams,
  deliberately no zoom or pan, no regions or containment (future work).
- Nobody has studied BLV access to a million-node zoomable network map.

**Usability of graph tools.** User research is thin; these describe problems, not tested designs.
- Sahu 2017: 89 users ranked scalability (45), visualization (39), query languages or APIs (39),
  usability (25) as top challenges.
- Li 2023 (19 KG practitioners): 9/19 called node-link diagrams "hairballs"; consumers
  preferred tables; they asked for interim results and summary "knowledge cards"; 4/19 used
  NetworkX.
- AlKadi 2022 (abstract): barriers come before the tool — setting goals, choosing structures
  and abstractions, choosing visual mappings.
- No study of NetworkX, igraph or graph-tool API users found.
- Measuring API usability: Meng 2025 has an LLM generate code at four prompt levels and an LLM
  grade it; agreement with > 80 human reviewers Spearman 0.75 / 0.71 over only 7 platforms, with
  visible disagreements. Ledo 2018: four evaluation strategies across 68 toolkit papers
  (demonstration 47, usage studies 35, technical benchmarks, heuristics); warns of the
  "usability trap" of small controlled studies. Faulkner (abstract): random groups of 5 users
  found 55–99% of problems; groups of 20 found ≥ 95%.

**Privacy of released graph summaries.**
- Naive anonymization fails: Narayanan–Shmatikov re-identified 30.8% of mapped users of an
  anonymized Twitter graph using Flickr as auxiliary data, at 12.1% error.
- Hierarchical counts can be released under DP with consistency post-processing (children sum
  to the parent; Hay 2010, Kuo 2018).
- DP community detection (Nguyen 2016, LouvainDP and ModDivisive — a k-ary top-down tree) needs
  ε of 0.1–0.5·ln n (0.5·ln n = 7.0 on youtube, 1.1 M nodes) to be useful — a weak guarantee —
  and modularity stays well below non-private Louvain (read from figures).
- Local edge-DP (Mundra 2025) assumes an untrusted curator; our analyst holds the whole graph,
  so it is the wrong model.

**Graphs for LLM agents.**
- GraphRAG extracts small graphs from text (8,564 nodes / 20,691 edges podcast; 15,754 / 19,520
  news), runs hierarchical Leiden (graspologic), LLM-summarizes communities bottom-up, answers by
  map-reduce over one level. Against vector RAG: comprehensiveness win rates 72–83% (LLM-judged);
  root-level summaries use 97% fewer tokens than summarizing the source.
- Zhou 2025: global GraphRAG up to 712 s and 448,762 tokens per query; filtering communities by
  relevance first (CheapRAG) cuts this.
- Fatemi 2024: LLMs reason poorly over graphs as text; on 5–20-node graphs accuracy swings 4.8%
  to 61.8% with the encoding — they should call tools that return exact numbers.
- Greshake 2023: retrieved text is an injection channel; node labels are attacker-controlled.

## 3. Decision by decision

| Ours | Best known | Verdict | Evidence and note |
|---|---|---|---|
| D16 fan-out-capped hierarchy (8–64, m1-05, m4-07) | Zong: trees with bounded rooms; ModDivisive: k-ary tree | **Ahead, by accident** | Fan-out ≤ 64 → root-to-node path on Friendster ≈ 5–6 steps (64⁴ = 16.7 M, 64⁵ = 1.07 B; arithmetic), each a list of ≤ 64 siblings: screen-reader-navigable. GraphRAG's uncapped hierarchy is not. |
| D18 / m5-05 WebGL2 canvas viewer | Data Navigator; ARIA tree with `aria-setsize`/`aria-posinset` | **Behind (not specified)** | m5-05 has no semantic layer; `viewer.html` exposes one `<canvas aria-label>` plus an `aria-live` panel. |
| m5-11 "screen-reader summary of the current view" | Zong, Data Navigator, WCAG 2.2 | **Behind** | A summary is alt text, which Zong shows is insufficient; no structure, no navigation model, no acceptance criterion. |
| Keyboard model, `foundations.html` § 9 | Zong: arrows through structure, separate keys spatially | **Contradiction in the approved design** | § 9: "arrow keys move between regions, Enter opens one, Esc goes up"; `viewer.html` (~line 1865) makes arrows pan and binds `/ + - 0` at document level — WCAG 2.1.4 allows single-key shortcuts only if they can be turned off/remapped or are active only on focus. |
| D24 contrast tokens | WCAG 1.4.3, 1.4.11, 1.4.1 | **Level for text; non-text unverified** | Region outlines (1.5 px, 12%/5% tint) and focus rings on the dark canvas need 3:1 (1.4.11); not measured. |
| Pointer interaction | WCAG 2.2: 2.5.7, 2.5.1, 2.5.8 | **Mostly level** | Zoom/fit buttons exist; 24 px targets stated. Panning without a drag needs keys or buttons — resolve with the keyboard model. |
| m5-06 "5 people, ≥ 4 of 5 succeed" | Ledo; Faulkner | **Weak as a claim** | 4/5 has an exact 95% lower bound of 0.28 (Clopper–Pearson). Fine as formative. |
| m6-08 "first map < 5 min, 3 people" | same | **Weak** | 3/3 → lower bound 0.29. |
| m8-08 pre-launch review by 3–5 practitioners | Ledo | **Level** | Add a cognitive-walkthrough script of the ten stories. |
| API error pattern (`ux-contract` § 6.1) | Becker'19 (abstract) | **Probably level, unverified** | Comparing the guidelines with § 6.1 is a cheap check. |
| CLI renderings (`ux-contract` § 2.2) | Sampath'21 (abstract) | **Small gap** | `TERM=dumb` only removes colour; redrawn bars remain, noise for a screen reader. Piped form should also apply under `TERM=dumb` or `--progress=plain`. `--json` everywhere is good. |
| Notebook widget (m5-10) | — | **Gap** | Esc = "up a level" in the map and "command mode" in JupyterLab; focus must not trap the keyboard (WCAG 2.1.2). |
| Analyst persona and preflight | AlKadi; Li | **Partly level** | Plan card, phases, flagged partial results, inspection panel ≈ knowledge cards, aggregation. Missing: a goal-setting step ("what to ask of this map"); m5-12 could own it. |
| DECISIONS "What a shared map reveals" (`reveal` required, `"full"`/`"summary"`) | Narayanan; disclosure control; Hay/Kuo | **Right decision; "summary" under-specified and, as specified, not safe** | m5-02's tiles hold individual nodes (id, degree, tree path) and per-node adjacency; without ids that is still a naively anonymized topology. Exact counts plus exact parent totals reveal any single hidden child by subtraction. Provenance and m5-13's attribute summaries and time slider add channels. |
| m8-09 "anonymized structure-only export (`--topology-only`)" | Narayanan | **Wrong word** | Topology alone is re-identifiable; acceptable with consent, never "anonymized". |
| D14 vs future DP noise | DP needs secret randomness | **Note for later** | Seeded noise is fine only if the seed never enters the file; repeated exact releases allow differencing. |
| m9-02 MCP server | GraphRAG/CheapRAG; Fatemi; Greshake | **Mostly ahead; one omission** | Exact tool answers with budgets and truncation flags are what LLMs need; a top-down walk of a capped tree is CheapRAG without LLM summaries. Missing: labels/properties are untrusted input (prompt injection); the reveal level should apply to agents. |
| m4-06 hierarchy artifact | GraphRAG cites communities by id | **Level, if region ids are stable** | D14 makes region ids reproducible; say so in m4-06. |

## 4. Challenges, ranked

1. **Define `reveal="summary"` precisely in the `.egmap` spec (m5-02) and the export (#75,
   m5-09) before the format freezes.** A summary map has: no node-level entities, no per-node
   adjacency, no search index, no node attributes; regions under k nodes merged into the existing
   `aggregate` region kind (merging, not hiding a number, keeps "parent = sum of shown children"
   exact and leaves nothing to subtract); inter-region edge aggregates below k merged into
   "other" with consistent totals; provenance reduced to a label; attribute summaries and the
   time slider off unless asked; k written in the metadata and the caption ("groups under k
   hidden"). Gain: closes the Narayanan and subtraction channels, keeps exact numbers for what is
   shown. Cost small; no risk to determinism. Test: an adversarial test that tries to reconstruct
   any hidden count from the file and must fail. Issue: amend m5-02, m5-09; new "Summary export:
   k-threshold merging, stripped provenance, a reconstruction test". In v0.1: yes.
2. **A non-visual route: the hierarchy as a navigable tree, synced with the canvas.** Structure
   layer rendering only the focused region (Data Navigator) with `aria-setsize`/`aria-posinset`
   ("3 of 64"); arrows down/up through levels, sideways between siblings ordered by size, Home to
   root; targeted navigation via search and breadcrumb landmarks; a description template (kind,
   exact count, position among siblings, strongest connection with weight, "shown X of Y", level
   spoken only on change); "children as a table" for every region; the canvas highlights the
   focused region. D16's cap makes it feasible. Cost medium (viewer). Experiment: a cooperative
   session (Zong's protocol) with 3–5 screen-reader users on lj, m5-06 tasks, formative. Issue:
   split out of m5-11 as "Non-visual navigation of the map hierarchy" with acceptance criteria.
3. **Fix the keyboard contradiction and single-key shortcuts.** Arrows for structure (as § 9
   promises), Shift+arrows or WASD for panning; `/ + - 0` active only while the map has focus or
   switchable (WCAG 2.1.4); resolve Esc in the Jupyter widget. A DECISIONS row and a preview edit
   (m5-06, m5-10).
4. **Restate the evidence behind usability claims.** 5-person sessions are formative; a success
   rate is claimed only from n ≥ 20 per task with an exact interval (18/20 → lower bound 0.68);
   report failures and participant profiles. Accessibility: WCAG 2.2 AA for the interface around
   the map (automated tools plus a manual checklist — Chartability's 50 criteria); "usable with a
   screen reader" only after a session with screen-reader users. Add Meng's LLM-in-the-loop run
   of the ten stories against our docs as a regression check (wrong calls counted), not as
   evidence. Amend m5-06, m6-08, m8-08; add the LLM docs check to m8-02.
5. **WCAG 2.2 AA checklist for the viewer in CI:** 1.4.11 non-text contrast on the dark canvas,
   2.5.7 panning without dragging, 1.4.10 reflow at 320 CSS px, 4.1.3 status messages (tile
   loading, "shown X of Y"), 2.3.3 reduced motion, 200% browser zoom scaling map text. Playwright
   beside m5-11's frame tests; run on `viewer.html` now to list failures.
6. **Screen-reader-readable command line:** piped form (no bars, no redraw, one line per phase)
   also under `TERM=dumb`, plus `--progress=plain`; document in `ux-contract` § 2.2 (m8-01).
7. **Stop calling `--topology-only` anonymized** (m8-09); print a one-line notice that structure
   alone can identify people.
8. **MCP (m9-02):** mark labels and properties as untrusted data; apply the reveal level to agent
   sessions; a "region card" tool (exact counts, top nodes by a metric, strongest links,
   children — a GraphRAG summary without an LLM, deterministic, citable by stable region id,
   m4-06); a top-down "drill" tool (CheapRAG on our capped tree). Extend the 30-question
   evaluation with two-level drill-downs and one injected label.
9. **DP export: not in v0.1.** It breaks promise 3's exact numbers; useful DP communities need ε
   around ln n; the partition itself would have to be private. Later: a labelled `reveal="dp"`
   with consistent noisy region counts (Hay/Kuo) over a public or DP hierarchy, seed kept out of
   the file. New m9 research issue.
10. **Goal-first onboarding for analysts** (AlKadi): a "questions to ask of this map" page linked
    from the plan card, in m5-12's docs page.

## 5. Ideas the literature does not have and we do, checked

| Claim | Honest check |
|---|---|
| A capped map hierarchy as the screen-reader structure for a whole large graph | **New as far as found.** Zong's trees are for charts; TADA is small diagrams without zoom or regions; Data Navigator has no user study; ModDivisive's k-ary tree is for privacy only. |
| Exact, deterministic region summaries as agent context, without LLM summarization | **New in combination.** GraphRAG: hierarchical Leiden + LLM summaries on ~10⁴-node graphs; RAPTOR: trees of text. Neither has exact counts or run-stable ids. |
| Small-count suppression by merging into typed `aggregate` regions, leaving nothing to subtract | **New as a map technique as far as found.** Merging small cells is standard disclosure-control practice (not read here); Hay/Kuo enforce the same consistency for noisy counts. |

## 6. What to ignore, and why

- Local edge-DP algorithms (Mundra 2025): assume no trusted curator; our analyst holds the graph.
- TADA's touch-and-audio interaction: needs a tablet and a fixed screen without zoom (its task
  list, from Lee et al.'s taxonomy, is worth reusing).
- LightRAG and other graph-RAG variants: they build graphs from text; we receive graphs.
- Sonification of the whole map: no evidence at our scale.
- LLM-judged API usability as a published claim: a 7-platform rank correlation is too weak.
- EU Accessibility Act and other legal duties: not researched; the target is WCAG 2.2 AA on merit.
