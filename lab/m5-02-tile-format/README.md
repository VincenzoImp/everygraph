# m5-02: the tile payload against MapLibre Tile and Arrow IPC

Issue #68 (`plan/issues/m5-02-tile-format.md`), `docs/spec/egmap.md` § 10. Byte counts
only: they do not depend on load. Exploratory; taken on 2026-11-26 while other work ran.

**Question.** On the same content, how many bytes does each payload take, raw and gzip:
the payload of `docs/spec/egmap.md` § 4, MapLibre Tile (MLT), and Arrow IPC?

**Content.** Every tile of the lab maps of lj (203,941 tiles) and orkut (142,710),
built by `maps.sh` as `lab/m1-06-edge-rendering/maps.sh` builds them (the same file:
895,034,297 bytes for lj). `payloads/` turns each lab tile into the records of the spec
(§ 3): the lab's regions and nodes, plus their ancestors up to the root (1,081,321
regions added on lj, 607,508 on orkut), numbered breadth first; `zmin` by the zoom rule
with the lab's thresholds (3 px for a region, 1.2 px for a node), capped at the tile's
zoom; rank among siblings by node count; colour of the top region modulo 5; node names
the original ids in decimal. No labels (the lab has none) and no edges (the spec's tiles
hold none). Every converted tile passed the writer's checks (§ 4.3) and decoded back to
itself: 0 refused on either graph.

**Payloads**, each holding exactly those fields:

- `egmap`: `Tile::encode` of `crates/everygraph-map`;
- `mlt_web_4g` and `mlt_min_size`: `mlt-core` 0.24.0, a layer for regions and one for
  nodes, a point geometry per entity (tile extent 4096) and every field as a property
  column, `x`, `y`, `r` as f32 (MLT's geometry is integer, so without them it would not
  be lossless); profiles `Web(Cellular4G)` (the MapLibre GL JS profile) and `MinSize`;
- `arrow_ipc`: arrow-rs 58.4, two IPC streams (regions, nodes), each with its schema.

Gzip is level 6 for all.

**Results** (`results/payloads_{lj,orkut}.json`):

| Graph | Payload | Raw total | Gzip total | Gzip median | Gzip p99 | Gzip max |
|---|---|---|---|---|---|---|
| lj | egmap | 960.8 MB | 453.0 MB | 1,419 B | 13,560 B | 18,203 B |
| lj | MLT Web 4G | 574.7 MB | 461.9 MB (+2.0%) | 1,555 B (+9.6%) | 12,457 B | 16,769 B |
| lj | MLT MinSize | 565.9 MB | 461.3 MB (+1.8%) | 1,554 B (+9.5%) | 12,399 B | 16,889 B |
| lj | Arrow IPC | 1,593.1 MB | 558.0 MB (+23.2%) | 2,002 B (+41%) | 13,270 B | 17,982 B |
| orkut | egmap | 651.4 MB | 324.7 MB | 1,477 B | 13,148 B | 17,998 B |
| orkut | MLT Web 4G | 404.5 MB | 330.5 MB (+1.8%) | 1,613 B (+9.2%) | 12,177 B | 17,024 B |
| orkut | MLT MinSize | 396.0 MB | 325.6 MB (+0.3%) | 1,607 B (+8.8%) | 11,570 B | 16,375 B |
| orkut | Arrow IPC | 1,091.1 MB | 396.7 MB (+22.2%) | 2,056 B (+39%) | 12,842 B | 17,675 B |

- Over HTTP (gzip), MLT is within 2% of the custom payload in total and within 10% at the
  median; its largest tiles are smaller. Raw, MLT is 40% smaller: it compresses inside.
- Arrow IPC is 22–23% larger in total and about 40% larger at the median: each tile
  carries its schema.
- The lab tiles with their edges were 830.6 MB of gzip on lj and 721.1 MB on orkut
  (`maps/tiles_{lj,orkut}.json` of the build); the same entities without edges, with
  their ancestors, are 453.0 and 324.7 MB.
- These are the lab's entity sets, not tiles placed by the rule of § 3.5 (every entity
  in every tile its disc meets): the budget question of #69 (deepest tiles over 32 KB)
  is not answered here.

**Not measured.** Decode time in JavaScript, of any payload. The adoption rule of #68
needs it for MLT; see the issue file.

**Reproduce** (8 GiB limit each, under a minute per graph at 8 threads):

```sh
lab/m5-02-tile-format/maps.sh
cd lab/m5-02-tile-format/payloads
CARGO_TARGET_DIR=$S/target-payloads cargo build --release
for g in lj orkut; do
  RAYON_NUM_THREADS=8 systemd-run --user --scope -p MemoryMax=8G -p MemorySwapMax=0 \
    $S/target-payloads/release/payloads $S/maps/$g.pmtiles $S/maps/$g-layout results/payloads_$g.json
done
```

with `S=/home/tux/graphlab/scratch/m5-02-tile-format`.
