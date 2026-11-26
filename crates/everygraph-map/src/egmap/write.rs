// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The writer of a map file (§ 2.3): tiles in tile-id order, then arrays, then the
//! directories, the metadata and the first 16 KiB.

use everygraph_core::{ByteSink, Memory, SegVec, usize_of, widen};

use super::census::ZoomRule;
use super::pmtiles::{Entry, HEADER_BYTES, Header, Span, encode_directory, tile_id};
use super::sections::{self, Element, Section, Values};
use super::{EgmapError, HEAD_BYTES, HEAD_LEN, MAX_DECODED_BYTES, RegionKind, Tile, gzip, invalid};

/// What the section table says about the whole map (§ 5.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MapInfo {
    /// Nodes of the graph.
    pub nodes: u64,
    /// Regions of the map's region tree, the root included.
    pub regions: u64,
    /// Pairs of the graph, a loop once.
    pub edges: u64,
    /// The `k` of a summary map (§ 7); 0 for a full map.
    pub summary: u32,
}

impl MapInfo {
    /// Why `tile` cannot be a tile of this map (§ 2.4), if it cannot.
    pub(super) fn refuses(&self, tile: &Tile) -> Option<&'static str> {
        let summary = self.summary > 0;
        if summary && !tile.nodes.is_empty() {
            return Some("a summary map holds no nodes");
        }
        if !summary && tile.regions.iter().any(|g| g.kind == RegionKind::Other) {
            return Some("only a summary map has regions of kind other");
        }
        let regions = tile.regions.iter().map(|g| g.id.max(g.partner));
        let nodes = tile.nodes.iter().map(|v| v.id);
        if regions
            .max()
            .is_some_and(|id| u64::from(id) >= self.regions)
            || nodes.max().is_some_and(|id| u64::from(id) >= self.nodes)
        {
            return Some("a tile holds an entity the map does not have");
        }
        None
    }
}

/// The descriptive metadata (§ 2.2), written as the archive's JSON metadata.
#[derive(Clone, Debug, PartialEq)]
pub struct Metadata {
    /// The graph's name.
    pub name: String,
    /// One line on what the map shows.
    pub description: String,
    /// Where the graph comes from: in a summary map, a label only.
    pub attribution: String,
    /// The pixel thresholds of the zoom rule the tiles were built with.
    pub zoom: ZoomRule,
}

/// An array being written.
#[derive(Debug)]
struct Open {
    section: Section,
    buffer: Vec<u8>,
    /// Relative offsets of the chunks written so far, and of the end of the last.
    index: SegVec<u64>,
}

/// Writes a map file into a sink (§ 2.3). Tiles come first, in increasing tile id;
/// then arrays, one at a time; then [`finish`](Self::finish).
#[derive(Debug)]
pub struct MapWriter<K: ByteSink> {
    sink: K,
    memory: Memory,
    info: MapInfo,
    at: u64,
    entries: SegVec<Entry>,
    zooms: Option<(u8, u8)>,
    tile_bytes: u64,
    sections: Vec<Section>,
    open: Option<Open>,
}

/// The largest leaf directory, in entries: at most 15 bytes each, so a leaf always
/// decodes within [`MAX_DECODED_BYTES`].
const MAX_LEAF_ENTRIES: usize = 65_536;

impl<K: ByteSink> MapWriter<K> {
    /// A writer into `sink`, which must be empty, for the map `info` describes. Its
    /// directory and chunk indexes are reserved through `memory`.
    ///
    /// # Errors
    ///
    /// [`EgmapError::Invalid`] if `sink` is not empty.
    pub fn create(sink: K, memory: &Memory, info: MapInfo) -> Result<Self, EgmapError> {
        if !sink.is_empty() {
            return Err(invalid("a map is written into an empty sink"));
        }
        Ok(Self {
            sink,
            memory: memory.clone(),
            info,
            at: HEAD_BYTES,
            entries: SegVec::new(memory, "map directory"),
            zooms: None,
            tile_bytes: 0,
            sections: Vec::new(),
            open: None,
        })
    }

