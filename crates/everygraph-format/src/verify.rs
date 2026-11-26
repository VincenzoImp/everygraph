// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Checking a graph file: what `everygraph verify` runs (issue #41).
//!
//! [`verify`] checks a `.eg` container against `docs/spec/eg-container.md` and every
//! adjacency in it against `docs/spec/eg-adjacency.md` § 5. It does not stop at the
//! first problem: the [`Report`] lists each one with the section it is in.
//!
//! - [`Depth::Fast`] reads a bounded part of the file: both header blocks (bytes 128 to
//!   4095 zero; the other block valid or empty, else a note), the table of contents, the file's
//!   length and the table's padding, the names and kinds of the adjacency sections, and
//!   the hash and padding of every section of at most [`FAST_SECTION_BYTES`].
//! - [`Depth::Deep`] reads everything: the hash and padding of every section, and the
//!   structure of every adjacency — its index, every record, lists strictly increasing
//!   with every neighbour below the node count; no self-loops and symmetric lists when
//!   its provenance declares `self_loops` = `drop` and `directed` = `false` (spec § 6);
//!   and an in-adjacency (`<base>.in`) that is the transpose of its out-adjacency.
//!
//! Sections are hashed in parallel (`everygraph_core::par`), each in 1 MiB pieces unless
//! the source is in memory or mapped; lists are decoded block by block, in parallel.
//! Symmetry and transposition are compared through a sum of pair hashes, so they cost
//! constant memory per block: a file larger than memory is checked through a memory map.
//!
//! The identifier dictionary (`ids.*`) is checked by `everygraph_ingest::ids`, which this
//! crate does not depend on.

use std::borrow::Cow;
use std::num::NonZeroUsize;

use everygraph_core::{ByteSource, ReadError, par};

use crate::adjacency::{Adjacency, AdjacencyIndex, Batch};
use crate::container::layout::{self, BLOCK_USIZE, Block, HEADER_USED, padded, small};
use crate::container::read::hash_range;
use crate::container::{Container, ContainerError, Entry, Kind};

/// The largest section whose hash [`Depth::Fast`] checks.
pub const FAST_SECTION_BYTES: u64 = 1 << 20;

/// Blocks of an adjacency decoded by one task.
const BLOCKS_PER_TASK: NonZeroUsize = NonZeroUsize::new(8).expect("8 > 0");

/// How much of the file [`verify`] reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Depth {
    /// The header, the table of contents and the small sections.
    Fast,
    /// Every byte and every invariant.
    Deep,
}

/// One thing that is wrong with the file.
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
}

/// What [`verify`] found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Report {
    /// The generation the file reads as; `None` when it could not be opened.
    pub generation: Option<u64>,
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

/// Checks the graph file held by `source`.
///
/// A file that cannot be opened as a container is a problem of the report, not an
/// error: "not a graph file" is something `verify` finds.
///
/// # Errors
///
/// [`ReadError::Backend`] when the source fails to deliver bytes (an I/O error): the
/// file could not be checked, which is not the same as damage.
pub fn verify<S: ByteSource + Sync>(source: &S, depth: Depth) -> Result<Report, ReadError> {
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
        check_adjacencies(&file, &mut report)?;
    }
    Ok(report)
}

/// The header blocks, the fallback state, the file's length and the table's padding.
fn check_commit<S: ByteSource>(file: &Container<S>, report: &mut Report) -> Result<(), ReadError> {
    let source = file.source();
    if let Some(newer) = file.recovered_from() {
        report.problem(
            None,
            format!(
                "the newest commit (generation {newer}) cannot be used; the file reads as \
                 generation {}",
                file.generation()
            ),
        );
    }
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
            _ => report.notes.push(Problem {
                section: None,
                what: format!(
                    "header block {index} holds no valid header: a commit was cut short, or \
                     the block is damaged; the file reads as generation {}",
                    file.generation()
                ),
            }),
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

/// What is wrong with one section's bytes, if anything.
fn check_section<S: ByteSource>(
    file: &Container<S>,
    entry: &Entry,
) -> Result<Option<&'static str>, ReadError> {
    let source = file.source();
    let end = entry.offset + entry.len;
    if source.len() < padded(entry.offset, entry.len) {
        return Ok(Some("the file ends inside it or its padding"));
    }
    if hash_range(source, entry.offset, entry.len).map_err(read_error)? != entry.hash {
        return Ok(Some("its bytes do not match its hash"));
    }
    if !is_zero(source, end, padded(entry.offset, entry.len))? {
        return Ok(Some("its padding is not zero"));
    }
    Ok(None)
}

