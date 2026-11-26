// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The tile payload (§ 4): what one tile holds at its zoom, as columns in blocks.

use everygraph_core::{le, usize_of, widen};

use super::{EgmapError, MAX_DECODED_BYTES, MAX_ZOOM, invalid, malformed};

const MAGIC: &[u8; 4] = b"EGMT";
const MAJOR: u16 = 1;
const MINOR: u16 = 0;
const HEADER: usize = 48;
const BLOCK_HEADER: usize = 16;
/// A reader that does not know a block with this bit refuses the tile.
const REQUIRED: u32 = 0x8000_0000;
const REGIONS: u32 = REQUIRED | 1;
const NODES: u32 = REQUIRED | 2;
const LABELS: u32 = REQUIRED | 3;
const STRINGS: u32 = REQUIRED | 4;
/// Bytes per row of each block, over its columns.
const REGION_WIDTHS: [usize; 15] = [4, 4, 4, 4, 4, 8, 8, 8, 4, 4, 4, 1, 1, 1, 1];
const NODE_WIDTHS: [usize; 8] = [4, 4, 4, 4, 4, 4, 8, 1];
const LABEL_WIDTHS: [usize; 5] = [4, 4, 1, 1, 1];
/// The number of region colours of the approved style (`design/DECISIONS.md`).
pub(super) const COLOURS: u8 = 5;

/// What a region is (§ 3.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RegionKind {
    /// A community of the hierarchy.
    Community = 0,
    /// A part of a community that the map hierarchy split to keep regions small.
    Subdivision = 1,
    /// Small communities of one parent drawn as one region.
    Aggregate = 2,
    /// In a summary map, the regions and pairs below its `k` (§ 7).
    Other = 3,
}

impl RegionKind {
    fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            0 => Self::Community,
            1 => Self::Subdivision,
            2 => Self::Aggregate,
            3 => Self::Other,
            _ => return None,
        })
    }
}

/// Where a label sits on its entity (§ 3.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Anchor {
    /// Centred on the disc's centre.
    Centre = 0,
    /// Centred under the disc: its top edge on the lowest point of the disc.
    Below = 1,
}

/// A region of the map hierarchy, as a disc (§ 3.2).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region {
    /// Its id in the map's region tree; 0 is the root, which is never drawn.
    pub id: u32,
    /// The region holding it; 0 at the top.
    pub parent: u32,
    /// Centre and radius, in units of the map's width.
    pub x: f32,
    /// See `x`.
    pub y: f32,
    /// See `x`.
    pub r: f32,
    /// Its nodes, exactly.
    pub nodes: u64,
    /// The weight of the pairs inside it, a loop once.
    pub internal: u64,
    /// The sum of its nodes' degrees, a loop twice. The cut is `volume − 2·internal`.
    pub volume: u64,
    /// For an aggregate or `Other`, the communities it holds; 0 otherwise.
    pub communities: u32,
    /// Its place among its siblings by node count, from 1: the `#3` of the key.
    pub rank: u32,
    /// The sibling it shares most weight with; 0 when it has none.
    pub partner: u32,
    /// What it is.
    pub kind: RegionKind,
    /// 1 for a child of the root.
    pub depth: u8,
    /// The colour index of its top region, below 5.
    pub colour: u8,
    /// The first zoom at which it is drawn (§ 3.1).
    pub zmin: u8,
}

/// A node of the graph (§ 3.3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Node {
    /// Its node id in the graph.
    pub id: u32,
    /// The deepest region that holds it.
    pub region: u32,
    /// Centre and radius, in units of the map's width.
    pub x: f32,
    /// See `x`.
    pub y: f32,
    /// See `x`.
    pub r: f32,
    /// Its original identifier, as text: an index into [`Tile::strings`].
    pub name: u32,
    /// Its degree in the undirected projection, a loop counted twice.
    pub degree: u64,
    /// The first zoom at which it is drawn (§ 3.1).
    pub zmin: u8,
}

/// A label chosen at build time (§ 3.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Label {
    /// The labelled entity: `i` is region `i` of the tile, `regions.len() + j` node `j`.
    pub entity: u32,
    /// Its text: an index into [`Tile::strings`].
    pub text: u32,
    /// Drawn from zoom `z_on` up to, not including, `z_off`.
    pub z_on: u8,
    /// See `z_on`.
    pub z_off: u8,
    /// Where it sits on its entity.
    pub anchor: Anchor,
}

