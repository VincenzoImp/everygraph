// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The whole-file check (§ 2.4): every tile and every chunk, the rules of the arrays
//! (§ 5.3), what ties tiles to the arrays and to each other (§ 3.6), and the census.

use everygraph_core::{ByteSource, Memory, SegVec, usize_of};

use super::census::{Census, CensusReport};
use super::read::{ArrayReader, MapReader};
use super::tile::{covers, inside};
use super::{EgmapError, RegionKind, Tile, malformed};

/// The children of one parent, as `check_regions` reads them.
#[derive(Clone, Copy, Debug, Default)]
struct Run {
    parent: u64,
    first: u64,
    nodes: u64,
    internal: u64,
    volume: u64,
    /// The smallest and largest partner named, 0 left out.
    partners: Option<(u64, u64)>,
}

/// What the tiles showed of a region: its first record.
#[derive(Clone, Copy, Debug, Default)]
struct Seen {
    disc: (f32, f32, f32),
    nodes: u64,
    kind: u8,
    colour: u8,
    zmin: u8,
    seen: bool,
}

impl<S: ByteSource> MapReader<S> {
    /// Reads all of the file and checks every rule a reader can (§ 2.4); gives the
    /// census of the whole pyramid (§ 6), whose faults are not errors of the format.
    /// It keeps 41 bytes per region and the census's per entity, reserved through
    /// `memory`, reads the tiles twice and each array chunk a bounded number of times.
    ///
    /// # Errors
    ///
    /// The first error of [`for_each_tile`](Self::for_each_tile) or
    /// [`chunk`](Self::chunk); [`EgmapError::Malformed`] for a broken rule;
    /// [`EgmapError::Memory`] if the budget refuses its memory.
    pub fn verify(&self, memory: &Memory) -> Result<CensusReport, EgmapError> {
        check_metadata(&self.metadata()?)?;
        let info = self.info();
        let count = usize::try_from(info.regions).map_err(|_| malformed("too many regions"))?;
        let mut seen = SegVec::try_with_len(memory, "map verify: regions", count, Seen::default())?;
        let mut census = Census::new(memory);
        self.for_each_tile(|t| {
            census.add(&t)?;
            for g in &t.regions {
                let s = &mut seen[usize_of(g.id)];
                if !s.seen {
                    *s = Seen {
                        disc: (g.x, g.y, g.r),
                        nodes: g.nodes,
                        kind: g.kind as u8,
                        colour: g.colour,
                        zmin: g.zmin,
                        seen: true,
                    };
                }
            }
            Ok(())
        })?;
        for s in self.sections() {
            for i in 0..s.chunks() {
                self.chunk(s, i)?;
            }
        }
        let (parents, children) = self.check_regions(&seen, memory)?;
        self.for_each_tile(|t| {
            Self::check_cover(&t, &seen, &parents)?;
            for v in &t.nodes {
                let g = seen[usize_of(v.region)];
                if g.seen
                    && (v.colour != g.colour || v.zmin < g.zmin || !inside((v.x, v.y, v.r), g.disc))
                {
                    return Err(malformed(format!(
                        "node {} disagrees with its region (colour, zmin or disc)",
                        v.id
                    )));
                }
            }
            Ok(())
        })?;
        self.check_arrays(&children)?;
        let report = census.finish(self.zooms().1);
        if report.inconsistent > 0 {
            return Err(malformed(format!(
                "{} entities or labels have two different records in two tiles",
                report.inconsistent
            )));
        }
        Ok(report)
    }

