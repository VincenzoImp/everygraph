---
title: Design previews of the debatable parts, for owner review
milestone: M0 · Foundations and decisions
labels: type:spec, area:viewer, P0, risk:medium, size:M
---
**Why.** The plan specifies mechanisms and acceptance numbers, but the parts a user actually touches — the map viewer, the Python API, the command-line messages — were never designed: there is no mockup, no visual style and no written user flow. The owner asked to see previews of these parts now, so disagreements surface before they are built rather than at the end.

**Scope**
- [ ] `design/preview/README.md`: one page, readable on GitHub, with screenshots and, for every preview, the explicit choices the owner is asked to make
- [ ] Viewer mockup: one self-contained HTML file with invented data, clickable, covering the main flows — first open, zoom from the whole graph to a node, select and inspect, search and fly to, path between two nodes, filter by time, share a view, and the "what am I looking at" caption; two visual alternatives for the map style (nested bubbles vs space-filling cells) and light/dark themes
- [ ] Python API preview: the ten user stories as code with realistic output, including `explain()` and an error
- [ ] CLI preview: terminal transcripts for ingest with preflight and progress, a refusal for insufficient memory, `info`, `doctor`
- [ ] Everything is labelled as a mockup with invented data; no number in it may be mistaken for a measurement

**Acceptance**
- The owner can open the page on GitHub and the mockup in a browser without installing anything
- Each preview ends with a short list of decisions; the owner's answers are recorded in `design/DECISIONS.md` and feed {{#m0-11-api-sketch}}, {{#m5-06-viewer-ux}} and {{#m8-01-cli}}

**Not in scope.** Production code. The mockup is thrown away once the real viewer exists.
