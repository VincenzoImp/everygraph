// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Writing a container: appending sections and committing them
//! (`docs/spec/eg-container.md` § 8), and compaction (§ 9).

use std::collections::BTreeMap;
use std::sync::Arc;

use everygraph_core::{ByteSink, ByteSource, WriteError};

use super::layout::{self, BLOCK_USIZE, Header, MINOR, pad_end, padded, small};
use super::read::Container;
use super::{
    BLOCK, Codec, ContainerError, Entry, HEADER_BYTES, Kind, MIN_PADDING, Provenance,
    SEGMENT_PREFIX, check_name,
};

/// Bytes copied at a time by [`Writer::copy_section`].
const COPY_CHUNK: usize = 1 << 20;
/// Enough zeros for the longest padding.
const ZEROS: [u8; BLOCK_USIZE + 16] = [0; BLOCK_USIZE + 16];
const _: () = assert!(MIN_PADDING == 16);

/// Writes a container: a new one ([`create`](Self::create)) or more sections into an
/// existing one ([`update`](Self::update)).
///
/// Sections are appended after the committed end of the file, so nothing a committed
/// table of contents refers to is ever written again; [`commit`](Self::commit) makes
/// them part of the file. Until then the file reads exactly as before, whatever happens
/// to the process or the machine. The writer can commit several times; each commit is
/// one generation. A failed write poisons the writer ([`ContainerError::Poisoned`]):
/// open the file again to go on.
///
/// One writer at a time: two writers updating the same file at once overwrite each
/// other's appended sections. The sink keeps them apart: `everygraph_io::FileSink`
/// holds a lock on its file that refuses a second writer.
#[derive(Debug)]
pub struct Writer<S: ByteSink> {
    sink: S,
    entries: BTreeMap<Arc<str>, Entry>,
    /// Generation of the last commit; 0 before the first.
    generation: u64,
    /// The header block that holds the last commit.
    newest_block: Option<u32>,
    required: u64,
    optional: u64,
    /// Where the next section goes: a block boundary at or after the committed end.
    position: u64,
    changed: bool,
    poisoned: bool,
}

impl<S: ByteSink> Writer<S> {
    /// Starts a new container in `sink`, which is emptied first. Nothing is a container
    /// until the first [`commit`](Self::commit) has returned: a file written to a
    /// temporary name and renamed after the commit is never seen half-written.
    ///
    /// # Errors
    ///
    /// [`ContainerError::Write`] if the sink cannot be emptied.
    pub fn create(mut sink: S) -> Result<Self, ContainerError> {
        sink.set_len(0)?;
        sink.set_len(HEADER_BYTES)?;
        Ok(Self {
            sink,
            entries: BTreeMap::new(),
            generation: 0,
            newest_block: None,
            required: 0,
            optional: 0,
            position: HEADER_BYTES,
            changed: true,
            poisoned: false,
        })
    }

    /// Opens the container in `sink` for adding and removing sections. Reads what
    /// [`Container::open`] reads; anything after the committed end (left by a writer
    /// that never committed) is overwritten and then cut off by the next commit.
    ///
    /// # Errors
    ///
    /// Whatever [`Container::open`] returns; [`ContainerError::NewerMinor`] for a file
    /// of a newer minor version than [`MINOR`]; [`ContainerError::Recovered`] when the
    /// file opened at an older commit than the newest it may hold
    /// ([`Container::recovered_from`], [`Container::unreadable_block`]), because the
    /// first commit would write over that newer commit's bytes.
    pub fn update(sink: S) -> Result<Self, ContainerError> {
        Self::open_for_update(sink, false)
    }

    /// [`update`](Self::update) of a file that opened at an older commit, on purpose:
    /// the newer commit that could not be read (damaged, or on a sector the source
    /// could not read) is given up, and the next commit writes over its bytes and its
    /// header block. Copy what can be saved first (`everygraph verify`, `compact`).
    ///
    /// # Errors
    ///
    /// As [`update`](Self::update), without [`ContainerError::Recovered`].
    pub fn update_recovered(sink: S) -> Result<Self, ContainerError> {
        Self::open_for_update(sink, true)
    }

