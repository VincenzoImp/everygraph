---
title: "Experiment: coordinate precision of the map at the largest scales, before the format freezes"
milestone: M5 · The map
labels: type:experiment, area:map, P0, risk:medium, size:S
---
**Why.** `.egmap` 1.0 stores f32 coordinates in one global frame, with `MAX_ZOOM` 16 (`docs/spec/egmap.md` § 1). Each depth of nested discs keeps about 0.63–0.65 of its parent's area (`lab/m1-07-layout-area/results/README.md`). On graphs of billions of nodes with deep hierarchies, leaf discs may fall below what f32 or zoom 16 can separate. The format freezes for 0.1, so this must be known first (owner's strategy discussion, 2026-11-26).

**Method** (arithmetic on hierarchies we already have; no new layout)
- [ ] From the map hierarchies of lj, orkut and Friendster (m4-07): depth, smallest leaf radius, and smallest distance between sibling discs, in map units
- [ ] Extrapolate to 1, 4 and 10 B nodes with the measured fan-out and depth growth; state the extrapolation as such
- [ ] Compare with the f32 spacing near 1.0 and with one pixel at zoom 16

**Done when.** Either the f32 global frame and zoom 16 are shown to suffice up to a stated scale, written into the spec, or a change is decided before the freeze (region-local frames or tile-local quantized coordinates), with its cost in bytes per entity.
