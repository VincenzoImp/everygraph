// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The finished dictionary: its sections, look-ups in both directions, opening it again
//! from its sections, and renumbering.

use std::sync::OnceLock;

use everygraph_core::{ByteSource, MemoryAccountant, NodeId, ReadError};

use super::build::for_each_record_chunk;
use super::dense::RankBitmap;
use super::hash::{KeyHash, index};
use super::kind::{
    ColumnType, Key, KeyKind, MAX_FIXED_WIDTH, MAX_KEY_BYTES, dense_base, dense_index,
};
use super::mphf::{Mphf, Part, PartRow, part_shape};
use super::prefix::PrefixIndex;
use super::scratch::{Reservation, Scatter, Scratch, Spill, WRITE_BUFFER, read_range, widen};
use super::{DictionaryError, IdOrder, Layout};

/// How the keys are found: through the hash function, or (dense integers in sorted
/// order) through the rank bitmap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Repr {
    Hashed,
    Dense,
}

/// The `ids.header` section (64 bytes, little-endian).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Header {
    pub(crate) kind: KeyKind,
    pub(crate) column: ColumnType,
    pub(crate) order: IdOrder,
    pub(crate) layout: Layout,
    pub(crate) repr: Repr,
    pub(crate) renumbered: bool,
    pub(crate) n: u64,
    pub(crate) hash_seed: u64,
    pub(crate) part_bits: u32,
    pub(crate) parts: u32,
}

const MAGIC: &[u8; 8] = b"EGIDDICT";
const VERSION: u32 = 1;
const HEADER_BYTES: usize = 64;

impl Header {
    fn encode(&self) -> [u8; HEADER_BYTES] {
        let mut h = [0_u8; HEADER_BYTES];
        h[0..8].copy_from_slice(MAGIC);
        h[8..12].copy_from_slice(&VERSION.to_le_bytes());
        h[12..16].copy_from_slice(&self.kind.encode());
        h[16..18].copy_from_slice(&self.column.encode());
        h[18] = match self.order {
            IdOrder::SortedKey => 2,
            _ => 1,
        };
        h[19] = match self.layout {
            Layout::Split => 1,
            Layout::SlotOrdered => 2,
        };
        h[20] = match self.repr {
            Repr::Hashed => 1,
            Repr::Dense => 2,
        };
        h[21] = u8::from(self.renumbered);
        h[24..32].copy_from_slice(&self.n.to_le_bytes());
        h[32..40].copy_from_slice(&self.hash_seed.to_le_bytes());
        h[40..44].copy_from_slice(&self.part_bits.to_le_bytes());
        h[44..48].copy_from_slice(&self.parts.to_le_bytes());
        h
    }

    fn decode(h: &[u8]) -> Result<Self, DictionaryError> {
        let bad = DictionaryError::Corrupt;
        if h.len() != HEADER_BYTES || &h[0..8] != MAGIC {
            return Err(bad("not an identifier dictionary header"));
        }
        let u32_at = |i: usize| u32::from_le_bytes(h[i..i + 4].try_into().expect("four bytes"));
        let u64_at = |i: usize| u64::from_le_bytes(h[i..i + 8].try_into().expect("eight bytes"));
        if u32_at(8) != VERSION {
            return Err(bad("unknown identifier dictionary version"));
        }
        if h[22..24].iter().chain(&h[48..64]).any(|&b| b != 0) || h[21] > 1 {
            return Err(bad("reserved header bytes are not zero"));
        }
        let header = Self {
            kind: KeyKind::decode(h[12..16].try_into().expect("four bytes"))
                .ok_or(bad("unknown key kind"))?,
            column: ColumnType::decode([h[16], h[17]]).ok_or(bad("unknown column type"))?,
            order: match h[18] {
                1 => IdOrder::FirstAppearance,
                2 => IdOrder::SortedKey,
                _ => return Err(bad("unknown id order")),
            },
            layout: match h[19] {
                1 => Layout::Split,
                2 => Layout::SlotOrdered,
                _ => return Err(bad("unknown layout")),
            },
            repr: match h[20] {
                1 => Repr::Hashed,
                2 => Repr::Dense,
                _ => return Err(bad("unknown representation")),
            },
            renumbered: h[21] == 1,
            n: u64_at(24),
            hash_seed: u64_at(32),
            part_bits: u32_at(40),
            parts: u32_at(44),
        };
        super::check_node_count(header.n)?;
        let parts_ok = match header.repr {
            Repr::Hashed => {
                header.part_bits <= 16 && u64::from(header.parts) == 1 << header.part_bits
            }
            Repr::Dense => header.part_bits == 0 && header.parts == 0,
        };
        if !parts_ok {
            return Err(bad("part count does not match part bits"));
        }
        Ok(header)
    }
}

/// The sections of a dictionary, by name. Which are present depends on the layout,
/// the representation and whether the dictionary was renumbered (module
/// documentation, "Sections").
pub const SECTION_NAMES: [&str; 14] = [
    "ids.header",
    "ids.parts",
    "ids.pilots",
    "ids.remap",
    "ids.dense",
    "ids.slot_to_id",
    "ids.keys",
    "ids.key_offsets",
    "ids.records",
    "ids.record_offsets",
    "ids.id_to_slot",
    "ids.order",
    "ids.rank_to_id",
    "ids.id_to_rank",
];

#[derive(Clone, Copy)]
enum Sec {
    Header,
    Parts,
    Pilots,
    Remap,
    Dense,
    SlotToId,
    Keys,
    KeyOffsets,
    Records,
    RecordOffsets,
    IdToSlot,
    Order,
    RankToId,
    IdToRank,
}

/// Writers of the sections a build produces.
pub(crate) struct SectionWriters<'a, W: std::io::Write> {
    pub(crate) keys: Option<Spill<W>>,
    pub(crate) key_offsets: Option<Spill<W>>,
    pub(crate) id_to_slot: Option<Spill<W>>,
    slot_main: Option<Spill<W>>,
    slot_offsets: Option<Spill<W>>,
    layout: Layout,
    /// The write buffers of the writers above and of the ones `finish` makes.
    _buffers: Option<Reservation<'a>>,
}

