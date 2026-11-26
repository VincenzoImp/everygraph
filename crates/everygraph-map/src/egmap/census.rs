// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Zoom consistency (§ 3.1, § 6): the zoom rule, and the census that counts what breaks
//! it in a set of tiles.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;

use everygraph_core::{Memory, MemoryKind, Reservation, usize_of, widen};

use super::{EgmapError, MAX_ZOOM, TILE_PX, Tile};

/// The first zoom at which a disc of radius `r` (in units of the map's width) is at
/// least `px` pixels across its radius: the least `z ≥ 0` with `r · 256 · 2^z ≥ px`, or
/// `MAX_ZOOM + 1` when no zoom of a file reaches it. Exact: each side is a `f32` times a
/// power of two, computed in `f64`.
#[must_use]
pub fn min_zoom(r: f32, px: f32) -> u8 {
    let r = f64::from(r) * f64::from(TILE_PX);
    let px = f64::from(px);
    (0..=MAX_ZOOM)
        .find(|&z| r * f64::from(1_u32 << z) >= px)
        .unwrap_or(MAX_ZOOM + 1)
}

/// The pixel thresholds of the zoom rule (§ 3.1). The builder chooses them; the file
/// records them in its metadata.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZoomRule {
    /// A region is drawn once its radius reaches this many pixels.
    pub region_px: f32,
    /// A region is open, and its children may be drawn, once its radius reaches this.
    pub open_px: f32,
    /// A node is drawn once its radius reaches this, and its region is open.
    pub node_px: f32,
    /// A region's label is no longer drawn once its radius reaches this.
    pub label_off_px: f32,
}

impl ZoomRule {
    /// The `zmin` of a region of radius `r`, given the radius and `zmin` of its parent
    /// (`None` under the root), in a map of zooms 0 to `zmax`.
    #[must_use]
    pub fn region_zmin(&self, r: f32, parent: Option<(f32, u8)>, zmax: u8) -> u8 {
        let own = min_zoom(r, self.region_px);
        let after = parent.map_or(0, |(pr, pz)| pz.max(min_zoom(pr, self.open_px)));
        own.max(after).min(zmax)
    }

    /// The `zmin` of a node of radius `r`, given the radius and `zmin` of its region.
    #[must_use]
    pub fn node_zmin(&self, r: f32, region: (f32, u8), zmax: u8) -> u8 {
        let open = region.1.max(min_zoom(region.0, self.open_px));
        min_zoom(r, self.node_px).max(open).min(zmax)
    }

    /// The `z_off` of the label of a region of radius `r`; a node's label has
    /// `zmax + 1`.
    #[must_use]
    pub fn label_off(&self, r: f32, zmax: u8) -> u8 {
        min_zoom(r, self.label_off_px).min(zmax + 1)
    }
}

/// What a [`Census`] found (§ 6). Each count is of entities or labels, not of tiles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CensusReport {
    /// Tiles given.
    pub tiles: u64,
    /// Distinct regions and nodes seen.
    pub entities: u64,
    /// Entities seen at a zoom, not at a deeper one, then seen again.
    pub popping: u64,
    /// Entities not seen at the deepest zoom.
    pub vanished: u64,
    /// Entities first seen after their `zmin`.
    pub late: u64,
    /// Entities or labels whose record differs between two tiles.
    pub inconsistent: u64,
    /// Distinct labels seen.
    pub labels: u64,
    /// Labels seen at a zoom, not at a deeper one, then seen again.
    pub label_popping: u64,
    /// Labels not seen at every zoom of their interval, up to the deepest zoom.
    pub label_cut: u64,
}

#[derive(Debug)]
struct Seen {
    zooms: u32,
    fingerprint: u64,
    zmin: u8,
    z_off: u8,
}

/// Counts the zoom-consistency faults of a set of tiles: a whole pyramid, or the
/// tiles of a scripted dive. Its memory is reserved per entity through a [`Memory`].
#[derive(Debug)]
pub struct Census {
    entities: HashMap<(bool, u32), Seen>,
    labels: HashMap<(bool, u32), Seen>,
    tiles: u64,
    inconsistent: u64,
    reservation: Reservation,
}

/// What one map entry costs, as an estimate of a hash map's slot and its slack.
const BYTES_PER_ENTRY: u64 = 64;

fn fingerprint(words: &[u64]) -> u64 {
    let mut h = DefaultHasher::new();
    for &w in words {
        h.write_u64(w);
    }
    h.finish()
}

fn text(h: &mut DefaultHasher, s: &str) -> u64 {
    h.write(s.as_bytes());
    h.finish()
}

