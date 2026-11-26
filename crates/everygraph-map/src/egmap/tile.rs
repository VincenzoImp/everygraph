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
const STRINGS: u32 = REQUIRED | 4;
/// The number of region colours of the approved style (`design/DECISIONS.md`).
pub(super) const COLOURS: u8 = 5;

/// What a region is (§ 3.2), with the codes of the files.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RegionKind {
    /// A community of the partition the map draws.
    Community = 0,
    /// A part of a community that the map tree split again.
    Subcommunity = 1,
    /// The smaller communities of one parent drawn as one region.
    Aggregate = 2,
    /// In a summary map, the regions and pairs below its `k` (§ 7).
    Other = 3,
    /// A region of the super level above the partition.
    Super = 4,
    /// Region 0, the whole graph; never in a tile.
    Root = 5,
}

impl RegionKind {
    /// The kind of `code`, if it is one.
    #[must_use]
    pub fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            0 => Self::Community,
            1 => Self::Subcommunity,
            2 => Self::Aggregate,
            3 => Self::Other,
            4 => Self::Super,
            5 => Self::Root,
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

/// A region as a tile draws it (§ 3.2, § 4.2); its other facts are in the region
/// table (§ 5.3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region {
    /// Its id in the map tree.
    pub id: u32,
    /// Centre and radius, in units of the map's width.
    pub x: f32,
    /// See `x`.
    pub y: f32,
    /// See `x`.
    pub r: f32,
    /// Its nodes, exactly.
    pub nodes: u64,
    /// What it is; never [`RegionKind::Root`].
    pub kind: RegionKind,
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
    /// Its region's colour index.
    pub colour: u8,
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
    /// The deepest region whose disc covers the tile; 0 when none does (§ 3.5).
    pub cover: u32,
    /// What the budget left out.
    pub omitted: Omitted,
    /// Regions by increasing id.
    pub regions: Vec<Region>,
    /// Nodes by increasing id.
    pub nodes: Vec<Node>,
    /// Labels by increasing entity; entity `regions.len() + nodes.len()` is the
    /// covering region.
    pub labels: Vec<Label>,
    /// The texts that nodes and labels refer to, in increasing byte order.
    pub strings: Vec<String>,
}

/// A row of a block: its tag, the width of each column, and each column's value as the
/// bits of a `u64` (a float as its bit pattern, an enum as its code).
trait Record: Copy {
    const TAG: u32;
    const WIDTHS: &'static [usize];
    fn empty() -> Self;
    fn get(&self, column: usize) -> u64;
    /// Sets a column from a value of its width; refuses a code no enum has.
    fn set(&mut self, column: usize, value: u64) -> Result<(), EgmapError>;
}

/// `value`, which a column of its width read, as the field's type.
fn narrow<T: TryFrom<u64>>(value: u64) -> T {
    T::try_from(value).unwrap_or_else(|_| unreachable!("a column holds values of its width"))
}

fn bits(v: f32) -> u64 {
    u64::from(v.to_bits())
}

impl Record for Region {
    const TAG: u32 = REQUIRED | 1;
    const WIDTHS: &'static [usize] = &[4, 4, 4, 4, 8, 1, 1, 1];
    fn empty() -> Self {
        Self {
            id: 0,
            x: 0.0,
            y: 0.0,
            r: 0.0,
            nodes: 0,
            kind: RegionKind::Community,
            colour: 0,
            zmin: 0,
        }
    }
    fn get(&self, column: usize) -> u64 {
        match column {
            0 => self.id.into(),
            1 => bits(self.x),
            2 => bits(self.y),
            3 => bits(self.r),
            4 => self.nodes,
            5 => self.kind as u64,
            6 => self.colour.into(),
            _ => self.zmin.into(),
        }
    }
    fn set(&mut self, column: usize, v: u64) -> Result<(), EgmapError> {
        match column {
            0 => self.id = narrow(v),
            1 => self.x = f32::from_bits(narrow(v)),
            2 => self.y = f32::from_bits(narrow(v)),
            3 => self.r = f32::from_bits(narrow(v)),
            4 => self.nodes = v,
            5 => {
                self.kind = RegionKind::from_code(narrow(v))
                    .ok_or_else(|| malformed("an unknown region kind"))?;
            }
            6 => self.colour = narrow(v),
            _ => self.zmin = narrow(v),
        }
        Ok(())
    }
}

