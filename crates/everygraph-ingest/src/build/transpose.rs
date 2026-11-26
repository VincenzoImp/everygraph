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
//! [`add_in_adjacency`] adds it to a graph file built without it; [`build`](super::build)
//! makes it for every directed graph unless told not to.

use std::num::NonZeroUsize;

use everygraph_core::{ByteSink, ByteSource, Memory, MemoryKind, NodeId, Reservation};
use everygraph_format::adjacency::{
    Adjacency, AdjacencyEvents, AdjacencyIndex, Batch, FormatError,
};
use everygraph_format::container::{Container, ContainerError, Entry, Writer};

use super::lists::{Context, Names, fill_lists, write_lists};
use super::rows::{BlockCounts, Items};
use super::{
    BuildError, Duplicates, GraphOptions, IN_DATA, IN_EVENTS, IN_INDEX, OUT_DATA, OUT_EVENTS,
    OUT_INDEX, PROVENANCE, Phase, Progress, SelfLoops, TUNING, Tuning, widen,
};
use crate::ids::Scratch;

/// The section names of the in-adjacency.
pub(super) const IN: Names = Names {
    data: IN_DATA,
    index: IN_INDEX,
    events: IN_EVENTS,
};

/// A block of out-lists holds at least this many entries, unless the lists run out.
const BLOCK_ENTRIES: u64 = 1 << 18;

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

/// The items of the in-adjacency, read from the out-adjacency.
struct Reversed<'a, R> {
    index: &'a AdjacencyIndex,
    events: Option<&'a AdjacencyEvents<'a>>,
    /// The container's bytes, and where the data section starts in them.
    source: &'a R,
    data_at: u64,
    /// The data section, when the source holds it in memory.
    mapped: Option<&'a [u8]>,
    /// First node of each block, then the node count.
    starts: Vec<u32>,
    /// The buffers of the blocks in flight.
    _buffers: Reservation,
}

impl<'a, R: ByteSource + Sync> Reversed<'a, R> {
    fn new(
        index: &'a AdjacencyIndex,
        events: Option<&'a AdjacencyEvents<'a>>,
        source: &'a R,
        data: &Entry,
        memory: &Memory,
        threads: usize,
    ) -> Result<Self, BuildError> {
        let entry = |v: u32| index.entry_offset(NodeId::new(v)).get();
        let mut starts = vec![0];
        for b in 0..index.blocks() {
            let end = index.block_nodes(b).end;
            let first = *starts.last().expect("a start");
            if entry(end) - entry(first) >= BLOCK_ENTRIES || end == index.nodes() {
                starts.push(end);
            }
        }
        let mapped = source.slice(data.offset..data.offset + data.len);
        // A block in flight holds its decoded lists and, when the section is not in
        // memory, its records.
        let most = starts
            .windows(2)
            .map(|w| {
                let entries = entry(w[1]) - entry(w[0]);
                let lists = u64::from(w[1] - w[0]);
                let records = index.data_range(w[0]..w[1]);
                let read = if mapped.is_some() {
                    0
                } else {
                    records.end - records.start
                };
                (entries + 8) * 4 + lists * 16 + read
            })
            .max()
            .unwrap_or(0);
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
            starts,
            _buffers: buffers,
        })
    }
}

impl<R: ByteSource + Sync> Items for Reversed<'_, R> {
    fn blocks(&self) -> usize {
        self.starts.len() - 1
    }

    fn items(&self, block: usize, mut f: impl FnMut(u32, u32)) -> Result<BlockCounts, BuildError> {
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
        let first = self.index.entry_offset(NodeId::new(nodes.start)).get();
        let mut multiplicities = match self.events {
            Some(events) => Some(events.multiplicities_from(first)?),
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
                    f(t, v.get());
                }
            }
        }
        Ok(BlockCounts::default())
    }
}

/// Reads section `name` of the container in `sink` into memory reserved in `held`.
fn read_section<R: ByteSource>(
    source: &R,
    entry: Option<&Entry>,
    name: &str,
    held: &mut Reservation,
) -> Result<Vec<u8>, BuildError> {
    let entry = entry.ok_or_else(|| ContainerError::NoSuchSection(name.into()))?;
    let len = usize::try_from(entry.len).map_err(|_| BuildError::TooLarge("a section"))?;
    held.try_grow(entry.len)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(len)
        .map_err(|_| everygraph_core::MemoryError::AllocationFailed {
            purpose: "out-adjacency section",
            bytes: entry.len,
        })?;
    bytes.resize(len, 0);
    source
        .read_at(entry.offset, &mut bytes)
        .map_err(|e| BuildError::Container(ContainerError::Read(e)))?;
    Ok(bytes)
}

/// Writes the in-adjacency of the directed graph whose out-adjacency `writer` holds
/// into `writer`, without committing.
pub(super) fn transpose_into<K: ByteSink + Sync, Sc: Scratch>(
    writer: &mut Writer<K>,
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
    let filled = {
        let items = Reversed::new(
            &index,
            events.as_ref(),
            writer.sink(),
            &data,
            memory,
            cx.threads,
        )?;
        fill_lists(&items, index.nodes(), false, &cx)?
    };
    let transposed = Transposed {
        spilled_bytes: filled.spilled_bytes,
        in_memory: filled.layout().resident() == filled.layout().buckets(),
        buckets: filled.layout().buckets(),
    };
    let written = write_lists(filled, writer, IN, &cx)?;
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
    {
        let file = Container::open(&sink)?;
        let parameters = file.read_provenance(PROVENANCE)?.parameters;
        if parameters.get("directed").map(String::as_str) != Some("true") {
            return Err(BuildError::Undirected);
        }
        if file.get(IN_INDEX).is_some() {
            return Ok((sink, false));
        }
        for name in [OUT_INDEX, OUT_DATA, OUT_EVENTS] {
            if file.get(name).is_some() {
                file.verify(name)?;
            }
        }
    }
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
    let transposed = transpose_into(&mut writer, &cx)?;
    progress.end(Phase::Building, widen(transposed.buckets));
    for name in [IN_DATA, IN_INDEX, IN_EVENTS] {
        if writer.get(name).is_some() {
            writer.set_provenance(name, Some(PROVENANCE))?;
        }
    }
    writer.commit()?;
    Ok((writer.into_sink(), true))
}

fn to_usize(x: u64) -> usize {
    usize::try_from(x).expect("a range of bytes held in memory")
}
