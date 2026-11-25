# Everygraph style — working specification ("Vernier")

The rules behind the style preview. `assets/tokens.css` holds the values; this file holds
what values cannot say. Pages in this folder show it; they must not contradict it.

## Character

A calm measuring instrument. It should feel like something you trust a number from:
warm paper, soft black, one ink-blue accent, hairlines instead of boxes, almost no shadow,
no gradients, no decoration that is not data. Friendly through plain words and generous
spacing, not through colour or rounded shapes.

## The name

| Where | Form |
|---|---|
| Running text, headings, titles | **Everygraph** — capital E, one word. Never EveryGraph, Every Graph, everyGraph, EVERYGRAPH. |
| The wordmark (logo) | **everygraph** — all lowercase, "every" regular weight, "graph" semibold, no space. |
| Anything typed: package, import, command, crates, repository, URLs | `everygraph` — lowercase. `pip install everygraph`, `import everygraph as eg`, `everygraph ingest …`, crates `everygraph-core` …, npm `@everygraph/viewer`. |
| Python alias | `eg` (in code only; never "EG" in prose). |
| File extensions | `.eg` (a graph), `.egmap` (a shareable map) — lowercase. |
| Parts of the product | plain lowercase nouns after the name: "the Everygraph viewer", "an Everygraph map". No coined sub-brands. |
| This design system | "Vernier" is an internal working name; it never appears in the product. |

## The mark

- **Nest**: two off-centre rings and a dot — a region, a sub-region, a node. In a 32-unit
  box: outer ring centre (16,16) r 13; inner ring centre (18.5,18) r 7; dot centre
  (20,19.2) r 2.6. Ring stroke 2.4 units (medium weight: thinner reads as fragile, heavier
  reads as a target). Rings in the text colour, dot in the accent.
- **Seal**: the Nest reversed out of a filled square with 3/32 corner radius, for 24 px and
  below (favicon, app icon, avatar). At 16 px the inner ring may be dropped to keep the dot.
- Clear space: the radius of the outer ring on every side. Minimum size: Nest 20 px, Seal 12 px.
- One-colour version: rings and dot in the same ink. Never: rotate, add a third ring,
  recolour the rings with the accent, set the dot outside the inner ring, add effects.

## Type

- Inter for everything read; JetBrains Mono for **every number**, code, identifiers, file
  names and keyboard keys. Numbers are tabular so columns align.
- Sentence case everywhere (buttons, headings, labels, menu items). No all-caps labels;
  the only exception is the tiny "overline" above a section, and it is not monospace.
- UI at 14 px; long-form reading at 16 px in a 720 px column; headings 600 weight with
  tight tracking from 24 px up. Two weights in running UI (400, 500/600); bold is rare.
- Exact numbers with thousands separators in results (`1,830,397,917`); rounded forms only
  in running text (`about 212.4 M`). Units follow a thin space: `9.3 GB`, `3 min 40 s`.

## Colour

- The accent means "this is interactive or selected". It is never used for decoration and
  never for status.
- Status colours (ok, warn, err, info) always come with an icon or a word; colour is
  never the only carrier.
- Data colours belong to data. Chrome never borrows them.
- Dark theme is a first-class theme, and the default surface for the map viewer.

## Components

- Controls are 32 px high (28 small, 40 large), 3 px radius, 1 px border.
- One primary button per view. Destructive actions are outlined in the error colour and
  ask for confirmation in words.
- Focus is always visible: a 2 px accent outline offset by 2 px.
- Popovers and dialogs are the only things with a shadow.
- Motion: 120 ms for hover/press, 190 ms for panels and fades, 320 ms for flying across
  the map; nothing bounces; reduced-motion is respected.
- Empty, loading, error and "cut short" states are designed for every component that can
  have them. A truncated result always says so in place.

## The map

- Default style: nested bubbles on the dark canvas. A region is a 1.5 px outline in its
  data colour with a 10–14% tint; nesting shows hierarchy.