impl Record for Node {
    const TAG: u32 = REQUIRED | 2;
    const WIDTHS: &'static [usize] = &[4, 4, 4, 4, 4, 4, 8, 1, 1];
    fn empty() -> Self {
        Self {
            id: 0,
            region: 0,
            x: 0.0,
            y: 0.0,
            r: 0.0,
            name: 0,
            degree: 0,
            colour: 0,
            zmin: 0,
        }
    }
    fn get(&self, column: usize) -> u64 {
        match column {
            0 => self.id.into(),
            1 => self.region.into(),
            2 => bits(self.x),
            3 => bits(self.y),
            4 => bits(self.r),
            5 => self.name.into(),
            6 => self.degree,
            7 => self.colour.into(),
            _ => self.zmin.into(),
        }
    }
    fn set(&mut self, column: usize, v: u64) -> Result<(), EgmapError> {
        match column {
            0 => self.id = narrow(v),
            1 => self.region = narrow(v),
            2 => self.x = f32::from_bits(narrow(v)),
            3 => self.y = f32::from_bits(narrow(v)),
            4 => self.r = f32::from_bits(narrow(v)),
            5 => self.name = narrow(v),
            6 => self.degree = v,
            7 => self.colour = narrow(v),
            _ => self.zmin = narrow(v),
        }
        Ok(())
    }
}

impl Record for Label {
    const TAG: u32 = REQUIRED | 3;
    const WIDTHS: &'static [usize] = &[4, 4, 1, 1, 1];
    fn empty() -> Self {
        Self {
            entity: 0,
            text: 0,
            z_on: 0,
            z_off: 0,
            anchor: Anchor::Centre,
        }
    }
    fn get(&self, column: usize) -> u64 {
        match column {
            0 => self.entity.into(),
            1 => self.text.into(),
            2 => self.z_on.into(),
            3 => self.z_off.into(),
            _ => self.anchor as u64,
        }
    }
    fn set(&mut self, column: usize, v: u64) -> Result<(), EgmapError> {
        match column {
            0 => self.entity = narrow(v),
            1 => self.text = narrow(v),
            2 => self.z_on = narrow(v),
            3 => self.z_off = narrow(v),
            _ => {
                self.anchor = match v {
                    0 => Anchor::Centre,
                    1 => Anchor::Below,
                    _ => return Err(malformed("an unknown label anchor")),
                };
            }
        }
        Ok(())
    }
}

const fn pad8(n: usize) -> usize {
    n.div_ceil(8) * 8
}

/// Zeros after the bytes from `start`, to a multiple of 8.
fn pad(out: &mut Vec<u8>, start: usize) {
    let len = out.len() - start;
    out.extend(std::iter::repeat_n(0, pad8(len) - len));
}

fn body_len<R: Record>(rows: usize) -> Option<usize> {
    R::WIDTHS.iter().try_fold(0_usize, |sum, &w| {
        sum.checked_add(pad8(rows.checked_mul(w)?))
    })
}

fn strings_len(strings: &[String]) -> Option<usize> {
    let bytes = strings
        .iter()
        .try_fold(0_usize, |sum, s| sum.checked_add(s.len()))?;
    pad8(strings.len().checked_mul(4)?).checked_add(pad8(bytes))
}

fn block_header(out: &mut Vec<u8>, tag: u32, rows: usize, len: usize) {
    out.extend_from_slice(&tag.to_le_bytes());
    out.extend_from_slice(&u32::try_from(rows).unwrap_or(u32::MAX).to_le_bytes());
    out.extend_from_slice(&widen(len).to_le_bytes());
}