    fn open_for_update(sink: S, recovered_allowed: bool) -> Result<Self, ContainerError> {
        let (header, entries, committed) = {
            let file = Container::open(&sink)?;
            let recovered = file.recovered_from().is_some() || file.unreadable_block().is_some();
            if recovered && !recovered_allowed {
                return Err(ContainerError::Recovered);
            }
            let header: Header = *file.header();
            let entries = file
                .entries()
                .iter()
                .map(|e| (Arc::clone(&e.name), e.clone()))
                .collect();
            (header, entries, file.committed_len())
        };
        if header.minor > MINOR {
            return Err(ContainerError::NewerMinor {
                minor: header.minor,
            });
        }
        Ok(Self {
            sink,
            entries,
            generation: header.generation,
            newest_block: Some(header.block),
            required: header.required,
            optional: header.optional,
            position: committed,
            changed: false,
            poisoned: false,
        })
    }

    /// The sections the next commit will hold, in increasing byte order of name.
    pub fn entries(&self) -> impl ExactSizeIterator<Item = &Entry> {
        self.entries.values()
    }

    /// The section named `name` in the next commit.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Entry> {
        self.entries.get(name)
    }

    /// The generation of the last commit (0 before the first commit of a new file).
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    fn check_new(&self, name: &str, provenance: Option<&str>) -> Result<(), ContainerError> {
        if self.poisoned {
            return Err(ContainerError::Poisoned);
        }
        check_name(name)?;
        if self.entries.contains_key(name) {
            return Err(ContainerError::NameTaken(name.to_owned()));
        }
        if let Some(record) = provenance {
            match self.entries.get(record) {
                None => return Err(ContainerError::NoSuchSection(record.to_owned())),
                Some(e) if e.kind != Kind::PROVENANCE => {
                    return Err(ContainerError::NotProvenance(record.to_owned()));
                }
                Some(_) => {}
            }
        }
        Ok(())
    }

    fn start(
        &mut self,
        name: &str,
        kind: Kind,
        codec: Codec,
        provenance: Option<&str>,
    ) -> Result<SectionWriter<'_, S>, ContainerError> {
        self.check_new(name, provenance)?;
        // Segments are reserved (spec § 13): this version writes neither their names
        // nor their manifest.
        if name.starts_with(SEGMENT_PREFIX) || kind == Kind::SEGMENTS {
            return Err(ContainerError::Reserved(name.to_owned()));
        }
        let offset = self.position;
        // A reference shares the record's name.
        let provenance = provenance
            .and_then(|record| self.entries.get(record))
            .map(|record| Arc::clone(&record.name));
        Ok(SectionWriter {
            entry: Entry {
                name: name.into(),
                kind,
                codec,
                offset,
                len: 0,
                hash: [0; 32],
                provenance,
            },
            hasher: blake3::Hasher::new(),
            writer: self,
        })
    }

    /// Starts appending section `name`, made as `provenance` (the name of a
    /// provenance record already added) says. Write its bytes with
    /// [`SectionWriter::write`], then [`SectionWriter::finish`] it; a section dropped
    /// before it is finished is not added.
    ///
    /// # Errors
    ///
    /// [`ContainerError::InvalidName`], [`ContainerError::NameTaken`] (remove the old
    /// section first to replace it), [`ContainerError::NoSuchSection`] or
    /// [`ContainerError::NotProvenance`] for the provenance reference,
    /// [`ContainerError::NotProvenance`] when `kind` is [`Kind::PROVENANCE`] (use
    /// [`add_provenance`](Self::add_provenance)), [`ContainerError::Poisoned`].
    pub fn begin_section(
        &mut self,
        name: &str,
        kind: Kind,
        provenance: Option<&str>,
    ) -> Result<SectionWriter<'_, S>, ContainerError> {
        if kind == Kind::PROVENANCE {
            return Err(ContainerError::NotProvenance(name.to_owned()));
        }
        self.start(name, kind, Codec::NONE, provenance)
    }

    /// Appends section `name` holding `bytes`.
    ///
    /// # Errors
    ///
    /// As [`begin_section`](Self::begin_section) and [`SectionWriter::write`].
    pub fn add_section(
        &mut self,
        name: &str,
        kind: Kind,
        bytes: &[u8],
        provenance: Option<&str>,
    ) -> Result<Entry, ContainerError> {
        let mut section = self.begin_section(name, kind, provenance)?;
        section.write(bytes)?;
        section.finish()
    }

    /// Appends a provenance record as section `name`, for sections added after it to
    /// refer to.
    ///
    /// # Errors
    ///
    /// As [`add_section`](Self::add_section), and [`ContainerError::InvalidProvenance`]
    /// or [`ContainerError::TooLarge`] for a record that cannot be encoded.
    pub fn add_provenance(
        &mut self,
        name: &str,
        record: &Provenance,
    ) -> Result<Entry, ContainerError> {
        let bytes = record.encode()?;
        let mut section = self.start(name, Kind::PROVENANCE, Codec::NONE, None)?;
        section.write(&bytes)?;
        section.finish()
    }

    /// Appends a copy of section `name` of `from` under the same name, kind, codec,
    /// hash and provenance reference (which must already be here). The bytes are
    /// checked against the hash as they are copied, so a damaged section is never
    /// copied as if it were whole.
    ///
    /// # Errors
    ///
    /// [`ContainerError::Damaged`] when the bytes read do not match the hash;
    /// [`ContainerError::Read`]; otherwise as [`begin_section`](Self::begin_section).
    pub fn copy_section<R: ByteSource>(
        &mut self,
        from: &Container<R>,
        name: &str,
    ) -> Result<Entry, ContainerError> {
        let source = from
            .get(name)
            .ok_or_else(|| ContainerError::NoSuchSection(name.to_owned()))?;
        let mut section = self.start(
            name,
            source.kind,
            source.codec,
            source.provenance.as_deref(),
        )?;
        let mut buf = Vec::new();
        let mut at = 0;
        while at < source.len {
            let n = small((source.len - at).min(COPY_CHUNK as u64));
            let range = source.offset + at..source.offset + at + n as u64;
            let piece = match from.source().slice(range) {
                Some(bytes) => bytes,
                None => {
                    buf.resize(n, 0);
                    from.source().read_at(source.offset + at, &mut buf)?;
                    &buf[..]
                }
            };
            section.write(piece)?;
            at += n as u64;
        }
        if *section.hasher.finalize().as_bytes() != source.hash {
            return Err(ContainerError::Damaged {
                name: name.to_owned(),
            });
        }
        section.finish()
    }

    /// Takes section `name` out of the next commit. Its bytes stay in the file, and in
    /// every earlier commit, until the file is [`compact`]ed.
    ///
    /// # Errors
    ///
    /// [`ContainerError::NoSuchSection`]; [`ContainerError::StillReferenced`] for a
    /// provenance record another section refers to; [`ContainerError::Poisoned`].
    pub fn remove(&mut self, name: &str) -> Result<Entry, ContainerError> {
        if self.poisoned {
            return Err(ContainerError::Poisoned);
        }
        if let Some(by) = self
            .entries
            .values()
            .find(|e| e.provenance.as_deref() == Some(name))
        {
            return Err(ContainerError::StillReferenced {
                name: name.to_owned(),
                by: by.name.to_string(),
            });
        }
        let removed = self
            .entries
            .remove(name)
            .ok_or_else(|| ContainerError::NoSuchSection(name.to_owned()))?;
        self.changed = true;
        Ok(removed)
    }

    /// Makes the sections added and removed since the last commit the file's state,
    /// durably, and returns the new generation. Nothing is written when nothing
    /// changed.
    ///
    /// The steps, in order: write the table of contents after the last section, set the
    /// file's length to its padded end, sync; write the header block that does not
    /// hold the last commit, sync. A crash before the second sync leaves the file
    /// reading as the last commit or, once the header block is on the disk, as this
    /// one; never as anything else.
    ///
    /// # Errors
    ///
    /// [`ContainerError::TooLarge`] when the table of contents would exceed 16 MiB;
    /// [`ContainerError::GenerationExhausted`] after 2^64 − 1 commits (nothing is
    /// written); [`ContainerError::Write`] when the sink fails (the writer is then
    /// poisoned); [`ContainerError::Poisoned`].
    pub fn commit(&mut self) -> Result<u64, ContainerError> {
        if self.poisoned {
            return Err(ContainerError::Poisoned);
        }
        if !self.changed {
            return Ok(self.generation);
        }
        // Checked before anything is written: a wrapped generation would be a commit
        // that returns `Ok` and never becomes the file's state.
        let generation = self
            .generation
            .checked_add(1)
            .ok_or(ContainerError::GenerationExhausted)?;
        let mut toc = layout::encode_toc(self.entries.values())?;
        let toc_offset = self.position;
        let toc_len = toc.len() as u64;
        let end = pad_end(toc_offset, toc_len)
            .ok_or(ContainerError::TooLarge("the file would exceed 2^64 bytes"))?;
        let header = Header {
            block: self.newest_block.map_or(0, |b| 1 - b),
            minor: MINOR,
            generation,
            required: self.required,
            optional: self.optional,
            toc_offset,
            toc_len,
            toc_hash: *blake3::hash(&toc).as_bytes(),
        };
        toc.resize(small(end - toc_offset), 0);
        let steps = |sink: &mut S| -> Result<(), WriteError> {
            sink.write_at(toc_offset, &toc)?;
            sink.set_len(end)?;
            sink.sync()?;
            sink.write_at(
                u64::from(header.block) * BLOCK,
                &layout::encode_header(&header),
            )?;
            sink.sync()
        };
        if let Err(e) = steps(&mut self.sink) {
            self.poisoned = true;
            return Err(e.into());
        }
        self.generation = header.generation;
        self.newest_block = Some(header.block);
        self.position = end;
        self.changed = false;
        Ok(self.generation)
    }

    /// The sink.
    #[must_use]
    pub fn sink(&self) -> &S {
        &self.sink
    }

    /// Gives the sink back. What was not committed is not part of the container.
    #[must_use]
    pub fn into_sink(self) -> S {
        self.sink
    }
}