impl<'a, W: std::io::Write + Send> SectionWriters<'a, W> {
    pub(crate) fn new<Sc: Scratch<Writer = W>>(
        scratch: &Sc,
        accountant: &'a dyn MemoryAccountant,
        layout: Layout,
        width: Option<usize>,
    ) -> Result<Self, DictionaryError> {
        let buffers = Reservation::new(
            accountant,
            "id dictionary write buffers",
            widen(4 * WRITE_BUFFER),
        )?;
        let spill = || Spill::new(scratch, "id dictionary section");
        let var = width.is_none();
        let mut w = Self {
            keys: None,
            key_offsets: None,
            id_to_slot: None,
            slot_main: Some(spill()?),
            slot_offsets: None,
            layout,
            _buffers: Some(buffers),
        };
        match layout {
            Layout::Split => {
                w.keys = Some(spill()?);
                if var {
                    let mut o = spill()?;
                    o.write(&0_u64.to_le_bytes())?;
                    w.key_offsets = Some(o);
                }
            }
            Layout::SlotOrdered => {
                w.id_to_slot = Some(spill()?);
                if var {
                    w.slot_offsets = Some(spill()?);
                }
            }
        }
        Ok(w)
    }

    pub(crate) fn new_dense() -> Self {
        Self {
            keys: None,
            key_offsets: None,
            id_to_slot: None,
            slot_main: None,
            slot_offsets: None,
            layout: Layout::Split,
            _buffers: None,
        }
    }

    /// The writers phase D fills: slot to id (split layout) or the records and their
    /// offsets (slot-ordered layout).
    pub(crate) fn slot_ordered(&mut self) -> (&mut Spill<W>, Option<&mut Spill<W>>) {
        (
            self.slot_main.as_mut().expect("hashed layout"),
            self.slot_offsets.as_mut(),
        )
    }

    /// Writes the remaining sections and opens the dictionary.
    pub(crate) fn finish<Sc: Scratch<Writer = W>>(
        self,
        scratch: &Sc,
        header: Header,
        mphf: Option<Mphf>,
        dense: Option<RankBitmap>,
        accountant: &'a dyn MemoryAccountant,
        resident: Vec<Reservation<'a>>,
    ) -> Result<Dictionary<'a, Sc::Source>, DictionaryError> {
        // One more write buffer at a time below (each section is sealed before the
        // next is made).
        let _buffer = Reservation::new(
            accountant,
            "id dictionary write buffers",
            widen(WRITE_BUFFER),
        )?;
        let mut sections: [Option<Sc::Source>; 14] = Default::default();
        let mut put = |sec: Sec, spill: Option<Spill<W>>| -> Result<(), DictionaryError> {
            if let Some(s) = spill {
                sections[sec as usize] = Some(s.seal(scratch)?);
            }
            Ok(())
        };
        let bytes_section = |bytes: &[u8]| -> Result<Spill<W>, DictionaryError> {
            let mut s = Spill::new(scratch, "id dictionary section")?;
            s.write(bytes)?;
            Ok(s)
        };
        put(Sec::Header, Some(bytes_section(&header.encode())?))?;
        if let Some(m) = &mphf {
            let mut rows = Spill::new(scratch, "id dictionary section")?;
            let mut row = Vec::with_capacity(PartRow::BYTES);
            for r in m.rows() {
                row.clear();
                r.encode(&mut row);
                rows.write(&row)?;
            }
            put(Sec::Parts, Some(rows))?;
            let mut pilots = Spill::new(scratch, "id dictionary section")?;
            for p in &m.parts {
                pilots.write(&p.pilots)?;
            }
            put(Sec::Pilots, Some(pilots))?;
            let mut remap = Spill::new(scratch, "id dictionary section")?;
            for p in &m.parts {
                for chunk in p.remap.chunks(1 << 12) {
                    let bytes: Vec<u8> = chunk.iter().flat_map(|r| r.to_le_bytes()).collect();
                    remap.write(&bytes)?;
                }
            }
            put(Sec::Remap, Some(remap))?;
        }
        if let Some(d) = &dense {
            let mut words = Spill::new(scratch, "id dictionary section")?;
            for chunk in d.words().chunks(1 << 13) {
                let bytes: Vec<u8> = chunk.iter().flat_map(|w| w.to_le_bytes()).collect();
                words.write(&bytes)?;
            }
            put(Sec::Dense, Some(words))?;
        }
        let (slot_sec, slot_off_sec) = match self.layout {
            Layout::Split => (Sec::SlotToId, Sec::RecordOffsets),
            Layout::SlotOrdered => (Sec::Records, Sec::RecordOffsets),
        };
        put(slot_sec, self.slot_main)?;
        put(slot_off_sec, self.slot_offsets)?;
        put(Sec::Keys, self.keys)?;
        put(Sec::KeyOffsets, self.key_offsets)?;
        put(Sec::IdToSlot, self.id_to_slot)?;
        Dictionary::assemble(
            header,
            mphf.unwrap_or_default(),
            dense,
            sections,
            accountant,
            resident,
        )
    }
}

/// An identifier dictionary: keys of one column and their dense node ids, in both
/// directions.
///
/// `S` is where the large arrays are read from (the sections of an `.eg` file, or a
/// [`Scratch`]); the hash function, or the rank bitmap, is held in memory and reserved
/// against the accountant for as long as the dictionary lives.
pub struct Dictionary<'a, S> {
    header: Header,
    mphf: Mphf,
    dense: Option<RankBitmap>,
    sections: [Option<S>; 14],
    accountant: &'a dyn MemoryAccountant,
    _resident: Vec<Reservation<'a>>,
    prefix: OnceLock<PrefixIndex<'a>>,
}

