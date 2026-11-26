// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Test maps built from the rules of `docs/spec/egmap.md` alone: a random tile that
//! passes § 4.3, and a whole pyramid placed by § 3 (every entity in every tile its disc
//! meets, from its `zmin` on), with the arrays of § 5.3.

#![allow(dead_code)]

use std::collections::BTreeMap;

use everygraph_core::{ByteSink, Memory};
use everygraph_map::egmap::{
    Anchor, EgmapError, Label, MapInfo, MapWriter, Metadata, Node, Region, RegionKind, Tile,
    Values, ZoomRule, tile_id,
};

/// `SplitMix64`: a fixed generator, so that every run builds the same maps.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    /// A value in `[lo, hi)`.
    pub fn unit(&mut self, lo: f32, hi: f32) -> f32 {
        let u = (self.next() >> 40) as f32 / (1_u64 << 24) as f32;
        lo + (hi - lo) * u
    }
}

pub const RULE: ZoomRule = ZoomRule {
    region_px: 3.0,
    open_px: 12.0,
    node_px: 1.5,
    label_off_px: 600.0,
};

/// Whether a disc meets the closed square of a tile, written from § 3.1 apart from the
/// reader's own test.
pub fn meets(x: f32, y: f32, r: f32, z: u8, tx: u32, ty: u32) -> bool {
    let w = 1.0 / f64::from(1_u32 << z);
    let gap = |c: f32, t: u32| {
        let lo = f64::from(t) * w;
        let hi = f64::from(t + 1) * w;
        let c = f64::from(c);
        if c < lo {
            lo - c
        } else if c > hi {
            c - hi
        } else {
            0.0
        }
    };
    let (dx, dy) = (gap(x, tx), gap(y, ty));
    dx * dx + dy * dy <= f64::from(r) * f64::from(r)
}

/// A random tile that passes § 4.3: every disc centred inside the tile.
pub fn random_tile(rng: &mut Rng, z: u8, regions: usize, nodes: usize) -> Tile {
    let n = 1_u32 << z;
    let (tx, ty) = (
        rng.below(u64::from(n)) as u32,
        rng.below(u64::from(n)) as u32,
    );
    let mut tile = Tile::new(z, tx, ty);
    tile.omitted.regions = rng.below(3);
    tile.omitted.region_nodes = rng.below(1000);
    tile.omitted.nodes = rng.next() >> rng.below(64);
    let w = 1.0 / n as f32;
    let point = |rng: &mut Rng| {
        let x = (tx as f32 + rng.unit(0.0, 1.0)) * w;
        let y = (ty as f32 + rng.unit(0.0, 1.0)) * w;
        (x.min((tx + 1) as f32 * w), y.min((ty + 1) as f32 * w))
    };
    let mut id = 0_u32;
    for i in 0..regions {
        id += 1 + rng.below(1000) as u32;
        let parent = if i == 0 || rng.below(3) == 0 {
            None
        } else {
            Some(tile.regions[rng.below(i as u64) as usize])
        };
        let (x, y) = point(rng);
        let kind = [
            RegionKind::Community,
            RegionKind::Subdivision,
            RegionKind::Aggregate,
            RegionKind::Other,
        ][rng.below(4) as usize];
        let internal = rng.next() >> 2;
        let zmin = parent.map_or(0, |p| p.zmin);
        tile.regions.push(Region {
            id,
            parent: parent.map_or(0, |p| p.id),
            x,
            y,
            r: rng.unit(1e-6, 0.5),
            nodes: 1 + (rng.next() >> 1),
            internal,
            volume: 2 * internal + (rng.next() >> 2),
            communities: if matches!(kind, RegionKind::Aggregate | RegionKind::Other) {
                1 + rng.below(50) as u32
            } else {
                0
            },
            rank: 1 + rng.below(300) as u32,
            partner: if rng.below(2) == 0 { 0 } else { id + 1 },
            kind,
            depth: parent.map_or(1, |p| p.depth + 1),
            colour: parent.map_or(rng.below(5) as u8, |p| p.colour),
            zmin: zmin + rng.below(u64::from(z - zmin) + 1) as u8,
        });
    }
    let strings = 1 + rng.below(8) as usize;
    tile.strings = (0..strings)
        .map(|i| ["", "a", "Ünïcødé", "0x1f", "名前"][i % 5].repeat(1 + i))
        .collect();
    let mut nid = 0_u32;
    for _ in 0..if regions == 0 { 0 } else { nodes } {
        nid += 1 + rng.below(10) as u32;
        let g = tile.regions[rng.below(regions as u64) as usize];
        let (x, y) = point(rng);
        tile.nodes.push(Node {
            id: nid,
            region: g.id,
            x,
            y,
            r: rng.unit(1e-7, 0.01),
            name: rng.below(strings as u64) as u32,
            degree: rng.next() >> rng.below(64),
            zmin: g.zmin + rng.below(u64::from(z - g.zmin) + 1) as u8,
        });
    }
    let entities = tile.regions.len() + tile.nodes.len();
    for e in 0..entities {
        if rng.below(3) != 0 {
            continue;
        }
        let zmin = tile
            .regions
            .get(e)
            .map_or_else(|| tile.nodes[e - tile.regions.len()].zmin, |g| g.zmin);
        tile.labels.push(Label {
            entity: e as u32,
            text: rng.below(strings as u64) as u32,
            z_on: zmin + rng.below(u64::from(z - zmin) + 1) as u8,
            z_off: z + 1 + rng.below(u64::from(20 - z) + 1) as u8,
            anchor: if rng.below(2) == 0 {
                Anchor::Centre
            } else {
                Anchor::Below
            },
        });
    }
    tile
}

