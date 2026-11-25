// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Opening a container and reading its sections (`docs/spec/eg-container.md` § 7).

use std::ops::Range;

use everygraph_core::{AccessHint, ByteSource, ReadError};

use super::layout::{self, BLOCK_USIZE, Block, Header, KNOWN_REQUIRED, padded, small};
use super::{
    Codec, ContainerError, Entry, HEADER_BYTES, Kind, MAX_PROVENANCE_BYTES, Provenance,
    SEGMENT_PREFIX,
};

/// Bytes read at a time when a section is hashed from a source that is not in memory.
const HASH_CHUNK: usize = 1 << 20;

/// An opened `.eg` container: the committed table of contents of a [`ByteSource`].
///
/// Opening reads the two header blocks (one read of their first 4,224 bytes) and the
/// table of contents they point to (one read), and nothing else; sections are read
/// when asked for. The
/// source must not change while the container is open (the `ByteSource` contract): a
/// writer commits to the file by appending, so a container opened before a commit
/// keeps reading the state it opened.
#[derive(Debug)]
pub struct Container<S> {
    source: S,
    header: Header,
    entries: Vec<Entry>,
    recovered_from: Option<u64>,
    unreadable_block: Option<u32>,
}

/// What the header read found.
struct Heads {
    blocks: [Block; 2],
    /// A block the source failed to read (the other one could be read).
    unreadable: Option<u32>,
    /// The error, kept for when no block turns out valid.
    error: Option<ReadError>,
}

