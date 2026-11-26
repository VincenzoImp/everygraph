// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Checking a graph file: what `everygraph verify` runs (issue #41).
//!
//! [`verify`] checks a `.eg` container against `docs/spec/eg-container.md` and every
//! adjacency in it against `docs/spec/eg-adjacency.md` § 5. It does not stop at the
//! first problem: the [`Report`] lists each one with the section it is in.
//!
//! - [`Depth::Fast`] reads the header blocks (bytes 128 to 4095 zero; the other block
//!   valid or empty, else a note), the table of contents, the file's length and the
//!   table's padding, the names and kinds of the adjacency sections, and the hash and
//!   padding of every section of at most [`FAST_SECTION_BYTES`].
//! - [`Depth::Deep`] reads everything: the hash and padding of every section, and the
//!   structure of every adjacency — its index, its `.events` section exactly when the
//!   index announces one (whole, § 5), every record, lists strictly increasing
//!   with every neighbour below the node count; no self-loops and symmetric lists when
//!   its provenance declares `self_loops` = `drop` and `directed` = `false` (spec § 6);
//!   and an in-adjacency (`<base>.in`) that is the transpose of its out-adjacency.
//!
//! Sections are hashed in parallel (`everygraph_core::par`), each in 1 MiB pieces unless
//! the source is in memory or mapped. Lists are decoded block by block, in parallel; from
//! a source that is not mapped, each block's bytes are read into a buffer reserved
//! against the caller's [`Memory`], so the memory used is that of the largest blocks, not
//! of the file. Symmetry and transposition are compared through a sum of pair hashes, in
//! constant memory per block. Each provenance record is decoded once.
//!
//! The identifier dictionary (`ids.*`) is checked by `everygraph_ingest::ids`, which this
//! crate does not depend on.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::sync::Arc;

use everygraph_core::{
    ByteSource, MAX_ALLOCATION_BYTES, Memory, MemoryKind, NodeId, ReadError, Reservation, par,
};

use crate::adjacency::{AdjacencyEvents, AdjacencyIndex, Decoder, FormatError};
use crate::container::layout::{self, BLOCK_USIZE, Block, HEADER_USED, MAGIC, padded, small};
use crate::container::read::hash_range;
use crate::container::{Container, ContainerError, Entry, Kind, MAX_TOC_BYTES};

/// The largest section whose hash [`Depth::Fast`] checks.
pub const FAST_SECTION_BYTES: u64 = 1 << 20;

/// The most bytes in one buffer: the limit of the project on a single allocation.
const MAX_READ: u64 = MAX_ALLOCATION_BYTES;

/// Blocks of an adjacency decoded by one task.
const BLOCKS_PER_TASK: NonZeroUsize = NonZeroUsize::new(8).expect("8 > 0");

/// What the buffers of the structural check are reserved as.
const PURPOSE: &str = "verify";

/// How much of the file [`verify`] reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Depth {
    /// The header, the table of contents and the small sections.
    Fast,
    /// Every byte and every invariant.
    Deep,
}

/// One thing that is wrong with the file, or worth a note.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Problem {
    /// The section it is in; `None` for the header blocks, the table of contents and
    /// the file as a whole.
    pub section: Option<String>,
    /// What is wrong: one sentence, without a full stop.
    pub what: String,
}

/// The nodes and entries of one adjacency that was checked.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct AdjacencyFacts {
    /// Its name: `adjacency.out` for the sections `adjacency.out.index` and `.data`.
    pub name: String,
    /// Its number of nodes.
    pub nodes: u32,
    /// Its number of entries (pairs).
    pub entries: u64,
    /// Its stored events, when it has a `.events` section (some pair has more than one).
    pub events: Option<u64>,
}

