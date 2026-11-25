# Design previews — for the owner's review

Everything here is a **mockup with invented data**. Nothing is a measurement and nothing
is built. The purpose is to disagree now, cheaply, rather than at the end.

How to answer: for each **Choice**, the recommended option is marked ★. Say only where
you disagree; silence means "go with ★" and it is recorded as provisional in
[`../DECISIONS.md`](../DECISIONS.md).

| Preview | What it settles |
|---|---|
| **[0. The chosen style, in detail](style/README.md)** | **the whole look, everywhere it will be used — start here** |
| [1. The map viewer](#1-the-map-viewer) | look, layout and interaction of the product's main screen |
| [2. Visual identity](#2-visual-identity) | mark, colours, type |
| [3. Python API](api.md) | how the code reads, what it prints |
| [4. Command line](cli.md) | messages, estimates, tone |
| [5. Notebook](#5-notebook) | rich output inside Jupyter |
| [6. Other choices](choices.md) | defaults, vocabulary, files, project |

---

## 1. The map viewer

**Try it:** [`viewer/index.html`](viewer/index.html) is one self-contained file. Download
it (on GitHub: open the file, then "Download raw file") and open it in a browser; or copy
it from the workstation with
`scp tux@<host>:projects/everygraph/design/preview/viewer/index.html .`
Wheel to zoom, click to select, `/` to search, and the **Design options** button (bottom
left) flips every alternative below. All states are in the URL, so a view can be bookmarked.

### First open: it says what it will do before doing it

| Drop a file | Plan and estimate | Progress |
|---|---|---|
| ![](screens/01-first-open.webp) | ![](screens/02-preflight.webp) | ![](screens/03-progress.webp) |

> **V1 — the plan card.** ★ As shown: what was detected, what will happen, time, memory
> and disk against what the machine has, then Start. Alternative: start immediately and
> show the same information while running.

### The map style — the biggest question of taste

| | Dark | Light |
|---|---|---|
| **A · Bubbles** | ![](screens/04-overview-bubbles-dark.webp) | ![](screens/06-overview-bubbles-light.webp) |
| **B · Cells** | ![](screens/05-overview-cells-dark.webp) | ![](screens/07-overview-cells-light.webp) |

> **V2 — map style.**
> ★ **Bubbles by default, Cells as a second view.** Bubbles show size exactly, nesting
> shows the hierarchy, and a parent can be clicked in the gaps. They waste space.
> **Cells by default.** They fill the screen and look like a real map (the light version
> is the most attractive image here), but area is only approximate, a shared border looks
> like "related" when it means nothing, and deep inside one region everything is one hue.
> **Only one of the two**, to halve the work.

> **V3 — default theme.** ★ Dark for the viewer, light for exported images. Or: follow
> the operating system. Or: light everywhere.

> **V4 — colour of regions.** With more than about eight regions, colours cannot all be
> told apart, least of all by colour-blind users.
> ★ **A few well-separated colours reused** for regions that do not touch (like a
> political map); names and borders carry identity.
> **One colour per top region** (as in the screenshots): prettier, but some pairs are too
> close and colours repeat anyway beyond eight.

### Zooming in, selecting, inspecting

| A region and its numbers | One address and its real links |
|---|---|
| ![](screens/08-region-selection.webp) | ![](screens/09-node-connections.webp) |

> **V5 — the panel.** ★ As shown: what kind of thing it is, exact counts, ranked strongest
> connections with exact weights, then actions. Tell me what you would want first that is
> not there (for an address: balance over time? labels? first and last activity?).

> **V6 — labels on the map.** ★ Names with counts; unnamed regions show the count only.
> Alternative: names only ([screenshot](screens/21-labels-names-only.webp)).
> Note: real graphs will have far fewer readable names than this mockup.

> **V7 — density.** ★ [Comfortable](screens/20b-density-comfortable.webp) by default,
> [compact](screens/20-density-compact.webp) as a preference.

### Links — when to draw them

| Only for the selection | Strongest always | Bundled along the hierarchy |
|---|---|---|
| ![](screens/17-links-on-selection.webp) | ![](screens/18-links-strongest.webp) | ![](screens/19-links-bundled.webp) |

> **V8 — links.** ★ **Only for the selection** by default, with exact numbers beside
> them; "strongest always" as an optional layer; drop "bundled" (it is the fog the lab
> warned about). Alternative: strongest links always on.

### Regions that are not communities

| A subdivision made for readability | Many small communities shown as one |
|---|---|
| ![](screens/22-subdivision.webp) | ![](screens/23-aggregate.webp) |

> **V9 — honesty about artefacts.** ★ As shown: hatched and labelled differently, so the
> map never presents its own bookkeeping as a finding. Alternative: hide the distinction
> for a cleaner picture.

### Search, paths, time

| Search and fly to | Path through a hub | Same path, avoiding hubs |
|---|---|---|
| ![](screens/10b-search-address.webp) | ![](screens/11-path-through-hub.webp) | ![](screens/12-path-avoiding-hubs.webp) |

| A time window | Colour by a label | Share |
|---|---|---|
| ![](screens/13-time-window.webp) | ![](screens/14-colour-by-label.webp) | ![](screens/15-share-dialog.webp) |

> **V10 — "avoid hubs" in paths.** ★ Off by default with a visible toggle. Alternative:
> on by default (paths through an exchange are rarely the interesting ones).

> **V11 — time.** ★ A strip that opens on demand; the layout stays fixed and numbers
> change. Alternative: always visible when the graph has timestamps.

### What am I looking at

![](screens/16-about-this-map.webp)

> **V12 — the caption strip at the bottom of every view** ("182 of 280 sub-regions on
> screen · no links drawn until you select something · distance is not a measurement").
> ★ Always visible, and always included in exported images. Alternative: shown on demand.

**What is faked and will be hard for real** (so the mockup does not promise too much):
readable names for every region; smooth label placement at 10⁵ regions; Cells derived
from a real layout; links to distant parts of the graph; counts that change with the time
window; path search with alternatives. Each is noted in issue #119.

---

## 2. Visual identity

Round 1 offered three directions. **Decided: the system of B with the mark of A.**

![](screens/b-01-identity-overview.webp)

### Round 2 — five systems descended from "B's system + A's mark"

![](screens/c-01-systems-overview.webp)

Each system is shown applied where it will live: a landing page, a terminal, a notebook
error card and the viewer around a map. These are the images to judge by.

| System | In context | Other theme | Everything |
|---|---|---|---|
| **1 · Datum** — B unchanged, the reference | [light](screens/c-21-system-1-context.webp) | [dark](screens/c-31-system-1-dark.webp) | [full](screens/c-11-system-1.webp) |
| **2 · Vernier** — a calmer instrument: warm paper, ink-blue accent, soft corners | [light](screens/c-22-system-2-context.webp) | [dark](screens/c-32-system-2-dark.webp) | [full](screens/c-12-system-2.webp) |
| **3 · Blueprint** — dark-first drafting sheet: ink blue, cyan lines, one yellow highlight | [dark](screens/c-23-system-3-context.webp) | [light](screens/c-33-system-3-light.webp) | [full](screens/c-13-system-3.webp) |
| **4 · Survey** — hairlines, a serif for headlines, muted sea blue | [light](screens/c-24-system-4-context.webp) | [dark](screens/c-34-system-4-dark.webp) | [full](screens/c-14-system-4.webp) |
| **5 · Plate** — black on white, colour only for data | [light](screens/c-25-system-5-context.webp) | [dark](screens/c-35-system-5-dark.webp) | [full](screens/c-15-system-5.webp) |

**Vernier, applied**

![](screens/c-22-system-2-context.webp)

**Blueprint, applied**

![](screens/c-23-system-3-context.webp)

What was measured: in Datum the orange accent is too close to the error red and, for some
colour-blind users, to the warning amber; the other systems fix this. Every data palette
keeps its ten colours distinguishable for the two common forms of colour blindness, none
by a wide margin. Heavy versions of the mark read as a target or an eye; thin and medium
weights read as nested contours.

> **I0 — which system?**
> ★ **Vernier**: keeps the instrument feel, fixes the measured colour problem, works in
> light and dark, and the dark map sits well in it.
> **Blueprint**: the most distinctive and the best home for the map, but it is dark-first
> and weakest on white (notebooks, papers).
> **A hybrid**: Vernier for documentation and notebooks, Blueprint's dark sheet for the
> viewer. **Survey**, **Datum unchanged**.

> **I0b — the mark.** ★ Two rings and a dot at medium weight ("Nest"), and the same
> inside a square for small sizes and icons ("Seal"). Alternative: three rings
> ("Strata"), which closes up below 32 px.

> **I0c — numbers.** ★ Monospace for every number (the instrument look you liked in B).
> Alternative: monospace only in tables and code.

> **I0d — fonts.** ★ Ship one open-licence family so it looks the same everywhere (Inter
> with JetBrains Mono, or IBM Plex Sans with Plex Mono). Alternative: whatever fonts the
> reader's system has, accepting differences.

> **I0e — "Plate" (black and white, colour only for data) as the style of exported
> figures**, whichever system wins? ★ Yes.

The landing page in the three round-1 directions:
[A](screens/b-05-landing-A.webp) · [B](screens/b-06-landing-B.webp) ·
[C](screens/b-07-landing-C.webp) · [B, second headline](screens/b-08-landing-headline2.webp)

> **I1 — headline.** ★ "Every graph, every size, every machine." Alternative: "Open the
> whole graph. See all of it."

> **I2 — numbers on the front page.** ★ None until the released software is measured;
> the mockup's dashed "lab prototype" boxes are placeholders. Alternative: show lab
> numbers with that label.

> **I3 — "When to use something else".** ★ Keep it and name the tools (Gephi, Neo4j,
> DuckDB…). Alternative: describe categories without names.

---

## 5. Notebook

| Styled cards | Plain text |
|---|---|
| ![](screens/b-09-notebook-html.webp) | ![](screens/b-10-notebook-text.webp) |

> **N1 — rich output.** ★ Styled cards in notebooks, the same content as plain text in a
> terminal. Alternative: plain text everywhere.

> **N2 — errors.** ★ The explanation first, the Python traceback folded away. Alternative:
> always show the traceback.

> **N3 — whose words.** ★ The user's ("addresses", "transfers", taken from the column
> names) in summaries, "nodes" and "edges" in the API. Alternative: always nodes/edges.

Note: some function names in the notebook and landing images were invented by the
mockup and differ from [`api.md`](api.md), which is the reference. The notebook also
shows communities computed with the graph on disk, which the lab has not yet achieved.