    /// Encodes, compresses and appends `tile`.
    ///
    /// # Errors
    ///
    /// [`EgmapError::Invalid`] if the tile breaks a rule of § 4.3, does not follow the
    /// last tile in tile-id order, holds nodes in a summary map, or comes after an
    /// array; [`EgmapError::Write`] or [`EgmapError::Memory`] from the sink and the
    /// budget.
    pub fn add_tile(&mut self, tile: &Tile) -> Result<(), EgmapError> {
        if !self.sections.is_empty() || self.open.is_some() {
            return Err(invalid("tiles come before the arrays"));
        }
        if let Some(what) = self.info.refuses(tile) {
            return Err(invalid(what));
        }
        let packed = gzip::compress(&tile.encode()?);
        let id = tile_id(tile.z, tile.x, tile.y);
        let last = self
            .entries
            .len()
            .checked_sub(1)
            .and_then(|i| self.entries.get(i));
        if last.is_some_and(|e| e.id >= id) {
            return Err(invalid("tiles must come in increasing tile id"));
        }
        self.entries.try_push(Entry {
            id,
            offset: self.tile_bytes,
            len: widen(packed.len()),
            run: 1,
        })?;
        self.sink.write_at(self.at, &packed)?;
        self.at += widen(packed.len());
        self.tile_bytes += widen(packed.len());
        let (lo, hi) = self.zooms.unwrap_or((tile.z, tile.z));
        self.zooms = Some((lo.min(tile.z), hi.max(tile.z)));
        Ok(())
    }

    /// Starts the array `name` of `element` values, `chunk` values per chunk.
    ///
    /// # Errors
    ///
    /// [`EgmapError::Invalid`] if the name is not valid or taken, if an array is
    /// still open, or if a chunk would decode to more than 1 MiB.
    pub fn begin_array(
        &mut self,
        name: &str,
        element: Element,
        chunk: u32,
    ) -> Result<(), EgmapError> {
        if self.open.is_some() {
            return Err(invalid("an array is still open"));
        }
        if !sections::valid_name(name) || self.sections.iter().any(|s| s.name == name) {
            return Err(invalid(format!("{name:?} is not a free section name")));
        }
        if chunk == 0 || usize_of(chunk).saturating_mul(element.width()) > MAX_DECODED_BYTES {
            return Err(invalid("a chunk holds 1 value to 1 MiB"));
        }
        let mut index = SegVec::new(&self.memory, "map chunk index");
        index.try_push(0)?;
        self.open = Some(Open {
            section: Section {
                name: name.to_owned(),
                element,
                chunk,
                count: 0,
                offset: self.at,
                len: 0,
            },
            buffer: Vec::new(),
            index,
        });
        Ok(())
    }

    /// Appends `values` to the open array.
    ///
    /// # Errors
    ///
    /// [`EgmapError::Invalid`] if no array is open, the values are of another type, or
    /// an `f32` is not finite or is −0.0; [`EgmapError::Write`] or
    /// [`EgmapError::Memory`] from the sink and the budget.
    pub fn write_values(&mut self, values: Values<'_>) -> Result<(), EgmapError> {
        let open = self
            .open
            .as_mut()
            .ok_or_else(|| invalid("no array is open"))?;
        if values.element() != open.section.element {
            return Err(invalid(format!(
                "array {} holds another type",
                open.section.name
            )));
        }
        let width = open.section.element.width();
        let per_chunk = usize_of(open.section.chunk);
        let mut done = 0;
        while done < values.len() {
            let room = per_chunk - open.buffer.len() / width;
            let take = room.min(values.len() - done);
            values.append(done..done + take, &mut open.buffer)?;
            done += take;
            if open.buffer.len() == per_chunk * width {
                Self::flush(&mut self.sink, &mut self.at, open)?;
            }
        }
        open.section.count += widen(values.len());
        Ok(())
    }

    fn flush(sink: &mut K, at: &mut u64, open: &mut Open) -> Result<(), EgmapError> {
        let packed = gzip::compress(&open.buffer);
        sink.write_at(*at, &packed)?;
        *at += widen(packed.len());
        open.index.try_push(*at - open.section.offset)?;
        open.buffer.clear();
        Ok(())
    }

    /// Ends the open array: its last chunk, then its chunk index.
    ///
    /// # Errors
    ///
    /// [`EgmapError::Invalid`] if no array is open; [`EgmapError::Write`] from the sink.
    pub fn end_array(&mut self) -> Result<(), EgmapError> {
        let mut open = self
            .open
            .take()
            .ok_or_else(|| invalid("no array is open"))?;
        if !open.buffer.is_empty() {
            Self::flush(&mut self.sink, &mut self.at, &mut open)?;
        }
        for part in open.index.segments() {
            self.sink.write_at(self.at, &sections::index_bytes(part))?;
            self.at += widen(part.len()) * 8;
        }
        open.section.len = self.at - open.section.offset;
        self.sections.push(open.section);
        Ok(())
    }