fn encode_block<R: Record>(out: &mut Vec<u8>, rows: &[R]) {
    if rows.is_empty() {
        return;
    }
    block_header(
        out,
        R::TAG,
        rows.len(),
        body_len::<R>(rows.len()).unwrap_or(0),
    );
    for (c, &w) in R::WIDTHS.iter().enumerate() {
        let start = out.len();
        for row in rows {
            out.extend_from_slice(&row.get(c).to_le_bytes()[..w]);
        }
        pad(out, start);
    }
}

/// The next `len` bytes of `body` from `*at`, then their zero padding.
fn take<'a>(body: &'a [u8], at: &mut usize, len: usize) -> Result<&'a [u8], EgmapError> {
    if pad8(len) > body.len() - *at {
        return Err(malformed("a tile column overruns its block"));
    }
    let column = &body[*at..*at + len];
    if body[*at + len..*at + pad8(len)].iter().any(|&b| b != 0) {
        return Err(malformed("tile column padding is not zero"));
    }
    *at += pad8(len);
    Ok(column)
}

fn decode_block<R: Record>(body: &[u8], rows: usize) -> Result<Vec<R>, EgmapError> {
    if body_len::<R>(rows) != Some(body.len()) {
        return Err(malformed("a tile block is not the length its rows give"));
    }
    let mut out: Vec<R> = (0..rows).map(|_| R::empty()).collect();
    let mut at = 0;
    for (c, &w) in R::WIDTHS.iter().enumerate() {
        let column = take(body, &mut at, rows * w)?;
        for (row, value) in out.iter_mut().zip(column.chunks_exact(w)) {
            let mut word = [0_u8; 8];
            word[..w].copy_from_slice(value);
            row.set(c, u64::from_le_bytes(word))?;
        }
    }
    Ok(out)
}

