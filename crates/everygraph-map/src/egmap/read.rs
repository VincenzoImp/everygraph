// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The reader of a map file (§ 2.4): the first 16 KiB on open, then one directory and
//! one tile, or one chunk, per request.

use std::ops::Range;

use everygraph_core::{ByteSource, Memory, le, widen};

use super::census::{Census, CensusReport};
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
                0 if e.offset == leaf_bytes => leaf_bytes += e.len,
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
            offset: self.header.leaves.offset + leaf.offset,
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
            offset: self.header.tiles.offset + e.offset,
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
        let mut visit = |e: &Entry| -> Result<(), EgmapError> {
            if e.offset != next {
                return Err(malformed("tiles share or skip bytes"));
            }
            next += e.len;
            count += 1;
            f(self.decode(e)?)
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

    /// Reads all of the file: every tile, every chunk, and the rules of § 5.3 on the
    /// arrays; and gives the census of the whole pyramid (§ 6), whose faults are not
    /// errors of the format.
    ///
    /// # Errors
    ///
    /// The first error of [`for_each_tile`](Self::for_each_tile) or
    /// [`chunk`](Self::chunk), [`EgmapError::Malformed`] for a broken rule of § 5.3,
    /// [`EgmapError::Memory`] if the census's memory is refused.
    pub fn verify(&self, memory: &Memory) -> Result<CensusReport, EgmapError> {
        self.metadata()?;
        let mut census = Census::new(memory);
        self.for_each_tile(|t| census.add(&t))?;
        for s in &self.sections {
            for i in 0..s.chunks() {
                self.chunk(s, i)?;
            }
        }
        if let Some(s) = self.section("positions") {
            let mut c = Values::new(self, s);
            for _ in 0..s.count {
                let v = f32::from_bits(c.next_u32()?);
                if !(0.0..=1.0).contains(&v) {
                    return Err(malformed("a position outside the unit square"));
                }
            }
        }
        // Symmetry, in one pass: the sum of h(u)·g(v) over the entries equals that of
        // h(v)·g(u) when every (u, v) has its (v, u). A list that is not symmetric passes
        // with a probability of about 2^-64 for one pair of mixes.
        let sums = std::cell::Cell::new((0_u64, 0_u64));
        self.check_lists("adjacency", self.info.nodes, |u, prev, v| {
            let v = u64::from(v);
            let (a, b) = sums.get();
            sums.set((
                a.wrapping_add(mix(u, 1).wrapping_mul(mix(v, 2))),
                b.wrapping_add(mix(v, 1).wrapping_mul(mix(u, 2))),
            ));
            v < self.info.nodes && v != u && prev.is_none_or(|p| u64::from(p) < v)
        })?;
        if sums.get().0 != sums.get().1 {
            return Err(malformed("adjacency.targets is not symmetric"));
        }
        self.check_lists("matrix", self.info.regions, |_, prev, p| {
            let (i, j) = (p >> 16, p & 0xFFFF);
            i >= 1 && i < j && prev.is_none_or(|q| q < p)
        })?;
        if let Some(w) = self.section("matrix.weights") {
            let mut c = Values::new(self, w);
            for _ in 0..w.count {
                if c.next_u64()? == 0 {
                    return Err(malformed("a matrix pair of weight 0"));
                }
            }
        }
        Ok(census.finish(self.header.max_zoom))
    }

    /// Checks `<group>.offsets` (`rows + 1` values from 0, not decreasing, ending at the
    /// length of the values) and each row's values against `ok(row, previous, value)`.
    fn check_lists(
        &self,
        group: &str,
        rows: u64,
        ok: impl Fn(u64, Option<u32>, u32) -> bool,
    ) -> Result<(), EgmapError> {
        let (Some(offsets), Some(values)) = (
            self.section(&format!("{group}.offsets")),
            self.section(&format!(
                "{group}.{}",
                if group == "matrix" {
                    "pairs"
                } else {
                    "targets"
                }
            )),
        ) else {
            return Ok(());
        };
        let mut o = Values::new(self, offsets);
        let mut v = Values::new(self, values);
        let mut start = o.next_u64()?;
        if start != 0 {
            return Err(malformed(format!("{group}.offsets does not start at 0")));
        }
        for row in 0..rows {
            let end = o.next_u64()?;
            if end < start || end > values.count {
                return Err(malformed(format!("{group}.offsets decreases or overruns")));
            }
            let mut prev = None;
            for _ in start..end {
                let x = v.next_u32()?;
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

fn span_range(s: Span) -> Range<usize> {
    let start = usize::try_from(s.offset).unwrap_or(usize::MAX);
    let end = usize::try_from(s.end()).unwrap_or(usize::MAX);
    start.min(end)..end
}

/// The values of an array, read in order one chunk at a time.
struct Values<'r, S> {
    reader: &'r MapReader<S>,
    section: &'r Section,
    chunk: u64,
    bytes: Vec<u8>,
    at: usize,
}

impl<'r, S: ByteSource> Values<'r, S> {
    fn new(reader: &'r MapReader<S>, section: &'r Section) -> Self {
        Self {
            reader,
            section,
            chunk: 0,
            bytes: Vec::new(),
            at: 0,
        }
    }

    fn next_bytes(&mut self, width: usize) -> Result<&[u8], EgmapError> {
        if self.at == self.bytes.len() {
            self.bytes = self.reader.chunk(self.section, self.chunk)?;
            self.chunk += 1;
            self.at = 0;
        }
        let v = &self.bytes[self.at..self.at + width];
        self.at += width;
        Ok(v)
    }

    fn next_u32(&mut self) -> Result<u32, EgmapError> {
        Ok(le::u32_at(self.next_bytes(4)?, 0))
    }

    fn next_u64(&mut self) -> Result<u64, EgmapError> {
        Ok(le::u64_at(self.next_bytes(8)?, 0))
    }
}