impl Census {
    /// An empty census.
    #[must_use]
    pub fn new(memory: &Memory) -> Self {
        Self {
            entities: HashMap::new(),
            labels: HashMap::new(),
            tiles: 0,
            inconsistent: 0,
            reservation: Reservation::empty(memory, "map census", MemoryKind::Anonymous),
        }
    }

    fn record(
        map: &mut HashMap<(bool, u32), Seen>,
        reservation: &mut Reservation,
        key: (bool, u32),
        z: u8,
        fingerprint: u64,
        zmin: u8,
        z_off: u8,
    ) -> Result<bool, EgmapError> {
        if let Some(seen) = map.get_mut(&key) {
            seen.zooms |= 1 << z;
            return Ok(seen.fingerprint == fingerprint);
        }
        reservation
            .try_grow(BYTES_PER_ENTRY)
            .map_err(|e| EgmapError::Memory(e.into()))?;
        map.insert(
            key,
            Seen {
                zooms: 1 << z,
                fingerprint,
                zmin,
                z_off,
            },
        );
        Ok(true)
    }

    /// Adds the entities and labels of `tile`, which must have passed the checks of
    /// [`Tile::decode`].
    ///
    /// # Errors
    ///
    /// [`EgmapError::Memory`] if the budget refuses the census more memory.
    pub fn add(&mut self, tile: &Tile) -> Result<(), EgmapError> {
        self.tiles += 1;
        let z = tile.z;
        let mut keys = Vec::new();
        for g in &tile.regions {
            let f = fingerprint(&[
                u64::from(g.parent),
                u64::from(g.x.to_bits()),
                u64::from(g.y.to_bits()),
                u64::from(g.r.to_bits()),
                g.nodes,
                g.internal,
                g.volume,
                u64::from(g.communities),
                u64::from(g.rank),
                u64::from(g.partner),
                g.kind as u64,
                u64::from(g.depth),
                u64::from(g.colour),
                u64::from(g.zmin),
            ]);
            let key = (false, g.id);
            keys.push(key);
            if !Self::record(
                &mut self.entities,
                &mut self.reservation,
                key,
                z,
                f,
                g.zmin,
                0,
            )? {
                self.inconsistent += 1;
            }
        }
        for v in &tile.nodes {
            let mut h = DefaultHasher::new();
            let name = text(&mut h, &tile.strings[usize_of(v.name)]);
            let f = fingerprint(&[
                u64::from(v.region),
                u64::from(v.x.to_bits()),
                u64::from(v.y.to_bits()),
                u64::from(v.r.to_bits()),
                name,
                v.degree,
                u64::from(v.zmin),
            ]);
            let key = (true, v.id);
            keys.push(key);
            if !Self::record(
                &mut self.entities,
                &mut self.reservation,
                key,
                z,
                f,
                v.zmin,
                0,
            )? {
                self.inconsistent += 1;
            }
        }
        for a in &tile.labels {
            let mut h = DefaultHasher::new();
            let words = text(&mut h, &tile.strings[usize_of(a.text)]);
            let f = fingerprint(&[
                words,
                u64::from(a.z_on),
                u64::from(a.z_off),
                a.anchor as u64,
            ]);
            let key = keys[usize_of(a.entity)];
            if !Self::record(
                &mut self.labels,
                &mut self.reservation,
                key,
                z,
                f,
                a.z_on,
                a.z_off,
            )? {
                self.inconsistent += 1;
            }
        }
        Ok(())
    }

    /// Counts the faults of everything added, for a pyramid of zooms 0 to `zmax`.
    #[must_use]
    pub fn finish(self, zmax: u8) -> CensusReport {
        let gaps = |zooms: u32| {
            let first = zooms.trailing_zeros();
            let last = 31 - zooms.leading_zeros();
            let span = (u32::MAX >> (31 - last)) & (u32::MAX << first);
            zooms != span
        };
        let mut report = CensusReport {
            tiles: self.tiles,
            entities: widen(self.entities.len()),
            labels: widen(self.labels.len()),
            inconsistent: self.inconsistent,
            ..CensusReport::default()
        };
        for s in self.entities.values() {
            report.popping += u64::from(gaps(s.zooms));
            report.vanished += u64::from(s.zooms & (1 << zmax) == 0);
            report.late += u64::from(s.zooms.trailing_zeros() > u32::from(s.zmin));
        }
        for s in self.labels.values() {
            report.label_popping += u64::from(gaps(s.zooms));
            let end = u32::from(s.z_off.min(zmax + 1));
            let want = (u32::MAX >> (32 - end)) & (u32::MAX << s.zmin);
            report.label_cut += u64::from(s.zooms != want);
        }
        report
    }
}
