# Preview — choices that are not about pixels or syntax

Defaults, vocabulary and project decisions where reasonable people differ. Each **Choice**
has a recommendation (★). Answers go to `design/DECISIONS.md`.

## A. What the engine does when you do not say

> **A1 — repeated edges in the input** (the same pair appears many times, e.g. transfers)
> ★ **Keep them as separate events** and analyze the pair once with a weight (count or
> summed amount). Nothing is lost; time filters work.
> **Collapse to one edge** at ingest: smaller file, history gone.

> **A2 — direction when looking for communities**
> ★ **Ignore direction** (a transfer links two addresses whichever way it went); say so in
> the result. **Use direction**: stricter, slower, fewer tools to compare against.

> **A3 — edges from a node to itself**
> ★ **Drop and count them** in the ingest report. **Keep them.**

> **A4 — is the graph directed?**
> ★ **You say** (`directed=True/False`), and if you do not, it is treated as directed when
> the columns are named like a direction (`from`/`to`, `source`/`target`) and undirected
> otherwise, with a line in the summary. **Always ask explicitly** and refuse to guess.

> **A5 — how much of the machine to use**
> ★ **All physical cores, half of free memory**, unless told otherwise.
> **A polite fraction** (half the cores, a fifth of memory) for shared machines.

## B. Words

> **B1 — the groups the algorithm finds**
> ★ "communities" in the API and documentation. Alternatives: "clusters", "groups".

> **B2 — the areas on the map**
> ★ "regions", because some areas are real communities and some are only subdivisions
> made so the map stays readable; the map says which is which.
> **Call everything a community** — simpler, less honest.

> **B3 — names of regions on the map**
> ★ **Automatic, from the most central members** ("around 0x28c6…1d60 and 2 others") or
> from a column you choose (`label`), with "Region 1842" as the fallback.
> **Numbers only** until the user supplies labels.

> **B4 — the memory limit**
> ★ "memory budget" (`memory="8GB"`). Alternatives: "limit", "cap".

## C. Files

> **C1 — extensions**
> ★ `.eg` for a graph, `.egmap` for a shareable map. Alternatives: `.everygraph`,
> `.graph`/`.map` (too generic).

> **C2 — one file or two**
> ★ **Two**: the graph (large, private, for analysis) and the map (smaller, made to be
> shared, contains no more than what the map shows).
> **One** file with everything — simpler, but sharing a map would share all the data.

> **C3 — what a shared map reveals**
> ★ **Choose at export**: full (every node id and the links shown), or summary only
> (regions and counts, no individual ids). **Always full.**

## D. The project

> **D1 — when does the repository become public?**
> ★ **At the first usable release (0.1)**, with benchmarks and one showcase map.
> **Now / from M2**: feedback and contributors earlier, half-built things on display,
> name and patent checks must be done first.
> **Later**, after a paper.

> **D2 — who must love version 0.1?**
> ★ **The Python data scientist or researcher with graphs of 10⁶–10⁹ edges.**
> The analyst who never opens a terminal (needs the desktop app or the browser tier,
> both planned for later). The investigator on blockchain data specifically.

> **D3 — the first public showcase**
> ★ **The Wikipedia link graph**: everyone understands it, labels are meaningful, no
> sensitive data. **OpenAlex citations**: the billion-edge proof, speaks to researchers.
> **An Ethereum period**: your field, but labels and interpretation are delicate.

> **D4 — licence**
> ★ **Apache-2.0 or MIT, at the user's choice** (the Rust convention; maximally permissive).
> **Apache-2.0 only** (explicit patent grant, slightly simpler). **A copyleft licence**
> (keeps derivatives open, discourages companies).

> **D5 — NetworkX compatibility in 0.1**
> ★ **Leave it for after 0.1** (it is the largest single deferrable item: 2–3 months).
> **Include it** as an adoption channel from day one.

> **D6 — platforms in 0.1**
> ★ **Linux and macOS fully; Windows building and tested but without the out-of-core
> performance work.** **All three equal** — more work before release.

> **D7 — telemetry**
> ★ **None, and say so.** **Opt-in anonymous usage counts.**

## E. How results are presented

> **E1 — when something is approximate or cut short**
> ★ **Always marked in the result itself** (`truncated`, `approximate ± x`), never only
> in the documentation.

> **E2 — comparisons with other tools in our documentation**
> ★ **Include a "when to use something else" section** naming the tools that are better
> at specific things. **Only our strengths.**

> **E3 — performance claims**
> ★ **Only numbers anyone can reproduce with a published script**, with the hardware
> stated. **Best-case numbers** with a footnote.
