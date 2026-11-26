---
title: Non-visual navigation of the map hierarchy
milestone: M5 · The map
labels: type:feature, area:viewer, P1, risk:medium, size:M
---
**Why.** {{#m5-11-perf-accessibility}} asked for a "screen-reader summary of the current view". A summary is alt text, and alt text plus a data table fails: Zong et al. (EuroVis'22) cite 211% more time and 61% lower accuracy with screen readers (Sharif et al. 2021). The best published approach is a navigable tree with structural, spatial and targeted moves (Zong et al., Olli), rendered so that only the focused element is in the DOM (Data Navigator), which suits a canvas with millions of marks. Our hierarchy already has the right shape: D16's fan-out cap of 64 makes a root-to-node path on Friendster about 5–6 steps (64⁴ = 16.7 M, 64⁵ = 1.07 B, arithmetic), each a list of at most 64 siblings. The viewer today exposes one `<canvas aria-label>` and an `aria-live` panel (`design/preview/style/viewer.html`). Nobody has studied blind or low-vision access to a million-node zoomable network map. Split out of {{#m5-11-perf-accessibility}}, decided 2026-11-25 (`docs/research/README.md` H2).

**Scope**
- [ ] A structure layer that renders only the focused region and its siblings into the DOM, as an ARIA tree with `aria-setsize`/`aria-posinset` ("3 of 64")
- [ ] Structural keys per the keyboard model of `design/DECISIONS.md` ("The map's keyboard model"): arrows down and up through levels, sideways between siblings ordered by size, Home to the root; targeted moves through search and the breadcrumb as landmarks
- [ ] A description template: kind, exact count, position among siblings, strongest connection with its weight, "shown X of Y"; the level spoken only when it changes
- [ ] "Children as a table" for every region (the table stays necessary because it is familiar: Zong et al.'s 13 participants)
- [ ] The canvas highlights the focused region, so a sighted helper and a screen-reader user see the same place
- [ ] A formative cooperative session (Zong et al.'s protocol) with 3–5 screen-reader users on lj with the tasks of {{#m5-06-viewer-ux}}; findings filed as issues. Formative only: no success rate is claimed from it

**Acceptance**
- Every region of the lj map can be reached and described with the keyboard alone, with a screen reader announcing position and counts (NVDA or VoiceOver, scripted in Playwright where possible)
- The formative session is held and its findings recorded; the docs say "usable with a screen reader" only after it

**Pointers.** `docs/research/sota-human-factors.md` § 2 ("Non-visual access"), § 3 (D16, D18 and m5-11 rows), § 4 challenge 2, § 5; WAI-ARIA tree view pattern.

**Depends on** {{#m5-05-viewer-core}}, {{#m4-07-map-hierarchy}}

**Design reference (decided 2026-11-25, `design/DECISIONS.md`).** The keyboard model row; `design/preview/style/foundations.html` § 9 (arrows move between regions, Enter opens one, Esc goes up). Anything visible this adds (the structure layer's focus, the table) follows `components.html`; a visible change to the approved viewer gets a preview first.