    fn array(&self, name: &str) -> Result<ArrayReader<'_, S>, EgmapError> {
        let s = self
            .section(name)
            .ok_or_else(|| malformed(format!("no array {name}")))?;
        Ok(ArrayReader::new(self, s))
    }

    /// The rules of the region table (§ 5.3) and its ties to the tiles (§ 3.6), in one
    /// forward pass over each array; gives each region's parent and number of children.
    fn check_regions(
        &self,
        seen: &SegVec<Seen>,
        memory: &Memory,
    ) -> Result<(SegVec<u32>, SegVec<u32>), EgmapError> {
        let info = self.info();
        let count = seen.len();
        let mut depth = SegVec::try_with_len(memory, "map verify: depths", count, 0_u8)?;
        let mut children = SegVec::try_with_len(memory, "map verify: children", count, 0_u32)?;
        let mut parents = SegVec::try_with_len(memory, "map verify: parents", count, 0_u32)?;
        let array = |n: &str| self.array(&format!("regions.{n}"));
        let (mut parent, mut kind, mut nodes) = (array("parent")?, array("kind")?, array("nodes")?);
        let (mut internal, mut volume) = (array("internal")?, array("volume")?);
        let (mut communities, mut partner) = (array("communities")?, array("partner")?);
        // The parent's side, read forwards: the parents never decrease along the ids.
        let mut up = [
            array("kind")?,
            array("nodes")?,
            array("internal")?,
            array("volume")?,
        ];
        // The children of the parent being read, which have consecutive ids.
        let mut run: Option<Run> = None;
        let close = |run: Option<Run>, end: u64, up: &mut [ArrayReader<'_, S>; 4]| {
            let Some(r) = run else {
                return Ok(());
            };
            let p = r.parent;
            if r.nodes > up[1].get(p)? || r.internal > up[2].get(p)? || r.volume > up[3].get(p)? {
                return Err(malformed(format!(
                    "the children of region {p} add up to more than it"
                )));
            }
            if r.partners.is_some_and(|(lo, hi)| lo < r.first || hi >= end) {
                return Err(malformed(format!(
                    "a child of region {p} has a partner that is not its sibling"
                )));
            }
            Ok(())
        };
        let (mut last_parent, mut last_nodes) = (0, 0);
        for i in 0..info.regions {
            let (p, k, n) = (parent.get(i)?, kind.get(i)?, nodes.get(i)?);
            let k = u8::try_from(k)
                .ok()
                .and_then(RegionKind::from_code)
                .ok_or_else(|| malformed(format!("region {i} has an unknown kind")))?;
            let iu = usize::try_from(i).unwrap_or(usize::MAX);
            if i == 0 {
                if p != 0
                    || k != RegionKind::Root
                    || n != info.nodes
                    || communities.get(0)? != 0
                    || partner.get(0)? != 0
                {
                    return Err(malformed("region 0 is not the root of all nodes"));
                }
                continue;
            }
            let smaller = i > 1 && p == last_parent && n > last_nodes;
            if p >= i || p < last_parent || k == RegionKind::Root || smaller {
                return Err(malformed(format!(
                    "region {i} is not numbered breadth first, larger siblings first"
                )));
            }
            (last_parent, last_nodes) = (p, n);
            let pu = usize::try_from(p).unwrap_or(usize::MAX);
            // A `u8` holds the deepest a region may be below the root, 255 (§ 3.2).
            depth[iu] = depth[pu]
                .checked_add(1)
                .ok_or_else(|| malformed(format!("region {i} is more than 255 deep")))?;
            children[pu] = children[pu].saturating_add(1);
            let parent_kind = RegionKind::from_code(u8::try_from(up[0].get(p)?).unwrap_or(u8::MAX));
            let (inner, vol, held, mate) = (
                internal.get(i)?,
                volume.get(i)?,
                communities.get(i)?,
                partner.get(i)?,
            );
            let holds = matches!(k, RegionKind::Aggregate | RegionKind::Other);
            if (k == RegionKind::Super
                && !matches!(parent_kind, Some(RegionKind::Root | RegionKind::Super)))
                || (k == RegionKind::Other && info.summary == 0)
                || n == 0
                || inner.checked_mul(2).is_none_or(|x| x > vol)
                || holds != (held > 0)
                || mate == i
            {
                return Err(malformed(format!("region {i} breaks a rule of § 5.3")));
            }
            let s = seen[iu];
            if s.seen && (s.nodes != n || s.kind != k as u8) {
                return Err(malformed(format!("region {i} has other counts in a tile")));
            }
            let q = seen[pu];
            if p != 0
                && s.seen
                && q.seen
                && (s.colour != q.colour || s.zmin < q.zmin || !inside(s.disc, q.disc))
            {
                return Err(malformed(format!(
                    "region {i} disagrees with its parent (colour, zmin or disc)"
                )));
            }
            parents[iu] = u32::try_from(p).unwrap_or(u32::MAX);
            if run.as_ref().is_some_and(|r| r.parent != p) {
                close(run.take(), i, &mut up)?;
            }
            let r = run.get_or_insert(Run {
                parent: p,
                first: i,
                ..Run::default()
            });
            let sum = |a: u64, b: u64| {
                a.checked_add(b)
                    .ok_or_else(|| malformed("counts past 2^64"))
            };
            (r.nodes, r.internal, r.volume) = (
                sum(r.nodes, n)?,
                sum(r.internal, inner)?,
                sum(r.volume, vol)?,
            );
            if mate != 0 {
                let (lo, hi) = r.partners.unwrap_or((mate, mate));
                r.partners = Some((lo.min(mate), hi.max(mate)));
            }
        }
        close(run, info.regions, &mut up)?;
        Ok((parents, children))
    }

    /// The tie of a tile's covering region to the tree (§ 3.6): every entity of the tile
    /// lies under it, it covers the tile, and none of its children on those paths does.
    fn check_cover(t: &Tile, seen: &SegVec<Seen>, parents: &SegVec<u32>) -> Result<(), EgmapError> {
        let c = t.cover;
        let covers_tile = |g: u32| {
            let s = seen[usize_of(g)];
            s.seen && covers(s.disc.0, s.disc.1, s.disc.2, t.z, t.x, t.y)
        };
        let bad = || {
            malformed(format!(
                "tile {}/{}/{}: its covering region is not the deepest one over its entities",
                t.z, t.x, t.y
            ))
        };
        let k = seen[usize_of(c)];
        if c != 0 && k.seen && (!covers_tile(c) || k.zmin > t.z) {
            return Err(bad());
        }
        let starts = t
            .regions
            .iter()
            .map(|g| g.id)
            .chain(t.nodes.iter().map(|v| v.region));
        for start in starts {
            let (mut x, mut below) = (start, None);
            // The tree is at most 255 deep (§ 3.2), as `check_regions` found.
            for _ in 0..=256 {
                if x == c || x == 0 {
                    break;
                }
                below = Some(x);
                x = parents[usize_of(x)];
            }
            if x != c || below.is_some_and(|b| b != start && covers_tile(b)) {
                return Err(bad());
            }
        }
        Ok(())
    }

    /// The rules of § 5.3 on the node and panel arrays.
    fn check_arrays(&self, children: &SegVec<u32>) -> Result<(), EgmapError> {
        let info = self.info();
        if let Some(s) = self.section("positions") {
            let mut c = ArrayReader::new(self, s);
            for i in 0..s.count {
                let v = f32::from_bits(u32::try_from(c.get(i)?).unwrap_or(u32::MAX));
                if !(0.0..=1.0).contains(&v) {
                    return Err(malformed("a position outside the unit square"));
                }
            }
        }
        // Symmetry in one pass: the sum of h1(u)·h2(v) over the entries equals that of
        // h1(v)·h2(u) when every (u, v) has its (v, u). The mixes are fixed, so a file
        // crafted against them can pass (§ 5.3).
        let (mut a, mut b) = (0_u64, 0_u64);
        self.check_lists("adjacency", info.nodes, "targets", |u, prev, v| {
            a = a.wrapping_add(mix(u, 1).wrapping_mul(mix(v, 2)));
            b = b.wrapping_add(mix(v, 1).wrapping_mul(mix(u, 2)));
            v < info.nodes && v != u && prev.is_none_or(|p| p < v)
        })?;
        if a != b {
            return Err(malformed("adjacency.targets is not symmetric"));
        }
        self.check_lists("matrix", info.regions, "pairs", |p, prev, pair| {
            let (i, j) = (pair >> 16, pair & 0xFFFF);
            let k = usize::try_from(p)
                .ok()
                .and_then(|p| children.get(p))
                .copied();
            1 <= i && i < j && k.is_some_and(|k| j <= u64::from(k)) && prev.is_none_or(|q| q < pair)
        })?;
        if let Some(w) = self.section("matrix.weights") {
            let mut c = ArrayReader::new(self, w);
            for i in 0..w.count {
                if c.get(i)? == 0 {
                    return Err(malformed("a matrix pair of weight 0"));
                }
            }
        }
        Ok(())
    }

    /// Checks `<group>.offsets` (`rows + 1` values from 0, not decreasing, ending at the
    /// length of `<group>.<values>`) and each row's values against
    /// `ok(row, previous, value)`.
    fn check_lists(
        &self,
        group: &str,
        rows: u64,
        values: &str,
        mut ok: impl FnMut(u64, Option<u64>, u64) -> bool,
    ) -> Result<(), EgmapError> {
        let (Some(offsets), Some(values)) = (
            self.section(&format!("{group}.offsets")),
            self.section(&format!("{group}.{values}")),
        ) else {
            return Ok(());
        };
        let (mut o, mut v) = (
            ArrayReader::new(self, offsets),
            ArrayReader::new(self, values),
        );
        let mut start = o.get(0)?;
        if start != 0 {
            return Err(malformed(format!("{group}.offsets does not start at 0")));
        }
        for row in 0..rows {
            let end = o.get(row + 1)?;
            if end < start || end > values.count {
                return Err(malformed(format!("{group}.offsets decreases or overruns")));
            }
            let mut prev = None;
            for i in start..end {
                let x = v.get(i)?;
                if !ok(row, prev, x) {
                    return Err(malformed(format!(
                        "{group}: row {row} breaks a rule of § 5.3"
                    )));
                }
                prev = Some(x);
            }
            start = end;
        }
        if start != values.count {
            return Err(malformed(format!(
                "{group}.offsets does not end at the last value"
            )));
        }
        Ok(())
    }
}