/// What [`verify`] found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Report {
    /// The generation the file reads as; `None` when it could not be opened.
    pub generation: Option<u64>,
    /// A newer commit that the file holds but that cannot be read, so that the file
    /// reads as [`generation`](Self::generation): its table of contents is damaged, or
    /// its header block is damaged or was cut short while it was written. No reader can
    /// tell these from a commit that never finished, so it is not a problem
    /// (`design/DECISIONS.md`, "A lost newest commit").
    pub lost_generation: Option<u64>,
    /// Sections in the table of contents.
    pub sections: usize,
    /// Sections whose hash was checked.
    pub sections_hashed: usize,
    /// Bytes hashed, padding not included.
    pub bytes_hashed: u64,
    /// The adjacencies whose structure was checked and found sound.
    pub adjacencies: Vec<AdjacencyFacts>,
    /// Everything that is wrong, header first, then in the order of the file.
    pub problems: Vec<Problem>,
    /// What may be damage or the trace of a commit that was cut short, which no reader
    /// can tell apart, and which leaves what the file holds whole.
    pub notes: Vec<Problem>,
    /// Checks that could not be made, each one sentence without a full stop.
    pub unchecked: Vec<String>,
}

impl Report {
    /// Whether no problem was found.
    #[must_use]
    pub fn is_intact(&self) -> bool {
        self.problems.is_empty()
    }

    /// Adds a problem: how callers add what they check beyond this crate (the
    /// identifier dictionary).
    pub fn problem(&mut self, section: Option<&str>, what: impl Into<String>) {
        self.problems.push(Problem {
            section: section.map(str::to_owned),
            what: what.into(),
        });
    }
}

/// Checks the graph file held by `source`. Buffers of the structural check are reserved
/// against `memory`; what does not fit is reported as unchecked.
///
/// A file that cannot be opened as a container is a problem of the report, not an
/// error: "not a graph file" is something `verify` finds.
///
/// # Errors
///
/// [`ReadError::Backend`] when the source fails to deliver bytes (an I/O error): the
/// file could not be checked, which is not the same as damage.
pub fn verify<S: ByteSource + Sync>(
    source: &S,
    depth: Depth,
    memory: &Memory,
) -> Result<Report, ReadError> {
    let mut report = Report::default();
    let file = match Container::open(source) {
        Ok(file) => file,
        Err(ContainerError::Read(e @ ReadError::Backend(_))) => return Err(e),
        Err(e) => {
            report.problem(None, e.to_string());
            return Ok(report);
        }
    };
    report.generation = Some(file.generation());
    report.sections = file.entries().len();
    check_commit(&file, &mut report)?;
    check_sections(&file, depth, &mut report)?;
    check_names(&file, &mut report);
    if depth == Depth::Deep {
        check_adjacencies(&file, memory, &mut report)?;
    }
    Ok(report)
}

/// The header blocks, the fallback state, the file's length and the table's padding.
fn check_commit<S: ByteSource>(file: &Container<S>, report: &mut Report) -> Result<(), ReadError> {
    let source = file.source();
    report.lost_generation = file.recovered_from();
    for index in 0..2u32 {
        if file.unreadable_block() == Some(index) {
            report.problem(None, format!("header block {index} cannot be read"));
            continue;
        }
        let start = u64::from(index) * BLOCK_USIZE as u64;
        let end = (start + BLOCK_USIZE as u64).min(source.len());
        let mut block = vec![0u8; small(end.saturating_sub(start))];
        source.read_at(start, &mut block)?;
        let (head, rest) = block.split_at(HEADER_USED.min(block.len()));
        // No commit writes there (spec § 2).
        if rest.iter().any(|&b| b != 0) {
            report.problem(
                None,
                format!("header block {index}: bytes 128 to 4095 are not zero"),
            );
        }
        match layout::decode_header(head, index) {
            Block::Valid(_) => {}
            Block::Absent if head.iter().all(|&b| b == 0) => {}
            // Not the block the file reads from (that one is valid): a commit cut short
            // leaves any bytes here, and the next commit writes the block whole (§ 8).
            _ => {
                report.notes.push(Problem {
                    section: None,
                    what: format!(
                        "header block {index} holds no valid header: a commit was cut short, \
                         or the block is damaged; the file reads as generation {}",
                        file.generation()
                    ),
                });
                if let Some(newer) = newer_table(source, head, file.generation())? {
                    report.lost_generation = report.lost_generation.max(Some(newer));
                }
            }
        }
    }
    let header = file.header();
    let toc_end = header.toc_offset + header.toc_len;
    let committed = file.committed_len();
    if source.len() < committed {
        report.problem(
            None,
            format!(
                "the file ends at byte {}, before the end of its last commit ({committed})",
                source.len()
            ),
        );
    } else if !is_zero(source, toc_end, committed)? {
        report.problem(None, "the padding after the table of contents is not zero");
    }
    Ok(())
}

