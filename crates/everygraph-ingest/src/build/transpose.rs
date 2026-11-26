// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The in-adjacency of a directed graph (DESIGN D6, ADR 0006): the transpose of the
//! out-adjacency, stored as `adjacency.in.*` beside it and derived from it alone.
//!
//! It is built by the passes of the out-adjacency (module `lists`) with other items:
//! for each entry `(u, v)` of the out-adjacency, the item `(v, u)`, once per stored
//! event. Blocks of out-lists are decoded in parallel; the counting sort into buckets of
//! target nodes is stable and each list is sorted when its bucket is finished, so the
//! bytes do not depend on the thread count or the budget, and the buckets that do not
//! fit the budget spill as the out-adjacency's do. A pair's events become as many items,
//! so the in-adjacency has a `.events` section exactly when the out-adjacency has one,
//! with the same `events` and, entry by entry, the multiplicity of the reversed pair
//! (`docs/spec/eg-adjacency.md` § 3.1, § 5).
//!
//! The columns stored beside the out-adjacency are stored beside the in-adjacency too,
//! each in its order (`docs/spec/eg-columns.md` § 6): each item carries the values the
//! out-adjacency stores for the event it reverses ([`Copied`]), which are copied, not
//! computed again.
//!
//! [`add_in_adjacency`] adds it to a graph file built without it; [`build`](super::build)
//! makes it for every directed graph unless told not to.

use std::num::NonZeroUsize;

use std::ops::Range;

use everygraph_core::{ByteSink, ByteSource, Memory, MemoryKind, NodeId, Reservation};
use everygraph_format::adjacency::{
    Adjacency, AdjacencyEvents, AdjacencyIndex, Batch, FormatError,
};
use everygraph_format::columns::{ColumnLayout, SectionNames};
use everygraph_format::container::{Container, ContainerError, Entry, Writer};

use super::columns::{Copied, Item, Out, outputs_of};
use super::lists::{Context, Names, fill_lists, write_lists};
use super::rows::{BlockCounts, Items};
use super::{
    BuildError, Duplicates, GraphOptions, IN_DATA, IN_EVENTS, IN_INDEX, OUT_DATA, OUT_EVENTS,
    OUT_INDEX, PROVENANCE, Phase, Progress, SelfLoops, TUNING, Tuning, read_section, widen,
};
use crate::ids::Scratch;

/// The section names of the in-adjacency.
pub(super) const IN: Names = Names {
    adjacency: "adjacency.in",
    data: IN_DATA,
    index: IN_INDEX,
    events: IN_EVENTS,
};

/// What the transposition wrote.
#[derive(Clone, Copy, Debug)]
pub(super) struct Transposed {
    /// Bytes written to spill files.
    pub(super) spilled_bytes: u64,
    /// Whether every bucket was held in memory.
    pub(super) in_memory: bool,
    /// Buckets of target nodes.
    pub(super) buckets: usize,
}

/// Rows of a column read at a time.
const PIECE_ROWS: u64 = 512;

/// A column of the out-adjacency, as the transposition reads it: 8-byte values without
/// nulls (`docs/spec/eg-columns.md` § 3), batch after batch.
struct OutColumn {
    per_event: bool,
    /// Where the column section starts in the container.
    section: u64,
    batch_rows: u64,
    /// Where each batch's values start in the section.
    batches: Vec<u64>,
}

impl OutColumn {
    /// Column `column` of the out-adjacency in `writer`, which must have `rows` rows.
    fn open<K: ByteSink>(
        writer: &Writer<K>,
        column: &Out,
        (entries, events): (u64, u64),
        held: &mut Reservation,
    ) -> Result<Self, BuildError> {
        let per_event = column.per_event();
        let scope = if per_event {
            "adjacency.out.event"
        } else {
            "adjacency.out.pair"
        };
        let names = SectionNames::new(scope, &column.name);
        let bytes = read_section(
            writer.sink(),
            writer.get(&names.layout),
            &names.layout,
            held,
        )?;
        let layout = ColumnLayout::parse(&bytes)?;
        let rows = if per_event { events } else { entries };
        let nulls = layout.batches().iter().any(|b| b.null_count > 0);
        if layout.rows() != rows || layout.ty().width() != 8 || nulls {
            return Err(FormatError::Invalid(
                "a column of the out-adjacency that does not fit its scope",
            )
            .into());
        }
        let section = writer
            .get(&names.column)
            .ok_or_else(|| ContainerError::NoSuchSection(names.column.clone()))?
            .offset;
        Ok(Self {
            per_event,
            section,
            batch_rows: layout.batch_rows(),
            batches: layout.batches().iter().map(|b| b.values.start).collect(),
        })
    }

