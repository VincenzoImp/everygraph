// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The PMTiles v3 container (§ 2): the 127-byte header, directories and tile ids, with
//! the narrower rules of a map file.

use everygraph_core::le;

use super::{EgmapError, HEAD_BYTES, MAX_ZOOM, malformed};

pub(super) const HEADER_BYTES: usize = 127;
const MAGIC: &[u8; 7] = b"PMTiles";
const GZIP: u8 = 2;
const CLUSTERED: u8 = 1;
const TILE_TYPE_OTHER: u8 = 0;

/// The tile id of tile `(z, x, y)`: the tiles of every shallower zoom, then the
/// position of `(x, y)` on the Hilbert curve of zoom `z`.
///
/// # Panics
///
/// If `z` is above [`MAX_ZOOM`](super::MAX_ZOOM) or `x` or `y` is not below `2^z`.
#[must_use]
pub fn tile_id(z: u8, x: u32, y: u32) -> u64 {
    assert!(z <= MAX_ZOOM, "zoom {z} is above {MAX_ZOOM}");
    let n = 1_u64 << z;
    let (mut x, mut y) = (u64::from(x), u64::from(y));
    assert!(x < n && y < n, "tile ({x}, {y}) is outside zoom {z}");
    let mut d = 0;
    let mut s = n / 2;
    while s > 0 {
        let rx = u64::from(x & s != 0);
        let ry = u64::from(y & s != 0);
        d += s * s * ((3 * rx) ^ ry);
        if ry == 0 {
            if rx == 1 {
                x = n - 1 - x;
                y = n - 1 - y;
            }
            std::mem::swap(&mut x, &mut y);
        }
        s /= 2;
    }
    tiles_above(z) + d
}

/// The tile `(z, x, y)` of `id`, or `None` past zoom [`MAX_ZOOM`](super::MAX_ZOOM).
#[must_use]
pub fn tile_of_id(id: u64) -> Option<(u8, u32, u32)> {
    let z = (0..=MAX_ZOOM).find(|&z| id < tiles_above(z + 1))?;
    let n = 1_u64 << z;
    let mut t = id - tiles_above(z);
    let (mut x, mut y) = (0_u64, 0_u64);
    let mut s = 1;
    while s < n {
        let rx = 1 & (t / 2);
        let ry = 1 & (t ^ rx);
        if ry == 0 {
            if rx == 1 {
                x = s - 1 - x;
                y = s - 1 - y;
            }
            std::mem::swap(&mut x, &mut y);
        }
        x += s * rx;
        y += s * ry;
        t /= 4;
        s *= 2;
    }
    Some((z, u32::try_from(x).ok()?, u32::try_from(y).ok()?))
}

/// The number of tiles of zooms `0 .. z`: `(4^z − 1) / 3`.
const fn tiles_above(z: u8) -> u64 {
    ((1_u64 << (2 * z)) - 1) / 3
}

/// A run of bytes in the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Span {
    pub offset: u64,
    pub len: u64,
}

impl Span {
    pub fn end(self) -> u64 {
        self.offset + self.len
    }
}

/// The fields of the header a map file uses; the others have fixed values (§ 2.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Header {
    pub root: Span,
    pub metadata: Span,
    pub leaves: Span,
    pub tiles: Span,
    pub tile_count: u64,
    pub min_zoom: u8,
    pub max_zoom: u8,
}

impl Header {
    pub fn encode(&self) -> [u8; HEADER_BYTES] {
        let mut h = [0_u8; HEADER_BYTES];
        h[..7].copy_from_slice(MAGIC);
        h[7] = 3;
        let fields = [
            self.root.offset,
            self.root.len,
            self.metadata.offset,
            self.metadata.len,
            self.leaves.offset,
            self.leaves.len,
            self.tiles.offset,
            self.tiles.len,
            self.tile_count,
            self.tile_count,
            self.tile_count,
        ];
        for (i, v) in fields.into_iter().enumerate() {
            h[8 + 8 * i..16 + 8 * i].copy_from_slice(&v.to_le_bytes());
        }
        h[96..102].copy_from_slice(&[
            CLUSTERED,
            GZIP,
            GZIP,
            TILE_TYPE_OTHER,
            self.min_zoom,
            self.max_zoom,
        ]);
        // The bounds of the whole Web Mercator square, in units of 1e-7 degree; the map
        // is not geographic, but clients that draw a preview read them.
        for (i, v) in [-1_800_000_000_i32, -850_511_287, 1_800_000_000, 850_511_287]
            .into_iter()
            .enumerate()
        {
            h[102 + 4 * i..106 + 4 * i].copy_from_slice(&v.to_le_bytes());
        }
        h
    }