/// A pyramid with its arrays.
pub struct Pyramid {
    pub info: MapInfo,
    pub zmax: u8,
    pub tiles: Vec<Tile>,
    pub positions: Vec<f32>,
    pub adjacency: (Vec<u64>, Vec<u32>),
    pub matrix: (Vec<u64>, Vec<u32>, Vec<u64>),
}

struct Disc {
    id: u32,
    parent: u32,
    x: f32,
    y: f32,
    r: f32,
    depth: u8,
    zmin: u8,
}

/// A random hierarchy of nested discs, its nodes, a random graph on them, and the tiles
/// of zooms 0 to `zmax` placed by § 3.
pub fn pyramid(seed: u64, zmax: u8) -> Pyramid {
    let mut rng = Rng(seed);
    // Regions, breadth first, so that a parent's id is below its children's.
    let mut regions: Vec<Disc> = vec![Disc {
        id: 0,
        parent: 0,
        x: 0.5,
        y: 0.5,
        r: 0.5,
        depth: 0,
        zmin: 0,
    }];
    let mut i = 0;
    while i < regions.len() {
        let (px, py, pr, depth, pzmin) = {
            let p = &regions[i];
            (p.x, p.y, p.r, p.depth, p.zmin)
        };
        if depth < 3 {
            for _ in 0..2 + rng.below(3) {
                let r = pr * rng.unit(0.15, 0.4);
                let room = (pr - r) * 0.7;
                let (x, y) = (px + rng.unit(-room, room), py + rng.unit(-room, room));
                let parent = (depth > 0).then_some((pr, pzmin));
                regions.push(Disc {
                    id: regions.len() as u32,
                    parent: i as u32,
                    x,
                    y,
                    r,
                    depth: depth + 1,
                    zmin: RULE.region_zmin(r, parent, zmax),
                });
            }
        }
        i += 1;
    }
    let leaves: Vec<usize> = (1..regions.len())
        .filter(|&g| !regions.iter().any(|c| c.parent as usize == g && c.id != 0))
        .collect();
    // Nodes: a few in each leaf region.
    let mut nodes: Vec<(usize, f32, f32, f32, u8)> = Vec::new();
    for &g in &leaves {
        let d = &regions[g];
        for _ in 0..1 + rng.below(5) {
            let r = d.r * 0.08;
            let room = (d.r - r) * 0.7;
            let (x, y) = (d.x + rng.unit(-room, room), d.y + rng.unit(-room, room));
            nodes.push((g, x, y, r, RULE.node_zmin(r, (d.r, d.zmin), zmax)));
        }
    }
    let n = nodes.len();
    // Every node's undirected neighbours, as sets.
    let mut adj: Vec<std::collections::BTreeSet<u32>> = vec![Default::default(); n];
    for _ in 0..n * 3 {
        let (a, b) = (rng.below(n as u64) as usize, rng.below(n as u64) as usize);
        if a != b {
            adj[a].insert(b as u32);
            adj[b].insert(a as u32);
        }
    }
    let edges: u64 = adj.iter().map(|s| s.len() as u64).sum::<u64>() / 2;
    let under = |g: usize| -> Vec<usize> {
        (0..n)
            .filter(|&v| {
                let mut t = nodes[v].0;
                loop {
                    if t == g {
                        return true;
                    }
                    if t == 0 {
                        return false;
                    }
                    t = regions[t].parent as usize;
                }
            })
            .collect()
    };
    let members: Vec<Vec<usize>> = (0..regions.len()).map(under).collect();
    let children = |p: usize| -> Vec<usize> {
        let mut c: Vec<usize> = (1..regions.len())
            .filter(|&g| regions[g].parent as usize == p)
            .collect();
        c.sort_by_key(|&g| (std::cmp::Reverse(members[g].len()), g));
        c
    };
    let record = |g: usize| -> Region {
        let d = &regions[g];
        let inside: std::collections::BTreeSet<u32> =
            members[g].iter().map(|&v| v as u32).collect();
        let volume: u64 = members[g].iter().map(|&v| adj[v].len() as u64).sum();
        let internal: u64 = members[g]
            .iter()
            .map(|&v| adj[v].iter().filter(|u| inside.contains(u)).count() as u64)
            .sum::<u64>()
            / 2;
        let siblings = children(d.parent as usize);
        let mut top = g;
        while regions[top].depth > 1 {
            top = regions[top].parent as usize;
        }
        Region {
            id: d.id,
            parent: d.parent,
            x: d.x,
            y: d.y,
            r: d.r,
            nodes: members[g].len().max(1) as u64,
            internal,
            volume,
            communities: 0,
            rank: 1 + siblings.iter().position(|&s| s == g).unwrap() as u32,
            partner: siblings.iter().copied().find(|&s| s != g).unwrap_or(0) as u32,
            kind: RegionKind::Community,
            depth: d.depth,
            colour: (top % 5) as u8,
            zmin: d.zmin,
        }
    };
    let records: Vec<Region> = (1..regions.len()).map(record).collect();
    // Placement: every entity in every tile its disc meets, from its zmin on.
    let mut tiles: BTreeMap<(u8, u32, u32), Tile> = BTreeMap::new();
    let mut cover = |x: f32, y: f32, r: f32, zmin: u8, f: &mut dyn FnMut(&mut Tile)| {
        for z in zmin..=zmax {
            let s = f64::from(1_u32 << z);
            let lo = |c: f32| ((f64::from(c) - f64::from(r)) * s).floor().max(0.0) as u32;
            let hi =
                |c: f32| (((f64::from(c) + f64::from(r)) * s).floor() as u32).min((1 << z) - 1);
            for tx in lo(x)..=hi(x) {
                for ty in lo(y)..=hi(y) {
                    if meets(x, y, r, z, tx, ty) {
                        f(tiles
                            .entry((z, tx, ty))
                            .or_insert_with(|| Tile::new(z, tx, ty)));
                    }
                }
            }
        }
    };
    for g in &records {
        cover(g.x, g.y, g.r, g.zmin, &mut |t| t.regions.push(*g));
    }
    for (v, &(g, x, y, r, zmin)) in nodes.iter().enumerate() {
        let node = Node {
            id: v as u32,
            region: g as u32,
            x,
            y,
            r,
            name: 0,
            degree: adj[v].len() as u64,
            zmin,
        };
        cover(x, y, r, zmin, &mut |t| t.nodes.push(node));
    }
    let mut tiles: Vec<Tile> = tiles.into_values().collect();
    for t in &mut tiles {
        t.regions.sort_by_key(|g| g.id);
        t.nodes.sort_by_key(|v| v.id);
        let mut strings: Vec<String> = Vec::new();
        let mut intern = |s: String| -> u32 {
            strings.iter().position(|t| *t == s).unwrap_or_else(|| {
                strings.push(s);
                strings.len() - 1
            }) as u32
        };
        for v in &mut t.nodes {
            v.name = intern(format!("node-{}", v.id));
        }
        let mut labels = Vec::new();
        for (e, g) in t.regions.iter().enumerate() {
            let z_off = RULE.label_off(g.r, zmax);
            if g.depth <= 2 && g.zmin <= t.z && t.z < z_off {
                labels.push(Label {
                    entity: e as u32,
                    text: intern(format!("#{} of {}", g.rank, g.parent)),
                    z_on: g.zmin,
                    z_off,
                    anchor: Anchor::Centre,
                });
            }
        }
        for (j, v) in t.nodes.iter().enumerate() {
            if v.id % 3 == 0 {
                labels.push(Label {
                    entity: (t.regions.len() + j) as u32,
                    text: intern(format!("node-{}", v.id)),
                    z_on: v.zmin,
                    z_off: zmax + 1,
                    anchor: Anchor::Below,
                });
            }
        }
        t.labels = labels;
        t.strings = strings;
    }
    tiles.sort_by_key(|t| tile_id(t.z, t.x, t.y));
    let positions = nodes.iter().flat_map(|&(_, x, y, _, _)| [x, y]).collect();
    let mut offsets = vec![0_u64];
    let mut targets = Vec::new();
    for s in &adj {
        targets.extend(s.iter().copied());
        offsets.push(targets.len() as u64);
    }
    // The pairs between each parent's children, by rank.
    let mut m_offsets = vec![0_u64];
    let (mut pairs, mut weights) = (Vec::new(), Vec::new());
    for p in 0..regions.len() {
        let kids = children(p);
        let rank_of = |v: usize| {
            kids.iter()
                .position(|&k| members[k].contains(&v))
                .map(|r| r as u32 + 1)
        };
        let mut w: BTreeMap<u32, u64> = BTreeMap::new();
        for &v in &members[p] {
            for &u in &adj[v] {
                if let (Some(i), Some(j)) = (rank_of(v), rank_of(u as usize))
                    && i < j
                {
                    *w.entry(i << 16 | j).or_default() += 1;
                }
            }
        }
        for (k, c) in w {
            pairs.push(k);
            weights.push(c);
        }
        m_offsets.push(pairs.len() as u64);
    }
    Pyramid {
        info: MapInfo {
            nodes: n as u64,
            regions: regions.len() as u64,
            edges,
            summary: 0,
        },
        zmax,
        tiles,
        positions,
        adjacency: (offsets, targets),
        matrix: (m_offsets, pairs, weights),
    }
}

