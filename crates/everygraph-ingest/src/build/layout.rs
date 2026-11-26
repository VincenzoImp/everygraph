// SPDX-License-Identifier: Apache-2.0 OR MIT

//! How the items are cut: buckets of consecutive source nodes, with exact boundaries
//! from the item counts of the first pass, grouped into rounds that are finished one at
//! a time; the first rounds are filled in memory, the others spilled.

use everygraph_core::{InsufficientBudget, MemoryError, widen};
use everygraph_format::adjacency::{events_section_bytes, index_bytes_bound, record_bound};

use super::columns::Item;
use super::fill::record;
use super::{BuildError, Regime, Tuning};

/// Bytes per item, which depend on what an item carries.
#[derive(Clone, Copy, Debug)]
pub(super) struct Costs {
    /// While its bucket waits: the item and its source local to the bucket (`u16`), in
    /// memory or in a spill record.
    pub(super) item: u64,
    /// While its bucket is finished: the item sorted, the record bytes (at most 4), the
    /// multiplicity when events are kept (`u32`), and a `u64` per stored column.
    pub(super) finish: u64,
}

impl Costs {
    /// The costs of items `T` with `columns` stored columns.
    pub(super) fn of<T: Item>(events: bool, columns: usize) -> Self {
        let size = widen(size_of::<T>());
        Self {
            item: (2 + size).max(widen(record::<T>())),
            finish: size + 4 + if events { 4 } else { 0 } + 8 * widen(columns),
        }
    }
}
/// Bytes of a cell of the scatter table: a count and a part of two columns.
const CELL_BYTES: u64 = 8 + 32;
/// The scatter table is kept below this by grouping text blocks.
const TABLE_LIMIT: u64 = 1 << 28;
/// Buckets hold at least this many items unless their nodes run out, so that the
/// bucket directory stays small next to the items.
const MIN_BUCKET_ITEMS: u64 = 1 << 12;

/// Bytes a bucket needs while it is finished: [`Costs::finish`] per item, the control
/// bytes of the records (`⌈d/4⌉` per list), and per node two counts, a degree and a
/// record length.
pub(super) fn finish_bytes(items: u64, nodes: u64, costs: Costs) -> u64 {
    items * costs.finish + items.div_ceil(4) + nodes * (4 + 4 + 4 + 8 + 1) + 64
}

/// The cut of the items.
#[derive(Debug)]
pub(super) struct Layout {
    /// First node of each bucket, then the node count.
    pub(super) starts: Vec<u32>,
    /// Items before each bucket, then the total.
    pub(super) offsets: Vec<u64>,
    /// First bucket of each round, then the bucket count.
    pub(super) rounds: Vec<usize>,
    /// Rounds below this are filled in memory; the others go to spill files.
    pub(super) resident_rounds: usize,
    /// Text blocks per block of the scatter.
    pub(super) group: usize,
    /// The first bucket of each window of `2^WINDOW_BITS` nodes.
    windows: Vec<u32>,
    /// Bytes per item.
    pub(super) costs: Costs,
}

const WINDOW_BITS: u32 = 16;