/// Reads the two header blocks: in one read, and if the source fails, one read per
/// block, so that one unreadable sector does not hide the other block.
fn read_headers<S: ByteSource>(source: &S) -> Result<Heads, ContainerError> {
    let used = layout::HEADER_USED;
    let mut head = [0u8; BLOCK_USIZE + layout::HEADER_USED];
    let head_len = small(source.len()).min(head.len());
    let error = match source.read_at(0, &mut head[..head_len]) {
        Ok(()) => {
            return Ok(Heads {
                blocks: [
                    layout::decode_header(&head[..head_len.min(used)], 0),
                    layout::decode_header(head.get(BLOCK_USIZE..head_len).unwrap_or(&[]), 1),
                ],
                unreadable: None,
                error: None,
            });
        }
        Err(e @ ReadError::Backend(_)) => e,
        Err(e) => return Err(e.into()),
    };
    let mut blocks = [Block::Absent, Block::Absent];
    let mut unreadable = Vec::with_capacity(2);
    for ((index, start), block) in [(0u32, 0usize), (1, BLOCK_USIZE)]
        .into_iter()
        .zip(blocks.iter_mut())
    {
        let end = (start + used).min(head_len);
        if start >= end {
            continue;
        }
        let mut b = [0u8; layout::HEADER_USED];
        match source.read_at(start as u64, &mut b[..end - start]) {
            Ok(()) => *block = layout::decode_header(&b[..end - start], index),
            Err(ReadError::Backend(_)) => unreadable.push(index),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(Heads {
        blocks,
        // Both unreadable: `open` finds no valid block and returns the error.
        unreadable: (unreadable.len() == 1).then(|| unreadable[0]),
        error: (!unreadable.is_empty()).then_some(error),
    })
}

impl<S: ByteSource> Container<S> {
    /// Opens the container held by `source`.
    ///
    /// The newest valid header block is used (block 0 on a tie). If the table of
    /// contents it points to cannot be used — its hash does not match, it breaks a rule
    /// of the specification, it lies outside the source, or the source fails to read it
    /// — and the other block is valid, that older commit is opened instead, and
    /// [`recovered_from`](Self::recovered_from) says so: the protocol of
    /// [`Writer::commit`](super::Writer::commit) never leaves such a file, but a disk
    /// that acknowledged a sync it had not done, a bad sector or a failing network can.
    /// If the source fails to read one header block but not the other, the other is
    /// used and [`unreadable_block`](Self::unreadable_block) says so. A caller that
    /// would rather retry than read an older commit checks both.
    ///
    /// # Errors
    ///
    /// [`ContainerError::NotAContainer`] or [`ContainerError::NotCommitted`] when no header
    /// block is valid; [`ContainerError::UnsupportedByteOrder`],
    /// [`ContainerError::UnsupportedVersion`] or [`ContainerError::UnsupportedFeatures`]
    /// for a file this reader does not understand (a foreign header block refuses the
    /// file even when the other block is valid); [`ContainerError::Corrupt`] when the
    /// table of contents breaks the specification; [`ContainerError::Read`] when the
    /// source fails.
    pub fn open(source: S) -> Result<Self, ContainerError> {
        let heads = read_headers(&source)?;
        let mut valid: Vec<Header> = Vec::with_capacity(2);
        let mut foreign = None;
        let mut any_magic = false;
        for block in heads.blocks {
            match block {
                Block::Valid(h) => valid.push(h),
                Block::Foreign(e) => foreign = Some(e),
                Block::Torn => any_magic = true,
                Block::Absent => {}
            }
        }
        // Newest first; on a tie, block 0.
        valid.sort_by_key(|h| (std::cmp::Reverse(h.generation), h.block));
        // A foreign block (a later major version, another byte order) refuses the file
        // even beside a valid one: it may hold the newest commit, and no tear can make
        // one (spec § 2, § 7).
        if let Some(e) = foreign {
            return Err(e);
        }
        let Some(newest) = valid.first().copied() else {
            return Err(match heads.error {
                Some(e) => ContainerError::Read(e),
                None if any_magic => ContainerError::NotCommitted,
                None => ContainerError::NotAContainer,
            });
        };
        let mut first_error = None;
        for (position, header) in valid.iter().enumerate() {
            // A commit that needs a feature this reader lacks is refused, not skipped.
            let unknown = header.required & !KNOWN_REQUIRED;
            if unknown != 0 {
                return Err(ContainerError::UnsupportedFeatures { required: unknown });
            }
            // Any failure of a table — damage, a rule broken, the source failing — opens
            // the older commit instead, and says so (spec § 7).
            match read_toc(&source, header) {
                Ok(entries) => {
                    return Ok(Self {
                        source,
                        header: *header,
                        entries,
                        // Whenever the block used is not the first in order, a tie
                        // included: an update then needs consent.
                        recovered_from: (position > 0).then_some(newest.generation),
                        unreadable_block: heads.unreadable,
                    });
                }
                Err(e) => {
                    first_error.get_or_insert(e);
                }
            }
        }
        Err(first_error.unwrap_or(ContainerError::NotCommitted))
    }

    /// The number of commits that led to this state (1 for a file written once).
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.header.generation
    }

    /// The format version of the file: major ([`MAJOR`](super::MAJOR)) and minor.
    #[must_use]
    pub fn version(&self) -> (u16, u16) {
        (super::MAJOR, self.header.minor)
    }

    /// The optional feature bits of the file (none are defined in 1.0).
    #[must_use]
    pub fn optional_features(&self) -> u64 {
        self.header.optional
    }

    /// The generation of a newer commit that could not be read, when the file was
    /// opened from a block other than the first in order (see [`open`](Self::open));
    /// `None` normally. On a tie whose block 0 is unusable it is the generation of
    /// both blocks.
    #[must_use]
    pub fn recovered_from(&self) -> Option<u64> {
        self.recovered_from
    }

    /// The header block the source failed to read, when the file was opened from the
    /// other one; `None` normally. That block may have held a newer commit.
    #[must_use]
    pub fn unreadable_block(&self) -> Option<u32> {
        self.unreadable_block
    }

    /// Every section, in increasing byte order of name, unknown kinds included.
    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The section named `name`.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Entry> {
        self.entries
            .binary_search_by(|e| (*e.name).cmp(name))
            .ok()
            .map(|i| &self.entries[i])
    }

    fn entry(&self, name: &str) -> Result<&Entry, ContainerError> {
        self.get(name)
            .ok_or_else(|| ContainerError::NoSuchSection(name.to_owned()))
    }

    /// The content of section `name`, as a byte source of its own.
    ///
    /// # Errors
    ///
    /// [`ContainerError::NoSuchSection`], or [`ContainerError::UnsupportedCodec`] when
    /// its codec is not [`Codec::NONE`].
    pub fn section(&self, name: &str) -> Result<Section<'_, S>, ContainerError> {
        let entry = self.entry(name)?;
        if entry.codec != Codec::NONE {
            return Err(ContainerError::UnsupportedCodec {
                name: entry.name.to_string(),
                codec: entry.codec,
            });
        }
        Ok(Section {
            source: &self.source,
            offset: entry.offset,
            len: entry.len,
        })
    }

    /// Checks section `name` against its hash, and that its padding is zero. Reads the
    /// whole section (in 1 MiB pieces unless the source is in memory).
    ///
    /// # Errors
    ///
    /// [`ContainerError::Damaged`] when it does not match; [`ContainerError::NoSuchSection`];
    /// [`ContainerError::Read`] when the source fails or is too short.
    pub fn verify(&self, name: &str) -> Result<(), ContainerError> {
        let entry = self.entry(name)?;
        let damaged = || ContainerError::Damaged {
            name: entry.name.to_string(),
        };
        if hash_range(&self.source, entry.offset, entry.len)? != entry.hash {
            return Err(damaged());
        }
        let end = entry.offset + entry.len;
        let mut pad = vec![0u8; small(padded(entry.offset, entry.len) - end)];
        self.source.read_at(end, &mut pad)?;
        if pad.iter().any(|&b| b != 0) {
            return Err(damaged());
        }
        Ok(())
    }

    /// [`verify`](Self::verify) for every section, in offset order; then that the
    /// table of contents' padding is zero and the file reaches the committed end
    /// (opening checks neither).
    ///
    /// # Errors
    ///
    /// The first section that fails; [`ContainerError::Corrupt`] for the table's
    /// padding or a file that ends inside it.
    pub fn verify_all(&self) -> Result<(), ContainerError> {
        let mut by_offset: Vec<&Entry> = self.entries.iter().collect();
        by_offset.sort_by_key(|e| e.offset);
        by_offset.iter().try_for_each(|e| self.verify(&e.name))?;
        let toc_end = self.header.toc_offset + self.header.toc_len;
        let mut pad = vec![0u8; small(self.committed_len() - toc_end)];
        match self.source.read_at(toc_end, &mut pad) {
            Ok(()) if pad.iter().all(|&b| b == 0) => Ok(()),
            Err(e @ ReadError::Backend(_)) => Err(e.into()),
            _ => Err(ContainerError::Corrupt(
                "the table of contents' padding is not zero, or the file ends inside it",
            )),
        }
    }

    /// The provenance record of section `name`, if it has one.
    ///
    /// # Errors
    ///
    /// [`ContainerError::NoSuchSection`]; [`ContainerError::InvalidProvenance`] or
    /// [`ContainerError::TooLarge`] for a record that cannot be decoded;
    /// [`ContainerError::Read`].
    pub fn provenance(&self, name: &str) -> Result<Option<Provenance>, ContainerError> {
        match &self.entry(name)?.provenance {
            None => Ok(None),
            Some(record) => self.read_provenance(record).map(Some),
        }
    }

    /// Decodes the provenance record stored in section `name`.
    ///
    /// # Errors
    ///
    /// As [`provenance`](Self::provenance), and [`ContainerError::NotProvenance`] when the
    /// section is not a provenance record.
    pub fn read_provenance(&self, name: &str) -> Result<Provenance, ContainerError> {
        let entry = self.entry(name)?;
        if entry.kind != Kind::PROVENANCE {
            return Err(ContainerError::NotProvenance(name.to_owned()));
        }
        if entry.len > MAX_PROVENANCE_BYTES {
            return Err(ContainerError::TooLarge(
                "a provenance record exceeds 16 MiB",
            ));
        }
        let section = self.section(name)?;
        let mut bytes = vec![0u8; small(entry.len)];
        section.read_at(0, &mut bytes)?;
        if *blake3::hash(&bytes).as_bytes() != entry.hash {
            return Err(ContainerError::Damaged {
                name: name.to_owned(),
            });
        }
        Provenance::decode(&bytes)
    }

    /// The end of the committed part of the file: the end of the TOC's padding. A
    /// writer appends from here; anything after it is left over from a write that was
    /// never committed.
    #[must_use]
    pub fn committed_len(&self) -> u64 {
        padded(self.header.toc_offset, self.header.toc_len)
    }

    /// Bytes of the file that the committed state uses: the header, each section and
    /// the TOC with their padding.
    #[must_use]
    pub fn live_bytes(&self) -> u64 {
        let sections: u64 = self
            .entries
            .iter()
            .map(|e| padded(e.offset, e.len) - e.offset)
            .sum();
        HEADER_BYTES + sections + (self.committed_len() - self.header.toc_offset)
    }

    /// Bytes that [`compact`](super::compact) would reclaim: replaced and removed
    /// sections, older tables of contents, and anything after the committed end.
    #[must_use]
    pub fn reclaimable_bytes(&self) -> u64 {
        self.source.len().saturating_sub(self.live_bytes())
    }

    /// The source.
    #[must_use]
    pub fn source(&self) -> &S {
        &self.source
    }

    /// Gives the source back.
    #[must_use]
    pub fn into_source(self) -> S {
        self.source
    }

    pub(crate) fn header(&self) -> &Header {
        &self.header
    }
}