impl<S> std::fmt::Debug for Dictionary<'_, S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Dictionary")
            .field("len", &self.header.n)
            .field("kind", &self.header.kind)
            .field("order", &self.header.order)
            .field("layout", &self.header.layout)
            .finish_non_exhaustive()
    }
}

/// A buffer for one key read back.
type KeyBuf = Vec<u8>;

impl<'a, S: ByteSource> Dictionary<'a, S> {
    fn assemble(
        header: Header,
        mphf: Mphf,
        dense: Option<RankBitmap>,
        sections: [Option<S>; 14],
        accountant: &'a dyn MemoryAccountant,
        resident: Vec<Reservation<'a>>,
    ) -> Result<Self, DictionaryError> {
        let d = Self {
            header,
            mphf,
            dense,
            sections,
            accountant,
            _resident: resident,
            prefix: OnceLock::new(),
        };
        d.validate()?;
        Ok(d)
    }

    /// Opens a dictionary from its sections, as listed by [`Dictionary::sections`].
    /// `section` returns the source of a section by name, or `None` if it is absent.
    /// The hash function (or the rank bitmap) is read into memory and reserved against
    /// `accountant`; the other sections are read on demand.
    ///
    /// # Errors
    ///
    /// [`DictionaryError::Corrupt`] if a section is missing, has the wrong size or an
    /// inconsistent content; budget and read errors.
    pub fn open(
        accountant: &'a dyn MemoryAccountant,
        mut section: impl FnMut(&str) -> Option<S>,
    ) -> Result<Self, DictionaryError> {
        Self::open_sections(accountant, &mut section)
    }

    fn open_sections(
        accountant: &'a dyn MemoryAccountant,
        section: &mut dyn FnMut(&str) -> Option<S>,
    ) -> Result<Self, DictionaryError> {
        let mut sections: [Option<S>; 14] = Default::default();
        for (slot, name) in sections.iter_mut().zip(SECTION_NAMES) {
            *slot = section(name);
        }
        let header_source = sections[Sec::Header as usize]
            .as_ref()
            .ok_or(DictionaryError::Corrupt("no ids.header section"))?;
        if header_source.len() != widen(HEADER_BYTES) {
            return Err(DictionaryError::Corrupt("ids.header is not 64 bytes"));
        }
        let header = Header::decode(&read_range(header_source, 0, widen(HEADER_BYTES))?)?;
        let len_of = |sec: Sec| sections[sec as usize].as_ref().map_or(0, ByteSource::len);
        let (mphf, dense, resident) = match header.repr {
            Repr::Hashed => {
                // The part table first (at most 2^16 rows): it says how long every
                // other array of the hash function must be, before any is read.
                let parts = sections[Sec::Parts as usize]
                    .as_ref()
                    .ok_or(DictionaryError::Corrupt("no ids.parts section"))?;
                let rows_len = widen(index(u64::from(header.parts)) * PartRow::BYTES);
                if parts.len() != rows_len {
                    return Err(DictionaryError::Corrupt("ids.parts has the wrong size"));
                }
                let rows: Vec<PartRow> = read_range(parts, 0, rows_len)?
                    .chunks_exact(PartRow::BYTES)
                    .map(PartRow::decode)
                    .collect();
                let (mut slot_base, mut buckets, mut remap) = (0_u64, 0_u64, 0_u64);
                for r in &rows {
                    let (slots, nb) = part_shape(r.n);
                    let ok = u64::from(r.slot_base) == slot_base
                        && r.slots == slots
                        && r.buckets == nb
                        && u64::from(r.bucket_base) == buckets
                        && u64::from(r.remap_base) == remap;
                    if !ok {
                        return Err(DictionaryError::Corrupt("ids.parts is inconsistent"));
                    }
                    slot_base += u64::from(r.n);
                    buckets += u64::from(r.buckets);
                    remap += u64::from(r.slots - r.n);
                }
                if slot_base != header.n
                    || len_of(Sec::Pilots) != buckets
                    || len_of(Sec::Remap) != 4 * remap
                {
                    return Err(DictionaryError::Corrupt(
                        "the hash function does not cover n keys",
                    ));
                }
                let bytes: u64 =
                    rows.iter().map(|r| Part::bytes_for(r.n)).sum::<u64>() + widen(rows.len()) * 4;
                let resident = Reservation::new(accountant, "id dictionary hash function", bytes)?;
                let pilots = sections[Sec::Pilots as usize]
                    .as_ref()
                    .ok_or(DictionaryError::Corrupt("no ids.pilots section"))?;
                let remaps = sections[Sec::Remap as usize]
                    .as_ref()
                    .ok_or(DictionaryError::Corrupt("no ids.remap section"))?;
                let mut mphf = Mphf {
                    part_bits: header.part_bits,
                    ..Mphf::default()
                };
                for r in &rows {
                    let remap_len = u64::from(r.slots - r.n);
                    let remap = read_range(remaps, 4 * u64::from(r.remap_base), 4 * remap_len)?
                        .chunks_exact(4)
                        .map(|c| u32::from_le_bytes(c.try_into().expect("four bytes")))
                        .collect();
                    mphf.push(Part {
                        n: r.n,
                        slots: r.slots,
                        buckets: r.buckets,
                        seed: r.seed,
                        pilots: read_range(pilots, u64::from(r.bucket_base), u64::from(r.buckets))?,
                        remap,
                    });
                }
                (mphf, None, resident)
            }
            Repr::Dense => {
                let source = sections[Sec::Dense as usize]
                    .as_ref()
                    .ok_or(DictionaryError::Corrupt("no ids.dense section"))?;
                let len = source.len();
                // At most 2^32 bits.
                if len % 8 != 0 || len > 1 << 29 {
                    return Err(DictionaryError::Corrupt("ids.dense has the wrong size"));
                }
                let words_len = index(len / 8);
                let resident = Reservation::new(
                    accountant,
                    "id dictionary dense bitmap",
                    RankBitmap::bytes(words_len),
                )?;
                let mut words = Vec::with_capacity(words_len);
                for_each_record_chunk(source, 8, |_, bytes| {
                    words.extend(
                        bytes
                            .chunks_exact(8)
                            .map(|c| u64::from_le_bytes(c.try_into().expect("eight bytes"))),
                    );
                    Ok(())
                })?;
                (Mphf::default(), Some(RankBitmap::new(words)?), resident)
            }
        };
        Self::assemble(header, mphf, dense, sections, accountant, vec![resident])
    }