/// A section being appended: see [`Writer::begin_section`].
#[derive(Debug)]
pub struct SectionWriter<'w, S: ByteSink> {
    writer: &'w mut Writer<S>,
    entry: Entry,
    hasher: blake3::Hasher,
}

impl<S: ByteSink> SectionWriter<'_, S> {
    /// Appends `bytes` to the section.
    ///
    /// # Errors
    ///
    /// [`ContainerError::Write`] when the sink fails (the writer is then poisoned);
    /// [`ContainerError::TooLarge`] past 2^64 bytes; [`ContainerError::Poisoned`].
    pub fn write(&mut self, bytes: &[u8]) -> Result<(), ContainerError> {
        if self.writer.poisoned {
            return Err(ContainerError::Poisoned);
        }
        let len = self
            .entry
            .len
            .checked_add(bytes.len() as u64)
            .filter(|&len| pad_end(self.entry.offset, len).is_some())
            .ok_or(ContainerError::TooLarge("the file would exceed 2^64 bytes"))?;
        if let Err(e) = self
            .writer
            .sink
            .write_at(self.entry.offset + self.entry.len, bytes)
        {
            self.writer.poisoned = true;
            return Err(e.into());
        }
        self.hasher.update(bytes);
        self.entry.len = len;
        Ok(())
    }

    /// Bytes written so far.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.entry.len
    }

    /// Whether nothing has been written yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entry.len == 0
    }

    /// Writes the padding and adds the section to the next commit.
    ///
    /// # Errors
    ///
    /// [`ContainerError::Write`] when the sink fails (the writer is then poisoned);
    /// [`ContainerError::Poisoned`].
    pub fn finish(mut self) -> Result<Entry, ContainerError> {
        if self.writer.poisoned {
            return Err(ContainerError::Poisoned);
        }
        let end = self.entry.offset + self.entry.len;
        let padded = padded(self.entry.offset, self.entry.len);
        let padding = small(padded - end);
        if let Err(e) = self.writer.sink.write_at(end, &ZEROS[..padding]) {
            self.writer.poisoned = true;
            return Err(e.into());
        }
        self.entry.hash = *self.hasher.finalize().as_bytes();
        self.writer
            .entries
            .insert(self.entry.name.clone(), self.entry.clone());
        self.writer.position = padded;
        self.writer.changed = true;
        Ok(self.entry)
    }
}

/// Writes a new container into `to` that holds the live sections of `from` and
/// nothing else, and commits it; returns the sink. Every section is checked against
/// its hash while it is copied. The original is not touched: the caller replaces it
/// with the new file (written under a temporary name, then renamed over it).
///
/// # Errors
///
/// [`ContainerError::Damaged`] for a section whose bytes do not match its hash (the
/// new container is then not committed); otherwise as [`Writer::copy_section`] and
/// [`Writer::commit`].
pub fn compact<R: ByteSource, S: ByteSink>(
    from: &Container<R>,
    to: S,
) -> Result<S, ContainerError> {
    let mut writer = Writer::create(to)?;
    writer.optional = from.optional_features();
    // Records first, so that references to them can be checked; then in file order.
    let mut order: Vec<&Entry> = from.entries().iter().collect();
    order.sort_by_key(|e| (e.kind != Kind::PROVENANCE, e.offset));
    for entry in order {
        writer.copy_section(from, &entry.name)?;
    }
    writer.commit()?;
    Ok(writer.into_sink())
}