/// What a tile leaves out, as its caption counts it (§ 3.5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Omitted {
    /// Regions left out by the tile budget.
    pub regions: u64,
    /// The nodes inside those regions, each node once.
    pub region_nodes: u64,
    /// Nodes left out by the tile budget.
    pub nodes: u64,
}

/// One tile (§ 4).
#[derive(Clone, Debug, PartialEq)]
pub struct Tile {
    /// Zoom.
    pub z: u8,
    /// Column, from the left.
    pub x: u32,
    /// Row, from the top.
    pub y: u32,
    /// What the budget left out.
    pub omitted: Omitted,
    /// Regions by increasing id.
    pub regions: Vec<Region>,
    /// Nodes by increasing id.
    pub nodes: Vec<Node>,
    /// Labels by increasing entity.
    pub labels: Vec<Label>,
    /// The texts that nodes and labels refer to.
    pub strings: Vec<String>,
}

const fn pad8(n: usize) -> usize {
    n.div_ceil(8) * 8
}

fn body_len(rows: usize, widths: &[usize]) -> Option<usize> {
    widths.iter().try_fold(0_usize, |sum, &w| {
        sum.checked_add(pad8(rows.checked_mul(w)?))
    })
}

fn strings_len(strings: &[String]) -> Option<usize> {
    let bytes = strings
        .iter()
        .try_fold(0_usize, |sum, s| sum.checked_add(s.len()))?;
    pad8(strings.len().checked_mul(4)?).checked_add(pad8(bytes))
}

/// Writes one column: a value per row, then zeros to a multiple of 8.
fn column<R, const W: usize>(out: &mut Vec<u8>, rows: &[R], mut value: impl FnMut(&R) -> [u8; W]) {
    let start = out.len();
    for row in rows {
        out.extend_from_slice(&value(row));
    }
    pad(out, start);
}

/// Zeros after the bytes from `start`, to a multiple of 8.
fn pad(out: &mut Vec<u8>, start: usize) {
    let len = out.len() - start;
    out.extend(std::iter::repeat_n(0, pad8(len) - len));
}

fn in_unit(v: f32) -> bool {
    (0.0..=1.0).contains(&v) && v.to_bits() != (-0.0_f32).to_bits()
}

/// Whether the disc `(x, y, r)` meets the closed square of tile `(z, tx, ty)`, computed
/// exactly in `f64` (every value is a `f32` or a multiple of a power of two).
pub(super) fn disc_meets_tile(x: f32, y: f32, r: f32, z: u8, tx: u32, ty: u32) -> bool {
    let w = 1.0 / f64::from(1_u32 << z);
    let gap = |c: f32, t: u32| {
        let (lo, hi) = (f64::from(t) * w, f64::from(t + 1) * w);
        (lo - f64::from(c)).max(f64::from(c) - hi).max(0.0)
    };
    let (dx, dy) = (gap(x, tx), gap(y, ty));
    dx * dx + dy * dy <= f64::from(r) * f64::from(r)
}

impl Tile {
    /// An empty tile at `(z, x, y)`.
    #[must_use]
    pub fn new(z: u8, x: u32, y: u32) -> Self {
        Self {
            z,
            x,
            y,
            omitted: Omitted::default(),
            regions: Vec::new(),
            nodes: Vec::new(),
            labels: Vec::new(),
            strings: Vec::new(),
        }
    }

