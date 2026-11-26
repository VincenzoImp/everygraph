// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Zoom consistency (§ 6): the census that counts what breaks § 3 in a set of tiles.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;

use everygraph_core::{Memory, MemoryKind, Reservation, usize_of, widen};

use super::{EgmapError, Tile};

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

/// What one map entry costs: an estimate of a hash map's slot and its slack. While the
/// map grows it briefly holds two tables, so its peak can pass this.
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
                u64::from(g.x.to_bits()),
                u64::from(g.y.to_bits()),
                u64::from(g.r.to_bits()),
                g.nodes,
                g.kind as u64,
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
                u64::from(v.colour),
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
            let key = keys
                .get(usize_of(a.entity))
                .copied()
                .unwrap_or((false, tile.cover));
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