    /// Appends the values of `rows` to `out`.
    fn read<R: ByteSource>(
        &self,
        source: &R,
        rows: Range<u64>,
        out: &mut Vec<u64>,
        bytes: &mut Vec<u8>,
    ) -> Result<(), BuildError> {
        let mut row = rows.start;
        while row < rows.end {
            let (batch, within) = (row / self.batch_rows, row % self.batch_rows);
            let n = (rows.end - row)
                .min(self.batch_rows - within)
                .min(PIECE_ROWS);
            let start = self.batches[to_usize(batch)] + within * 8;
            bytes.resize(to_usize(n * 8), 0);
            source
                .read_at(self.section + start, bytes)
                .map_err(|e| BuildError::Container(ContainerError::Read(e)))?;
            out.extend(
                bytes
                    .chunks_exact(8)
                    .map(|b| u64::from_le_bytes(b.try_into().expect("8 bytes"))),
            );
            row += n;
        }
        Ok(())
    }
}

/// The items of the in-adjacency, read from the out-adjacency.
struct Reversed<'a, R> {
    index: &'a AdjacencyIndex,
    events: Option<&'a AdjacencyEvents<'a>>,
    /// The container's bytes, and where the data section starts in them.
    source: &'a R,
    data_at: u64,
    /// The data section, when the source holds it in memory.
    mapped: Option<&'a [u8]>,
    /// The out-adjacency's columns, carried by [`Copied`] items.
    columns: Vec<OutColumn>,
    /// First node of each block, then the node count.
    starts: Vec<u32>,
    /// The buffers of the blocks in flight.
    _buffers: Reservation,
}

impl<'a, R: ByteSource + Sync> Reversed<'a, R> {
    #[expect(clippy::too_many_arguments, reason = "the parts of the out-adjacency")]
    fn new(
        index: &'a AdjacencyIndex,
        events: Option<&'a AdjacencyEvents<'a>>,
        source: &'a R,
        data: &Entry,
        columns: Vec<OutColumn>,
        memory: &Memory,
        threads: usize,
        block_entries: u64,
    ) -> Result<Self, BuildError> {
        let entry = |v: u32| index.entry_offset(NodeId::new(v)).get();
        let event = |e: u64| -> Result<u64, FormatError> {
            match events {
                Some(ev) if e == ev.entries() => Ok(ev.events()),
                Some(ev) => ev.event_range(e).map(|r| r.start),
                None => Ok(e),
            }
        };
        let starts = block_starts(index, block_entries);
        let mapped = source.slice(data.offset..data.offset + data.len);
        // A block in flight holds its decoded lists, its records when the section is not
        // in memory, and the values of its columns.
        let (per_event, per_pair) = (
            widen(columns.iter().filter(|c| c.per_event).count()),
            widen(columns.iter().filter(|c| !c.per_event).count()),
        );
        let mut most = 0;
        for w in starts.windows(2) {
            let (first, last) = (entry(w[0]), entry(w[1]));
            let entries = last - first;
            let events = event(last)? - event(first)?;
            let lists = u64::from(w[1] - w[0]);
            let records = index.data_range(w[0]..w[1]);
            let read = if mapped.is_some() {
                0
            } else {
                records.end - records.start
            };
            // Each column's values, and the piece of bytes they are read through.
            let values = (events * per_event + entries * per_pair) * 8 + PIECE_ROWS * 8;
            most = most.max((entries + 8) * 4 + lists * 16 + read + values);
        }
        let buffers = memory.reserve(
            "out-lists in flight",
            MemoryKind::Anonymous,
            most.saturating_mul(widen(threads)),
        )?;
        Ok(Self {
            index,
            events,
            source,
            data_at: data.offset,
            mapped,
            columns,
            starts,
            _buffers: buffers,
        })
    }