/// Reads and decodes the table of contents `header` points to.
fn read_toc<S: ByteSource>(source: &S, header: &Header) -> Result<Vec<Entry>, ContainerError> {
    layout::check_toc_pointer(header)?;
    let mut toc = vec![0u8; small(header.toc_len)];
    source.read_at(header.toc_offset, &mut toc)?;
    if *blake3::hash(&toc).as_bytes() != header.toc_hash {
        return Err(ContainerError::Corrupt(
            "the table of contents does not match its hash",
        ));
    }
    let entries = layout::decode_toc(&toc, header.toc_offset)?;
    // The segment manifest and `seg/` names go with required bit 0 (spec § 4, § 10); a
    // file with the bit never gets here, so one here is a file without it.
    if entries
        .iter()
        .any(|e| e.kind == Kind::SEGMENTS || e.name.starts_with(SEGMENT_PREFIX))
    {
        return Err(ContainerError::Corrupt(
            "a segment manifest or segment section without the segments feature",
        ));
    }
    Ok(entries)
}

/// BLAKE3 of `len` bytes of `source` at `offset`.
pub(crate) fn hash_range<S: ByteSource + ?Sized>(
    source: &S,
    offset: u64,
    len: u64,
) -> Result<[u8; 32], ContainerError> {
    let mut hasher = blake3::Hasher::new();
    if let Some(all) = source.as_slice() {
        let range = usize::try_from(offset)
            .ok()
            .zip(usize::try_from(offset.saturating_add(len)).ok())
            .and_then(|(start, end)| all.get(start..end))
            .ok_or(ReadError::OutOfBounds {
                offset,
                len,
                source_len: source.len(),
            })?;
        hasher.update(range);
    } else {
        let mut buf = vec![0u8; HASH_CHUNK];
        let mut at = 0;
        while at < len {
            let n = small((len - at).min(HASH_CHUNK as u64));
            source.read_at(offset + at, &mut buf[..n])?;
            hasher.update(&buf[..n]);
            at += n as u64;
        }
    }
    Ok(*hasher.finalize().as_bytes())
}