/// The generation an invalid header block `head` names, when it is newer than
/// `generation` and the table of contents it points to is in the file and matches its
/// hash: a commit whose sections and table are whole, lost by its header block alone.
fn newer_table<S: ByteSource>(
    source: &S,
    head: &[u8],
    generation: u64,
) -> Result<Option<u64>, ReadError> {
    if head.len() < HEADER_USED || head[..8] != MAGIC {
        return Ok(None);
    }
    let u64_at = |at: usize| u64::from_le_bytes(head[at..at + 8].try_into().expect("8 bytes"));
    let (named, offset, len) = (u64_at(24), u64_at(48), u64_at(56));
    let fits = offset
        .checked_add(len)
        .is_some_and(|end| end <= source.len());
    if named <= generation || len > MAX_TOC_BYTES || !fits {
        return Ok(None);
    }
    let mut toc = vec![0u8; small(len)];
    source.read_at(offset, &mut toc)?;
    Ok((blake3::hash(&toc).as_bytes()[..] == head[64..96]).then_some(named))
}

/// Whether bytes `start..end` of `source` are zero (a range of at most one block).
fn is_zero<S: ByteSource>(source: &S, start: u64, end: u64) -> Result<bool, ReadError> {
    let mut bytes = vec![0u8; small(end - start)];
    source.read_at(start, &mut bytes)?;
    Ok(bytes.iter().all(|&b| b == 0))
}

/// The hash and padding of each section, in parallel.
fn check_sections<S: ByteSource + Sync>(
    file: &Container<S>,
    depth: Depth,
    report: &mut Report,
) -> Result<(), ReadError> {
    let mut chosen: Vec<&Entry> = file
        .entries()
        .iter()
        .filter(|e| depth == Depth::Deep || e.len <= FAST_SECTION_BYTES)
        .collect();
    chosen.sort_by_key(|e| e.offset);
    let found = par::map_ranges(chosen.len(), NonZeroUsize::MIN, |range| {
        range
            .map(|i| check_section(file, chosen[i]))
            .collect::<Vec<_>>()
    });
    for (entry, outcome) in chosen.iter().zip(found.into_iter().flatten()) {
        if let Some(what) = outcome? {
            report.problem(Some(&entry.name), what);
        }
        report.sections_hashed += 1;
        report.bytes_hashed += entry.len;
    }
    Ok(())
}

/// What is wrong with one section's bytes, if anything. Opening has checked that every
/// section and its padding end before the table of contents, inside the file.
fn check_section<S: ByteSource>(
    file: &Container<S>,
    entry: &Entry,
) -> Result<Option<&'static str>, ReadError> {
    let source = file.source();
    let hash = hash_range(source, entry.offset, entry.len).map_err(|e| match e {
        ContainerError::Read(e) => e,
        other => ReadError::Backend(other.into()),
    })?;
    if hash != entry.hash {
        return Ok(Some("its bytes do not match its hash"));
    }
    let end = entry.offset + entry.len;
    if !is_zero(source, end, padded(entry.offset, entry.len))? {
        return Ok(Some("its padding is not zero"));
    }
    Ok(None)
}

/// The names the adjacency format gives its sections, against their kinds (adjacency
/// spec § 3.1, § 5): `<adj>.index` (kind 2) and `<adj>.data` (kind 3) in pairs, and
/// `<adj>.events` (kind 5, and only that kind) beside an index.
fn check_names<S: ByteSource>(file: &Container<S>, report: &mut Report) {
    let has = |name: &str, kind: Kind| file.get(name).is_some_and(|e| e.kind == kind);
    for e in file.entries() {
        let name = &*e.name;
        let (suffix, partner) = match e.kind {
            Kind::ADJACENCY_INDEX => (".index", (".data", Kind::ADJACENCY_DATA)),
            Kind::ADJACENCY_DATA => (".data", (".index", Kind::ADJACENCY_INDEX)),
            Kind::ADJACENCY_EVENTS => (".events", (".index", Kind::ADJACENCY_INDEX)),
            _ if name.ends_with(".events") => {
                report.problem(
                    Some(name),
                    "it is named like an events section but is not one",
                );
                continue;
            }
            _ => continue,
        };
        match name.strip_suffix(suffix) {
            None => report.problem(
                Some(name),
                format!("an adjacency section whose name does not end in {suffix}"),
            ),
            Some(base) if !has(&format!("{base}{}", partner.0), partner.1) => report.problem(
                Some(name),
                format!("the adjacency has no {base}{} section", partner.0),
            ),
            Some(_) => {}
        }
    }
}