    /// The offset of the first event of out-entry `entry` (`entries` gives `events`).
    fn event_of(&self, entry: u64) -> Result<u64, FormatError> {
        match self.events {
            Some(events) if entry == events.entries() => Ok(events.events()),
            Some(events) => events.event_range(entry).map(|r| r.start),
            None => Ok(entry),
        }
    }

    /// Calls `f(source, target, out event, out entry)` for each event of the out-lists
    /// of block `block`, reversed: in out-adjacency order.
    fn each(&self, block: usize, mut f: impl FnMut(u32, u32, u64, u64)) -> Result<(), BuildError> {
        let nodes = self.starts[block]..self.starts[block + 1];
        let range = self.index.data_range(nodes.clone());
        let mut read = Vec::new();
        let records = match self.mapped {
            Some(all) => &all[to_usize(range.start)..to_usize(range.end)],
            None => {
                read.resize(to_usize(range.end - range.start), 0);
                self.source
                    .read_at(self.data_at + range.start, &mut read)
                    .map_err(|e| BuildError::Container(ContainerError::Read(e)))?;
                &read[..]
            }
        };
        let window = Adjacency::window(self.index, nodes.clone(), records)?;
        let mut batch = Batch::default();
        window
            .decode_nodes(nodes.clone(), usize::MAX, &mut batch)
            .map_err(|_| FormatError::Invalid("out-adjacency records that do not decode"))?;
        let mut entry = self.index.entry_offset(NodeId::new(nodes.start)).get();
        let mut event = self.event_of(entry)?;
        let mut multiplicities = match self.events {
            Some(events) => Some(events.multiplicities_from(entry)?),
            None => None,
        };
        let n = self.index.nodes();
        for (v, list) in batch.iter() {
            let increasing = list.windows(2).all(|w| w[0] < w[1]);
            if !increasing || list.last().is_some_and(|&t| t >= n) {
                return Err(FormatError::Invalid(
                    "an out-list that is not strictly increasing or names a node beyond the graph",
                )
                .into());
            }
            for &t in list {
                let m = multiplicities.as_mut().map_or(Some(1), Iterator::next);
                for _ in 0..m.ok_or(FormatError::Invalid(".events ends before the entries"))? {
                    f(t, v.get(), event, entry);
                    event += 1;
                }
                entry += 1;
            }
        }
        Ok(())
    }
}

/// The first node of each block of out-lists the transposition reads, then the node
/// count: runs of the index's blocks of at least `block_entries` entries, the last one
/// shorter.
pub(super) fn block_starts(index: &AdjacencyIndex, block_entries: u64) -> Vec<u32> {
    let entry = |v: u32| index.entry_offset(NodeId::new(v)).get();
    let mut starts = vec![0];
    for b in 0..index.blocks() {
        let end = index.block_nodes(b).end;
        let first = *starts.last().expect("a start");
        if entry(end) - entry(first) >= block_entries || end == index.nodes() {
            starts.push(end);
        }
    }
    starts
}

impl<R: ByteSource + Sync> Items<u32> for Reversed<'_, R> {
    fn blocks(&self) -> usize {
        self.starts.len() - 1
    }

    fn items(&self, block: usize, mut f: impl FnMut(u32, u32)) -> Result<BlockCounts, BuildError> {
        self.each(block, |t, v, _, _| f(t, v))?;
        Ok(BlockCounts::default())
    }
}