/// The content of one section: a [`ByteSource`] of its own, positioned at the
/// section's first byte. Zero-copy when the container's source is in memory or mapped.
#[derive(Debug)]
pub struct Section<'a, S> {
    source: &'a S,
    offset: u64,
    len: u64,
}

impl<S> Section<'_, S> {
    /// Where the section starts in the file.
    #[must_use]
    pub fn offset(&self) -> u64 {
        self.offset
    }
}

impl<S: ByteSource> ByteSource for Section<'_, S> {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), ReadError> {
        let out_of_bounds = || ReadError::OutOfBounds {
            offset,
            len: buf.len() as u64,
            source_len: self.len,
        };
        match offset.checked_add(buf.len() as u64) {
            Some(end) if end <= self.len => self.source.read_at(self.offset + offset, buf),
            _ => Err(out_of_bounds()),
        }
    }

    fn as_slice(&self) -> Option<&[u8]> {
        let all = self.source.as_slice()?;
        let start = usize::try_from(self.offset).ok()?;
        let end = usize::try_from(self.offset + self.len).ok()?;
        all.get(start..end)
    }

    fn advise(&self, range: Range<u64>, hint: AccessHint) {
        let start = range.start.min(self.len);
        let end = range.end.min(self.len);
        self.source
            .advise(self.offset + start..self.offset + end, hint);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::container::layout::{encode_header, encode_toc, pad_end};
    use crate::container::{Codec, MINOR};

    /// A file of `len` bytes that holds `pieces` and zeros elsewhere, without the
    /// zeros in memory; it counts the reads made through it.
    struct Sparse {
        len: u64,
        pieces: Vec<(u64, Vec<u8>)>,
        reads: Cell<usize>,
    }

    impl ByteSource for Sparse {
        fn len(&self) -> u64 {
            self.len
        }

        fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), ReadError> {
            self.reads.set(self.reads.get() + 1);
            let end = offset + buf.len() as u64;
            assert!(end <= self.len);
            buf.fill(0);
            for (at, bytes) in &self.pieces {
                let (lo, hi) = (offset.max(*at), end.min(at + bytes.len() as u64));
                if lo < hi {
                    // Positions in the file stay `u64` (they pass 4 GiB); only the
                    // distances inside `buf` and `bytes` become `usize`.
                    let index = |x: u64| usize::try_from(x).unwrap();
                    let (dst, src, n) = (index(lo - offset), index(lo - at), index(hi - lo));
                    buf[dst..dst + n].copy_from_slice(&bytes[src..src + n]);
                }
            }
            Ok(())
        }
    }

    #[test]
    fn opening_a_15_gb_file_costs_one_header_read_and_one_table_read() {
        let gb15 = 15_000_000_000;
        let entries: Vec<Entry> = ["adjacency.out.data", "adjacency.out.index", "ids.keys"]
            .iter()
            .enumerate()
            // Three 5 GB sections, one after another: a 15 GB file.
            .map(|(i, name)| Entry {
                name: (*name).into(),
                kind: Kind::ID_DICTIONARY,
                codec: Codec::NONE,
                offset: HEADER_BYTES + i as u64 * pad_end(0, gb15 / 3).unwrap(),
                len: gb15 / 3,
                hash: [0; 32],
                provenance: None,
            })
            .collect();
        let last = entries.last().unwrap();
        let toc_offset = pad_end(last.offset, last.len).unwrap();
        let toc = encode_toc(entries.iter()).unwrap();
        let header = encode_header(&Header {
            block: 0,
            minor: MINOR,
            generation: 1,
            required: 0,
            optional: 0,
            toc_offset,
            toc_len: toc.len() as u64,
            toc_hash: *blake3::hash(&toc).as_bytes(),
        });
        let source = Sparse {
            len: pad_end(toc_offset, toc.len() as u64).unwrap(),
            pieces: vec![(0, header.to_vec()), (toc_offset, toc)],
            reads: Cell::new(0),
        };
        assert!((15_000_000_000..15_000_100_000).contains(&source.len()));
        // The table lies past 4 GiB: on a 32-bit target (i686, wasm32) this checks that
        // no position the reader handles goes through `usize`.
        assert!(toc_offset > u64::from(u32::MAX));
        let file = Container::open(&source).unwrap();
        assert_eq!(file.entries(), &entries[..]);
        assert_eq!(source.reads.get(), 2);
    }
}
