// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The reader of a map file (§ 2.4): the first 16 KiB on open, then one directory and
//! one tile, or one chunk, per request.

use std::ops::Range;

use everygraph_core::{ByteSource, le, widen};

use super::pmtiles::{Entry, Header, Span, parse_directory, tile_id, tile_of_id};
use super::sections::{self, Element, Section};
use super::write::MapInfo;
use super::{EgmapError, HEAD_BYTES, HEAD_LEN, MAX_DECODED_BYTES, Tile, gzip, invalid, malformed};

/// The most compressed bytes that can decode within [`MAX_DECODED_BYTES`]: deflate
/// stores incompressible data with 5 bytes per 65,535, and gzip adds 18.
const MAX_PACKED_BYTES: u64 = widen(MAX_DECODED_BYTES) + 4096;

/// An open map file.
#[derive(Debug)]
pub struct MapReader<S> {
    source: S,
    header: Header,
    root: Vec<Entry>,
    info: MapInfo,
    sections: Vec<Section>,
}

impl<S: ByteSource> MapReader<S> {
    /// Opens the map in `source`: reads its first 16 KiB and checks the header, the
    /// root directory and the section table (§ 2.4).
    ///
    /// # Errors
    ///
    /// [`EgmapError::Foreign`] for a file that is not a map, [`EgmapError::Unsupported`]
    /// for a later version, [`EgmapError::Malformed`] for a broken rule,
    /// [`EgmapError::Read`] if the source fails.
    pub fn open(source: S) -> Result<Self, EgmapError> {
        let file_len = source.len();
        let mut head = Vec::new();
        let head_len = usize::try_from(file_len.min(HEAD_BYTES)).unwrap_or(HEAD_LEN);
        head.extend(std::iter::repeat_n(0, head_len));
        source.read_at(0, &mut head)?;
        let header = Header::parse(&head, file_len)?;
        let root_span = span_range(header.root);
        let root_bytes = head
            .get(root_span.clone())
            .ok_or_else(|| malformed("the root directory lies outside the file"))?;
        let root = parse_directory(&gzip::decompress(
            root_bytes,
            MAX_DECODED_BYTES,
            "the root directory",
        )?)?;
        let mut leaf_bytes = 0_u64;
        for e in &root {
            match e.run {
                0 if e.offset == leaf_bytes => {
                    leaf_bytes = leaf_bytes
                        .checked_add(e.len)
                        .ok_or_else(|| malformed("leaf directories do not fit in 64 bits"))?;
                }
                0 => return Err(malformed("leaf directories share or skip bytes")),
                1 if e
                    .offset
                    .checked_add(e.len)
                    .is_some_and(|end| end <= header.tiles.len) => {}
                1 => {
                    return Err(malformed(
                        "a root directory entry points outside the tile data",
                    ));
                }
                _ => return Err(malformed("a directory entry repeats a tile")),
            }
        }
        if leaf_bytes != header.leaves.len {
            return Err(malformed("the leaves do not fill the leaf directories"));
        }
        let table_at = root_span.end;
        let table_len = sections::table_len(&head[table_at..])?;
        let table = head
            .get(table_at..table_at + table_len)
            .ok_or_else(|| malformed("the section table does not end within the first 16 KiB"))?;
        let (info, sections) = sections::parse_table(table, file_len)?;
        let mut spans = vec![
            Span {
                offset: 0,
                len: widen(table_at + table_len),
            },
            header.metadata,
            header.leaves,
            header.tiles,
        ];
        spans.extend(sections.iter().map(|s| Span {
            offset: s.offset,
            len: s.len,
        }));
        spans.retain(|s| s.len > 0);
        spans.sort_by_key(|s| s.offset);
        if spans.windows(2).any(|w| w[1].offset < w[0].end()) {
            return Err(malformed("two regions of the file overlap"));
        }
        Ok(Self {
            source,
            header,
            root,
            info,
            sections,
        })
    }

    /// What the section table says about the map.
    #[must_use]
    pub fn info(&self) -> MapInfo {
        self.info
    }

    /// The shallowest and deepest zoom with a tile.
    #[must_use]
    pub fn zooms(&self) -> (u8, u8) {
        (self.header.min_zoom, self.header.max_zoom)
    }

    /// The number of tiles.
    #[must_use]
    pub fn tile_count(&self) -> u64 {
        self.header.tile_count
    }

    /// The arrays, by name.
    #[must_use]
    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    /// The array `name`, if the map has it.
    #[must_use]
    pub fn section(&self, name: &str) -> Option<&Section> {
        self.sections.iter().find(|s| s.name == name)
    }