pub fn metadata() -> Metadata {
    Metadata {
        name: "test \"graph\"\n".into(),
        description: "a pyramid of nested discs".into(),
        attribution: "tests/support".into(),
        zoom: RULE,
    }
}

/// Writes `p` into `sink`, every array in chunks of `chunk` values.
pub fn write_into<K: ByteSink>(p: &Pyramid, sink: K, chunk: u32) -> Result<K, EgmapError> {
    let mut w = MapWriter::create(sink, &Memory::unlimited(), p.info)?;
    for t in &p.tiles {
        w.add_tile(t)?;
    }
    let arrays: [(&str, Values<'_>); 6] = [
        ("adjacency.offsets", Values::U64(&p.adjacency.0)),
        ("adjacency.targets", Values::U32(&p.adjacency.1)),
        ("matrix.weights", Values::U64(&p.matrix.2)),
        ("matrix.offsets", Values::U64(&p.matrix.0)),
        ("matrix.pairs", Values::U32(&p.matrix.1)),
        ("positions", Values::F32(&p.positions)),
    ];
    for (name, values) in arrays {
        w.begin_array(name, values.element(), chunk)?;
        // In two calls, so that a chunk is filled across them.
        let half = values.len() / 3;
        let (a, b) = split(values, half);
        w.write_values(a)?;
        w.write_values(b)?;
        w.end_array()?;
    }
    w.finish(&metadata())
}

pub fn split(v: Values<'_>, at: usize) -> (Values<'_>, Values<'_>) {
    match v {
        Values::U32(x) => (Values::U32(&x[..at]), Values::U32(&x[at..])),
        Values::U64(x) => (Values::U64(&x[..at]), Values::U64(&x[at..])),
        Values::F32(x) => (Values::F32(&x[..at]), Values::F32(&x[at..])),
        _ => unreachable!(),
    }
}