/// What one pass over the lists of an adjacency found.
#[derive(Clone, Copy, Debug, Default)]
struct Scan {
    loops: u64,
    /// The sum of `pair(v, t)` over every entry `t` of the list of `v`.
    forward: u64,
    /// The sum of `pair(t, v)` over the same entries.
    backward: u64,
}

impl Scan {
    fn add(self, other: Self) -> Self {
        Self {
            loops: self.loops + other.loops,
            forward: self.forward.wrapping_add(other.forward),
            backward: self.backward.wrapping_add(other.backward),
        }
    }
}

/// A 64-bit hash of the ordered pair `(u, v)`: the finalizer of `SplitMix64`, a
/// bijection, so two different pairs never share a hash. Damage makes the sums of
/// [`Scan`] of two different sets of pairs equal only by a 2^-64 accident; a file
/// crafted to make them collide can, which the section hashes do not prevent either.
fn pair(u: u32, v: u32) -> u64 {
    let mut z = (u64::from(u) << 32) | u64::from(v);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Why a scan stopped.
enum Stop {
    /// A list is wrong: at this node, this.
    Bad(u32, &'static str),
    /// A buffer does not fit in the memory budget or the single-allocation limit.
    NoMemory,
    /// The source failed.
    Read(ReadError),
}

/// The bytes of one section: borrowed when the source is in memory or mapped, read in
/// pieces otherwise.
struct Bytes<'f, S> {
    source: &'f S,
    mapped: Option<&'f [u8]>,
    offset: u64,
    len: u64,
}

impl<'f, S: ByteSource> Bytes<'f, S> {
    fn of(file: &'f Container<S>, entry: &Entry) -> Self {
        let mapped = usize::try_from(entry.offset)
            .ok()
            .zip(usize::try_from(entry.offset + entry.len).ok())
            .zip(file.source().as_slice())
            .and_then(|((start, end), all)| all.get(start..end));
        Self {
            source: file.source(),
            mapped,
            offset: entry.offset,
            len: entry.len,
        }
    }

    /// Bytes `range` of the section, borrowed or read into `buf` (reserved in `held`).
    fn get<'b>(
        &self,
        start: u64,
        end: u64,
        buf: &'b mut Vec<u8>,
        held: &mut Reservation,
    ) -> Result<&'b [u8], Stop>
    where
        'f: 'b,
    {
        if let Some(all) = self.mapped {
            return Ok(&all[small(start)..small(end)]);
        }
        let len = end - start;
        if len > MAX_READ || held.try_resize(held.bytes().max(len)).is_err() {
            return Err(Stop::NoMemory);
        }
        buf.resize(small(len), 0);
        self.source
            .read_at(self.offset + start, buf)
            .map_err(Stop::Read)?;
        Ok(buf)
    }
}

