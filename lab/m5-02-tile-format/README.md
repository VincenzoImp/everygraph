# m5-02: the tile payload against MapLibre Tile and Arrow IPC, and the region table

Issue #68 (`plan/issues/m5-02-tile-format.md`), `docs/spec/egmap.md` § 10. Byte counts
only, which do not depend on load. Exploratory; measured on 2026-11-26 while other work
ran.

**Questions.**
1. On the same content, how many bytes does each payload take, raw and gzip: the payload
   of `docs/spec/egmap.md` § 4, MapLibre Tile (MLT), and Arrow IPC?
2. How large is the region table of § 5.3?

**Content.** Every tile of the lab maps of lj (203,941 tiles) and orkut (142,710 tiles).
`maps.sh` builds them as `lab/m1-06-edge-rendering/maps.sh` does; the lj file is the
same, 895,034,297 bytes. `payloads/` turns each lab tile into the records of the spec
(§ 4.2):
- the lab's regions and nodes, without the regions whose disc covers the tile (§ 3.5):
  567,871 records dropped on lj and 496,654 on orkut;
- regions numbered breadth first;
- `zmin` from the zoom rule with the lab's thresholds (3 px for a region and 1.2 px for
  a node), capped at the tile's zoom;
- the colour of the top region, modulo 5;
- node names are the original ids in decimal, sorted.

There are no labels, because the lab has none, and no edges, because the spec's tiles
hold none. Every converted tile passed the writer's checks (§ 4.3) and decoded back to
itself; none was refused on either graph.

These are the lab's entity sets, not tiles placed by the rule of § 3.5. That rule puts
every entity in every tile its disc meets, so the budget question of #69 (are the
deepest tiles over 32 KB?) is not answered here.

**Payloads.** Each holds exactly those fields:

- `egmap`: `Tile::encode` of `crates/everygraph-map`;
- `mlt_web_4g` and `mlt_min_size`: `mlt-core` 0.24.0, with the profiles `Web(Cellular4G)`
  (the MapLibre GL JS profile) and `MinSize`:
  - one layer for regions and one for nodes;
  - a point geometry per entity, at a tile extent of 4096;
  - every field as a property column, with `x`, `y` and `r` as f32 (MLT's geometry is
    integer, so it would not be lossless without them);
- `arrow_ipc`: arrow-rs 58.4, two IPC streams (regions and nodes), each with its own
  schema.

Gzip is level 6 for all of them.

**Results** (`results/payloads_{lj,orkut}.json`):

| Graph | Payload | Raw total | Gzip total | Gzip median | Gzip p99 | Gzip max |
|---|---|---|---|---|---|---|
| lj | egmap | 695.6 MB | 378.8 MB | 1,139 B | 12,051 B | 14,477 B |
| lj | MLT Web 4G | 480.0 MB | 379.2 MB (+0.1%) | 1,220 B (+7.1%) | 11,025 B | 14,011 B |
| lj | MLT MinSize | 471.3 MB | 378.3 MB (−0.2%) | 1,219 B (+7.0%) | 10,945 B | 13,681 B |
| lj | Arrow IPC | 1,193.4 MB | 454.1 MB (+19.9%) | 1,562 B (+37%) | 11,640 B | 14,040 B |
| orkut | egmap | 492.9 MB | 273.9 MB | 1,188 B | 11,898 B | 14,563 B |
| orkut | MLT Web 4G | 341.2 MB | 273.5 MB (−0.1%) | 1,264 B (+6.4%) | 11,019 B | 14,057 B |
| orkut | MLT MinSize | 332.7 MB | 268.4 MB (−2.0%) | 1,258 B (+5.9%) | 10,382 B | 13,520 B |
| orkut | Arrow IPC | 839.4 MB | 324.9 MB (+18.6%) | 1,610 B (+36%) | 11,452 B | 14,133 B |

- In gzip, MLT and this payload are within 2% of each other in total. MLT is 6–7%
  larger at the median and smaller at p99. Raw, MLT is about 31% smaller, because it
  compresses inside.
- Arrow IPC is 19–20% larger in total and about 36% larger at the median: each tile
  carries its schema.
- The region table of § 5.3, in chunks of 65,536 values and written by the product
  writer, is 8.25 MB on lj (1,637,273 regions) and 4.95 MB on orkut (872,753). Each
  count includes the lab's dead tree nodes. Its parts on lj: `parent` 3.72 MB, `volume`
  2.15 MB, `internal` 1.25 MB, `nodes` 1.11 MB, the others under 10 kB each. The
  `partner` values are all 0 here.
- Against tiles that keep, in every tile, each region's parent, counts, rank and depth,
  and the regions' ancestors: those were 453.0 MB on lj and 324.7 MB on orkut with the
  same tool and content. These tiles plus the region table are 387.1 MB and 278.9 MB,
  or −15% and −14%.
- The lab tiles with their edges were 830.6 MB of gzip on lj and 721.1 MB on orkut
  (`maps/tiles_{lj,orkut}.json` of the build).

**Not measured.** Decode time in JavaScript, of any payload.

**Reproduce** (an 8 GiB limit each; under a minute per graph at 8 threads):

```sh
lab/m5-02-tile-format/maps.sh
cd lab/m5-02-tile-format/payloads
CARGO_TARGET_DIR=$S/target-payloads cargo build --release
for g in lj orkut; do
  RAYON_NUM_THREADS=8 systemd-run --user --scope -p MemoryMax=8G -p MemorySwapMax=0 \
    $S/target-payloads/release/payloads $S/maps/$g.pmtiles $S/maps/$g-layout results/payloads_$g.json
done
```

Here `S=/home/tux/graphlab/scratch/m5-02-tile-format`.