fn read_error(e: ContainerError) -> ReadError {
    match e {
        ContainerError::Read(e) => e,
        other => ReadError::Backend(other.into()),
    }
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
            Kind::ADJACENCY_DATA | Kind::ADJACENCY_EVENTS => {
                let own = if e.kind == Kind::ADJACENCY_DATA {
                    ".data"
                } else {
                    ".events"
                };
                (own, (".index", Kind::ADJACENCY_INDEX))
            }
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

/// A 64-bit hash of the ordered pair `(u, v)`: the finalizer of `SplitMix64`, a bijection, so
/// two different pairs never share a hash and the sums of [`Scan`] are equal for two
/// sets of pairs only by a 2^-64 accident.
fn pair(u: u32, v: u32) -> u64 {
    let mut z = (u64::from(u) << 32) | u64::from(v);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Decodes every list of `adj`, in parallel by blocks, and checks it.
fn scan(adj: Adjacency<'_>) -> Result<Scan, (u32, &'static str)> {
    let index = adj.index();
    let nodes = index.nodes();
    let per_task = par::map_ranges(index.blocks(), BLOCKS_PER_TASK, |blocks| {
        let mut batch = Batch::default();
        let mut found = Scan::default();
        for b in blocks {
            adj.decode_nodes(index.block_nodes(b), usize::MAX, &mut batch)
                .map_err(|e| (e.first, "its record does not decode"))?;
            for (v, list) in batch.iter() {
                let v = v.get();
                let mut previous = None;
                for &t in list {
                    if t >= nodes {
                        return Err((v, "a neighbour is not below the number of nodes"));
                    }
                    if previous.is_some_and(|p| t <= p) {
                        return Err((v, "its list is not strictly increasing"));
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

/// The bytes of section `name`: borrowed from a source in memory or mapped, otherwise
/// read; `None` (with the problem reported) when they cannot be had.
fn bytes<'f, S: ByteSource>(
    file: &'f Container<S>,
    name: &str,
    report: &mut Report,
) -> Result<Option<Cow<'f, [u8]>>, ReadError> {
    let entry = match file.section(name) {
        Ok(_) => file.get(name).expect("the section exists"),
        Err(e) => {
            report.problem(Some(name), e.to_string());
            return Ok(None);
        }
    };
    let range = usize::try_from(entry.offset)
        .ok()
        .zip(usize::try_from(entry.offset + entry.len).ok());
    if let Some(slice) = file
        .source()
        .as_slice()
        .zip(range)
        .and_then(|(all, (start, end))| all.get(start..end))
    {
        return Ok(Some(Cow::Borrowed(slice)));
    }
    let mut buf = Vec::new();
    let Some(len) = usize::try_from(entry.len)
        .ok()
        .filter(|&len| buf.try_reserve_exact(len).is_ok())
    else {
        report
            .unchecked
            .push(format!("{name}: not enough memory to read it"));
        return Ok(None);
    };
    buf.resize(len, 0);
    match file.source().read_at(entry.offset, &mut buf) {
        Ok(()) => Ok(Some(Cow::Owned(buf))),
        // The section check has reported a file that ends inside it.
        Err(ReadError::OutOfBounds { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

/// One adjacency whose lists were decoded and found sound.
struct Checked {
    facts: AdjacencyFacts,
    scan: Scan,
}

/// The structure of every adjacency, and the in-adjacencies against their out-adjacency.
fn check_adjacencies<S: ByteSource + Sync>(
    file: &Container<S>,
    report: &mut Report,
) -> Result<(), ReadError> {
    let bases: Vec<String> = file
        .entries()
        .iter()
        .filter(|e| e.kind == Kind::ADJACENCY_INDEX)
        .filter_map(|e| e.name.strip_suffix(".index"))
        .map(str::to_owned)
        .collect();
    let mut checked = Vec::with_capacity(bases.len());
    for base in &bases {
        if let Some(c) = check_adjacency(file, base, report)? {
            checked.push(c);
        }
    }
    for out in &checked {
        let Some(base) = out.facts.name.strip_suffix(".out") else {
            continue;
        };
        let Some(inn) = checked
            .iter()
            .find(|c| c.facts.name == format!("{base}.in"))
        else {
            continue;
        };
        if out.facts.nodes != inn.facts.nodes
            || out.facts.entries != inn.facts.entries
            || out.scan.forward != inn.scan.backward
        {
            report.problem(
                Some(&format!("{base}.in.data")),
                format!("the in-adjacency is not the transpose of {base}.out"),
            );
        }
    }
    report.adjacencies = checked.into_iter().map(|c| c.facts).collect();
    Ok(())
}

fn check_adjacency<S: ByteSource + Sync>(
    file: &Container<S>,
    base: &str,
    report: &mut Report,
) -> Result<Option<Checked>, ReadError> {
    let (index_name, data_name) = (format!("{base}.index"), format!("{base}.data"));
    let events_name = format!("{base}.events");
    if file.get(&data_name).is_none() {
        return Ok(None);
    }
    let Some(index_bytes) = bytes(file, &index_name, report)? else {
        return Ok(None);
    };
    let index = match AdjacencyIndex::parse(&index_bytes) {
        Ok(index) => index,
        Err(e) => {
            report.problem(Some(&index_name), e.to_string());
            return Ok(None);
        }
    };
    // This version's index refuses flag bit 1, so a parsed index never announces one.
    if file.get(&events_name).is_some() {
        report.problem(
            Some(&events_name),
            format!("an events section, but {index_name} does not announce one"),
        );
    }
    let Some(data) = bytes(file, &data_name, report)? else {
        return Ok(None);
    };
    let adj = match Adjacency::new(&index, &data) {
        Ok(adj) => adj,
        Err(e) => {
            report.problem(Some(&data_name), e.to_string());
            return Ok(None);
        }
    };
    let scan = match scan(adj) {
        Ok(scan) => scan,
        Err((node, what)) => {
            report.problem(Some(&data_name), format!("node {node}: {what}"));
            return Ok(None);
        }
    };
    let declared = |key: &str| {
        file.provenance(&index_name)
            .ok()
            .flatten()
            .and_then(|p| p.parameters.get(key).cloned())
    };
    match declared("self_loops").as_deref() {
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
        match declared("directed").as_deref() {
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
        },
        scan,
    }))
}