- Region colours: a small set of well-separated colours reused for regions that do not
  touch; names and outlines carry identity, not hue alone.
- Three kinds of region are always distinguishable: a found community (solid outline), a
  subdivision made for readability (dashed outline), many small communities shown as one
  (hatched fill).
- Labels: region name in Inter 600, exact count beneath in JetBrains Mono; a halo in the
  canvas colour keeps them legible. Unnamed regions show the count only.
- Links are drawn only for the selection by default: curved bands in the ink colour at
  55% opacity, width by weight, with the exact figure in the panel.
- Selection: a double ring in the ink colour (no extra hue), the rest of the map dims to 60%.
- A caption strip under every view says what is on screen, what is omitted and what
  position means. Exported images always carry it.

## Words

- Plain, exact, unhurried. Say what happened, why (with the arithmetic when there is one),
  and what to do next. No exclamation marks, no "oops", no "simply", no marketing adjectives.
- Errors: one sentence of what, one of why, then the ways out as commands that can be copied.
- Prefer the user's own nouns (addresses, transfers) in summaries when the columns reveal
  them; "nodes" and "edges" in API names.

## Terminal

- Works without colour and without Unicode; colour and box characters are an enhancement.
- Dim for secondary text, green ✓ for done, red ✗ for failure, accent for the next
  suggested command. Progress bars are plain blocks with the phase name and elapsed time.

## Figures for papers and slides

- Exported figures use white, black and data colours only, with the caption and a scale
  of counts included; no chrome, no accent.

## Settled while building the detailed preview

Each of these came out of a measurement or of seeing the rule applied; they refine the
sections above and win where the two differ.

- **Contrast is measured, not assumed.** Text tokens reach 4.5:1 on every surface they are
  allowed on, control borders 3:1; `foundations.html` computes the table live from the
  tokens. `--muted`, `--ok`, `--warn`, `--err` and `--line-strong` were adjusted to pass.
- **The data palette is measured.** The first palette, chosen by eye, failed (two pairs
  were indistinguishable to colour-blind readers, and one colour equalled the accent). The
  ten colours now keep ΔE ≥ 15 for normal vision and about 7.3 for protanopia and
  deuteranopia, which is why all ten are only ever used with labels. No data colour equals
  the accent or a status colour. Syntax colours have their own tokens for the same reason.
- **Regions use five colours, reused** for regions that do not touch (a graph colouring,
  most-constrained region first). Children inherit their top-level region's colour.
  "One hue per region" remains a setting for small graphs.
- **Tint depends on what the region is doing:** 12% when it is a leaf on screen, 5% when
  its children are showing, none when it covers the whole view (nested tints would cast
  the canvas).
- **A path dims the rest of the map** to 60%, like a selection.
- **Smallest text on the map is 12 px.** Fewer labels, all legible.
- **Monospace never uses ligatures** (`--`, `->`, `>=` stay as typed).
- **Number and unit:** the digits in monospace, the unit in the text face after a narrow
  no-break space (U+202F): `9.3 GB`. Column headers are words and stay in the text face.
- **The small Seal** (16 px and below) is redrawn heavier — ring 3.6, dot r 3.6 — because
  the full geometry rasterizes to grey.
- **Terminal:** the suggested next command is bold, not blue (blue measured 2.2:1 on stock
  terminals); lines stay within 80 columns; progress bars are 16 cells.
- **Progress fills are ink, not accent** (progress is not an interaction).
- **Toasts may carry the pop-up shadow**: they float above the page like a popover.
- **Selection has two forms on purpose:** the accent-soft fill in lists and tables, an ink
  double ring on the map, where every hue already means data.
- **"Budget" means memory only.** Traversal limits are `limit=` (nodes) and `timeout=`.
- **Figures for print** use `--print-paper` and `--print-ink`; at single-column width
  (85 mm) regions carry letter keys and a legend instead of names.