    /// Parses and checks the header of a file of `file_len` bytes.
    pub fn parse(h: &[u8], file_len: u64) -> Result<Self, EgmapError> {
        if h.len() < HEADER_BYTES || &h[..7] != MAGIC {
            return Err(EgmapError::Foreign("no PMTiles header"));
        }
        if h[7] != 3 {
            return Err(EgmapError::Foreign("a PMTiles version other than 3"));
        }
        if h[96..100] != [CLUSTERED, GZIP, GZIP, TILE_TYPE_OTHER] {
            return Err(EgmapError::Foreign(
                "a PMTiles archive that is not a map (tile type, compression or order)",
            ));
        }
        let span = |i: usize| Span {
            offset: le::u64_at(h, 8 + 16 * i),
            len: le::u64_at(h, 16 + 16 * i),
        };
        let header = Self {
            root: span(0),
            metadata: span(1),
            leaves: span(2),
            tiles: span(3),
            tile_count: le::u64_at(h, 72),
            min_zoom: h[100],
            max_zoom: h[101],
        };
        if le::u64_at(h, 80) != header.tile_count || le::u64_at(h, 88) != header.tile_count {
            return Err(malformed(
                "the counts of addressed tiles, entries and contents differ: a map stores each tile once",
            ));
        }
        if header.max_zoom > MAX_ZOOM || header.min_zoom > header.max_zoom {
            return Err(malformed(format!(
                "zooms {} to {}: at most 0 to {MAX_ZOOM}",
                header.min_zoom, header.max_zoom
            )));
        }
        let root = header.root;
        if root.offset != u64::try_from(HEADER_BYTES).unwrap_or(u64::MAX)
            || root.len == 0
            || root.len > HEAD_BYTES
            || root.end() > HEAD_BYTES
        {
            return Err(malformed(
                "the root directory must follow the header and end within the first 16384 bytes",
            ));
        }
        for (name, s) in [
            ("metadata", header.metadata),
            ("leaf directories", header.leaves),
            ("tile data", header.tiles),
        ] {
            if s.offset.checked_add(s.len).is_none_or(|end| end > file_len) {
                return Err(malformed(format!("the {name} lie outside the file")));
            }
        }
        if header.metadata.len == 0 {
            return Err(malformed("no metadata"));
        }
        Ok(header)
    }
}

/// One directory entry. `run` is 1 for a tile and 0 for a leaf directory; a map never
/// stores runs of equal tiles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Entry {
    pub id: u64,
    pub offset: u64,
    pub len: u64,
    pub run: u64,
}

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push(u8::try_from(v & 0x7F).unwrap_or(0) | 0x80);
        v >>= 7;
    }
    out.push(u8::try_from(v).unwrap_or(0));
}

/// The uncompressed bytes of a directory (PMTiles v3 § 4).
pub(super) fn encode_directory(entries: &[Entry]) -> Vec<u8> {
    let mut out = Vec::new();
    put_varint(&mut out, everygraph_core::widen(entries.len()));
    let mut last = 0;
    for e in entries {
        put_varint(&mut out, e.id - last);
        last = e.id;
    }
    for e in entries {
        put_varint(&mut out, e.run);
    }
    for e in entries {
        put_varint(&mut out, e.len);
    }
    for (i, e) in entries.iter().enumerate() {
        let follows = i > 0 && e.offset == entries[i - 1].offset + entries[i - 1].len;
        put_varint(&mut out, if follows { 0 } else { e.offset + 1 });
    }
    out
}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Cursor<'_> {
    fn varint(&mut self) -> Result<u64, EgmapError> {
        let mut v = 0_u64;
        for shift in (0..64).step_by(7) {
            let Some(&b) = self.bytes.get(self.at) else {
                return Err(malformed("a directory ends inside a number"));
            };
            self.at += 1;
            if shift == 63 && b > 1 {
                return Err(malformed("a directory number does not fit in 64 bits"));
            }
            v |= u64::from(b & 0x7F) << shift;
            if b < 0x80 {
                return Ok(v);
            }
        }
        Err(malformed("a directory number does not fit in 64 bits"))
    }
}

