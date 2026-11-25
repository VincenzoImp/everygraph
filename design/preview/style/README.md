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

![](../screens/g-full-index-light.png)

[The same page in the dark theme](../screens/g-full-index-dark.png)

## The viewer

The product's main screen, fully in the style ([`viewer.html`](viewer.html), navigable).

| The whole graph | A region selected |
|---|---|
| ![](../screens/s-21-viewer-04-overview-dark.png) | ![](../screens/s-21-viewer-08-region-selected.png) |

| One address and its links | A path between two addresses |
|---|---|
| ![](../screens/s-21-viewer-09-node-links.png) | ![](../screens/s-21-viewer-11-path.png) |

| Light theme | "Cells" style, light |
|---|---|
| ![](../screens/s-21-viewer-05-overview-light.png) | ![](../screens/s-21-viewer-07-cells-light.png) |

More states: [first open](../screens/s-21-viewer-01-first-open.png) ·
[plan before starting](../screens/s-21-viewer-02-preflight.png) ·
[progress](../screens/s-21-viewer-03-progress.png) ·
[cells, dark](../screens/s-21-viewer-06-cells-dark.png) ·
[search](../screens/s-21-viewer-10-search.png) ·
[path avoiding hubs](../screens/s-21-viewer-12-path-avoiding-hubs.png) ·
[time window](../screens/s-21-viewer-13-time-window.png) ·
[colour by attribute](../screens/s-21-viewer-14-colour-by-attribute.png) ·
[colour by metric](../screens/s-21-viewer-15-colour-by-metric.png) ·
[share](../screens/s-21-viewer-16-share-dialog.png) ·
[about this map](../screens/s-21-viewer-17-about-this-map.png) ·
[compact](../screens/s-21-viewer-18-compact.png) ·
[narrow window](../screens/s-21-viewer-19-narrow-1024x700.png) ·
[one colour per region](../screens/s-21-viewer-20-region-colours-one-per-region.png)

## Foundations

**The mark** — construction, sizes, the small cut, what never to do.

![](../screens/g-foundations-01-mark.png)

**Colour** — every value with its measured contrast.

![](../screens/g-foundations-03-colour.png)

**Type**

![](../screens/g-foundations-04-type.png)

Also: [wordmark and lockups](../screens/g-foundations-02-wordmark.png) ·
[space and layout](../screens/g-foundations-05-space.png) ·
[shape, borders, elevation](../screens/g-foundations-06-shape.png) ·
[motion](../screens/g-foundations-07-motion.png) ·
[icons](../screens/g-foundations-08-icons.png) ·
[accessibility summary](../screens/g-foundations-09-a11y.png)

## Components

Every component in all its states, light and dark side by side.

![](../screens/g-components-01-actions.png)

![](../screens/g-components-07-data.png)

**A whole panel assembled from the components**

![](../screens/g-components-08-assembled.png)

Also: [inputs](../screens/g-components-02-inputs.png) ·
[tags, keys, tooltips](../screens/g-components-03-small.png) ·
[menus, dialogs, toasts](../screens/g-components-04-overlays.png) ·
[progress, loading, empty states](../screens/g-components-05-waiting.png) ·
[pagination, breadcrumb, disclosure](../screens/g-components-06-nav.png)

## Data and map

**The palette, measured** (including for colour-blind readers)

![](../screens/g-data-01-s1.png)

**How regions are coloured** — the one open question with real consequences

![](../screens/g-data-02-s2.png)

**Anatomy of the map**

![](../screens/g-data-04-s4.png)

Also (light · dark): scales [l](../screens/g-data-03-s3.png) · [d](../screens/g-data-03-s3-dark.png) ·
zoom levels [l](../screens/g-data-05-s5.png) · [d](../screens/g-data-05-s5-dark.png) ·
states of the map [l](../screens/g-data-06-s6.png) · [d](../screens/g-data-06-s6-dark.png) ·
labels [l](../screens/g-data-07-s7.png) · [d](../screens/g-data-07-s7-dark.png) ·
small charts [l](../screens/g-data-08-s8.png) · [d](../screens/g-data-08-s8-dark.png) ·
result tables [l](../screens/g-data-09-s9.png) · [d](../screens/g-data-09-s9-dark.png) ·
palette, dark [d](../screens/g-data-01-s1-dark.png) · anatomy, dark [d](../screens/g-data-04-s4-dark.png)

## Web and docs

**Landing page**

![](../screens/g-full-landing-light.png)

[Dark](../screens/g-full-landing-dark.png) ·
[on a phone](../screens/s-30-web-03b-landing-mobile-sheet.png) ·
[documentation page](../screens/g-full-docs-light.png)

The guide's sections for the web, in order:
[1](../screens/g-web-01-section.png) · [2](../screens/g-web-02-section.png) ·
[3](../screens/g-web-03-section.png) · [4](../screens/g-web-04-section.png) ·
[5](../screens/g-web-05-section.png) · [6](../screens/g-web-06-section.png) ·
[7](../screens/g-web-07-section.png) · [8](../screens/g-web-08-section.png)
(landing, documentation, API reference, search, 404, changelog, README header, social card and slide)

## Terminal

**One transcript three ways**: colour, no colour, ASCII only.

![](../screens/g-terminal-01-three.png)

**When it refuses, errs or warns**

![](../screens/g-terminal-08-refusals.png)

Also: [colours](../screens/g-terminal-02-colour.png) ·
[symbols and fallbacks](../screens/g-terminal-03-symbols.png) ·
[progress and alignment](../screens/g-terminal-04-anatomy.png) ·
[help](../screens/g-terminal-05-help.png) ·
[from a file to a map](../screens/g-terminal-06-file-to-map.png) ·
[questions, info, doctor](../screens/g-terminal-07-questions.png) ·
[interrupt and resume](../screens/g-terminal-09-interrupt.png) ·
[for scripts](../screens/g-terminal-10-scripts.png) ·
[on other terminals](../screens/g-terminal-11-light.png)

## Notebook

![](../screens/g-full-notebook-light.png)

[Dark](../screens/g-full-notebook-dark.png) ·
[states](../screens/g-notebook-04-states.png) ·
[the same objects as plain text](../screens/g-notebook-05-text.png)

## Figures and export

**A map as a figure for a paper** — black, white and data colours only, caption included.

![](../screens/g-export-01-map-figure.png)

Also: [charts](../screens/g-export-02-charts.png) ·
[the page a recipient sees](../screens/g-export-03-shared.png) ·
[embedded in an article](../screens/g-export-04-embed.png) ·
[how to cite](../screens/g-export-05-cite.png) ·
[file icons](../screens/g-export-06-files.png) ·
[message preview](../screens/g-export-07-message.png)

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