    fn read(&self, span: Span, what: &str) -> Result<Vec<u8>, EgmapError> {
        if span.len > MAX_PACKED_BYTES {
            return Err(malformed(format!("{what} is larger than its cap")));
        }
        let len = usize::try_from(span.len)
            .map_err(|_| malformed(format!("{what} is larger than its cap")))?;
        let mut buf = Vec::new();
        buf.extend(std::iter::repeat_n(0, len));
        self.source.read_at(span.offset, &mut buf)?;
        Ok(buf)
    }

    /// The JSON metadata (§ 2.2), checked to be UTF-8 only.
    ///
    /// # Errors
    ///
    /// [`EgmapError::Malformed`] if it is not gzip within the cap or not UTF-8;
    /// [`EgmapError::Read`] if the source fails.
    pub fn metadata(&self) -> Result<String, EgmapError> {
        let packed = self.read(self.header.metadata, "the metadata")?;
        let text = gzip::decompress(&packed, MAX_DECODED_BYTES, "the metadata")?;
        String::from_utf8(text).map_err(|_| malformed("the metadata is not UTF-8"))
    }

    /// The entries of the leaf directory `leaf` points to, which must hold the tile ids
    /// `ids`, contiguous tile bytes and no further leaves.
    fn leaf(&self, leaf: &Entry, ids: Range<u64>) -> Result<Vec<Entry>, EgmapError> {
        let span = Span {
            offset: self
                .header
                .leaves
                .offset
                .checked_add(leaf.offset)
                .ok_or_else(|| malformed("a leaf directory lies past 2^64"))?,
            len: leaf.len,
        };
        let packed = self.read(span, "a leaf directory")?;
        let entries = parse_directory(&gzip::decompress(
            &packed,
            MAX_DECODED_BYTES,
            "a leaf directory",
        )?)?;
        if entries.first().is_none_or(|e| e.id != leaf.id) {
            return Err(malformed(
                "a leaf directory does not start at its root entry",
            ));
        }
        for (i, e) in entries.iter().enumerate() {
            if e.run != 1 || !ids.contains(&e.id) {
                return Err(malformed(
                    "a leaf directory holds a leaf, a run or a foreign id",
                ));
            }
            if e.offset
                .checked_add(e.len)
                .is_none_or(|end| end > self.header.tiles.len)
            {
                return Err(malformed("a tile lies outside the tile data"));
            }
            if i > 0 && e.offset != entries[i - 1].offset + entries[i - 1].len {
                return Err(malformed("tiles share or skip bytes"));
            }
        }
        Ok(entries)
    }

    /// The ids root entry `i` covers.
    fn ids_of(&self, i: usize) -> Range<u64> {
        self.root[i].id..self.root.get(i + 1).map_or(u64::MAX, |e| e.id)
    }

    fn decode(&self, e: &Entry) -> Result<Tile, EgmapError> {
        let span = Span {
            offset: self
                .header
                .tiles
                .offset
                .checked_add(e.offset)
                .ok_or_else(|| malformed("a tile lies past 2^64"))?,
            len: e.len,
        };
        let packed = self.read(span, "a tile")?;
        let tile = Tile::decode(&gzip::decompress(&packed, MAX_DECODED_BYTES, "a tile")?)?;
        if tile_of_id(e.id) != Some((tile.z, tile.x, tile.y)) {
            return Err(malformed(format!(
                "tile {} is not stored under its own id",
                e.id
            )));
        }
        if tile.z < self.header.min_zoom || tile.z > self.header.max_zoom {
            return Err(malformed("a tile outside the header's zooms"));
        }
        if tile
            .labels
            .iter()
            .any(|a| a.z_off > self.header.max_zoom + 1)
        {
            return Err(malformed("a label is drawn past the deepest zoom"));
        }
        if let Some(what) = self.info.refuses(&tile) {
            return Err(malformed(what));
        }
        Ok(tile)
    }

    /// The tile `(z, x, y)`, or `None` if the map has none there.
    ///
    /// # Errors
    ///
    /// As [`open`](Self::open), for the directory and the tile it reads.
    pub fn tile(&self, z: u8, x: u32, y: u32) -> Result<Option<Tile>, EgmapError> {
        if z > self.header.max_zoom || x >= 1 << z || y >= 1 << z {
            return Ok(None);
        }
        let id = tile_id(z, x, y);
        let Some(i) = self.root.partition_point(|e| e.id <= id).checked_sub(1) else {
            return Ok(None);
        };
        let e = self.root[i];
        let found = if e.run == 0 {
            let leaf = self.leaf(&e, self.ids_of(i))?;
            leaf.binary_search_by_key(&id, |e| e.id)
                .ok()
                .map(|j| leaf[j])
        } else {
            (e.id == id).then_some(e)
        };
        found.map(|e| self.decode(&e)).transpose()
    }