/// Decodes every list of an adjacency, in parallel by blocks, and checks it.
fn scan<S: ByteSource + Sync>(
    index: &AdjacencyIndex,
    data: &Bytes<'_, S>,
    memory: &Memory,
) -> Result<Scan, Stop> {
    let nodes = index.nodes();
    let decoder = Decoder::best();
    let per_task = par::map_ranges(index.blocks(), BLOCKS_PER_TASK, |blocks| {
        let (mut piece, mut list) = (Vec::new(), Vec::new());
        let mut piece_held = Reservation::empty(memory, PURPOSE, MemoryKind::Anonymous);
        let mut list_held = Reservation::empty(memory, PURPOSE, MemoryKind::Anonymous);
        let mut found = Scan::default();
        for b in blocks {
            let block = index.block_nodes(b);
            let start = index.record_range(NodeId::new(block.start)).start;
            let end = index.record_range(NodeId::new(block.end - 1)).end;
            let bytes = data.get(start, end, &mut piece, &mut piece_held)?;
            for v in block {
                let d = index.degree(NodeId::new(v));
                if d == 0 {
                    continue;
                }
                // Room for the SIMD decoders' last group (the codec's `Batch` keeps it).
                let room = usize::try_from(d).map_or(usize::MAX, |d| d.saturating_add(3));
                let need = (room as u64).saturating_mul(4);
                if need > MAX_READ || list_held.try_resize(list_held.bytes().max(need)).is_err() {
                    return Err(Stop::NoMemory);
                }
                if list.len() < room {
                    list.resize(room, 0);
                }
                let record = index.record_range(NodeId::new(v));
                let corrupt = || Stop::Bad(v, "its record does not decode");
                let rec = &bytes[small(record.start - start)..];
                let used = decoder
                    .list(rec, v, small(u64::from(d)), small(u64::from(d)), &mut list)
                    .map_err(|_| corrupt())?;
                if used as u64 != record.end - record.start {
                    return Err(corrupt());
                }
                let mut previous = None;
                for &t in &list[..small(u64::from(d))] {
                    if t >= nodes {
                        return Err(Stop::Bad(v, "a neighbour is not below the number of nodes"));
                    }
                    if previous.is_some_and(|p| t <= p) {
                        return Err(Stop::Bad(v, "its list is not strictly increasing"));
                    }
                    previous = Some(t);
                    found.loops += u64::from(t == v);
                    found.forward = found.forward.wrapping_add(pair(v, t));
                    found.backward = found.backward.wrapping_add(pair(t, v));
                }
            }
        }
        Ok(found)
    });
    per_task
        .into_iter()
        .try_fold(Scan::default(), |all, task| Ok(all.add(task?)))
}

/// One adjacency whose lists were decoded and found sound.
struct Checked {
    facts: AdjacencyFacts,
    scan: Scan,
}

/// The parameters of each provenance record, decoded at most once (spec § 7, "No
/// amplification").
type Declared = BTreeMap<Arc<str>, Option<BTreeMap<String, String>>>;

/// The structure of every adjacency, and the in-adjacencies against their out-adjacency.
fn check_adjacencies<S: ByteSource + Sync>(
    file: &Container<S>,
    memory: &Memory,
    report: &mut Report,
) -> Result<(), ReadError> {
    let mut declared = Declared::new();
    let mut checked = BTreeMap::new();
    for e in file.entries() {
        let Some(base) = e.name.strip_suffix(".index") else {
            continue;
        };
        if e.kind == Kind::ADJACENCY_INDEX {
            if let Some(c) = check_adjacency(file, base, memory, &mut declared, report)? {
                checked.insert(base.to_owned(), c);
            }
        }
    }
    for (name, out) in &checked {
        let Some(base) = name.strip_suffix(".out") else {
            continue;
        };
        let Some(inn) = checked.get(&format!("{base}.in")) else {
            continue;
        };
        if out.facts.nodes != inn.facts.nodes
            || out.facts.entries != inn.facts.entries
            || out.facts.events != inn.facts.events
            || out.scan.forward != inn.scan.backward
        {
            report.problem(
                Some(&format!("{base}.in.data")),
                format!("the in-adjacency is not the transpose of {base}.out"),
            );
        }
    }
    report.adjacencies = checked.into_values().map(|c| c.facts).collect();
    Ok(())
}

/// The bytes of section `entry` (an index or a `.events` section), whole, as their
/// parsers read them; `None` when they do not fit in `held`.
fn section_bytes<'f, S: ByteSource>(
    file: &'f Container<S>,
    entry: &Entry,
    held: &mut Reservation,
) -> Result<Option<Cow<'f, [u8]>>, ReadError> {
    let bytes = Bytes::of(file, entry);
    if let Some(all) = bytes.mapped {
        return Ok(Some(Cow::Borrowed(all)));
    }
    let mut buf = Vec::new();
    match bytes.get(0, bytes.len, &mut buf, held).map(drop) {
        Ok(()) => Ok(Some(Cow::Owned(buf))),
        Err(Stop::Read(e)) => Err(e),
        Err(_) => Ok(None),
    }
}