impl<R: ByteSource + Sync, const N: usize> Items<Copied<N>> for Reversed<'_, R> {
    fn blocks(&self) -> usize {
        self.starts.len() - 1
    }

    fn items(
        &self,
        block: usize,
        mut f: impl FnMut(u32, Copied<N>),
    ) -> Result<BlockCounts, BuildError> {
        // The values of the block's events and entries, column by column.
        let entry = |v: u32| self.index.entry_offset(NodeId::new(v)).get();
        let (entry0, entry1) = (entry(self.starts[block]), entry(self.starts[block + 1]));
        let (event0, event1) = (self.event_of(entry0)?, self.event_of(entry1)?);
        let mut bytes = Vec::new();
        let mut values = Vec::with_capacity(self.columns.len());
        for column in &self.columns {
            let rows = if column.per_event {
                event0..event1
            } else {
                entry0..entry1
            };
            // Exactly the rows reserved for (`Reversed::new`): it never grows past them.
            let mut out = Vec::with_capacity(to_usize(rows.end - rows.start));
            column.read(self.source, rows, &mut out, &mut bytes)?;
            values.push(out);
        }
        self.each(block, |t, v, event, entry| {
            let mut item = [0; N];
            for ((k, column), value) in self.columns.iter().enumerate().zip(&values) {
                let row = if column.per_event {
                    event - event0
                } else {
                    entry - entry0
                };
                item[k] = value[to_usize(row)];
            }
            f(
                t,
                Copied {
                    target: v,
                    at: event,
                    values: item,
                },
            );
        })?;
        Ok(BlockCounts::default())
    }
}

/// Writes the in-adjacency of the directed graph whose out-adjacency `writer` holds
/// into `writer`, with the columns `columns` the out-adjacency stores, without
/// committing.
pub(super) fn transpose_into<K: ByteSink + Sync, Sc: Scratch>(
    writer: &mut Writer<K>,
    columns: &[Out],
    cx: &Context<'_, Sc>,
) -> Result<Transposed, BuildError> {
    let memory = cx.memory;
    let mut held = Reservation::empty(memory, "out-adjacency", MemoryKind::Anonymous);
    let bytes = read_section(writer.sink(), writer.get(OUT_INDEX), OUT_INDEX, &mut held)?;
    let index = AdjacencyIndex::parse(&bytes)?;
    drop(bytes);
    held.try_resize(widen(index.heap_bytes()))?;
    let events_bytes = if index.has_events() {
        Some(read_section(
            writer.sink(),
            writer.get(OUT_EVENTS),
            OUT_EVENTS,
            &mut held,
        )?)
    } else {
        None
    };
    let events = match &events_bytes {
        Some(bytes) => {
            let events = AdjacencyEvents::parse(bytes, &index)?;
            events.check()?;
            Some(events)
        }
        None => None,
    };
    let data = writer
        .get(OUT_DATA)
        .ok_or_else(|| ContainerError::NoSuchSection(OUT_DATA.into()))?
        .clone();
    if data.len != index.data_bytes() {
        return Err(FormatError::Length {
            expected: index.data_bytes(),
            actual: data.len,
        }
        .into());
    }
    let cx = Context {
        graph: GraphOptions {
            directed: true,
            self_loops: SelfLoops::Keep,
            duplicates: if events.is_some() {
                Duplicates::Events
            } else {
                Duplicates::Collapse
            },
        },
        ..*cx
    };
    let shape = (
        index.entries(),
        events
            .as_ref()
            .map_or(index.entries(), AdjacencyEvents::events),
    );
    let read = columns
        .iter()
        .map(|c| OutColumn::open(writer, c, shape, &mut held))
        .collect::<Result<Vec<_>, _>>()?;
    let out = Source {
        index: &index,
        events: events.as_ref(),
        data: &data,
        columns: read,
    };
    // An item carries as many values as there are columns (at most four, § 6).
    let (transposed, written) = match columns.len() {
        0 => write_in::<u32, _, _>(out, writer, columns, &cx)?,
        1 => write_in::<Copied<1>, _, _>(out, writer, columns, &cx)?,
        2 => write_in::<Copied<2>, _, _>(out, writer, columns, &cx)?,
        3 => write_in::<Copied<3>, _, _>(out, writer, columns, &cx)?,
        4 => write_in::<Copied<4>, _, _>(out, writer, columns, &cx)?,
        _ => return Err(FormatError::Invalid("more columns than an item carries").into()),
    };
    let out_events = events
        .as_ref()
        .map_or(index.entries(), AdjacencyEvents::events);
    if written.pairs.entries != index.entries() || written.pairs.events != out_events {
        return Err(FormatError::Invalid(
            "the in-adjacency does not hold the out-adjacency's pairs",
        )
        .into());
    }
    Ok(transposed)
}

/// What the transposition reads of the out-adjacency.
struct Source<'a> {
    index: &'a AdjacencyIndex,
    events: Option<&'a AdjacencyEvents<'a>>,
    data: &'a Entry,
    columns: Vec<OutColumn>,
}