/// Parses an uncompressed directory. Ids must increase, each entry must have bytes,
/// and every byte must be read.
pub(super) fn parse_directory(bytes: &[u8]) -> Result<Vec<Entry>, EgmapError> {
    let mut c = Cursor { bytes, at: 0 };
    let count = c.varint()?;
    // Each entry takes at least one byte in each of its four columns.
    if count > everygraph_core::widen(bytes.len()) / 4 {
        return Err(malformed(format!(
            "a directory of {} bytes cannot hold {count} entries",
            bytes.len()
        )));
    }
    let count = usize::try_from(count).map_err(|_| malformed("too many entries"))?;
    let mut entries: Vec<Entry> = (0..count)
        .map(|_| Entry {
            id: 0,
            offset: 0,
            len: 0,
            run: 0,
        })
        .collect();
    let mut id = 0_u64;
    for (i, e) in entries.iter_mut().enumerate() {
        let delta = c.varint()?;
        if i > 0 && delta == 0 {
            return Err(malformed("directory ids do not increase"));
        }
        id = id
            .checked_add(delta)
            .ok_or_else(|| malformed("a tile id does not fit in 64 bits"))?;
        e.id = id;
    }
    for e in &mut entries {
        e.run = c.varint()?;
    }
    for e in &mut entries {
        e.len = c.varint()?;
        if e.len == 0 {
            return Err(malformed("a directory entry of 0 bytes"));
        }
    }
    for i in 0..count {
        let v = c.varint()?;
        entries[i].offset = match (v, i) {
            (0, 0) => return Err(malformed("the first directory entry has no offset")),
            (0, _) => entries[i - 1]
                .offset
                .checked_add(entries[i - 1].len)
                .ok_or_else(|| malformed("an offset does not fit in 64 bits"))?,
            (v, _) => v - 1,
        };
    }
    if c.at != bytes.len() {
        return Err(malformed("bytes follow the directory"));
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_ids_match_the_pmtiles_examples_and_invert() {
        // From the tests of the PMTiles reference implementations.
        assert_eq!(tile_id(0, 0, 0), 0);
        assert_eq!(tile_id(1, 0, 0), 1);
        assert_eq!(tile_id(1, 0, 1), 2);
        assert_eq!(tile_id(1, 1, 1), 3);
        assert_eq!(tile_id(1, 1, 0), 4);
        assert_eq!(tile_id(2, 0, 0), 5);
        for z in 0..=6 {
            let mut ids: Vec<u64> = (0..1_u32 << z)
                .flat_map(|x| (0..1_u32 << z).map(move |y| (x, y)))
                .map(|(x, y)| {
                    let id = tile_id(z, x, y);
                    assert_eq!(tile_of_id(id), Some((z, x, y)));
                    id
                })
                .collect();
            ids.sort_unstable();
            assert_eq!(
                ids,
                (tiles_above(z)..tiles_above(z + 1)).collect::<Vec<_>>()
            );
        }
        let last = tiles_above(MAX_ZOOM + 1) - 1;
        assert!(tile_of_id(last).is_some());
        assert_eq!(tile_of_id(last + 1), None);
    }

    #[test]
    fn varints_refuse_more_than_64_bits() {
        let mut ten = [0xFF_u8; 10];
        ten[9] = 0x01;
        assert_eq!(Cursor { bytes: &ten, at: 0 }.varint().unwrap(), u64::MAX);
        ten[9] = 0x02;
        assert!(Cursor { bytes: &ten, at: 0 }.varint().is_err());
        assert!(
            Cursor {
                bytes: &[0x80; 11],
                at: 0
            }
            .varint()
            .is_err()
        );
    }
}