    fn sec(&self, sec: Sec) -> Option<&S> {
        self.sections[sec as usize].as_ref()
    }

    fn need(&self, sec: Sec) -> Result<&S, DictionaryError> {
        self.sec(sec).ok_or(DictionaryError::Corrupt(
            "a section the header requires is missing",
        ))
    }

    /// Checks that every section is present with the size the header implies, and
    /// that the hash function's tables are consistent.
    fn validate(&self) -> Result<(), DictionaryError> {
        let h = &self.header;
        let n = h.n;
        let width = h.kind.width().map(widen);
        let size = |sec: Sec, expected: u64| -> Result<(), DictionaryError> {
            if self.need(sec)?.len() == expected {
                Ok(())
            } else {
                Err(DictionaryError::Corrupt("a section has the wrong size"))
            }
        };
        let absent = |secs: &[Sec]| -> Result<(), DictionaryError> {
            if secs.iter().any(|&s| self.sec(s).is_some()) {
                Err(DictionaryError::Corrupt(
                    "a section that does not belong to this layout is present",
                ))
            } else {
                Ok(())
            }
        };
        match h.repr {
            Repr::Dense => {
                let d = self
                    .dense
                    .as_ref()
                    .ok_or(DictionaryError::Corrupt("no dense bitmap"))?;
                if u64::from(d.len()) != n {
                    return Err(DictionaryError::Corrupt("ids.dense does not hold n keys"));
                }
                absent(&[
                    Sec::Parts,
                    Sec::Pilots,
                    Sec::Remap,
                    Sec::SlotToId,
                    Sec::Keys,
                    Sec::KeyOffsets,
                    Sec::Records,
                    Sec::RecordOffsets,
                    Sec::IdToSlot,
                    Sec::Order,
                ])?;
                if h.renumbered {
                    size(Sec::RankToId, 4 * n)?;
                    size(Sec::IdToRank, 4 * n)?;
                } else {
                    absent(&[Sec::RankToId, Sec::IdToRank])?;
                }
            }
            Repr::Hashed => {
                let m = &self.mphf;
                if m.parts.len() != index(u64::from(h.parts)) {
                    return Err(DictionaryError::Corrupt("ids.parts has the wrong size"));
                }
                let mut slot_base = 0_u64;
                for (p, &base) in m.parts.iter().zip(&m.slot_base) {
                    let (slots, nb) = part_shape(p.n);
                    let ok = u64::from(base) == slot_base
                        && p.slots == slots
                        && p.buckets == nb
                        && p.pilots.len() == index(u64::from(nb))
                        && p.remap.len() == index(u64::from(slots - p.n));
                    if !ok {
                        return Err(DictionaryError::Corrupt("ids.parts is inconsistent"));
                    }
                    // Every remapped slot is a slot of the part.
                    if p.remap.iter().any(|&x| x >= p.n.max(1)) {
                        return Err(DictionaryError::Corrupt(
                            "ids.remap points outside its part",
                        ));
                    }
                    slot_base += u64::from(p.n);
                }
                if slot_base != n {
                    return Err(DictionaryError::Corrupt(
                        "the hash function does not cover n keys",
                    ));
                }
                absent(&[Sec::Dense, Sec::RankToId, Sec::IdToRank])?;
                match (h.layout, width) {
                    (Layout::Split, Some(w)) => {
                        size(Sec::SlotToId, 4 * n)?;
                        size(Sec::Keys, w * n)?;
                        absent(&[
                            Sec::KeyOffsets,
                            Sec::Records,
                            Sec::RecordOffsets,
                            Sec::IdToSlot,
                        ])?;
                    }
                    (Layout::Split, None) => {
                        size(Sec::SlotToId, 4 * n)?;
                        size(Sec::KeyOffsets, 8 * (n + 1))?;
                        self.offsets_cover(Sec::KeyOffsets, Sec::Keys, n)?;
                        absent(&[Sec::Records, Sec::RecordOffsets, Sec::IdToSlot])?;
                    }
                    (Layout::SlotOrdered, Some(w)) => {
                        size(Sec::Records, (w + 4) * n)?;
                        size(Sec::IdToSlot, 4 * n)?;
                        absent(&[
                            Sec::SlotToId,
                            Sec::Keys,
                            Sec::KeyOffsets,
                            Sec::RecordOffsets,
                        ])?;
                    }
                    (Layout::SlotOrdered, None) => {
                        size(Sec::RecordOffsets, 8 * (n + 1))?;
                        size(Sec::IdToSlot, 4 * n)?;
                        self.offsets_cover(Sec::RecordOffsets, Sec::Records, n)?;
                        absent(&[Sec::SlotToId, Sec::Keys, Sec::KeyOffsets])?;
                    }
                }
                if h.renumbered {
                    size(Sec::Order, 4 * n)?;
                } else {
                    absent(&[Sec::Order])?;
                }
            }
        }
        Ok(())
    }

    /// Checks that the `n + 1` offsets of `offsets` start at 0 and end at the length of
    /// `data` (each look-up then checks its own pair).
    fn offsets_cover(&self, offsets: Sec, data: Sec, n: u64) -> Result<(), DictionaryError> {
        let offsets = self.need(offsets)?;
        let data = self.need(data)?;
        let first = read_u64(offsets, 0)?;
        let last = read_u64(offsets, n)?;
        if first != 0 || last != data.len() {
            return Err(DictionaryError::Corrupt(
                "offsets do not cover their section",
            ));
        }
        Ok(())
    }