/// Passes 2 to 4 of the in-adjacency, with items `T`, from the out-adjacency `out` that
/// `writer` holds.
fn write_in<T, K, Sc>(
    out: Source<'_>,
    writer: &mut Writer<K>,
    columns: &[Out],
    cx: &Context<'_, Sc>,
) -> Result<(Transposed, super::lists::Written), BuildError>
where
    T: Item,
    K: ByteSink + Sync,
    Sc: Scratch,
    for<'x> Reversed<'x, K>: Items<T>,
{
    let filled = {
        let items = Reversed::new(
            out.index,
            out.events,
            writer.sink(),
            out.data,
            out.columns,
            cx.memory,
            cx.threads,
            cx.tuning.transpose_block,
        )?;
        fill_lists(&items, out.index.nodes(), false, columns.len(), cx)?
    };
    let transposed = Transposed {
        spilled_bytes: filled.spilled_bytes,
        in_memory: filled.layout().resident() == filled.layout().buckets(),
        buckets: filled.layout().buckets(),
    };
    let written = write_lists(filled, writer, IN, columns, cx)?;
    Ok((transposed, written))
}

/// Adds the in-adjacency to the directed graph in `sink`, a graph file that
/// [`build`](super::build) wrote without one, and commits. The new sections point to the
/// build's provenance record: they are the bytes the build would have written.
/// `threads` is the size of the thread pool the call runs in, as for
/// [`BuildOptions::threads`](super::BuildOptions::threads). Returns the sink and whether
/// the sections were added: a file that holds them already is left as it is.
///
/// # Errors
///
/// [`BuildError::Undirected`] for an undirected graph; [`BuildError::Container`] for a
/// file that is not a graph or whose out-adjacency sections fail their hash;
/// [`BuildError::Budget`] when a step cannot fit the budget; the build's errors.
pub fn add_in_adjacency<K, Sc>(
    sink: K,
    memory: &Memory,
    scratch: &Sc,
    threads: NonZeroUsize,
    progress: &dyn Progress,
) -> Result<(K, bool), BuildError>
where
    K: ByteSink + Sync,
    Sc: Scratch,
{
    add_tuned(sink, memory, scratch, threads, progress, &TUNING)
}

pub(super) fn add_tuned<K, Sc>(
    sink: K,
    memory: &Memory,
    scratch: &Sc,
    threads: NonZeroUsize,
    progress: &dyn Progress,
    tuning: &Tuning,
) -> Result<(K, bool), BuildError>
where
    K: ByteSink + Sync,
    Sc: Scratch,
{
    let columns = {
        let file = Container::open(&sink)?;
        let parameters = file.read_provenance(PROVENANCE)?.parameters;
        let get = |key: &str| parameters.get(key).map(String::as_str);
        if get("directed") != Some("true") {
            return Err(BuildError::Undirected);
        }
        if file.get(IN_INDEX).is_some() {
            return Ok((sink, false));
        }
        let duplicates = match get("duplicates") {
            Some("sum") => Duplicates::Sum,
            Some("collapse") => Duplicates::Collapse,
            _ => Duplicates::Events,
        };
        let columns = outputs_of(get("weight"), get("time"), duplicates);
        for e in file.entries() {
            if e.name.starts_with("adjacency.out.") {
                file.verify(&e.name)?;
            }
        }
        columns
    };
    let mut writer = Writer::update(sink)?;
    let cx = Context {
        graph: GraphOptions::default(),
        memory,
        scratch,
        tuning,
        threads: threads.get(),
        progress,
    };
    progress.begin(Phase::Building, None);
    let transposed = transpose_into(&mut writer, &columns, &cx)?;
    progress.end(Phase::Building, widen(transposed.buckets));
    let added: Vec<String> = writer
        .entries()
        .filter(|e| e.name.starts_with("adjacency.in."))
        .map(|e| e.name.to_string())
        .collect();
    for name in added {
        writer.set_provenance(&name, Some(PROVENANCE))?;
    }
    writer.commit()?;
    Ok((writer.into_sink(), true))
}

fn to_usize(x: u64) -> usize {
    usize::try_from(x).expect("a range of bytes held in memory")
}
