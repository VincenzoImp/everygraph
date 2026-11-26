// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Test maps built from the rules of `docs/spec/egmap.md` alone: a random tile that
//! passes § 4.3, and a whole pyramid placed by § 3 (every entity in every tile its disc
//! meets and does not cover, from its `zmin` on), with the arrays of § 5.3. The zoom
//! rule of § 3.1 is written here, as the builder (#69) will write it.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use everygraph_core::{ByteSink, Memory};
use everygraph_map::egmap::{
    Anchor, EgmapError, Label, MapInfo, MapWriter, Metadata, Node, Region, RegionKind, TILE_PX,
    Tile, Values, ZoomRule, tile_id, tile_of_id,
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

/// `size(r, px)` of § 3.1: the least zoom at which a radius `r` reaches `px` pixels,
/// or 17 when no zoom of a file reaches it.
pub fn size(r: f32, px: f32) -> u8 {
    (0..=16)
        .find(|&z| f64::from(r) * f64::from(TILE_PX) * f64::from(1_u32 << z) >= f64::from(px))
        .unwrap_or(17)
}

/// `zmin` of § 3.1 for a region under the parent `(radius, zmin)`, `None` under the root.
pub fn region_zmin(r: f32, parent: Option<(f32, u8)>, zmax: u8) -> u8 {
    let after = parent.map_or(0, |(pr, pz)| pz.max(size(pr, RULE.open_px)));
    size(r, RULE.region_px).max(after).min(zmax)
}

/// `zmin` of § 3.1 for a node in the region `(radius, zmin)`.
pub fn node_zmin(r: f32, region: (f32, u8), zmax: u8) -> u8 {
    let open = region.1.max(size(region.0, RULE.open_px));
    size(r, RULE.node_px).max(open).min(zmax)
}

/// The square of a tile, `(x0, x1, y0, y1)`.
fn square(z: u8, tx: u32, ty: u32) -> (f64, f64, f64, f64) {
    let w = 1.0 / f64::from(1_u32 << z);
    (
        f64::from(tx) * w,
        f64::from(tx + 1) * w,
        f64::from(ty) * w,
        f64::from(ty + 1) * w,
    )
}

/// Whether a disc meets the closed square of a tile (§ 3.5), written apart from the
/// reader's own test.
pub fn meets(x: f32, y: f32, r: f32, z: u8, tx: u32, ty: u32) -> bool {
    let (x0, x1, y0, y1) = square(z, tx, ty);
    let gap = |c: f64, lo: f64, hi: f64| {
        if c < lo {
            lo - c
        } else if c > hi {
            c - hi
        } else {
            0.0
        }
    };
    let (dx, dy) = (gap(f64::from(x), x0, x1), gap(f64::from(y), y0, y1));
    dx * dx + dy * dy <= f64::from(r) * f64::from(r)
}

/// Whether a disc covers the closed square of a tile: every corner inside (§ 3.5).
pub fn covers(x: f32, y: f32, r: f32, z: u8, tx: u32, ty: u32) -> bool {
    let (x0, x1, y0, y1) = square(z, tx, ty);
    let far = |c: f64, lo: f64, hi: f64| if c - lo > hi - c { c - lo } else { hi - c };
    let (ex, ey) = (far(f64::from(x), x0, x1), far(f64::from(y), y0, y1));
    ex * ex + ey * ey <= f64::from(r) * f64::from(r)
}

/// Sorts and dedups a tile's strings, and renumbers what refers to them.
pub fn sort_strings(tile: &mut Tile) {
    let mut sorted = tile.strings.clone();
    sorted.sort();
    sorted.dedup();
    let at = |s: &str| sorted.binary_search_by(|t| t.as_str().cmp(s)).unwrap() as u32;
    for v in &mut tile.nodes {
        v.name = at(&tile.strings[v.name as usize]);
    }
    for a in &mut tile.labels {
        a.text = at(&tile.strings[a.text as usize]);
    }
    tile.strings = sorted;
}

/// A random tile that passes § 4.3: every disc centred inside the tile, too small to
/// cover it.
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
    let kinds = [
        RegionKind::Community,
        RegionKind::Subcommunity,
        RegionKind::Aggregate,
        RegionKind::Other,
        RegionKind::Super,
    ];
    let mut id = 0_u32;
    for _ in 0..regions {
        id += 1 + rng.below(1000) as u32;
        let (x, y) = point(rng);
        tile.regions.push(Region {
            id,
            x,
            y,
            r: w * rng.unit(1e-6, 0.6),
            nodes: 1 + (rng.next() >> 1),
            kind: kinds[rng.below(5) as usize],
            colour: rng.below(5) as u8,
            zmin: rng.below(u64::from(z) + 1) as u8,
        });
    }
    let words = ["", "a", "Ünïcødé", "0x1f", "名前"];
    tile.strings = (0..1 + rng.below(8) as usize)
        .map(|i| words[i % 5].repeat(1 + i))
        .collect();
    let strings = tile.strings.len() as u64;
    let mut nid = 0_u32;
    for _ in 0..nodes {
        nid += 1 + rng.below(10) as u32;
        let (x, y) = point(rng);
        tile.nodes.push(Node {
            id: nid,
            region: 1 + rng.below(5000) as u32,
            x,
            y,
            r: w * rng.unit(1e-7, 0.05),
            name: rng.below(strings) as u32,
            degree: rng.next() >> rng.below(64),
            colour: rng.below(5) as u8,
            zmin: rng.below(u64::from(z) + 1) as u8,
        });
    }
    for e in 0..tile.regions.len() + tile.nodes.len() {
        if rng.below(3) != 0 {
            continue;
        }
        let zmin = tile
            .regions
            .get(e)
            .map_or_else(|| tile.nodes[e - tile.regions.len()].zmin, |g| g.zmin);
        tile.labels.push(Label {
            entity: e as u32,
            text: rng.below(strings) as u32,
            z_on: zmin + rng.below(u64::from(z - zmin) + 1) as u8,
            z_off: z + 1 + rng.below(u64::from(16 - z) + 1) as u8,
            anchor: if rng.below(2) == 0 {
                Anchor::Centre
            } else {
                Anchor::Below
            },
        });
    }
    sort_strings(&mut tile);
    tile
}