    /// The payload bytes of the tile, before compression.
    ///
    /// # Errors
    ///
    /// [`EgmapError::Invalid`] if the tile breaks a rule of § 4.3, or its payload would
    /// be larger than [`MAX_DECODED_BYTES`].
    pub fn encode(&self) -> Result<Vec<u8>, EgmapError> {
        self.check().map_err(|e| match e {
            EgmapError::Malformed(what) => EgmapError::Invalid(what),
            e => e,
        })?;
        let too_large = || invalid(format!("tile {} decodes to more than 1 MiB", self.name()));
        let blocks = [
            (
                self.regions.len(),
                body_len(self.regions.len(), &REGION_WIDTHS),
            ),
            (self.nodes.len(), body_len(self.nodes.len(), &NODE_WIDTHS)),
            (
                self.labels.len(),
                body_len(self.labels.len(), &LABEL_WIDTHS),
            ),
            (self.strings.len(), strings_len(&self.strings)),
        ];
        let mut total = HEADER;
        for (rows, len) in blocks {
            if rows > 0 {
                total = len
                    .and_then(|l| total.checked_add(BLOCK_HEADER + l))
                    .ok_or_else(too_large)?;
            }
        }
        if total > MAX_DECODED_BYTES {
            return Err(too_large());
        }
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&MAJOR.to_le_bytes());
        out.extend_from_slice(&MINOR.to_le_bytes());
        out.extend_from_slice(&[self.z, 0, 0, 0]);
        out.extend_from_slice(&self.x.to_le_bytes());
        out.extend_from_slice(&self.y.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        for v in [
            self.omitted.regions,
            self.omitted.region_nodes,
            self.omitted.nodes,
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        let block = |out: &mut Vec<u8>, tag: u32, rows: usize, len: Option<usize>| {
            out.extend_from_slice(&tag.to_le_bytes());
            out.extend_from_slice(&u32::try_from(rows).unwrap_or(u32::MAX).to_le_bytes());
            out.extend_from_slice(&widen(len.unwrap_or(0)).to_le_bytes());
        };
        let r = &self.regions;
        if !r.is_empty() {
            block(&mut out, REGIONS, r.len(), blocks[0].1);
            column(&mut out, r, |g| g.id.to_le_bytes());
            column(&mut out, r, |g| g.parent.to_le_bytes());
            column(&mut out, r, |g| g.x.to_le_bytes());
            column(&mut out, r, |g| g.y.to_le_bytes());
            column(&mut out, r, |g| g.r.to_le_bytes());
            column(&mut out, r, |g| g.nodes.to_le_bytes());
            column(&mut out, r, |g| g.internal.to_le_bytes());
            column(&mut out, r, |g| g.volume.to_le_bytes());
            column(&mut out, r, |g| g.communities.to_le_bytes());
            column(&mut out, r, |g| g.rank.to_le_bytes());
            column(&mut out, r, |g| g.partner.to_le_bytes());
            column(&mut out, r, |g| [g.kind as u8]);
            column(&mut out, r, |g| [g.depth]);
            column(&mut out, r, |g| [g.colour]);
            column(&mut out, r, |g| [g.zmin]);
        }
        let n = &self.nodes;
        if !n.is_empty() {
            block(&mut out, NODES, n.len(), blocks[1].1);
            column(&mut out, n, |v| v.id.to_le_bytes());
            column(&mut out, n, |v| v.region.to_le_bytes());
            column(&mut out, n, |v| v.x.to_le_bytes());
            column(&mut out, n, |v| v.y.to_le_bytes());
            column(&mut out, n, |v| v.r.to_le_bytes());
            column(&mut out, n, |v| v.name.to_le_bytes());
            column(&mut out, n, |v| v.degree.to_le_bytes());
            column(&mut out, n, |v| [v.zmin]);
        }
        let l = &self.labels;
        if !l.is_empty() {
            block(&mut out, LABELS, l.len(), blocks[2].1);
            column(&mut out, l, |a| a.entity.to_le_bytes());
            column(&mut out, l, |a| a.text.to_le_bytes());
            column(&mut out, l, |a| [a.z_on]);
            column(&mut out, l, |a| [a.z_off]);
            column(&mut out, l, |a| [a.anchor as u8]);
        }
        let s = &self.strings;
        if !s.is_empty() {
            block(&mut out, STRINGS, s.len(), blocks[3].1);
            let mut end = 0_u32;
            column(&mut out, s, |t| {
                end += u32::try_from(t.len()).unwrap_or(u32::MAX);
                end.to_le_bytes()
            });
            let start = out.len();
            for t in s {
                out.extend_from_slice(t.as_bytes());
            }
            pad(&mut out, start);
        }
        debug_assert_eq!(out.len(), total);
        Ok(out)
    }

    /// Parses and checks a tile payload (§ 4.2, § 4.3).
    ///
    /// # Errors
    ///
    /// [`EgmapError::Unsupported`] for a later major version or an unknown required
    /// block; [`EgmapError::Malformed`] for any other broken rule.
    pub fn decode(bytes: &[u8]) -> Result<Self, EgmapError> {
        if bytes.len() > MAX_DECODED_BYTES {
            return Err(malformed("a tile payload larger than 1 MiB"));
        }
        if bytes.len() < HEADER || &bytes[..4] != MAGIC {
            return Err(malformed("not a tile payload"));
        }
        let major = le::u16_at(bytes, 4);
        if major != MAJOR {
            return Err(EgmapError::Unsupported(format!(
                "tile payload version {major}"
            )));
        }
        if bytes[9..12] != [0; 3] || bytes[20..24] != [0; 4] {
            return Err(malformed("reserved tile header bytes are not zero"));
        }
        let mut tile = Self::new(bytes[8], le::u32_at(bytes, 12), le::u32_at(bytes, 16));
        tile.omitted = Omitted {
            regions: le::u64_at(bytes, 24),
            region_nodes: le::u64_at(bytes, 32),
            nodes: le::u64_at(bytes, 40),
        };
        let mut at = HEADER;
        let mut last_tag = 0_u32;
        while at < bytes.len() {
            if bytes.len() - at < BLOCK_HEADER {
                return Err(malformed("a tile ends inside a block header"));
            }
            let tag = le::u32_at(bytes, at);
            let rows = usize_of(le::u32_at(bytes, at + 4));
            let len = le::u64_at(bytes, at + 8);
            at += BLOCK_HEADER;
            if tag & !REQUIRED <= last_tag {
                return Err(malformed("tile blocks are not in increasing order"));
            }
            last_tag = tag & !REQUIRED;
            let body = usize::try_from(len)
                .ok()
                .filter(|&l| l <= bytes.len() - at && l % 8 == 0)
                .map(|l| &bytes[at..at + l])
                .ok_or_else(|| malformed("a tile block overruns the payload"))?;
            at += body.len();
            if rows == 0 {
                return Err(malformed("a tile block of 0 rows"));
            }
            let mut c = Columns { body, at: 0, rows };
            match tag {
                REGIONS => tile.regions = c.regions()?,
                NODES => tile.nodes = c.nodes()?,
                LABELS => tile.labels = c.labels()?,
                STRINGS => tile.strings = c.strings()?,
                t if t & !REQUIRED <= STRINGS & !REQUIRED => {
                    return Err(malformed(format!("block {t:#010x} lacks its required bit")));
                }
                t if t & REQUIRED != 0 => {
                    return Err(EgmapError::Unsupported(format!(
                        "required tile block {t:#010x}"
                    )));
                }
                _ => continue,
            }
            if c.at != body.len() {
                return Err(malformed(format!(
                    "tile block {tag:#010x} is not the length its rows give"
                )));
            }
        }
        tile.check()?;
        Ok(tile)
    }

    fn name(&self) -> String {
        format!("{}/{}/{}", self.z, self.x, self.y)
    }

    /// The rules of § 4.3 that do not depend on the bytes.
    fn check(&self) -> Result<(), EgmapError> {
        let here = self.name();
        let fail = |what: String| Err(malformed(format!("tile {here}: {what}")));
        if self.z > MAX_ZOOM || self.x >= 1 << self.z || self.y >= 1 << self.z {
            return fail("not a tile of zooms 0 to 20".into());
        }
        let geometry = |x: f32, y: f32, r: f32| {
            in_unit(x)
                && in_unit(y)
                && r > 0.0
                && r <= 1.0
                && disc_meets_tile(x, y, r, self.z, self.x, self.y)
        };
        let region = |id: u32| self.regions.binary_search_by_key(&id, |g| g.id).ok();
        for (i, g) in self.regions.iter().enumerate() {
            let id = g.id;
            if i > 0 && self.regions[i - 1].id >= id {
                return fail("region ids do not increase".into());
            }
            if g.parent >= id {
                return fail(format!("region {id} does not follow its parent"));
            }
            if g.parent == 0 {
                if g.depth != 1 {
                    return fail(format!("region {id} is at the top but not at depth 1"));
                }
            } else {
                let Some(p) = region(g.parent).map(|i| &self.regions[i]) else {
                    return fail(format!("the parent of region {id} is not in the tile"));
                };
                if g.depth != p.depth.wrapping_add(1) || g.colour != p.colour || g.zmin < p.zmin {
                    return fail(format!("region {id} disagrees with its parent"));
                }
            }
            let holds = matches!(g.kind, RegionKind::Aggregate | RegionKind::Other);
            if g.colour >= COLOURS
                || g.zmin > self.z
                || g.nodes == 0
                || g.rank == 0
                || g.partner == id
                || holds != (g.communities > 0)
                || g.internal.checked_mul(2).is_none_or(|x| x > g.volume)
            {
                return fail(format!("region {id} breaks a rule of § 4.3"));
            }
            if !geometry(g.x, g.y, g.r) {
                return fail(format!("region {id} is not a disc that meets the tile"));
            }
        }
        let strings = self.strings.len();
        for (i, v) in self.nodes.iter().enumerate() {
            let id = v.id;
            if i > 0 && self.nodes[i - 1].id >= id {
                return fail("node ids do not increase".into());
            }
            let Some(g) = region(v.region).map(|i| &self.regions[i]) else {
                return fail(format!("the region of node {id} is not in the tile"));
            };
            if v.zmin > self.z || v.zmin < g.zmin || usize_of(v.name) >= strings {
                return fail(format!("node {id} breaks a rule of § 4.3"));
            }
            if !geometry(v.x, v.y, v.r) {
                return fail(format!("node {id} is not a disc that meets the tile"));
            }
        }
        let entities = self.regions.len() + self.nodes.len();
        for (i, a) in self.labels.iter().enumerate() {
            let e = usize_of(a.entity);
            if i > 0 && self.labels[i - 1].entity >= a.entity {
                return fail("label entities do not increase".into());
            }
            if e >= entities || usize_of(a.text) >= strings {
                return fail(format!("label {i} refers to nothing"));
            }
            let zmin = self
                .regions
                .get(e)
                .map_or_else(|| self.nodes[e - self.regions.len()].zmin, |g| g.zmin);
            if !(zmin <= a.z_on && a.z_on <= self.z && self.z < a.z_off && a.z_off <= MAX_ZOOM + 1)
            {
                return fail(format!("label {i} is not drawn at this zoom"));
            }
        }
        Ok(())
    }
}

/// Reads the columns of one block body.
struct Columns<'a> {
    body: &'a [u8],
    at: usize,
    rows: usize,
}

