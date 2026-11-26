// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Passes 2 to 4 for items `T` ([`Item`]): the counts, the buckets and the lists of the
//! out-adjacency, written as its sections, with the columns stored beside it.

use std::sync::{Mutex, PoisonError};

use everygraph_core::{ByteSink, ByteSource, MAX_ALLOCATION_BYTES, Memory, MemoryKind, widen};
use everygraph_format::adjacency::{IndexBuilder, encode_events, events_section_bytes};
use everygraph_format::container::{Kind, Writer};

use super::columns::{Item, Out, Stored};
use super::finish::{Finish, Multiplicities, Output, PairCounts, finish_round, multiplicities};
use super::layout::{Costs, Layout};
use super::rows::{self, BlockCounts, Fingerprints, ItemCounts, Items, Rows, each_block};
use super::{
    BuildError, BuildOptions, Duplicates, OUT_DATA, OUT_EVENTS, OUT_INDEX, Phase, Progress, Regime,
    SLACK_SHIFT, Tuning, fill, fixed_bytes,
};
use crate::ids::{Dictionary, Scratch};
use crate::text::TextReader;

/// The name of the out-adjacency, the prefix of its sections and column scopes.
const OUT: &str = "adjacency.out";

/// The input of the passes, after the dictionary.
pub(super) struct Input<'a, 'd, S, D> {
    pub(super) reader: &'a TextReader<S>,
    pub(super) dictionary: &'a Dictionary<'d, D>,
    pub(super) fingerprints: &'a Fingerprints,
    /// Rows of the input.
    pub(super) rows: u64,
    /// Rows of its largest block.
    pub(super) most_rows: u64,
    /// The columns stored beside the lists.
    pub(super) columns: Vec<Out>,
}

/// What the passes need besides their input.
pub(super) struct Context<'a, Sc> {
    pub(super) options: &'a BuildOptions,
    pub(super) memory: &'a Memory,
    pub(super) scratch: &'a Sc,
    pub(super) tuning: &'a Tuning,
    pub(super) progress: &'a dyn Progress,
}

/// What the passes found and wrote.
pub(super) struct Lists {
    pub(super) pass: BlockCounts,
    /// Nodes that no item starts or, on a directed graph, ends at.
    pub(super) isolates: u64,
    pub(super) pairs: PairCounts,
    /// Bytes of the `.events` section; 0 when there is none.
    pub(super) events_bytes: u64,
    pub(super) regime: Regime,
    pub(super) buckets: usize,
    pub(super) spilled_bytes: u64,
}

/// Builds the out-adjacency of `input` with items `T` into a new container in `out`:
/// its data, index and `.events` sections, and its columns.
pub(super) fn write_lists<T, S, D, Sc, K>(
    input: &Input<'_, '_, S, D>,
    cx: &Context<'_, Sc>,
    out: K,
) -> Result<(Writer<K>, Lists), BuildError>
where
    T: Item,
    S: ByteSource + Sync,
    D: ByteSource + Sync,
    Sc: Scratch,
    K: ByteSink,
{
    let (memory, scratch) = (cx.memory, cx.scratch);
    // A segment of the bucket columns is one allocation.
    let most_bits = (MAX_ALLOCATION_BYTES / widen(size_of::<T>())).ilog2();
    let tuning = &Tuning {
        segment_bits: cx.tuning.segment_bits.min(most_bits),
        ..*cx.tuning
    };
    let graph = cx.options.graph;
    let threads = cx.options.threads.get();
    let nodes = input.dictionary.len();
    let blocks = input.fingerprints.blocks();
    // The mapped rows are kept for the next passes when they take at most an eighth of
    // what is left: they are freed before the lists are finished, but the buckets
    // filled in memory live beside them.
    let cache_bytes = input.rows.saturating_mul(rows::cache_bytes_per_row::<T>());
    let cached_rows = (cache_bytes <= memory.available() / 8).then_some(input.rows);
    let flight = rows::Flight {
        threads,
        blocks,
        block_bytes: widen(input.reader.block_bytes()),
        most_rows: input.most_rows,
        row_bytes: rows::block_bytes_per_row::<T>(),
    };
    let cached_rows = cached_rows.filter(|_| tuning.cache_rows);
    let mut rows: Rows<'_, '_, S, D, T> = Rows::new(
        input.reader,
        input.dictionary,
        graph,
        input.fingerprints,
        &flight,
        memory,
        cached_rows,
    )?;
    let counts = ItemCounts::new(memory, nodes, graph.directed)?;
    let pass = each_block(blocks, |block| {
        rows.items(block, |u, v: T| {
            counts.add(u);
            if graph.directed {
                counts.target(v.target());
            }
        })
    })?;
    let isolates = counts.isolates();
    let index = IndexBuilder::new(memory, nodes)?;

    let events = graph.duplicates == Duplicates::Events;
    let available = memory.available() + counts.bytes();
    let values = counts
        .values()
        .ok_or(BuildError::TooLarge("a node with 2^32 items or more"))?;
    let fixed = fixed_bytes::<T>(threads, tuning, input.columns.len());
    let work = (available - (available >> SLACK_SHIFT)).saturating_sub(fixed);
    let costs = Costs::of::<T>(events, input.columns.len());
    let layout = Layout::new(values, nodes, blocks, work, (events, costs), tuning)?;
    drop(counts);

    let (mut resident, spills, spilled_bytes) =
        fill::fill(&rows, &layout, memory, scratch, tuning, threads)?;
    rows.release_cache();

    let mut writer = Writer::create(out)?;
    let mut output = Output {
        data: writer.begin_section(OUT_DATA, Kind::ADJACENCY_DATA, None)?,
        index,
        multiplicities: Multiplicities::new(memory),
        columns: Stored::new(&input.columns, scratch, memory)?,
        counts: PairCounts::default(),
    };
    let how = Finish {
        graph,
        columns: &input.columns,
    };
    for round in 0..layout.resident_rounds {
        for bucket in finish_round(&mut resident, 0, &layout, round, how, memory)? {
            output.take(bucket, scratch)?;
        }
        cx.progress.advance(Phase::Building, 1);
    }
    drop(resident);
    for (round, spill) in (layout.resident_rounds..layout.round_count()).zip(&spills) {
        let mut columns = fill::load_round::<T, _>(spill, &layout, round, memory, tuning)?;
        let held_from = layout.offsets[layout.rounds[round]];
        for bucket in finish_round(&mut columns, held_from, &layout, round, how, memory)? {
            output.take(bucket, scratch)?;
        }
        cx.progress.advance(Phase::Building, 1);
    }
    drop(spills);
    drop(rows);
    cx.progress.end(Phase::Building, widen(layout.buckets()));

    let Output {
        data,
        index,
        multiplicities: stored,
        columns,
        counts: pairs,
    } = output;
    data.finish()?;
    let stored = stored.finish(scratch)?;
    let (index, held) = index.finish(stored.is_some())?;
    writer.add_section(OUT_INDEX, Kind::ADJACENCY_INDEX, &index, None)?;
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
        writer.add_section(OUT_EVENTS, Kind::ADJACENCY_EVENTS, &section, None)?;
    }
    let shape = (pairs.entries, pairs.events);
    columns.write(&input.columns, OUT, shape, scratch, &mut writer, memory)?;
    let lists = Lists {
        pass,
        isolates,
        pairs,
        events_bytes,
        regime: layout.regime(),
        buckets: layout.buckets(),
        spilled_bytes,
    };
    Ok((writer, lists))
}