/// The region table of § 5.3, by region id.
#[derive(Clone, Debug, Default)]
pub struct RegionTable {
    pub parent: Vec<u32>,
    pub kind: Vec<u8>,
    pub nodes: Vec<u64>,
    pub internal: Vec<u64>,
    pub volume: Vec<u64>,
    pub communities: Vec<u32>,
    pub partner: Vec<u32>,
}

impl RegionTable {
    /// The table of a map with the root alone.
    pub fn root(nodes: u64) -> Self {
        Self {
            parent: vec![0],
            kind: vec![RegionKind::Root as u8],
            nodes: vec![nodes],
            internal: vec![0],
            volume: vec![0],
            communities: vec![0],
            partner: vec![0],
        }
    }
}

/// A pyramid with its arrays.
#[derive(Clone)]
pub struct Pyramid {
    pub info: MapInfo,
    pub zmax: u8,
    pub tiles: Vec<Tile>,
    pub table: RegionTable,
    pub positions: Vec<f32>,
    pub adjacency: (Vec<u64>, Vec<u32>),
    pub matrix: (Vec<u64>, Vec<u32>, Vec<u64>),
}

#[derive(Clone, Copy)]
struct Disc {
    parent: usize,
    x: f32,
    y: f32,
    r: f32,
    depth: u8,
}

/// A disc inside `(px, py, pr)` with radius `pr · f`, `f` in `[lo, hi)`: its centre is
/// at most `0.99 (pr − r)` from the parent's, so containment (§ 3.5) holds after
/// rounding.
fn child(rng: &mut Rng, px: f32, py: f32, pr: f32, lo: f32, hi: f32) -> (f32, f32, f32) {
    let r = pr * rng.unit(lo, hi);
    let room = (pr - r) * 0.7;
    (px + rng.unit(-room, room), py + rng.unit(-room, room), r)
}