impl Layout {
    /// Cuts `nodes` nodes with these item counts for `blocks` text blocks, within
    /// `work` bytes of memory. Every later step that could be refused for memory is
    /// checked here, so that a refusal comes before any work.
    ///
    /// # Errors
    ///
    /// [`BuildError::Budget`] when the bucket of the largest list, the index or the
    /// `.events` section cannot be made within `work`; [`BuildError::Memory`] when a
    /// node has more items than a segment of the bucket columns holds (its list could
    /// not be lent as one slice).
    pub(super) fn new(
        counts: impl Iterator<Item = u32>,
        nodes: u32,
        blocks: usize,
        work: u64,
        (events, costs): (bool, Costs),
        tuning: &Tuning,
    ) -> Result<Self, BuildError> {
        let round_bytes = work / 4;
        let per_item = finish_bytes(1 << 20, 0, costs) >> 20;
        let round_items = (round_bytes / per_item.max(1)).max(MIN_BUCKET_ITEMS);
        // A bucket is lent as one slice of the bucket columns, so it is cut at a
        // segment's rows unless one node has more.
        let bucket_items = (round_items / 8)
            .max(MIN_BUCKET_ITEMS)
            .min(1 << tuning.segment_bits);
        let max_nodes = u64::from(tuning.bucket_nodes);

        let (mut starts, mut offsets) = (vec![0_u32], vec![0_u64]);
        let (mut in_bucket, mut items, mut first) = (0_u64, 0_u64, 0_u32);
        for (v, d) in (0..nodes).zip(counts) {
            let d = u64::from(d);
            let size = u64::from(v - first);
            if size == max_nodes || (size > 0 && in_bucket + d > bucket_items) {
                starts.push(v);
                offsets.push(items);
                (first, in_bucket) = (v, 0);
            }
            in_bucket += d;
            items += d;
        }
        if nodes > 0 {
            starts.push(nodes);
            offsets.push(items);
        }
        let buckets = starts.len() - 1;
        let size = |b: usize| {
            (
                offsets[b + 1] - offsets[b],
                u64::from(starts[b + 1] - starts[b]),
            )
        };

        if let Some(b) = (0..buckets).find(|&b| size(b).0 > 1 << tuning.segment_bits) {
            return Err(BuildError::Memory(MemoryError::AllocationFailed {
                purpose: "the items of one node",
                bytes: costs.item * size(b).0,
            }));
        }
        // After the lists: the index, then the `.events` section built in memory and
        // copied once. Entries and events are at most the items; records at most
        // `⌈d/4⌉ + 4d` bytes per list.
        let index = index_bytes_bound(
            u64::from(nodes),
            items,
            record_bound(items) + u64::from(nodes),
        )
        .unwrap_or(u64::MAX);
        let events_section = if events {
            events_section_bytes(items, items).map_or(u64::MAX, |b| b.saturating_mul(2))
        } else {
            0
        };
        for (purpose, needed) in [
            ("the adjacency index", index),
            ("the .events section", events_section),
        ] {
            if needed > work {
                return Err(BuildError::Budget(InsufficientBudget::new(
                    purpose, needed, work,
                )));
            }
        }

        // The largest bucket must be finished on its own, loaded from a spill.
        let largest = (0..buckets)
            .map(|b| {
                let (items, nodes) = size(b);
                costs.item * items + finish_bytes(items, nodes, costs)
            })
            .max()
            .unwrap_or(0);
        if largest > work {
            return Err(BuildError::Budget(InsufficientBudget::new(
                "the largest bucket of lists",
                largest,
                work,
            )));
        }

        // The longest prefix of buckets that fits in memory with its scatter table and
        // one round of finishing.
        let resident_limit = work.saturating_sub(round_bytes.max(largest));
        let mut resident = 0;
        while resident < buckets {
            let next = resident + 1;
            let table = scatter_table(blocks, next).1;
            if costs.item * offsets[next] + table > resident_limit
                || offsets[next] > tuning.resident_items
            {
                break;
            }
            resident = next;
        }

        let mut rounds = vec![0];
        let (mut in_round, mut round_start) = (0_u64, 0);
        for b in 0..buckets {
            let (items, nodes) = size(b);
            let spilled = b >= resident;
            let cost =
                finish_bytes(items, nodes, costs) + if spilled { costs.item * items } else { 0 };
            let limit = if spilled {
                work
            } else {
                round_bytes.max(largest)
            };
            let boundary = b == resident && b > round_start;
            if boundary || (b > round_start && in_round + cost > limit) {
                rounds.push(b);
                (round_start, in_round) = (b, 0);
            }
            in_round += cost;
        }
        if buckets > 0 {
            rounds.push(buckets);
        }
        let resident_rounds = rounds.partition_point(|&r| r < resident);

        let windows = (0..=(u64::from(nodes) >> WINDOW_BITS))
            .map(|w| {
                let node = w << WINDOW_BITS;
                let b = starts
                    .partition_point(|&s| u64::from(s) <= node)
                    .saturating_sub(1);
                u32::try_from(b).expect("fewer buckets than nodes")
            })
            .collect();
        Ok(Self {
            group: scatter_table(blocks, resident).0,
            starts,
            offsets,
            rounds,
            resident_rounds,
            windows,
            costs,
        })
    }

    /// Number of buckets.
    pub(super) fn buckets(&self) -> usize {
        self.starts.len() - 1
    }

    /// The bucket of node `v`.
    pub(super) fn bucket_of(&self, v: u32) -> usize {
        let w = usize::try_from(v >> WINDOW_BITS).expect("usize has at least 32 bits");
        let low = usize::try_from(self.windows[w]).expect("usize has at least 32 bits");
        let high = self
            .windows
            .get(w + 1)
            .map_or(self.buckets(), |&b| {
                usize::try_from(b).expect("32 bits") + 1
            })
            .min(self.buckets());
        low + self.starts[low..high].partition_point(|&s| s <= v) - 1
    }

    /// Buckets `..resident()` are filled in memory.
    pub(super) fn resident(&self) -> usize {
        self.rounds[self.resident_rounds]
    }

    /// Items of the resident buckets.
    pub(super) fn resident_items(&self) -> u64 {
        self.offsets[self.resident()]
    }

    /// Number of rounds.
    pub(super) fn round_count(&self) -> usize {
        self.rounds.len().saturating_sub(1)
    }

    /// In memory, or with spills.
    pub(super) fn regime(&self) -> Regime {
        if self.resident() == self.buckets() {
            Regime::InMemory
        } else {
            Regime::OnDisk
        }
    }
}

/// Text blocks per scatter block, and the bytes of the scatter table, for `buckets`
/// buckets: blocks are grouped so that the table stays below [`TABLE_LIMIT`].
fn scatter_table(blocks: usize, buckets: usize) -> (usize, u64) {
    let (blocks, buckets) = (widen(blocks), widen(buckets));
    let cells = blocks.saturating_mul(buckets).saturating_mul(CELL_BYTES);
    let group = cells.div_ceil(TABLE_LIMIT).max(1);
    let grouped = blocks.div_ceil(group);
    let bytes = grouped * buckets * CELL_BYTES + grouped * 64;
    (usize::try_from(group).unwrap_or(usize::MAX), bytes)
}
