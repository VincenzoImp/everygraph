// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Passes 2 to 4 for any source of items ([`Items`]): the counts, the buckets and the
//! lists of one adjacency, written as its sections. The out-adjacency reads the rows;
//! the in-adjacency reads the out-adjacency (module `transpose`).

use std::sync::{Mutex, PoisonError};

use everygraph_core::{ByteSink, Memory, MemoryKind};
use everygraph_format::adjacency::{IndexBuilder, encode_events, events_section_bytes};
use everygraph_format::container::{Kind, Writer};

use super::fill::{self, Resident};
use super::finish::{Multiplicities, Output, PairCounts, finish_round, multiplicities};
use super::layout::Layout;
use super::rows::{BlockCounts, ItemCounts, Items, each_block};
use super::{BuildError, Duplicates, GraphOptions, Phase, Progress, SLACK_SHIFT, Tuning, widen};
use crate::ids::Scratch;

/// The section names of one adjacency.
#[derive(Clone, Copy, Debug)]
pub(super) struct Names {
    pub(super) data: &'static str,
    pub(super) index: &'static str,
    pub(super) events: &'static str,
}

/// What the build of one adjacency needs besides its items.
pub(super) struct Context<'a, Sc> {
    pub(super) graph: GraphOptions,
    pub(super) memory: &'a Memory,
    pub(super) scratch: &'a Sc,
    pub(super) tuning: &'a Tuning,
    pub(super) threads: usize,
    pub(super) progress: &'a dyn Progress,
}

/// The items of an adjacency in their buckets: in memory, or spilled.
pub(super) struct Filled<R> {
    layout: Layout,
    resident: Resident,
    spills: Vec<R>,
    index: IndexBuilder,
    /// What the counting pass counted.
    pub(super) pass: BlockCounts,
    /// Nodes that no item starts at or, with `ends`, ends at.
    pub(super) isolates: u64,
    /// Bytes written to spill files.
    pub(super) spilled_bytes: u64,
}

impl<R> Filled<R> {
    /// The cut of the items.
    pub(super) fn layout(&self) -> &Layout {
        &self.layout
    }
}

/// What [`write_lists`] wrote.
pub(super) struct Written {
    pub(super) pairs: PairCounts,
    /// Bytes of the `.events` section; 0 when there is none.
    pub(super) events_bytes: u64,
}

/// Buffers of the spill files and of the multiplicities, held whatever the input.
pub(super) fn fixed_bytes(threads: usize, tuning: &Tuning) -> u64 {
    widen(threads) * widen(tuning.spill_batch) * 8
        + 8 * fill::SPILL_BUFFER
        + widen(super::finish::MULTIPLICITY_BUFFER)
}

/// Passes 2 and 3: the items of each of `nodes` nodes counted, cut into buckets within
/// the budget, and sent to them. With `ends`, the nodes items end at are marked too, for
/// the isolates of a directed graph.
pub(super) fn fill_lists<I: Items, Sc: Scratch>(
    items: &I,
    nodes: u32,
    ends: bool,
    cx: &Context<'_, Sc>,
) -> Result<Filled<Sc::Source>, BuildError> {
    let (graph, memory) = (cx.graph, cx.memory);
    let counts = ItemCounts::new(memory, nodes, ends)?;
    let pass = each_block(items.blocks(), |block| {
        items.items(block, |u, v| {
            counts.add(u);
            if ends {
                counts.target(v);
            }
        })
    })?;
    let isolates = counts.isolates();
    let index = IndexBuilder::new(memory, nodes)?;

    let events = graph.duplicates == Duplicates::Events;
    let available = memory.available() + counts.bytes();
    let fixed = fixed_bytes(cx.threads, cx.tuning);
    let work = (available - (available >> SLACK_SHIFT)).saturating_sub(fixed);
    let values = counts
        .values()
        .ok_or(BuildError::TooLarge("a node with 2^32 items or more"))?;
    let layout = Layout::new(values, nodes, items.blocks(), work, events, cx.tuning)?;
    drop(counts);

    let (resident, spills, spilled_bytes) =
        fill::fill(items, &layout, memory, cx.scratch, cx.tuning, cx.threads)?;
    Ok(Filled {
        layout,
        resident,
        spills,
        index,
        pass,
        isolates,
        spilled_bytes,
    })
}

/// Pass 4: the buckets finished in node order, their records written to the data
/// section, then the index and, when some pair repeats, the `.events` section.
pub(super) fn write_lists<K: ByteSink, Sc: Scratch>(
    filled: Filled<Sc::Source>,
    writer: &mut Writer<K>,
    names: Names,
    cx: &Context<'_, Sc>,
) -> Result<Written, BuildError> {
    let Filled {
        layout,
        mut resident,
        spills,
        index,
        ..
    } = filled;
    let (graph, memory, scratch) = (cx.graph, cx.memory, cx.scratch);
    let mut output = Output {
        data: writer.begin_section(names.data, Kind::ADJACENCY_DATA, None)?,
        index,
        multiplicities: Multiplicities::new(memory),
        counts: PairCounts::default(),
    };
    for round in 0..layout.resident_rounds {
        for bucket in finish_round(&mut resident, 0, &layout, round, graph, memory)? {
            output.take(bucket, scratch)?;
        }
        cx.progress.advance(Phase::Building, 1);
    }
    drop(resident);
    for (round, spill) in (layout.resident_rounds..layout.round_count()).zip(&spills) {
        let mut columns = fill::load_round(spill, &layout, round, memory, cx.tuning)?;
        let held_from = layout.offsets[layout.rounds[round]];
        for bucket in finish_round(&mut columns, held_from, &layout, round, graph, memory)? {
            output.take(bucket, scratch)?;
        }
        cx.progress.advance(Phase::Building, 1);
    }
    drop(spills);

    let Output {
        data,
        index,
        multiplicities: stored,
        counts: pairs,
    } = output;
    data.finish()?;
    let stored = stored.finish(scratch)?;
    let (index, held) = index.finish(stored.is_some())?;
    writer.add_section(names.index, Kind::ADJACENCY_INDEX, &index, None)?;
    drop((index, held));
    let mut events_bytes = 0;
    if let Some((ones, file)) = &stored {
        let section_bytes = events_section_bytes(pairs.entries, pairs.events)
            .ok_or(BuildError::TooLarge("the .events section"))?;
        // The sequence, then its bytes beside it.
        let mut held = memory.reserve(
            "events section",
            MemoryKind::Anonymous,
            section_bytes.saturating_mul(2),
        )?;
        let error = Mutex::new(None);
        let sequence = multiplicities(*ones, file, memory, &error)?;
        let section = encode_events(pairs.entries, pairs.events, sequence);
        if let Some(e) = error.into_inner().unwrap_or_else(PoisonError::into_inner) {
            return Err(e);
        }
        let section = section?;
        held.shrink(section_bytes);
        events_bytes = widen(section.len());
        writer.add_section(names.events, Kind::ADJACENCY_EVENTS, &section, None)?;
    }
    Ok(Written {
        pairs,
        events_bytes,
    })
}