/// A random map tree of nested discs (a super level, communities, subcommunities and
/// one aggregate), its nodes, a random graph on them, and the tiles of zooms 0 to
/// `zmax` placed by § 3.
pub fn pyramid(seed: u64, zmax: u8) -> Pyramid {
    let mut rng = Rng(seed);
    // The tree in the order it is made; numbered breadth first below.
    let mut discs = vec![Disc {
        parent: 0,
        x: 0.5,
        y: 0.5,
        r: 0.5,
        depth: 0,
    }];
    let mut i = 0;
    // The first region of depth 2 stays a leaf: the aggregate.
    let mut aggregate = 0;
    while i < discs.len() {
        let d = discs[i];
        if d.depth == 2 && aggregate == 0 {
            aggregate = i;
        } else if d.depth < 3 {
            // Siblings do not overlap, as in disc packing (§ 3.5): a few tries each.
            let mut placed: Vec<(f32, f32, f32)> = Vec::new();
            for _ in 0..2 + rng.below(3) {
                for _ in 0..200 {
                    let (x, y, r) = child(&mut rng, d.x, d.y, d.r, 0.15, 0.4);
                    let apart = placed.iter().all(|&(a, b, q)| {
                        let (dx, dy) = (f64::from(x - a), f64::from(y - b));
                        dx * dx + dy * dy > f64::from(r + q) * f64::from(r + q) * 1.0001
                    });
                    if apart {
                        placed.push((x, y, r));
                        discs.push(Disc {
                            parent: i,
                            x,
                            y,
                            r,
                            depth: d.depth + 1,
                        });
                        break;
                    }
                }
            }
        }
        i += 1;
    }
    let parent_of: Vec<usize> = discs.iter().map(|d| d.parent).collect();
    let kids_of = |p: usize| -> Vec<usize> {
        (1..parent_of.len())
            .filter(|&c| parent_of[c] == p)
            .collect()
    };
    let leaves: Vec<usize> = (1..discs.len())
        .filter(|&g| kids_of(g).is_empty())
        .collect();
    let mut nodes: Vec<(usize, f32, f32, f32)> = Vec::new();
    for &g in &leaves {
        let d = discs[g];
        for _ in 0..1 + rng.below(5) {
            let (x, y, r) = child(&mut rng, d.x, d.y, d.r, 0.08, 0.081);
            nodes.push((g, x, y, r));
        }
    }
    let n = nodes.len();
    let mut adj: Vec<BTreeSet<u32>> = vec![BTreeSet::new(); n];
    for _ in 0..n * 3 {
        let (a, b) = (rng.below(n as u64) as usize, rng.below(n as u64) as usize);
        if a != b {
            adj[a].insert(b as u32);
            adj[b].insert(a as u32);
        }
    }
    let edges: u64 = adj.iter().map(|s| s.len() as u64).sum::<u64>() / 2;
    let holds = |g: usize, v: usize| {
        let mut t = nodes[v].0;
        while t != 0 && t != g {
            t = discs[t].parent;
        }
        t == g
    };
    let members: Vec<Vec<usize>> = (0..discs.len())
        .map(|g| (0..n).filter(|&v| holds(g, v)).collect())
        .collect();
    // Breadth first, children by node count, larger first (§ 3.2).
    let children = |p: usize| {
        let mut kids = kids_of(p);
        kids.sort_by_key(|&c| (std::cmp::Reverse(members[c].len()), c));
        kids
    };
    let mut order = vec![0_usize];
    let mut k = 0;
    while k < order.len() {
        order.extend(children(order[k]));
        k += 1;
    }
    let mut id_of = vec![0_u32; discs.len()];
    for (new, &old) in order.iter().enumerate() {
        id_of[old] = new as u32;
    }
    let kind = |g: usize| match discs[g].depth {
        0 => RegionKind::Root,
        1 => RegionKind::Super,
        2 if g == aggregate => RegionKind::Aggregate,
        2 => RegionKind::Community,
        _ => RegionKind::Subcommunity,
    };
    let top = |mut g: usize| {
        while discs[g].depth > 1 {
            g = discs[g].parent;
        }
        g
    };
    let mut zmin = vec![0_u8; discs.len()];
    for &g in &order[1..] {
        let p = discs[g].parent;
        let parent = (p != 0).then(|| (discs[p].r, zmin[p]));
        zmin[g] = region_zmin(discs[g].r, parent, zmax);
    }
    let mut table = RegionTable::default();
    for &g in &order {
        let inside: BTreeSet<u32> = members[g].iter().map(|&v| v as u32).collect();
        table.parent.push(id_of[discs[g].parent]);
        table.kind.push(kind(g) as u8);
        table.nodes.push(members[g].len() as u64);
        table
            .volume
            .push(members[g].iter().map(|&v| adj[v].len() as u64).sum());
        let pairs: u64 = members[g]
            .iter()
            .map(|&v| adj[v].iter().filter(|u| inside.contains(u)).count() as u64)
            .sum();
        table.internal.push(pairs / 2);
        table.communities.push(if kind(g) == RegionKind::Aggregate {
            3
        } else {
            0
        });
        let mate = (g != 0)
            .then(|| children(discs[g].parent).into_iter().find(|&s| s != g))
            .flatten();
        table.partner.push(mate.map_or(0, |s| id_of[s]));
    }
    let first_of = |p: u32| table.parent.iter().position(|&q| q == p).unwrap() as u32;
    // Placement: every entity in every tile its disc meets and does not cover.
    let mut tiles: BTreeMap<(u8, u32, u32), Tile> = BTreeMap::new();
    let mut place = |x: f32, y: f32, r: f32, zmin: u8, f: &mut dyn FnMut(&mut Tile)| {
        for z in zmin..=zmax {
            let s = f64::from(1_u32 << z);
            let lo = |c: f32| ((f64::from(c) - f64::from(r)) * s).floor().max(0.0) as u32;
            let hi =
                |c: f32| (((f64::from(c) + f64::from(r)) * s).floor() as u32).min((1 << z) - 1);
            for tx in lo(x)..=hi(x) {
                for ty in lo(y)..=hi(y) {
                    if meets(x, y, r, z, tx, ty) && !covers(x, y, r, z, tx, ty) {
                        f(tiles
                            .entry((z, tx, ty))
                            .or_insert_with(|| Tile::new(z, tx, ty)));
                    }
                }
            }
        }
    };
    for &g in &order[1..] {
        let d = discs[g];
        let record = Region {
            id: id_of[g],
            x: d.x,
            y: d.y,
            r: d.r,
            nodes: members[g].len() as u64,
            kind: kind(g),
            colour: (id_of[top(g)] % 5) as u8,
            zmin: zmin[g],
        };
        place(d.x, d.y, d.r, zmin[g], &mut |t| t.regions.push(record));
    }
    for (v, &(g, x, y, r)) in nodes.iter().enumerate() {
        let node = Node {
            id: v as u32,
            region: id_of[g],
            x,
            y,
            r,
            name: 0,
            degree: adj[v].len() as u64,
            colour: (id_of[top(g)] % 5) as u8,
            zmin: node_zmin(r, (discs[g].r, zmin[g]), zmax),
        };
        place(x, y, r, node.zmin, &mut |t| t.nodes.push(node));
    }
    // The covering region of each tile: the deepest drawn region that covers it. A tile
    // without entities is written where its cover differs from its parent tile's (§ 3.5).
    let cover = |z: u8, tx: u32, ty: u32| -> usize {
        order[1..]
            .iter()
            .copied()
            .filter(|&g| zmin[g] <= z && covers(discs[g].x, discs[g].y, discs[g].r, z, tx, ty))
            .max_by_key(|&g| discs[g].depth)
            .unwrap_or(0)
    };
    for &g in &order[1..] {
        let d = discs[g];
        for z in zmin[g]..=zmax {
            let s = f64::from(1_u32 << z);
            let lo = |c: f32| ((f64::from(c) - f64::from(d.r)) * s).floor().max(0.0) as u32;
            let hi =
                |c: f32| (((f64::from(c) + f64::from(d.r)) * s).floor() as u32).min((1 << z) - 1);
            for tx in lo(d.x)..=hi(d.x) {
                for ty in lo(d.y)..=hi(d.y) {
                    let up = if z == 0 {
                        0
                    } else {
                        cover(z - 1, tx / 2, ty / 2)
                    };
                    let c = cover(z, tx, ty);
                    if c != 0 && c == g && c != up {
                        tiles
                            .entry((z, tx, ty))
                            .or_insert_with(|| Tile::new(z, tx, ty));
                    }
                }
            }
        }
    }
    let mut tiles: Vec<Tile> = tiles.into_values().collect();
    for t in &mut tiles {
        let c = cover(t.z, t.x, t.y);
        t.cover = id_of[c];
        t.regions.sort_by_key(|g| g.id);
        t.nodes.sort_by_key(|v| v.id);
        let mut strings: Vec<String> = Vec::new();
        let mut intern = |s: String| -> u32 {
            strings.push(s);
            strings.len() as u32 - 1
        };
        for v in &mut t.nodes {
            v.name = intern(format!("node-{}", v.id));
        }
        let mut labels = Vec::new();
        for (e, g) in t.regions.iter().enumerate() {
            let z_off = size(g.r, RULE.label_off_px).min(zmax + 1);
            let rank = g.id - first_of(table.parent[g.id as usize]) + 1;
            if g.zmin <= t.z && t.z < z_off {
                labels.push(Label {
                    entity: e as u32,
                    text: intern(format!("#{rank}")),
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
        if c != 0 {
            let z_off = size(discs[c].r, RULE.label_off_px).min(zmax + 1);
            let rank = t.cover - first_of(table.parent[t.cover as usize]) + 1;
            if t.z < z_off {
                labels.push(Label {
                    entity: (t.regions.len() + t.nodes.len()) as u32,
                    text: intern(format!("#{rank}")),
                    z_on: zmin[c],
                    z_off,
                    anchor: Anchor::Centre,
                });
            }
        }
        t.labels = labels;
        t.strings = strings;
        sort_strings(t);
    }
    tiles.sort_by_key(|t| tile_id(t.z, t.x, t.y));
    let positions = nodes.iter().flat_map(|&(_, x, y, _)| [x, y]).collect();
    let mut offsets = vec![0_u64];
    let mut targets = Vec::new();
    for s in &adj {
        targets.extend(s.iter().copied());
        offsets.push(targets.len() as u64);
    }
    // The pairs between each parent's children, by rank.
    let mut m_offsets = vec![0_u64];
    let (mut pairs, mut weights) = (Vec::new(), Vec::new());
    for &p in &order {
        let kids = children(p);
        let rank_of = |v: usize| kids.iter().position(|&k| holds(k, v)).map(|r| r as u32 + 1);
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
        pairs.extend(w.keys());
        weights.extend(w.values());
        m_offsets.push(pairs.len() as u64);
    }
    Pyramid {
        info: MapInfo {
            nodes: n as u64,
            regions: discs.len() as u64,
            edges,
            summary: 0,
        },
        zmax,
        tiles,
        table,
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

/// Writes one array whole, in chunks of `chunk` values, in two calls.
pub fn array<K: ByteSink>(
    w: &mut MapWriter<K>,
    name: &str,
    values: Values<'_>,
    chunk: u32,
) -> Result<(), EgmapError> {
    w.begin_array(name, values.element(), chunk)?;
    let (a, b) = split(values, values.len() / 3);
    w.write_values(a)?;
    w.write_values(b)?;
    w.end_array()
}

/// Writes the region table.
pub fn write_table<K: ByteSink>(
    w: &mut MapWriter<K>,
    t: &RegionTable,
    chunk: u32,
) -> Result<(), EgmapError> {
    array(w, "regions.parent", Values::U32(&t.parent), chunk)?;
    array(w, "regions.kind", Values::U8(&t.kind), chunk)?;
    array(w, "regions.nodes", Values::U64(&t.nodes), chunk)?;
    array(w, "regions.internal", Values::U64(&t.internal), chunk)?;
    array(w, "regions.volume", Values::U64(&t.volume), chunk)?;
    array(w, "regions.communities", Values::U32(&t.communities), chunk)?;
    array(w, "regions.partner", Values::U32(&t.partner), chunk)
}

/// Writes `p` into `sink`, every array in chunks of `chunk` values.
pub fn write_into<K: ByteSink>(p: &Pyramid, sink: K, chunk: u32) -> Result<K, EgmapError> {
    let mut w = MapWriter::create(sink, &Memory::unlimited(), p.info)?;
    for t in &p.tiles {
        w.add_tile(t)?;
    }
    array(
        &mut w,
        "adjacency.offsets",
        Values::U64(&p.adjacency.0),
        chunk,
    )?;
    array(
        &mut w,
        "adjacency.targets",
        Values::U32(&p.adjacency.1),
        chunk,
    )?;
    array(&mut w, "matrix.weights", Values::U64(&p.matrix.2), chunk)?;
    array(&mut w, "matrix.offsets", Values::U64(&p.matrix.0), chunk)?;
    array(&mut w, "matrix.pairs", Values::U32(&p.matrix.1), chunk)?;
    array(&mut w, "positions", Values::F32(&p.positions), chunk)?;
    write_table(&mut w, &p.table, chunk)?;
    w.finish(&metadata())
}

/// A map of `count` empty tiles from zoom 9 on, with the root alone: enough tiles for
/// leaf directories.
pub fn empty_tiles<K: ByteSink>(sink: K, count: u64) -> Result<K, EgmapError> {
    let info = MapInfo {
        regions: 1,
        ..MapInfo::default()
    };
    let mut w = MapWriter::create(sink, &Memory::unlimited(), info)?;
    let first = tile_id(9, 0, 0);
    for id in first..first + count {
        let (z, x, y) = tile_of_id(id).unwrap();
        w.add_tile(&Tile::new(z, x, y))?;
    }
    write_table(&mut w, &RegionTable::root(0), 1024)?;
    w.finish(&metadata())
}

pub fn split(v: Values<'_>, at: usize) -> (Values<'_>, Values<'_>) {
    match v {
        Values::U8(x) => (Values::U8(&x[..at]), Values::U8(&x[at..])),
        Values::U32(x) => (Values::U32(&x[..at]), Values::U32(&x[at..])),
        Values::U64(x) => (Values::U64(&x[..at]), Values::U64(&x[at..])),
        Values::F32(x) => (Values::F32(&x[..at]), Values::F32(&x[at..])),
    }
}