fn decode_strings(body: &[u8], rows: usize) -> Result<Vec<String>, EgmapError> {
    let mut at = 0;
    let ends: Vec<usize> = take(body, &mut at, rows * 4)?
        .chunks_exact(4)
        .map(|c| usize_of(le::u32_at(c, 0)))
        .collect();
    let bytes = take(body, &mut at, *ends.last().unwrap_or(&0))?;
    if at != body.len() {
        return Err(malformed(
            "the strings block is not the length its strings give",
        ));
    }
    let mut start = 0;
    ends.iter()
        .map(|&end| {
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

fn in_unit(v: f32) -> bool {
    (0.0..=1.0).contains(&v) && v.to_bits() != (-0.0_f32).to_bits()
}

/// The square of tile `(z, tx, ty)` in binary64: `(x0, x1, y0, y1)`, exact.
fn square(z: u8, tx: u32, ty: u32) -> (f64, f64, f64, f64) {
    let w = 1.0 / f64::from(1_u32 << z);
    (
        f64::from(tx) * w,
        f64::from(tx + 1) * w,
        f64::from(ty) * w,
        f64::from(ty + 1) * w,
    )
}

/// Whether the disc `(x, y, r)` meets the closed square of tile `(z, tx, ty)` and does
/// not cover it (§ 3.5): when it is placed in the tile.
pub(super) fn placed(x: f32, y: f32, r: f32, z: u8, tx: u32, ty: u32) -> bool {
    let (x0, x1, y0, y1) = square(z, tx, ty);
    let (cx, cy, rr) = (f64::from(x), f64::from(y), f64::from(r));
    let (dx, dy) = (
        (x0 - cx).max(cx - x1).max(0.0),
        (y0 - cy).max(cy - y1).max(0.0),
    );
    dx * dx + dy * dy <= rr * rr && !covers(x, y, r, z, tx, ty)
}

/// Whether the disc `(x, y, r)` holds every corner of tile `(z, tx, ty)` (§ 3.5).
pub(super) fn covers(x: f32, y: f32, r: f32, z: u8, tx: u32, ty: u32) -> bool {
    let (x0, x1, y0, y1) = square(z, tx, ty);
    let (cx, cy, r) = (f64::from(x), f64::from(y), f64::from(r));
    let (ex, ey) = ((cx - x0).max(x1 - cx), (cy - y0).max(y1 - cy));
    ex * ex + ey * ey <= r * r
}

/// Whether the disc `(x, y, r)` is inside the disc `(px, py, pr)` (§ 3.5).
pub(super) fn inside(disc: (f32, f32, f32), outer: (f32, f32, f32)) -> bool {
    let (x, y, r) = (f64::from(disc.0), f64::from(disc.1), f64::from(disc.2));
    let (px, py, pr) = (f64::from(outer.0), f64::from(outer.1), f64::from(outer.2));
    let (dx, dy) = (x - px, y - py);
    r <= pr && dx * dx + dy * dy <= (pr - r) * (pr - r)
}

impl Tile {
    /// An empty tile at `(z, x, y)`.
    #[must_use]
    pub fn new(z: u8, x: u32, y: u32) -> Self {
        Self {
            z,
            x,
            y,
            cover: 0,
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
            (self.regions.len(), body_len::<Region>(self.regions.len())),
            (self.nodes.len(), body_len::<Node>(self.nodes.len())),
            (self.labels.len(), body_len::<Label>(self.labels.len())),
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
        out.extend_from_slice(&self.cover.to_le_bytes());
        let o = &self.omitted;
        for v in [o.regions, o.region_nodes, o.nodes] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        encode_block(&mut out, &self.regions);
        encode_block(&mut out, &self.nodes);
        encode_block(&mut out, &self.labels);
        if !self.strings.is_empty() {
            block_header(
                &mut out,
                STRINGS,
                self.strings.len(),
                blocks[3].1.unwrap_or(0),
            );
            let start = out.len();
            let mut end = 0_u32;
            for s in &self.strings {
                end += u32::try_from(s.len()).unwrap_or(u32::MAX);
                out.extend_from_slice(&end.to_le_bytes());
            }
            pad(&mut out, start);
            let start = out.len();
            for s in &self.strings {
                out.extend_from_slice(s.as_bytes());
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
        if bytes[9..12] != [0; 3] {
            return Err(malformed("reserved tile header bytes are not zero"));
        }
        let mut tile = Self::new(bytes[8], le::u32_at(bytes, 12), le::u32_at(bytes, 16));
        tile.cover = le::u32_at(bytes, 20);
        tile.omitted = Omitted {
            regions: le::u64_at(bytes, 24),
            region_nodes: le::u64_at(bytes, 32),
            nodes: le::u64_at(bytes, 40),
        };
        let mut at = HEADER;
        let mut last = 0_u32;
        while at < bytes.len() {
            if bytes.len() - at < BLOCK_HEADER {
                return Err(malformed("a tile ends inside a block header"));
            }
            let tag = le::u32_at(bytes, at);
            let rows = usize_of(le::u32_at(bytes, at + 4));
            let len = le::u64_at(bytes, at + 8);
            at += BLOCK_HEADER;
            if tag & !REQUIRED <= last {
                return Err(malformed(
                    "tile block numbers do not start at 1 and increase",
                ));
            }
            last = tag & !REQUIRED;
            let body = usize::try_from(len)
                .ok()
                .filter(|&l| l <= bytes.len() - at && l % 8 == 0)
                .map(|l| &bytes[at..at + l])
                .ok_or_else(|| malformed("a tile block overruns the payload"))?;
            at += body.len();
            if rows == 0 {
                return Err(malformed("a tile block of 0 rows"));
            }
            match tag {
                Region::TAG => tile.regions = decode_block(body, rows)?,
                Node::TAG => tile.nodes = decode_block(body, rows)?,
                Label::TAG => tile.labels = decode_block(body, rows)?,
                STRINGS => tile.strings = decode_strings(body, rows)?,
                t if t & !REQUIRED <= STRINGS & !REQUIRED => {
                    return Err(malformed(format!("block {t:#010x} lacks its required bit")));
                }
                t if t & REQUIRED != 0 => {
                    return Err(EgmapError::Unsupported(format!(
                        "required tile block {t:#010x}"
                    )));
                }
                _ => {}
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
        let fail = |what: String| Err(malformed(format!("tile {}: {what}", self.name())));
        if self.z > MAX_ZOOM || self.x >= 1 << self.z || self.y >= 1 << self.z {
            return fail(format!("not a tile of zooms 0 to {MAX_ZOOM}"));
        }
        if self.regions.iter().any(|g| g.id == self.cover) {
            return fail("the covering region is in the tile".into());
        }
        let geometry = |x: f32, y: f32, r: f32| {
            in_unit(x)
                && in_unit(y)
                && r > 0.0
                && r <= 1.0
                && placed(x, y, r, self.z, self.x, self.y)
        };
        for (i, g) in self.regions.iter().enumerate() {
            if (i == 0 && g.id == 0) || (i > 0 && self.regions[i - 1].id >= g.id) {
                return fail("region ids do not increase from 1".into());
            }
            if g.kind == RegionKind::Root || g.colour >= COLOURS || g.zmin > self.z || g.nodes == 0
            {
                return fail(format!("region {} breaks a rule of § 4.3", g.id));
            }
            if !geometry(g.x, g.y, g.r) {
                return fail(format!("region {} is not a disc placed in the tile", g.id));
            }
        }
        let strings = self.strings.len();
        for (i, v) in self.nodes.iter().enumerate() {
            if i > 0 && self.nodes[i - 1].id >= v.id {
                return fail("node ids do not increase".into());
            }
            if v.region == 0
                || v.colour >= COLOURS
                || v.zmin > self.z
                || usize_of(v.name) >= strings
            {
                return fail(format!("node {} breaks a rule of § 4.3", v.id));
            }
            if !geometry(v.x, v.y, v.r) {
                return fail(format!("node {} is not a disc placed in the tile", v.id));
            }
        }
        if self.strings.windows(2).any(|w| w[0] >= w[1]) {
            return fail("strings are not in increasing order".into());
        }
        let regions = self.regions.len();
        for (i, a) in self.labels.iter().enumerate() {
            let e = usize_of(a.entity);
            if i > 0 && self.labels[i - 1].entity >= a.entity {
                return fail("label entities do not increase".into());
            }
            let entities = regions + self.nodes.len();
            let covering = e == entities && self.cover != 0;
            if (e >= entities && !covering) || usize_of(a.text) >= strings {
                return fail(format!("label {i} refers to nothing"));
            }
            // The covering region's `zmin` is not in the tile: § 3.6 checks it.
            let zmin = match (self.regions.get(e), self.nodes.get(e.wrapping_sub(regions))) {
                (Some(g), _) => g.zmin,
                (None, Some(v)) if !covering => v.zmin,
                _ => 0,
            };
            if !(zmin <= a.z_on && a.z_on <= self.z && self.z < a.z_off && a.z_off <= MAX_ZOOM + 1)
            {
                return fail(format!("label {i} is not drawn at this zoom"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inside_and_placed_are_closed_and_exact_at_the_boundary() {
        assert!(inside((0.5, 0.5, 0.25), (0.5, 0.5, 0.25)));
        assert!(inside((0.75, 0.5, 0.25), (0.5, 0.5, 0.5)));
        assert!(!inside((0.75, 0.5, 0.25), (0.5, 0.5, 0.4999)));
        assert!(!inside((0.5, 0.5, 0.3), (0.5, 0.5, 0.25)));
        // Tile 1/0/0 is [0, 0.5]²: a disc touching its edge meets it; one holding every
        // corner covers it, so it is not placed there.
        assert!(placed(0.75, 0.25, 0.25, 1, 0, 0));
        assert!(!placed(0.75, 0.25, 0.2499, 1, 0, 0));
        assert!(!placed(0.25, 0.25, 0.36, 1, 0, 0));
        assert!(placed(0.25, 0.25, 0.35, 1, 0, 0));
    }
}