    /// Writes the directories, the metadata, the section table and the header, makes
    /// the file durable, and gives the sink back.
    ///
    /// # Errors
    ///
    /// [`EgmapError::Invalid`] if an array is open, the arrays break § 5.3 for this
    /// map, the metadata holds a number that is not finite, or the directory cannot
    /// fit the first 16 KiB; [`EgmapError::Write`] from the sink.
    pub fn finish(mut self, metadata: &Metadata) -> Result<K, EgmapError> {
        if self.open.is_some() {
            return Err(invalid("an array is still open"));
        }
        self.sections.sort_by(|a, b| a.name.cmp(&b.name));
        sections::check_known(&self.info, &self.sections).map_err(|e| match e {
            EgmapError::Malformed(what) => EgmapError::Invalid(what),
            e => e,
        })?;
        let table = sections::encode_table(&self.info, &self.sections);
        let room = HEAD_LEN
            .checked_sub(HEADER_BYTES + table.len())
            .ok_or_else(|| invalid("the section table does not fit the first 16 KiB"))?;
        let leaves_start = self.at;
        let root = self.directories(room)?;
        let leaves = Span {
            offset: leaves_start,
            len: self.at - leaves_start,
        };
        let meta = gzip::compress(metadata_json(metadata)?.as_bytes());
        let metadata_span = Span {
            offset: self.at,
            len: widen(meta.len()),
        };
        self.sink.write_at(self.at, &meta)?;
        self.at += widen(meta.len());
        self.sink.set_len(self.at)?;
        let (min_zoom, max_zoom) = self.zooms.unwrap_or((0, 0));
        let header = Header {
            root: Span {
                offset: widen(HEADER_BYTES),
                len: widen(root.len()),
            },
            metadata: metadata_span,
            leaves,
            tiles: Span {
                offset: HEAD_BYTES,
                len: self.tile_bytes,
            },
            tile_count: widen(self.entries.len()),
            min_zoom,
            max_zoom,
        };
        let mut head = Vec::new();
        head.extend_from_slice(&header.encode());
        head.extend_from_slice(&root);
        head.extend_from_slice(&table);
        self.sink.write_at(0, &head)?;
        self.sink.sync()?;
        Ok(self.sink)
    }

    /// Writes the leaf directories at the current offset, if any are needed, and gives
    /// the compressed root directory, which must fit in `room` bytes.
    fn directories(&mut self, room: usize) -> Result<Vec<u8>, EgmapError> {
        let count = self.entries.len();
        let chunk = |range: std::ops::Range<usize>| -> Vec<Entry> {
            range.filter_map(|i| self.entries.get(i).copied()).collect()
        };
        if count <= MAX_LEAF_ENTRIES {
            let root = gzip::compress(&encode_directory(&chunk(0..count)));
            if root.len() <= room {
                return Ok(root);
            }
        }
        let start = self.at;
        let mut per_leaf = 4096;
        while per_leaf <= MAX_LEAF_ENTRIES {
            let mut at = start;
            let mut roots = Vec::new();
            for first in (0..count).step_by(per_leaf) {
                let entries = chunk(first..count.min(first + per_leaf));
                let leaf = gzip::compress(&encode_directory(&entries));
                roots.push(Entry {
                    id: entries[0].id,
                    offset: at - start,
                    len: widen(leaf.len()),
                    run: 0,
                });
                self.sink.write_at(at, &leaf)?;
                at += widen(leaf.len());
            }
            let root = gzip::compress(&encode_directory(&roots));
            if root.len() <= room {
                self.at = at;
                return Ok(root);
            }
            per_leaf *= 2;
        }
        Err(invalid(format!(
            "{count} tiles do not fit a root directory and one level of leaves"
        )))
    }
}

fn json_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if u32::from(c) < 0x20 => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// The JSON metadata of § 2.2.
fn metadata_json(m: &Metadata) -> Result<String, EgmapError> {
    let mut out = String::from("{\"format\":\"everygraph-egmap\",\"version\":\"1.0\"");
    for (key, value) in [
        ("name", &m.name),
        ("description", &m.description),
        ("attribution", &m.attribution),
    ] {
        out.push_str(&format!(",\"{key}\":"));
        json_string(&mut out, value);
    }
    let z = &m.zoom;
    let numbers = [
        ("region_px", z.region_px),
        ("open_px", z.open_px),
        ("node_px", z.node_px),
        ("label_off_px", z.label_off_px),
    ];
    out.push_str(",\"zoom\":{");
    for (i, (key, v)) in numbers.into_iter().enumerate() {
        if !v.is_finite() {
            return Err(invalid("a zoom threshold is not finite"));
        }
        out.push_str(&format!("{}\"{key}\":{v}", if i > 0 { "," } else { "" }));
    }
    out.push_str("}}");
    Ok(out)
}
