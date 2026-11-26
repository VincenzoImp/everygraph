// SPDX-License-Identifier: Apache-2.0 OR MIT

//! What a build will need, from a sample of its input, by the rules the build itself
//! applies (`docs/ux-contract.md` § 3, the plan before any work).

use std::num::NonZeroUsize;

use everygraph_core::ByteSource;

use super::layout::ITEM_BYTES;
use super::rows::Flight;
use super::{Regime, SLACK_SHIFT, TUNING, fixed_bytes};
use crate::text::{TextError, TextReader};

/// The identifier dictionary's smallest working set: the table of its dense bitmap
/// (65,536 segment slots) and one worker's base, rounded up.
const DICTIONARY_BYTES: u64 = 2 << 20;
/// Output bytes per adjacency entry. A placeholder of the cost model, not a measurement
/// of this input: `docs/spec/eg-adjacency.md` § 7 gives 2.0–2.9 for four social graphs.
const OUTPUT_BYTES_PER_ENTRY: u64 = 3;

/// The input as sampled before the build.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct Sample {
    /// Rows of the input (an estimate unless the sample was the whole input).
    pub rows: u64,
    /// Rows of the largest block seen.
    pub rows_per_block: u64,
    /// Bytes of text of a block.
    pub block_bytes: u64,
    /// Blocks of the input.
    pub blocks: u64,
}

impl Sample {
    /// A sample of `rows` rows in `blocks` blocks of `block_bytes`, the largest of
    /// which holds `rows_per_block`.
    #[must_use]
    pub fn new(rows: u64, rows_per_block: u64, block_bytes: u64, blocks: u64) -> Self {
        Self {
            rows,
            rows_per_block,
            block_bytes,
            blocks,
        }
    }

    /// The sample of a plain-text input: its first block, as the build reads it, and
    /// as many rows in each block. `None` for a compressed input, which has no blocks
    /// until it is decompressed.
    ///
    /// # Errors
    ///
    /// The first block could not be read.
    pub fn of<S: ByteSource + Sync>(reader: &TextReader<S>) -> Result<Option<Self>, TextError> {
        let Some(blocks) = reader.blocks()? else {
            return Ok(None);
        };
        let wide = |x: usize| u64::try_from(x).unwrap_or(u64::MAX);
        let block_bytes = wide(reader.block_bytes());
        if blocks == 0 {
            return Ok(Some(Self::new(0, 0, block_bytes, 0)));
        }
        let rows = wide(reader.read_block(0)?.0.len());
        let blocks = wide(blocks);
        Ok(Some(Self::new(
            rows.saturating_mul(blocks),
            rows,
            block_bytes,
            blocks,
        )))
    }
}

/// What a build of a sampled input will need under a budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Estimate {
    /// In memory, or with spills.
    pub regime: Regime,
    /// The items the build sorts, 6 bytes each (`regime.working_bytes`).
    pub working_bytes: u64,
    /// The smallest budget that can hold the blocks in flight and the fixed buffers;
    /// a budget below it is refused before any work.
    pub smallest_budget: u64,
    /// The most memory the build will hold.
    pub peak_bytes: u64,
    /// Bytes spilled to scratch files.
    pub scratch_bytes: u64,
    /// Bytes of the graph file.
    pub output_bytes: u64,
}

/// What a build of `sample` will need with `threads` threads and `budget` bytes. The
/// per-node state is left out: the number of nodes is not known before the input is
/// read, and the build refuses, before it writes anything, a budget that cannot hold it.
/// So the build holds no more items in memory than this estimate says, and spills no
/// less.
#[must_use]
pub fn estimate(sample: &Sample, directed: bool, threads: NonZeroUsize, budget: u64) -> Estimate {
    let flight = Flight {
        threads: threads.get(),
        blocks: usize::try_from(sample.blocks).unwrap_or(usize::MAX),
        block_bytes: sample.block_bytes,
        most_rows: sample.rows_per_block,
    };
    let held = flight.bytes() + fixed_bytes(threads.get(), &TUNING);
    let smallest_budget = held + DICTIONARY_BYTES;
    let items = sample.rows.saturating_mul(if directed { 1 } else { 2 });
    let working_bytes = items.saturating_mul(ITEM_BYTES);
    // The dictionary's working set is given back before the items are counted.
    let available = budget.saturating_sub(held);
    let work = available - (available >> SLACK_SHIFT);
    // As `Layout::new`: a quarter of the work finishes a round; the rest holds items.
    let resident = work - work / 4;
    let regime = if working_bytes <= resident {
        Regime::InMemory
    } else {
        Regime::OnDisk
    };
    Estimate {
        regime,
        working_bytes,
        smallest_budget,
        peak_bytes: (held + working_bytes.min(resident) + work / 4).min(budget),
        scratch_bytes: working_bytes.saturating_sub(resident),
        output_bytes: items.saturating_mul(OUTPUT_BYTES_PER_ENTRY),
    }
}