impl<'a> Columns<'a> {
    /// The next column of `width`-byte values, with its zero padding.
    fn next(&mut self, width: usize) -> Result<&'a [u8], EgmapError> {
        let len = self
            .rows
            .checked_mul(width)
            .filter(|&l| pad8(l) <= self.body.len() - self.at)
            .ok_or_else(|| malformed("a tile column overruns its block"))?;
        let column = &self.body[self.at..self.at + len];
        let padding = &self.body[self.at + len..self.at + pad8(len)];
        if padding.iter().any(|&b| b != 0) {
            return Err(malformed("tile column padding is not zero"));
        }
        self.at += pad8(len);
        Ok(column)
    }

    fn u8s(&mut self) -> Result<&'a [u8], EgmapError> {
        self.next(1)
    }

    /// The next `len` bytes, with their zero padding.
    fn bytes(&mut self, len: usize) -> Result<&'a [u8], EgmapError> {
        let rows = std::mem::replace(&mut self.rows, len);
        let bytes = self.next(1);
        self.rows = rows;
        bytes
    }

    fn u32s(&mut self) -> Result<impl Iterator<Item = u32> + 'a, EgmapError> {
        Ok(self.next(4)?.chunks_exact(4).map(|c| le::u32_at(c, 0)))
    }

    fn f32s(&mut self) -> Result<impl Iterator<Item = f32> + 'a, EgmapError> {
        Ok(self.u32s()?.map(f32::from_bits))
    }

    fn u64s(&mut self) -> Result<impl Iterator<Item = u64> + 'a, EgmapError> {
        Ok(self.next(8)?.chunks_exact(8).map(|c| le::u64_at(c, 0)))
    }

    fn regions(&mut self) -> Result<Vec<Region>, EgmapError> {
        let mut out: Vec<Region> = self
            .u32s()?
            .map(|id| Region {
                id,
                parent: 0,
                x: 0.0,
                y: 0.0,
                r: 0.0,
                nodes: 0,
                internal: 0,
                volume: 0,
                communities: 0,
                rank: 0,
                partner: 0,
                kind: RegionKind::Community,
                depth: 0,
                colour: 0,
                zmin: 0,
            })
            .collect();
        out.iter_mut()
            .zip(self.u32s()?)
            .for_each(|(g, v)| g.parent = v);
        out.iter_mut().zip(self.f32s()?).for_each(|(g, v)| g.x = v);
        out.iter_mut().zip(self.f32s()?).for_each(|(g, v)| g.y = v);
        out.iter_mut().zip(self.f32s()?).for_each(|(g, v)| g.r = v);
        out.iter_mut()
            .zip(self.u64s()?)
            .for_each(|(g, v)| g.nodes = v);
        out.iter_mut()
            .zip(self.u64s()?)
            .for_each(|(g, v)| g.internal = v);
        out.iter_mut()
            .zip(self.u64s()?)
            .for_each(|(g, v)| g.volume = v);
        out.iter_mut()
            .zip(self.u32s()?)
            .for_each(|(g, v)| g.communities = v);
        out.iter_mut()
            .zip(self.u32s()?)
            .for_each(|(g, v)| g.rank = v);
        out.iter_mut()
            .zip(self.u32s()?)
            .for_each(|(g, v)| g.partner = v);
        for (g, &k) in out.iter_mut().zip(self.u8s()?) {
            g.kind = RegionKind::from_code(k).ok_or_else(|| malformed("an unknown region kind"))?;
        }
        out.iter_mut()
            .zip(self.u8s()?)
            .for_each(|(g, &v)| g.depth = v);
        out.iter_mut()
            .zip(self.u8s()?)
            .for_each(|(g, &v)| g.colour = v);
        out.iter_mut()
            .zip(self.u8s()?)
            .for_each(|(g, &v)| g.zmin = v);
        Ok(out)
    }

    fn nodes(&mut self) -> Result<Vec<Node>, EgmapError> {
        let mut out: Vec<Node> = self
            .u32s()?
            .map(|id| Node {
                id,
                region: 0,
                x: 0.0,
                y: 0.0,
                r: 0.0,
                name: 0,
                degree: 0,
                zmin: 0,
            })
            .collect();
        out.iter_mut()
            .zip(self.u32s()?)
            .for_each(|(v, x)| v.region = x);
        out.iter_mut().zip(self.f32s()?).for_each(|(v, x)| v.x = x);
        out.iter_mut().zip(self.f32s()?).for_each(|(v, x)| v.y = x);
        out.iter_mut().zip(self.f32s()?).for_each(|(v, x)| v.r = x);
        out.iter_mut()
            .zip(self.u32s()?)
            .for_each(|(v, x)| v.name = x);
        out.iter_mut()
            .zip(self.u64s()?)
            .for_each(|(v, x)| v.degree = x);
        out.iter_mut()
            .zip(self.u8s()?)
            .for_each(|(v, &x)| v.zmin = x);
        Ok(out)
    }

    fn labels(&mut self) -> Result<Vec<Label>, EgmapError> {
        let mut out: Vec<Label> = self
            .u32s()?
            .map(|entity| Label {
                entity,
                text: 0,
                z_on: 0,
                z_off: 0,
                anchor: Anchor::Centre,
            })
            .collect();
        out.iter_mut()
            .zip(self.u32s()?)
            .for_each(|(a, x)| a.text = x);
        out.iter_mut()
            .zip(self.u8s()?)
            .for_each(|(a, &x)| a.z_on = x);
        out.iter_mut()
            .zip(self.u8s()?)
            .for_each(|(a, &x)| a.z_off = x);
        for (a, &x) in out.iter_mut().zip(self.u8s()?) {
            a.anchor = match x {
                0 => Anchor::Centre,
                1 => Anchor::Below,
                _ => return Err(malformed("an unknown label anchor")),
            };
        }
        Ok(out)
    }

    fn strings(&mut self) -> Result<Vec<String>, EgmapError> {
        let ends: Vec<u32> = self.u32s()?.collect();
        let bytes = self.bytes(usize_of(*ends.last().unwrap_or(&0)))?;
        let mut start = 0_usize;
        ends.iter()
            .map(|&end| {
                let end = usize_of(end);
                let text = bytes
                    .get(start..end)
                    .ok_or_else(|| malformed("string ends do not increase"))?;
                start = end;
                std::str::from_utf8(text)
                    .map(str::to_owned)
                    .map_err(|_| malformed("a string is not UTF-8"))
            })
            .collect()
    }
}