    /// Calls `f` with every tile in tile-id order, checking that the tiles fill the
    /// tile data exactly, each byte once.
    ///
    /// # Errors
    ///
    /// As [`open`](Self::open), and the first error `f` returns.
    pub fn for_each_tile(
        &self,
        mut f: impl FnMut(Tile) -> Result<(), EgmapError>,
    ) -> Result<(), EgmapError> {
        let mut next = 0_u64;
        let mut count = 0_u64;
        let mut zooms: Option<(u8, u8)> = None;
        let mut visit = |e: &Entry| -> Result<(), EgmapError> {
            if e.offset != next {
                return Err(malformed("tiles share or skip bytes"));
            }
            // In the tile data, which `open` and `leaf` checked.
            next += e.len;
            count += 1;
            let tile = self.decode(e)?;
            let (lo, hi) = zooms.unwrap_or((tile.z, tile.z));
            zooms = Some((lo.min(tile.z), hi.max(tile.z)));
            f(tile)
        };
        for (i, e) in self.root.iter().enumerate() {
            if e.run == 0 {
                for t in &self.leaf(e, self.ids_of(i))? {
                    visit(t)?;
                }
            } else {
                visit(e)?;
            }
        }
        if next != self.header.tiles.len || count != self.header.tile_count {
            return Err(malformed("the directory does not account for every tile"));
        }
        if zooms.unwrap_or((0, 0)) != (self.header.min_zoom, self.header.max_zoom) {
            return Err(malformed("the header's zooms are not those of the tiles"));
        }
        Ok(())
    }

    /// The little-endian bytes of chunk `i` of `section`: exactly its values.
    ///
    /// # Errors
    ///
    /// [`EgmapError::Invalid`] if there is no chunk `i`; [`EgmapError::Malformed`] if
    /// the index or the chunk breaks § 5.2; [`EgmapError::Read`] if the source fails.
    pub fn chunk(&self, section: &Section, i: u64) -> Result<Vec<u8>, EgmapError> {
        if i >= section.chunks() {
            return Err(invalid(format!(
                "section {} has no chunk {i}",
                section.name
            )));
        }
        let index = section.index_start().unwrap_or(0);
        let mut pair = [0_u8; 16];
        self.source
            .read_at(section.offset + index + 8 * i, &mut pair)?;
        let span = sections::chunk_span(section, le::u64_at(&pair, 0), le::u64_at(&pair, 8))?;
        if i == 0 && span.start != 0 || i + 1 == section.chunks() && span.end != index {
            return Err(malformed(format!(
                "section {}: chunks do not fill it",
                section.name
            )));
        }
        let packed = self.read(
            Span {
                offset: section.offset + span.start,
                len: span.end - span.start,
            },
            "a chunk",
        )?;
        let want = sections::chunk_bytes(section, i);
        let bytes = gzip::decompress(&packed, want, "a chunk")?;
        if bytes.len() != want {
            return Err(malformed(format!(
                "section {}: chunk {i} is short",
                section.name
            )));
        }
        if section.element == Element::F32
            && bytes.chunks_exact(4).any(|c| {
                let v = f32::from_le_bytes([c[0], c[1], c[2], c[3]]);
                !v.is_finite() || v.to_bits() == (-0.0_f32).to_bits()
            })
        {
            return Err(malformed(format!(
                "section {}: not a canonical f32",
                section.name
            )));
        }
        Ok(bytes)
    }
}

fn span_range(s: Span) -> Range<usize> {
    let start = usize::try_from(s.offset).unwrap_or(usize::MAX);
    let end = usize::try_from(s.end()).unwrap_or(usize::MAX);
    start.min(end)..end
}

/// The values of an array, read through a cache of one chunk.
pub(super) struct ArrayReader<'r, S> {
    reader: &'r MapReader<S>,
    section: &'r Section,
    chunk: Option<(u64, Vec<u8>)>,
}

impl<'r, S: ByteSource> ArrayReader<'r, S> {
    pub(super) fn new(reader: &'r MapReader<S>, section: &'r Section) -> Self {
        Self {
            reader,
            section,
            chunk: None,
        }
    }

    /// Value `i`, widened to a `u64` (an f32 as its bits).
    pub(super) fn get(&mut self, i: u64) -> Result<u64, EgmapError> {
        let s = self.section;
        if i >= s.count {
            return Err(malformed(format!("section {} has no value {i}", s.name)));
        }
        let c = i / u64::from(s.chunk);
        if self.chunk.as_ref().is_none_or(|(k, _)| *k != c) {
            self.chunk = Some((c, self.reader.chunk(s, c)?));
        }
        let w = s.element.width();
        let at = usize::try_from(i - c * u64::from(s.chunk)).unwrap_or(usize::MAX) * w;
        let bytes = self.chunk.as_ref().map_or(&[][..], |(_, b)| &b[at..at + w]);
        let mut word = [0_u8; 8];
        word[..w].copy_from_slice(bytes);
        Ok(u64::from_le_bytes(word))
    }
}