    /// An id read from a section, checked to be below `n`.
    fn checked_id(&self, raw: u32) -> Result<u32, DictionaryError> {
        if u64::from(raw) < self.header.n {
            Ok(raw)
        } else {
            Err(DictionaryError::Corrupt(
                "an id in a section is not below n",
            ))
        }
    }

    /// The sections to store, by name, in the order of [`SECTION_NAMES`].
    pub fn sections(&self) -> impl Iterator<Item = (&'static str, &S)> {
        SECTION_NAMES
            .iter()
            .zip(&self.sections)
            .filter_map(|(name, s)| s.as_ref().map(|s| (*name, s)))
    }

    /// Number of keys (and of nodes).
    #[must_use]
    pub fn len(&self) -> u32 {
        // Checked below 2^32 when the header was decoded or built.
        u32::try_from(self.header.n).unwrap_or(u32::MAX)
    }

    /// Whether the dictionary has no keys.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.header.n == 0
    }

    /// The kind its keys were recognized as.
    #[must_use]
    pub fn kind(&self) -> KeyKind {
        self.header.kind
    }

    /// The type of the input column; reverse look-ups give keys back in it.
    #[must_use]
    pub fn column(&self) -> ColumnType {
        self.header.column
    }

    /// How ids were assigned: [`IdOrder::FirstAppearance`] or [`IdOrder::SortedKey`].
    #[must_use]
    pub fn order(&self) -> IdOrder {
        self.header.order
    }

    /// The layout of its arrays.
    #[must_use]
    pub fn layout(&self) -> Layout {
        self.header.layout
    }

    /// Whether ids were changed by [`Dictionary::renumber`] since the build.
    #[must_use]
    pub fn is_renumbered(&self) -> bool {
        self.header.renumbered
    }

    /// Bytes held in memory: the hash function, or the rank bitmap.
    #[must_use]
    pub fn resident_bytes(&self) -> u64 {
        self.mphf.bytes()
            + self
                .dense
                .as_ref()
                .map_or(0, |d| RankBitmap::bytes(d.words().len()))
    }

    fn normalize<'k>(&self, key: Key<'k>, buf: &'k mut [u8; MAX_FIXED_WIDTH]) -> Option<&'k [u8]> {
        self.header.kind.normalize(self.header.column, key, buf)
    }

    /// The id of `key`, or `None` if it is not in the dictionary. Hexadecimal and UUID
    /// keys are matched in any case; integer keys may be given as integers or as
    /// decimal text.
    ///
    /// # Errors
    ///
    /// A section could not be read, or holds a value it cannot hold
    /// ([`DictionaryError::Corrupt`]).
    pub fn lookup(&self, key: Key<'_>) -> Result<Option<NodeId>, DictionaryError> {
        let mut buf = [0_u8; MAX_FIXED_WIDTH];
        let Some(norm) = self.normalize(key, &mut buf) else {
            return Ok(None);
        };
        self.lookup_normalized(norm, true)
    }

    /// The id of `key`, which must be one of the dictionary's keys: the key read back
    /// is not compared, which saves one read in the split layout. For any other key the
    /// result is some id, or `None` if the key is not of the dictionary's kind. This is
    /// the mapping of the build's second pass, whose keys are those of the first.
    ///
    /// # Errors
    ///
    /// A section could not be read, or holds a value it cannot hold
    /// ([`DictionaryError::Corrupt`]).
    pub fn id_of_member(&self, key: Key<'_>) -> Result<Option<NodeId>, DictionaryError> {
        let mut buf = [0_u8; MAX_FIXED_WIDTH];
        let Some(norm) = self.normalize(key, &mut buf) else {
            return Ok(None);
        };
        self.lookup_normalized(norm, false)
    }

    fn lookup_normalized(
        &self,
        norm: &[u8],
        verify: bool,
    ) -> Result<Option<NodeId>, DictionaryError> {
        if self.header.n == 0 {
            return Ok(None);
        }
        if let Some(d) = &self.dense {
            let Some(rank) = dense_index(self.header.column, norm).and_then(|i| d.rank(i)) else {
                return Ok(None);
            };
            return self.rank_to_id(rank).map(Some);
        }
        let slot = self.mphf.eval(KeyHash::of(norm, self.header.hash_seed));
        let width = self.header.kind.width();
        let mut key = KeyBuf::new();
        let id = match self.header.layout {
            Layout::Split => {
                let id = self.checked_id(read_u32(
                    self.sec(Sec::SlotToId).expect("validated"),
                    u64::from(slot),
                )?)?;
                if verify {
                    self.read_key_split(id, width, &mut key)?;
                    if key != norm {
                        return Ok(None);
                    }
                }
                id
            }
            Layout::SlotOrdered => {
                let id = self.checked_id(self.read_record(slot, width, &mut key)?)?;
                if key != norm {
                    return Ok(None);
                }
                id
            }
        };
        Ok(Some(NodeId::new(id)))
    }

    fn rank_to_id(&self, rank: u32) -> Result<NodeId, DictionaryError> {
        if self.header.renumbered {
            let raw = read_u32(self.sec(Sec::RankToId).expect("validated"), u64::from(rank))?;
            Ok(NodeId::new(self.checked_id(raw)?))
        } else {
            Ok(NodeId::new(rank))
        }
    }

    /// Key of `id` in the split layout, normalized.
    fn read_key_split(
        &self,
        id: u32,
        width: Option<usize>,
        out: &mut KeyBuf,
    ) -> Result<(), DictionaryError> {
        let keys = self.sec(Sec::Keys).expect("validated");
        match width {
            Some(w) => {
                out.resize(w, 0);
                keys.read_at(u64::from(id) * widen(w), out)?;
            }
            None => {
                let offsets = self.sec(Sec::KeyOffsets).expect("validated");
                let (a, b) = read_offsets(offsets, u64::from(id), keys.len(), 0)?;
                out.resize(index(b - a), 0);
                keys.read_at(a, out)?;
            }
        }
        Ok(())
    }

    /// Record of `slot` in the slot-ordered layout: its key into `out`, its id returned.
    fn read_record(
        &self,
        slot: u32,
        width: Option<usize>,
        out: &mut KeyBuf,
    ) -> Result<u32, DictionaryError> {
        let records = self.sec(Sec::Records).expect("validated");
        match width {
            Some(w) => {
                out.resize(w + 4, 0);
                records.read_at(u64::from(slot) * widen(w + 4), out)?;
                let id = u32::from_le_bytes(out[w..].try_into().expect("four bytes"));
                out.truncate(w);
                Ok(id)
            }
            None => {
                let offsets = self.sec(Sec::RecordOffsets).expect("validated");
                let (a, b) = read_offsets(offsets, u64::from(slot), records.len(), 4)?;
                out.resize(index(b - a), 0);
                records.read_at(a, out)?;
                if out.len() < 4 {
                    return Err(DictionaryError::Corrupt("a record shorter than its id"));
                }
                let id = u32::from_le_bytes(out[..4].try_into().expect("four bytes"));
                out.drain(..4);
                Ok(id)
            }
        }
    }

    /// The normalized bytes of the key of `id` (8 big-endian bytes for integers, the
    /// decoded bytes for hexadecimal, UUID and base58 keys).
    ///
    /// # Errors
    ///
    /// [`DictionaryError::UnknownId`] if `id` is not below [`Dictionary::len`]; read
    /// errors.
    pub fn normalized_key(&self, id: NodeId, out: &mut Vec<u8>) -> Result<(), DictionaryError> {
        if u64::from(id.get()) >= self.header.n {
            return Err(DictionaryError::UnknownId(id));
        }
        if let Some(d) = &self.dense {
            let rank = if self.header.renumbered {
                self.checked_id(read_u32(self.need(Sec::IdToRank)?, u64::from(id.get()))?)?
            } else {
                id.get()
            };
            let bit = d
                .select(rank)
                .ok_or(DictionaryError::Corrupt("rank out of the bitmap"))?;
            let code = dense_base(self.header.column) + u64::from(bit);
            out.clear();
            out.extend_from_slice(&code.to_be_bytes());
            return Ok(());
        }
        let width = self.header.kind.width();
        match self.header.layout {
            Layout::Split => self.read_key_split(id.get(), width, out)?,
            Layout::SlotOrdered => {
                let slot =
                    self.checked_id(read_u32(self.need(Sec::IdToSlot)?, u64::from(id.get()))?)?;
                self.read_record(slot, width, out)?;
            }
        }
        Ok(())
    }

    /// The key of `id`, in the type of the input column: hexadecimal and UUIDs as
    /// lowercase text with the prefix the input used.
    ///
    /// # Errors
    ///
    /// [`DictionaryError::UnknownId`] if `id` is not below [`Dictionary::len`]; read
    /// errors.
    pub fn key(&self, id: NodeId) -> Result<super::OwnedKey, DictionaryError> {
        let mut buf = Vec::new();
        self.normalized_key(id, &mut buf)?;
        Ok(self.header.kind.render(self.header.column, &buf))
    }

    /// The position of `id` in graph order: equal to the id until the dictionary is
    /// renumbered, and unchanged by renumbering.
    ///
    /// # Errors
    ///
    /// [`DictionaryError::UnknownId`] if `id` is not below [`Dictionary::len`]; read
    /// errors.
    pub fn graph_position(&self, id: NodeId) -> Result<u32, DictionaryError> {
        if u64::from(id.get()) >= self.header.n {
            return Err(DictionaryError::UnknownId(id));
        }
        if !self.header.renumbered {
            return Ok(id.get());
        }
        let sec = match self.header.repr {
            Repr::Dense => Sec::IdToRank,
            Repr::Hashed => Sec::Order,
        };
        self.checked_id(read_u32(self.need(sec)?, u64::from(id.get()))?)
    }

    /// Ids of the keys that start with `prefix`, in key order, at most `limit` of
    /// them. The sorted index this needs is built on the first call (from the key
    /// store, reserved against the accountant) and kept.
    ///
    /// # Errors
    ///
    /// [`DictionaryError::Unsupported`] for kinds whose text is not stored as it is
    /// (integers, hexadecimal, UUIDs, base58); budget and read errors.
    pub fn prefix_search(
        &self,
        prefix: &[u8],
        limit: usize,
    ) -> Result<Vec<NodeId>, DictionaryError> {
        Ok(self.prefix_index()?.prefix(prefix, limit))
    }

    /// Ids of the keys in `start..end` (bytewise), in key order, at most `limit`.
    ///
    /// # Errors
    ///
    /// As [`Dictionary::prefix_search`].
    pub fn range_search(
        &self,
        start: &[u8],
        end: &[u8],
        limit: usize,
    ) -> Result<Vec<NodeId>, DictionaryError> {
        Ok(self.prefix_index()?.range(start, end, limit))
    }

    fn prefix_index(&self) -> Result<&PrefixIndex<'a>, DictionaryError> {
        if let Some(p) = self.prefix.get() {
            return Ok(p);
        }
        if !matches!(self.header.kind, KeyKind::Bytes | KeyKind::Fixed { .. }) {
            return Err(DictionaryError::Unsupported(
                "prefix search is for keys stored as they are (text and bytes)",
            ));
        }
        let built = PrefixIndex::build(self, self.accountant)?;
        Ok(self.prefix.get_or_init(|| built))
    }

    /// A dictionary with new ids: the key of old id `i` gets id `permutation[i]`. The
    /// hash function and the keys are kept; graph order is not changed
    /// ([`Dictionary::graph_position`]). Used after a locality renumbering (#20).
    ///
    /// # Errors
    ///
    /// [`DictionaryError::InvalidPermutation`] if `permutation` is not a permutation of
    /// `0..len`; budget, read and scratch errors.
    pub fn renumber<'b, Sc: Scratch>(
        &self,
        permutation: &[u32],
        scratch: &'b Sc,
        accountant: &'b dyn MemoryAccountant,
    ) -> Result<Dictionary<'b, Sc::Source>, DictionaryError> {
        self.renumber_into(permutation, scratch, accountant)
    }

    fn renumber_into<'b, Sc: Scratch>(
        &self,
        permutation: &[u32],
        scratch: &'b Sc,
        accountant: &'b dyn MemoryAccountant,
    ) -> Result<Dictionary<'b, Sc::Source>, DictionaryError> {
        let n = self.header.n;
        if widen(permutation.len()) != n {
            return Err(DictionaryError::InvalidPermutation);
        }
        {
            let _seen_memory =
                Reservation::new(accountant, "id dictionary renumbering", n.div_ceil(8))?;
            let mut seen = vec![0_u64; index(n.div_ceil(64))];
            for &p in permutation {
                let p = index(u64::from(p));
                if widen(p) >= n || seen[p / 64] & (1 << (p % 64)) != 0 {
                    return Err(DictionaryError::InvalidPermutation);
                }
                seen[p / 64] |= 1 << (p % 64);
            }
        }
        // Staging: a write buffer, a read buffer of about 1 MiB, a mapped chunk, and a
        // 64 KiB buffer per scatter range (reserved by each scatter).
        let _staging = Reservation::new(
            accountant,
            "id dictionary renumbering",
            widen(2 * WRITE_BUFFER) + (3 << 20),
        )?;
        let task_bytes = (accountant.available() / 8).clamp(1 << 20, 1 << 30);
        let mut sections: [Option<Sc::Source>; 14] = Default::default();
        let copy = |src: &S| -> Result<Sc::Source, DictionaryError> {
            let mut s = Spill::new(scratch, "id dictionary section")?;
            for_each_record_chunk(src, 1, |_, b| Ok(s.write(b)?))?;
            Ok(s.seal(scratch)?)
        };
        // Old id -> u32 value, scattered to new ids: graph positions, ranks, slots.
        let scatter_u32 = |value_of: &dyn Fn(u64, &[u8]) -> u32,
                           source: Option<&S>|
         -> Result<Sc::Source, DictionaryError> {
            let mut scatter = Scatter::new(scratch, accountant, n, Some(4), 8, task_bytes / 3)?;
            let _ranges = Reservation::new(
                accountant,
                "id dictionary renumbering",
                widen(scatter.range_count()) * (2 << 16),
            )?;
            let mut ranges: Vec<Vec<u8>> = vec![Vec::new(); scatter.range_count()];
            let mut emit = |old: u64, raw: &[u8]| -> Result<(), DictionaryError> {
                let new = permutation[index(old)];
                let r = scatter.range_of(new);
                Scatter::<'_, Sc>::encode(
                    Some(4),
                    new,
                    &value_of(old, raw).to_le_bytes(),
                    &mut ranges[r],
                );
                if ranges[r].len() >= 1 << 16 {
                    scatter.append(r, &ranges[r])?;
                    ranges[r].clear();
                }
                Ok(())
            };
            match source {
                Some(src) => for_each_record_chunk(src, 4, |first, bytes| {
                    for (i, raw) in bytes.chunks_exact(4).enumerate() {
                        emit(first + widen(i), raw)?;
                    }
                    Ok(())
                })?,
                None => {
                    for old in 0..n {
                        emit(old, &[])?;
                    }
                }
            }
            for (r, bytes) in ranges.iter().enumerate() {
                scatter.append(r, bytes)?;
            }
            let mut out = Spill::new(scratch, "id dictionary section")?;
            scatter.finish(scratch, accountant, &mut out, None)?;
            Ok(out.seal(scratch)?)
        };
        let as_u32 = |raw: &[u8]| u32::from_le_bytes(raw.try_into().expect("four bytes"));
        let identity = |old: u64, _: &[u8]| u32::try_from(old).expect("ids below 2^32");
        let stored = |_: u64, raw: &[u8]| as_u32(raw);
        // old id -> new id, applied to a stream of ids.
        let map_ids = |src: &S| -> Result<Sc::Source, DictionaryError> {
            let mut s = Spill::new(scratch, "id dictionary section")?;
            for_each_record_chunk(src, 4, |_, bytes| {
                let mut mapped = Vec::with_capacity(bytes.len());
                for c in bytes.chunks_exact(4) {
                    let old = self.checked_id(as_u32(c))?;
                    mapped.extend_from_slice(&permutation[index(u64::from(old))].to_le_bytes());
                }
                Ok(s.write(&mapped)?)
            })?;
            Ok(s.seal(scratch)?)
        };
        let put = |sections: &mut [Option<Sc::Source>; 14], sec: Sec, s: Sc::Source| {
            sections[sec as usize] = Some(s);
        };
        let mut header = self.header;
        header.renumbered = true;
        let resident = match self.header.repr {
            Repr::Dense => {
                put(&mut sections, Sec::Dense, copy(self.need(Sec::Dense)?)?);
                // rank -> new id, and new id -> rank.
                let rank_to_id = match self.sec(Sec::RankToId) {
                    Some(src) => map_ids(src)?,
                    None => {
                        let mut s = Spill::new(scratch, "id dictionary section")?;
                        for chunk in permutation.chunks(1 << 16) {
                            let bytes: Vec<u8> =
                                chunk.iter().flat_map(|p| p.to_le_bytes()).collect();
                            s.write(&bytes)?;
                        }
                        s.seal(scratch)?
                    }
                };
                put(&mut sections, Sec::RankToId, rank_to_id);
                let id_to_rank = match self.sec(Sec::IdToRank) {
                    Some(src) => scatter_u32(&stored, Some(src))?,
                    None => scatter_u32(&identity, None)?,
                };
                put(&mut sections, Sec::IdToRank, id_to_rank);
                Reservation::new(
                    accountant,
                    "id dictionary dense bitmap",
                    self.resident_bytes(),
                )?
            }
            Repr::Hashed => {
                for sec in [Sec::Parts, Sec::Pilots, Sec::Remap] {
                    put(&mut sections, sec, copy(self.need(sec)?)?);
                }
                let order = match self.sec(Sec::Order) {
                    Some(src) => scatter_u32(&stored, Some(src))?,
                    None => scatter_u32(&identity, None)?,
                };
                put(&mut sections, Sec::Order, order);
                let width = self.header.kind.width();
                match self.header.layout {
                    Layout::Split => {
                        put(
                            &mut sections,
                            Sec::SlotToId,
                            map_ids(self.need(Sec::SlotToId)?)?,
                        );
                        // Keys to their new ids.
                        let record = width.map_or(8 + 16, |w| 4 + w);
                        let mut scatter = Scatter::new(
                            scratch,
                            accountant,
                            n,
                            width,
                            widen(record),
                            task_bytes / 3,
                        )?;
                        let _ranges = Reservation::new(
                            accountant,
                            "id dictionary renumbering",
                            widen(scatter.range_count()) * (2 << 16) + widen(MAX_KEY_BYTES),
                        )?;
                        let mut ranges: Vec<Vec<u8>> = vec![Vec::new(); scatter.range_count()];
                        let mut key = Vec::new();
                        for old in 0..n {
                            let id = NodeId::new(u32::try_from(old).expect("below 2^32"));
                            self.read_key_split(id.get(), width, &mut key)?;
                            let new = permutation[index(old)];
                            let r = scatter.range_of(new);
                            Scatter::<'_, Sc>::encode(width, new, &key, &mut ranges[r]);
                            if ranges[r].len() >= 1 << 16 {
                                scatter.append(r, &ranges[r])?;
                                ranges[r].clear();
                            }
                        }
                        for (r, bytes) in ranges.iter().enumerate() {
                            scatter.append(r, bytes)?;
                        }
                        let mut keys = Spill::new(scratch, "id dictionary section")?;
                        let mut offsets = Spill::new(scratch, "id dictionary section")?;
                        scatter.finish(
                            scratch,
                            accountant,
                            &mut keys,
                            width.is_none().then_some(&mut offsets),
                        )?;
                        put(&mut sections, Sec::Keys, keys.seal(scratch)?);
                        if width.is_none() {
                            put(&mut sections, Sec::KeyOffsets, offsets.seal(scratch)?);
                        }
                    }
                    Layout::SlotOrdered => {
                        put(
                            &mut sections,
                            Sec::IdToSlot,
                            scatter_u32(&stored, Some(self.need(Sec::IdToSlot)?))?,
                        );
                        let records = self.need(Sec::Records)?;
                        let mut s = Spill::new(scratch, "id dictionary section")?;
                        match width {
                            Some(w) => for_each_record_chunk(records, w + 4, |_, bytes| {
                                let mut chunk = bytes.to_vec();
                                for rec in chunk.chunks_exact_mut(w + 4) {
                                    let old = self.checked_id(as_u32(&rec[w..]))?;
                                    rec[w..].copy_from_slice(
                                        &permutation[index(u64::from(old))].to_le_bytes(),
                                    );
                                }
                                Ok(s.write(&chunk)?)
                            })?,
                            None => {
                                let offsets = self.need(Sec::RecordOffsets)?;
                                let mut rec = Vec::new();
                                for slot in 0..n {
                                    let (a, b) = read_offsets(offsets, slot, records.len(), 4)?;
                                    rec.resize(index(b - a), 0);
                                    records.read_at(a, &mut rec)?;
                                    let old = self.checked_id(as_u32(&rec[..4]))?;
                                    rec[..4].copy_from_slice(
                                        &permutation[index(u64::from(old))].to_le_bytes(),
                                    );
                                    s.write(&rec)?;
                                }
                                put(&mut sections, Sec::RecordOffsets, copy(offsets)?);
                            }
                        }
                        put(&mut sections, Sec::Records, s.seal(scratch)?);
                    }
                }
                Reservation::new(
                    accountant,
                    "id dictionary hash function",
                    self.resident_bytes(),
                )?
            }
        };
        let mut hs = Spill::new(scratch, "id dictionary section")?;
        hs.write(&header.encode())?;
        put(&mut sections, Sec::Header, hs.seal(scratch)?);
        Dictionary::assemble(
            header,
            self.mphf.clone(),
            self.dense.clone(),
            sections,
            accountant,
            vec![resident],
        )
    }
}