/// `SplitMix64`'s finalizer of `x` under the key `k`.
fn mix(x: u64, k: u64) -> u64 {
    let mut z = x ^ k.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// The metadata rules of § 2.2.
fn check_metadata(text: &str) -> Result<(), EgmapError> {
    let bad = || malformed("the metadata is not the JSON object of § 2.2");
    let value: serde_json::Value = serde_json::from_str(text).map_err(|_| bad())?;
    let m = value.as_object().ok_or_else(bad)?;
    let version = m.get("version").and_then(|v| v.as_str()).ok_or_else(bad)?;
    let minor = version.strip_prefix("1.").ok_or_else(bad)?;
    let strings = ["name", "description", "attribution"]
        .iter()
        .all(|k| m.get(*k).is_some_and(serde_json::Value::is_string));
    let zoom = m.get("zoom").and_then(|z| z.as_object()).ok_or_else(bad)?;
    let numbers = ["region_px", "open_px", "node_px", "label_off_px"]
        .iter()
        .all(|k| zoom.get(*k).is_some_and(serde_json::Value::is_number));
    if m.get("format").and_then(|v| v.as_str()) != Some("everygraph-egmap")
        || minor.is_empty()
        || !minor.bytes().all(|b| b.is_ascii_digit())
        || !strings
        || !numbers
    {
        return Err(bad());
    }
    Ok(())
}
