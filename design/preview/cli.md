# Preview — the command line

**Approved by the owner on 2026-11-25. Not working software.** All figures are invented
to show shape and tone. In each **Choice** the option marked ★ is the decision. Later rows
of `design/DECISIONS.md` change some details of this page. `docs/ux-contract.md` and
`docs/api-sketch.md` are the current reference for commands and messages.

The command is `everygraph` (the short `eg` is already another program on PyPI).

> **Choice 1 — a short alias?**
> ★ **A.** `everygraph` only; people add their own shell alias.
> **B.** Also install a short alias. Candidates that looked free: `evg`, `egr`.

---

## 1. From a file to a map, three commands

```console
$ everygraph ingest transfers.parquet --source from --target to --weight value --time block_time
transfers.parquet · 38.2 GB · 1,830,402,118 rows

  ids       hex addresses (20 bytes) · ~212.4 M distinct
  edges     directed · duplicates kept as events · 4,201 self-loops dropped
  columns   value → weight · block_time → time · ignored: gas, nonce, input

  machine   16 cores · 61 GB RAM (44 GB free) · NVMe, 1.4 TB free
  budget    8.0 GB memory (from --memory or the default; see Choice 4)
  plan      build on disk — the graph is 4.6× the memory budget
  estimate  3 min 40 s · peak 5.9 GB · 61 GB scratch in /tmp · result 9.3 GB

  reading       ████████████████  1,830,402,118 rows          1 min 41 s
  identifiers   ████████████████  212,418,907 addresses            38 s
  building      ████████████████  2,048 parts                      59 s
  checking      ████████████████  sorted · symmetric counts        12 s

✓ transfers.eg · 9.3 GB · 212,418,907 nodes · 1,830,397,917 edges · 3 min 30 s
  next: everygraph map transfers.eg
```

```console
$ everygraph map transfers.eg
  communities   ████████████████  31,960 communities · modularity 0.603  2 min 11 s
  hierarchy     ████████████████  6 zoom levels                              18 s
  layout        ████████████████                                             41 s
  tiles         ████████████████  1,204,118 tiles                         1 min 2 s

✓ transfers.egmap · 4.1 GB
  open it:   everygraph open transfers.eg
  share it:  upload transfers.egmap and viewer.html to any static host
```

```console
$ everygraph open transfers.eg
Serving on http://127.0.0.1:7461 (this computer only) — opening your browser.
Ctrl-C to stop.
```

> **Choice 2 — how much does it say?**
> ★ **A.** As above: a summary of what it understood, the plan with estimates, one line
> per phase, a result line and a suggested next command.
> **B.** Quiet by default (one progress line, one result line); the detail with `-v`.

> **Choice 3 — does it wait for a yes?**
> ★ **A.** It starts immediately; `--dry-run` shows the plan without running.
> **B.** It asks "Proceed? [Y/n]" when the estimate exceeds 10 minutes or 20 GB of
> scratch; `--yes` skips the question.

> **Choice 4 — default memory budget**
> **A.** A fifth of free RAM (shown above): polite on a shared machine, slower.
> ★ **B.** Half of free RAM: usually what a person at their own computer wants.
> **C.** Everything that is free.

---

## 2. When it does not fit

```console
$ everygraph run communities transfers.eg --memory 4GB
✗ Not enough memory for this step.

  communities needs 5.1 GB for per-node state (212,418,907 nodes × 24 bytes).
  The budget is 4.0 GB. The edges stay on disk either way.

  Ways forward:
    everygraph run communities transfers.eg --memory 6GB
        the smallest budget that works
    everygraph run communities transfers.eg --precision low
        needs 3.4 GB; slightly lower quality

Nothing was changed.
```

## 3. Questions from the terminal

```console
$ everygraph run pagerank transfers.eg --top 5
node                                         pagerank    rank
0x28c6c06298d514db089934071355e5743bf21d60   0.00412907     1
0xdac17f958d2ee523a2206206994597c13d831ec7   0.00388114     2
0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48   0.00301775     3
0x21a31ee1afc51d94c2efccaa2092ad1028285549   0.00214466     4
0x7a250d5630b4cf539739df2c5dacb4c659f2488d   0.00198203     5

the full table is stored in transfers.eg · write it out with --out pagerank.parquet

$ everygraph query path transfers.eg 0x4f3a8c…91bc 0x9e12d0…07aa --max-hops 5
# (addresses shortened here; the command takes them in full)
4 hops · 1.3 ms
0x4f3a…91bc → 0x77e0…c1d4 → 0xb81c…5520 → 0x02fd…a9e3 → 0x9e12…07aa
```

> **Choice 5 — command shape**
> ★ **A.** Verb then thing: `everygraph run pagerank FILE`, `everygraph query path FILE A B`.
> **B.** Flat: `everygraph pagerank FILE`, `everygraph path FILE A B` — shorter, but the
> list of top-level commands grows with every algorithm.

> **Choice 6 — numbers for people or for programs?**
> ★ **A.** Readable on a terminal (thousands separators, aligned columns) and plain
> machine-readable output when piped or with `--json` / `--out file.parquet`.
> **B.** Always plain; readability is the job of other tools.

---

## 4. Looking at what you have

```console
$ everygraph info transfers.eg
transfers.eg · 9.3 GB · created 2026-11-24 from transfers.parquet

  nodes   212,418,907      edges   1,830,397,917 (directed, with duplicates as events)
  ids     hex addresses    time    2024-01-01 … 2024-12-31
  columns value (weight), block_time (time)

  computed so far
    degrees                           at ingest
    communities (Leiden, repeatable)  31,960 communities · 2 min 11 s · 2026-11-24
    pagerank                          41 s · 2026-11-24
    map                               transfers.egmap · 4.1 GB

$ everygraph doctor
  cores        16 physical (32 threads)         ✓
  memory       61 GB · 44 GB free               ✓
  storage      NVMe · 5.4 GB/s sequential read  ✓  out-of-core work will be fast
  scratch      /tmp on the same disk · 1.4 TB   ✓
  limits       no container memory limit        ✓
  graphics     discrete GPU found               ✓  the map will be smooth

  Suggested: --memory 22GB for single jobs on this machine.
```

---

## 5. Details of style

> **Choice 7 — colour and symbols**
> ★ **A.** Restrained colour (green ✓, red ✗, dim secondary text), Unicode bars; plain
> ASCII automatically when the terminal cannot show them or output is piped.
> **B.** No colour, ASCII only, always.

> **Choice 8 — large numbers**
> ★ **A.** Exact with separators in results (`1,830,397,917`), rounded in running text
> (`~212.4 M distinct`).
> **B.** Always exact. **C.** Always abbreviated (`1.83 B`).

> **Choice 9 — sizes**
> ★ **A.** `GB` meaning 10⁹ bytes, as disks and most tools show. **B.** `GiB` (2³⁰),
> technically unambiguous, unfamiliar to many.

> **Choice 10 — language**
> ★ **A.** English only for 0.1. **B.** Messages translatable from the start (Italian
> first) — more work in every message, forever.