fn check_adjacency<S: ByteSource + Sync>(
    file: &Container<S>,
    base: &str,
    memory: &Memory,
    declared: &mut Declared,
    report: &mut Report,
) -> Result<Option<Checked>, ReadError> {
    let (index_name, data_name) = (format!("{base}.index"), format!("{base}.data"));
    let events_name = format!("{base}.events");
    let (Some(index_entry), Some(data_entry)) = (file.get(&index_name), file.get(&data_name))
    else {
        return Ok(None);
    };
    for name in [&index_name, &data_name] {
        if let Err(e) = file.section(name) {
            report.problem(Some(name), e.to_string());
            return Ok(None);
        }
    }
    let unchecked = |report: &mut Report| {
        report.unchecked.push(format!(
            "{base}: its structure (not enough memory for its buffers)"
        ));
    };
    let mut held = Reservation::empty(memory, PURPOSE, MemoryKind::Anonymous);
    let Some(index_bytes) = section_bytes(file, index_entry, &mut held)? else {
        unchecked(report);
        return Ok(None);
    };
    let index = match AdjacencyIndex::parse(&index_bytes) {
        Ok(index) => index,
        Err(e) => {
            report.problem(Some(&index_name), e.to_string());
            return Ok(None);
        }
    };
    // Index flag bit 1 is set exactly when the `.events` section exists (spec § 3, § 5).
    let events = match (index.has_events(), file.get(&events_name)) {
        (false, None) => None,
        (false, Some(_)) => {
            report.problem(
                Some(&events_name),
                format!("an events section, but {index_name} does not announce one"),
            );
            None
        }
        (true, None) => {
            report.problem(
                Some(&index_name),
                format!("it announces an events section, and {events_name} is missing"),
            );
            None
        }
        (true, Some(entry)) => {
            let Some(bytes) = section_bytes(file, entry, &mut held)? else {
                unchecked(report);
                return Ok(None);
            };
            match AdjacencyEvents::parse(&bytes, &index).and_then(|e| {
                e.check()?;
                Ok(e.events())
            }) {
                Ok(events) => Some(events),
                Err(e) => {
                    report.problem(Some(&events_name), e.to_string());
                    None
                }
            }
        }
    };
    let data = Bytes::of(file, data_entry);
    if data.len != index.data_bytes() {
        let e = FormatError::Length {
            expected: index.data_bytes(),
            actual: data.len,
        };
        report.problem(Some(&data_name), e.to_string());
        return Ok(None);
    }
    let scan = match scan(&index, &data, memory) {
        Ok(scan) => scan,
        Err(Stop::Bad(node, what)) => {
            report.problem(Some(&data_name), format!("node {node}: {what}"));
            return Ok(None);
        }
        Err(Stop::NoMemory) => {
            unchecked(report);
            return Ok(None);
        }
        Err(Stop::Read(e)) => return Err(e),
    };
    let params = index_entry.provenance.as_ref().and_then(|record| {
        declared
            .entry(Arc::clone(record))
            .or_insert_with(|| file.read_provenance(record).ok().map(|p| p.parameters))
            .as_ref()
    });
    let param = |key: &str| params.and_then(|p| p.get(key)).map(String::as_str);
    match param("self_loops") {
        Some("drop") if scan.loops > 0 => report.problem(
            Some(&data_name),
            format!(
                "{} self-loops, but the graph was built with self-loops dropped",
                scan.loops
            ),
        ),
        Some("drop" | "keep") => {}
        _ => report.unchecked.push(format!(
            "{base}: self-loops (its provenance does not say whether they were kept)"
        )),
    }
    // An in-adjacency is checked against its out-adjacency, not for symmetry.
    if !base.ends_with(".in") {
        match param("directed") {
            Some("false") if scan.forward != scan.backward => report.problem(
                Some(&data_name),
                "the lists are not symmetric, but the graph is undirected",
            ),
            Some("false" | "true") => {}
            _ => report.unchecked.push(format!(
                "{base}: symmetry (its provenance does not say whether the graph is directed)"
            )),
        }
    }
    Ok(Some(Checked {
        facts: AdjacencyFacts {
            name: base.to_owned(),
            nodes: index.nodes(),
            entries: index.entries(),
            events,
        },
        scan,
    }))
}