pub(crate) fn read_u32(source: &dyn ByteSource, i: u64) -> Result<u32, ReadError> {
    let mut b = [0_u8; 4];
    source.read_at(i * 4, &mut b)?;
    Ok(u32::from_le_bytes(b))
}

fn read_u64(source: &dyn ByteSource, i: u64) -> Result<u64, ReadError> {
    let mut b = [0_u8; 8];
    source.read_at(i * 8, &mut b)?;
    Ok(u64::from_le_bytes(b))
}

/// Offsets `i` and `i + 1` of an offsets section, checked before anything is
/// allocated: increasing, within `data_len`, and at most a key (plus `extra` bytes)
/// apart.
fn read_offsets(
    source: &dyn ByteSource,
    i: u64,
    data_len: u64,
    extra: usize,
) -> Result<(u64, u64), DictionaryError> {
    let mut b = [0_u8; 16];
    source.read_at(i * 8, &mut b)?;
    let a = u64::from_le_bytes(b[..8].try_into().expect("eight bytes"));
    let c = u64::from_le_bytes(b[8..].try_into().expect("eight bytes"));
    if c < a || c > data_len || c - a > widen(MAX_KEY_BYTES + extra) {
        return Err(DictionaryError::Corrupt("an offset is out of its section"));
    }
    Ok((a, c))
}
