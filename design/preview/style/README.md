# The Everygraph style, in detail

One style, shown everywhere it will be used. Working name **Vernier**: a calm measuring
instrument — warm paper, soft black, one ink-blue accent, every number in monospace,
hairlines instead of boxes, almost no shadow.

Everything below is a preview with invented data. The rules are in [`SPEC.md`](SPEC.md),
the values in [`assets/tokens.css`](assets/tokens.css). Nine HTML pages show them live
(`index.html` is the entry); this page shows the same content as images so it can be read
on GitHub. To browse the pages themselves, clone the repository and open
`design/preview/style/index.html`.

**Contents** · [At a glance](#at-a-glance) · [The viewer](#the-viewer) ·
[Foundations](#foundations) · [Components](#components) · [Data and map](#data-and-map) ·
[Web and docs](#web-and-docs) · [Terminal](#terminal) · [Notebook](#notebook) ·
[Figures and export](#figures-and-export) · [What I decided, what I ask you](#what-i-decided-what-i-ask-you)

---

## At a glance

![](../screens/g-full-index-light.webp)

[The same page in the dark theme](../screens/g-full-index-dark.webp)

## The viewer

The product's main screen, fully in the style ([`viewer.html`](viewer.html), navigable).

| The whole graph | A region selected |
|---|---|
| ![](../screens/s-21-viewer-04-overview-dark.webp) | ![](../screens/s-21-viewer-08-region-selected.webp) |

| One address and its links | A path between two addresses |
|---|---|
| ![](../screens/s-21-viewer-09-node-links.webp) | ![](../screens/s-21-viewer-11-path.webp) |

| Light theme | "Cells" style, light |
|---|---|
| ![](../screens/s-21-viewer-05-overview-light.webp) | ![](../screens/s-21-viewer-07-cells-light.webp) |

More states: [first open](../screens/s-21-viewer-01-first-open.webp) ·
[plan before starting](../screens/s-21-viewer-02-preflight.webp) ·
[progress](../screens/s-21-viewer-03-progress.webp) ·
[cells, dark](../screens/s-21-viewer-06-cells-dark.webp) ·
[search](../screens/s-21-viewer-10-search.webp) ·
[path avoiding hubs](../screens/s-21-viewer-12-path-avoiding-hubs.webp) ·
[time window](../screens/s-21-viewer-13-time-window.webp) ·
[colour by attribute](../screens/s-21-viewer-14-colour-by-attribute.webp) ·
[colour by metric](../screens/s-21-viewer-15-colour-by-metric.webp) ·
[share](../screens/s-21-viewer-16-share-dialog.webp) ·
[about this map](../screens/s-21-viewer-17-about-this-map.webp) ·
[compact](../screens/s-21-viewer-18-compact.webp) ·
[narrow window](../screens/s-21-viewer-19-narrow-1024x700.webp) ·
[one colour per region](../screens/s-21-viewer-20-region-colours-one-per-region.webp)

## Foundations

**The mark** — construction, sizes, the small cut, what never to do.

![](../screens/g-foundations-01-mark.webp)

**Colour** — every value with its measured contrast.

![](../screens/g-foundations-03-colour.webp)

**Type**

![](../screens/g-foundations-04-type.webp)

Also: [wordmark and lockups](../screens/g-foundations-02-wordmark.webp) ·
[space and layout](../screens/g-foundations-05-space.webp) ·
[shape, borders, elevation](../screens/g-foundations-06-shape.webp) ·
[motion](../screens/g-foundations-07-motion.webp) ·
[icons](../screens/g-foundations-08-icons.webp) ·
[accessibility summary](../screens/g-foundations-09-a11y.webp)

## Components

Every component in all its states, light and dark side by side.

![](../screens/g-components-01-actions.webp)

![](../screens/g-components-07-data.webp)

**A whole panel assembled from the components**

![](../screens/g-components-08-assembled.webp)

Also: [inputs](../screens/g-components-02-inputs.webp) ·
[tags, keys, tooltips](../screens/g-components-03-small.webp) ·
[menus, dialogs, toasts](../screens/g-components-04-overlays.webp) ·
[progress, loading, empty states](../screens/g-components-05-waiting.webp) ·
[pagination, breadcrumb, disclosure](../screens/g-components-06-nav.webp)

## Data and map

**The palette, measured** (including for colour-blind readers)

![](../screens/g-data-01-s1.webp)

**How regions are coloured** — the one open question with real consequences

![](../screens/g-data-02-s2.webp)

**Anatomy of the map**

![](../screens/g-data-04-s4.webp)

Also (light · dark): scales [l](../screens/g-data-03-s3.webp) · [d](../screens/g-data-03-s3-dark.webp) ·
zoom levels [l](../screens/g-data-05-s5.webp) · [d](../screens/g-data-05-s5-dark.webp) ·
states of the map [l](../screens/g-data-06-s6.webp) · [d](../screens/g-data-06-s6-dark.webp) ·
labels [l](../screens/g-data-07-s7.webp) · [d](../screens/g-data-07-s7-dark.webp) ·
small charts [l](../screens/g-data-08-s8.webp) · [d](../screens/g-data-08-s8-dark.webp) ·
result tables [l](../screens/g-data-09-s9.webp) · [d](../screens/g-data-09-s9-dark.webp) ·
palette, dark [d](../screens/g-data-01-s1-dark.webp) · anatomy, dark [d](../screens/g-data-04-s4-dark.webp)

## Web and docs

**Landing page**

![](../screens/g-full-landing-light.webp)

[Dark](../screens/g-full-landing-dark.webp) ·
[on a phone](../screens/s-30-web-03b-landing-mobile-sheet.webp) ·
[documentation page](../screens/g-full-docs-light.webp)

The guide's sections for the web, in order:
[1](../screens/g-web-01-section.webp) · [2](../screens/g-web-02-section.webp) ·
[3](../screens/g-web-03-section.webp) · [4](../screens/g-web-04-section.webp) ·
[5](../screens/g-web-05-section.webp) · [6](../screens/g-web-06-section.webp) ·
[7](../screens/g-web-07-section.webp) · [8](../screens/g-web-08-section.webp)
(landing, documentation, API reference, search, 404, changelog, README header, social card and slide)

## Terminal

**One transcript three ways**: colour, no colour, ASCII only.

![](../screens/g-terminal-01-three.webp)

**When it refuses, errs or warns**

![](../screens/g-terminal-08-refusals.webp)

Also: [colours](../screens/g-terminal-02-colour.webp) ·
[symbols and fallbacks](../screens/g-terminal-03-symbols.webp) ·
[progress and alignment](../screens/g-terminal-04-anatomy.webp) ·
[help](../screens/g-terminal-05-help.webp) ·
[from a file to a map](../screens/g-terminal-06-file-to-map.webp) ·
[questions, info, doctor](../screens/g-terminal-07-questions.webp) ·
[interrupt and resume](../screens/g-terminal-09-interrupt.webp) ·
[for scripts](../screens/g-terminal-10-scripts.webp) ·
[on other terminals](../screens/g-terminal-11-light.webp)

## Notebook

![](../screens/g-full-notebook-light.webp)

[Dark](../screens/g-full-notebook-dark.webp) ·
[states](../screens/g-notebook-04-states.webp) ·
[the same objects as plain text](../screens/g-notebook-05-text.webp)

## Figures and export

**A map as a figure for a paper** — black, white and data colours only, caption included.

![](../screens/g-export-01-map-figure.webp)

Also: [charts](../screens/g-export-02-charts.webp) ·
[the page a recipient sees](../screens/g-export-03-shared.webp) ·
[embedded in an article](../screens/g-export-04-embed.webp) ·
[how to cite](../screens/g-export-05-cite.webp) ·
[file icons](../screens/g-export-06-files.webp) ·
[message preview](../screens/g-export-07-message.webp)

---

## What I decided, what I ask you

**Decided while building this** (details in `SPEC.md`, last section). Tell me if any is wrong:

- The first data palette I picked by eye failed measurement and was replaced by a measured one.
- Contrast of text and control borders was adjusted until it passed; borders are a little heavier for it.
- Regions on the map use five colours, reused where regions do not touch.
- Numbers in monospace, units and column headers in the text face; no ligatures in code.
- In the terminal, the suggested next command is bold rather than blue; lines fit 80 columns.
- "Budget" means memory only; traversal limits are called `limit`.

**Where the style strains** (so you see it before I do anything about it):

- When most nodes live in small communities, the grey hatched "many small communities"
  regions are the largest shapes on the map. Honest, but it is the first thing the eye sees.
- With the accent reserved for interactive things, the landing page is quiet: one blue button.
- The icon set is a first pass; about six icons need redrawing.
- A single-column figure (85 mm) is too small for names; it uses letter keys and a legend.

**Questions — only where your taste matters:**

1. **The whole direction.** Is this the style? If something feels wrong overall (too
   beige, too quiet, too technical), say it in a word and I iterate.
2. **Viewer by default:** dark canvas ★, or light.
3. **Map style:** bubbles ★, or cells (see the two "light theme" images above).
4. **Landing headline:** all in ink ★ (as shown), or the last words in the accent blue.
5. **Hatched "small communities" regions:** as drawn ★, or lighter so they recede.
6. **The mark at small sizes:** the heavier small cut ★, or always the full mark.
7. **A tagline under the wordmark?** None ★, or "Every graph, every size, every machine."
