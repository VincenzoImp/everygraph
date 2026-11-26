// SPDX-License-Identifier: Apache-2.0 OR MIT

//! [`store`]: a [`Communities`] result into the graph file, level by level.

use std::cmp::Reverse;
use std::num::NonZeroUsize;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering::Relaxed;

use everygraph_core::{ByteSink, Memory, MemoryKind, par};
use everygraph_format::adjacency::{
    IndexBuilder, append_record, encode_events, events_section_bytes, record_bound,
};
use everygraph_format::container::{Kind, Provenance, ResultKey, Writer};
use everygraph_kernels::edge_map::{Error, Plan, Settings, Source};

use super::{HierarchyError, INTERNAL, KIND, MEMBERSHIP, NODES, PARENT, QUOTIENT, VOLUME, part};
use crate::Communities;
use crate::aggregate::{Members, Parts, Quotient, ix, quotient};
use crate::column::{Column, GRAIN};
use crate::level::Weights;
use crate::quality::{Table, tally};
use crate::table::Tables;

/// Entries of a quotient encoded at a time, about: the window's buffers hold about 30
/// bytes per entry.
pub(crate) const WINDOW: u64 = 1 << 21;

/// One level, finest first: the map from the level below and its numbering.
struct Level<'c> {
    parts: Parts<'c>,
    /// Input nodes in each community.
    size: Column<u32>,
    /// The community of each new id, and the new id of each community.
    old_of: Column<u32>,
    new_of: Column<u32>,
}

/// Stores the hierarchy of `communities`, found on `source` (the graph of the file the
/// writer holds, as Leiden read it), as the result of `key` with the record `about`;
/// a hierarchy stored under `key` is replaced. Nothing is stored until the writer
/// commits.
///
/// The levels are those of `communities` when they compose to its membership, and the
/// membership alone otherwise (after prolongation). Each level is checked as it is
/// written: its table from the level below equals its quotient's under singletons.
///
/// # Errors
///
/// [`HierarchyError::Invalid`] for communities that are not a partition of `source`
/// numbered by smallest node, or a conservation check that fails; the errors of the
/// source, the budget and the writer. An error found before the result is added
/// (while the communities are checked and numbered) leaves the writer as it was. Any
/// error from adding the result on poisons it ([`Writer::poison`]), even one found
/// before a byte is written (a `key` of another graph), so that no commit can make a
/// result with missing parts, and the file keeps reading as its last commit, a
/// hierarchy stored before included.
pub fn store<G: Source, K: ByteSink>(
    writer: &mut Writer<K>,
    key: &ResultKey,
    about: &Provenance,
    source: &G,
    memory: &Memory,
    settings: Settings,
    communities: &Communities,
) -> Result<(), HierarchyError> {
    let maps = levels_of(source.nodes(), communities);
    store_levels(writer, key, about, source, memory, settings, &maps)
}

/// [`store`] for any nested partitions: `maps[0]` is the community of each node of
/// `source` at the finest level, and each next map the community of each community of
/// the level before, every one numbered by smallest node (each value at most one more
/// than the largest before it). So `maps.last()` is the partition, level 1.
///
/// # Errors
///
/// As [`store`]; [`HierarchyError::Invalid`] for no map.
pub fn store_levels<G: Source, K: ByteSink>(
    writer: &mut Writer<K>,
    key: &ResultKey,
    about: &Provenance,
    source: &G,
    memory: &Memory,
    settings: Settings,
    maps: &[&[u32]],
) -> Result<(), HierarchyError> {
    let run = Run {
        source,
        memory,
        settings,
        window: WINDOW,
    };
    store_with(writer, key, about, &run, maps, &[])
}

/// The graph a hierarchy is stored from, and how.
pub(crate) struct Run<'a, G> {
    pub(crate) source: &'a G,
    pub(crate) memory: &'a Memory,
    pub(crate) settings: Settings,
    /// Entries of a quotient encoded at a time, about.
    pub(crate) window: u64,
}

/// [`store_levels`] with the encoding window of `run`, and, unless `kinds` is empty, a
/// part `<level>/kind` per level: `kinds[j]` holds a value per community of `maps[j]`.
pub(crate) fn store_with<G: Source, K: ByteSink>(
    writer: &mut Writer<K>,
    key: &ResultKey,
    about: &Provenance,
    run: &Run<'_, G>,
    maps: &[&[u32]],
    kinds: &[&[u32]],
) -> Result<(), HierarchyError> {
    let n = run.source.nodes();
    if maps.is_empty() {
        return Err(HierarchyError::Invalid("communities: no level".into()));
    }
    let levels = numbered(run.memory, &chain(n, maps)?)?;
    let fits = kinds.len() == levels.len()
        && kinds
            .iter()
            .zip(&levels)
            .all(|(k, l)| k.len() == l.parts.count as usize);
    if !(kinds.is_empty() || fits) {
        return Err(HierarchyError::Invalid(
            "communities: kinds that are not one per community of each level".into(),
        ));
    }
    let written = write_levels(writer, key, about, run, &levels, kinds);
    if written.is_err() {
        writer.poison();
    }
    written
}

/// Every level, from the finest.
fn write_levels<G: Source, K: ByteSink>(
    writer: &mut Writer<K>,
    key: &ResultKey,
    about: &Provenance,
    run: &Run<'_, G>,
    levels: &[Level<'_>],
    kinds: &[&[u32]],
) -> Result<(), HierarchyError> {
    let (n, memory, settings) = (run.source.nodes(), run.memory, run.settings);
    let top = u32::try_from(levels.len())
        .map_err(|_| HierarchyError::Invalid("communities: too many levels".into()))?;
    writer.add_result(key, about, &[u64::from(top), u64::from(n)])?;
    let first = &levels[0];
    writer.add_result_part(
        key,
        MEMBERSHIP,
        first.parts.map.iter().map(|&c| first.new_of[c as usize]),
    )?;
    let mut below: Option<Quotient> = None;
    let mut total = None;
    for ((j, at), level) in levels.iter().enumerate().zip((1..=top).rev()) {
        let (q, table) = match &below {
            None => Plan::with(run.source, memory, settings, |plan| {
                aggregate(&plan, memory, Weights::Unit, at.parts)
            }),
            Some(b) => {
                let csr = b.csr()?;
                Plan::with(&csr, memory, settings, |plan| {
                    aggregate(&plan, memory, Weights::Entries(&b.weights), at.parts)
                })
            }
        }?;
        drop(below.take());
        let volume: u64 = table.volume.iter().sum();
        let finest = *total.get_or_insert(volume);
        if finest != volume {
            return Err(HierarchyError::Invalid(format!(
                "conservation: level {level} has volume {volume}, the finest {finest}"
            )));
        }
        write_level(writer, key, level, at, levels.get(j + 1), &table)?;
        if let Some(kinds) = kinds.get(j) {
            writer.add_result_part(
                key,
                &part(level, KIND),
                at.old_of.iter().map(|&r| kinds[r as usize]),
            )?;
        }
        write_quotient(writer, key, run, level, &q, at, &table)?;
        below = Some(q);
    }
    Ok(())
}

/// The levels of `c`, finest first: its levels when they compose to its membership,
/// else the membership alone.
fn levels_of(n: u32, c: &Communities) -> Vec<&[u32]> {
    let composes = !c.levels.is_empty()
        && c.levels[0].len() == n as usize
        && par::map_ranges(n as usize, GRAIN, |r| {
            r.into_iter().all(|v| {
                let up = c
                    .levels
                    .iter()
                    .try_fold(u32::try_from(v).expect("a node"), |x, l| {
                        l.get(x as usize).copied()
                    });
                up == c.membership.get(v).copied()
            })
        })
        .into_iter()
        .all(|ok| ok);
    if composes {
        c.levels.iter().map(|l| &l[..]).collect()
    } else {
        vec![&c.membership[..]]
    }
}

/// The maps of the levels, finest first, checked: each a partition of the level below
/// numbered by smallest node.
fn chain<'c>(n: u32, maps: &[&'c [u32]]) -> Result<Vec<Parts<'c>>, HierarchyError> {
    let mut len = n as usize;
    let mut out = Vec::new();
    for &map in maps {
        if map.len() != len {
            return Err(HierarchyError::Invalid(
                "communities: a level is not one value per node of the level below".into(),
            ));
        }
        let mut next = 0_u32;
        for &x in map {
            if x > next {
                return Err(HierarchyError::Invalid(
                    "communities: a level is not numbered by smallest node".into(),
                ));
            }
            next = next.max(x + 1);
        }
        out.push(Parts { map, count: next });
        len = next as usize;
    }
    Ok(out)
}

/// The node counts and the numbering of every level: by decreasing node count, then by
/// smallest node. Numbering by smallest node orders the communities of a level as their
/// smallest input nodes, so the old number breaks ties.
fn numbered<'c>(memory: &Memory, maps: &[Parts<'c>]) -> Result<Vec<Level<'c>>, HierarchyError> {
    let mut levels: Vec<Level<'c>> = Vec::new();
    for &parts in maps {
        let count = parts.count as usize;
        let size = Column::new(memory, "community nodes", count, |_| AtomicU32::new(0))?;
        let below = levels.last().map(|l| &l.size);
        par::for_each_range(parts.map.len(), GRAIN, |r| {
            for x in r {
                let s = below.map_or(1, |b| b[x]);
                size[parts.map[x] as usize].fetch_add(s, Relaxed);
            }
        });
        let size = size.map(AtomicU32::into_inner);
        let mut old_of = Column::new(memory, "community order", count, |c| {
            u32::try_from(c).expect("a community")
        })?;
        old_of.sort_unstable_by_key(|&c| (Reverse(size[c as usize]), c));
        let mut new_of = Column::new(memory, "community numbers", count, |_| 0_u32)?;
        for (r, &c) in old_of.iter().enumerate() {
            new_of[c as usize] = u32::try_from(r).expect("a community");
        }
        levels.push(Level {
            parts,
            size,
            old_of,
            new_of,
        });
    }
    Ok(levels)
}

/// The quotient graph of `parts` and its table, checked against each other: the
/// volume and the internal weight of every community, by two separate passes.
fn aggregate<S: Source>(
    plan: &Plan<'_, '_, S>,
    memory: &Memory,
    w: Weights<'_>,
    parts: Parts<'_>,
) -> Result<(Quotient, Table), Error> {
    let table = tally(plan, memory, w, parts.map, parts.count)?;
    let members = Members::new(memory, parts)?;
    let (q, k) = quotient(plan, memory, w, parts, &members, &Tables::new(memory))?;
    drop(members);
    let agree = par::map_ranges(parts.count as usize, GRAIN, |r| {
        r.into_iter().all(|c| {
            let row = ix(q.offsets[c])..ix(q.offsets[c + 1]);
            let inside = q.targets[row.clone()]
                .iter()
                .zip(&q.weights[row])
                .find(|&(&t, _)| t as usize == c)
                .map_or(0, |(_, &x)| x);
            k.of[c] == table.volume[c] && inside == 2 * table.internal[c]
        })
    });
    if !agree.into_iter().all(|ok| ok) {
        return Err(Error::Invalid(
            "conservation: a quotient row does not hold its community's volume and internal weight",
        ));
    }
    Ok((q, table))
}

/// The table parts of a level, in its new numbering, and its parents one level up.
fn write_level<K: ByteSink>(
    writer: &mut Writer<K>,
    key: &ResultKey,
    level: u32,
    at: &Level<'_>,
    up: Option<&Level<'_>>,
    table: &Table,
) -> Result<(), HierarchyError> {
    let old = |r: &u32| *r as usize;
    writer.add_result_part(
        key,
        &part(level, NODES),
        at.old_of.iter().map(|r| at.size[old(r)]),
    )?;
    writer.add_result_part(
        key,
        &part(level, INTERNAL),
        at.old_of.iter().map(|r| table.internal[old(r)]),
    )?;
    writer.add_result_part(
        key,
        &part(level, VOLUME),
        at.old_of.iter().map(|r| table.volume[old(r)]),
    )?;
    if let Some(up) = up {
        writer.add_result_part(
            key,
            &part(level, PARENT),
            at.old_of
                .iter()
                .map(|r| up.new_of[up.parts.map[old(r)] as usize]),
        )?;
    }
    Ok(())
}

/// One window of rows, encoded.
#[derive(Default)]
struct Piece {
    records: Vec<u8>,
    /// The degree and the record length of each row.
    lists: Vec<(u32, u64)>,
    /// The weight of each entry, as events: `A`, and half of it on the diagonal.
    events: Vec<u64>,
}

/// Rows `rows` of the quotient in the new numbering, each sorted by neighbour.
fn encode(
    q: &Quotient,
    at: &Level<'_>,
    rows: std::ops::Range<usize>,
) -> Result<Piece, HierarchyError> {
    let mut piece = Piece::default();
    let mut row = Vec::new();
    for r in rows {
        let c = at.old_of[r] as usize;
        let span = ix(q.offsets[c])..ix(q.offsets[c + 1]);
        row.clear();
        row.extend(
            q.targets[span.clone()]
                .iter()
                .zip(&q.weights[span])
                .map(|(&t, &x)| {
                    let t = at.new_of[t as usize];
                    (t, if t as usize == r { x / 2 } else { x })
                }),
        );
        row.sort_unstable_by_key(|&(t, _)| t);
        let before = piece.records.len();
        let r = u32::try_from(r).expect("a community");
        let degree = append_record(r, row.iter().map(|&(t, _)| t), &mut piece.records)?;
        piece
            .lists
            .push((degree, (piece.records.len() - before) as u64));
        piece.events.extend(row.iter().map(|&(_, x)| x));
    }
    Ok(piece)
}

/// The quotient graph of a level as a compressed adjacency in its new numbering: the
/// data streamed to the file a window at a time, then the index and the events.
fn write_quotient<G, K: ByteSink>(
    writer: &mut Writer<K>,
    key: &ResultKey,
    run: &Run<'_, G>,
    level: u32,
    q: &Quotient,
    at: &Level<'_>,
    table: &Table,
) -> Result<(), HierarchyError> {
    let (memory, window) = (run.memory, run.window);
    let count = at.parts.count;
    let entries = q.targets.len() as u64;
    let events = table.volume.iter().sum::<u64>() - table.internal.iter().sum::<u64>();
    let mut index = IndexBuilder::new(memory, count)?;
    let mut data = writer.begin_result_part(
        key,
        &part(level, &format!("{QUOTIENT}.data")),
        Kind::RESULT_ADJACENCY_DATA,
    )?;
    let degree = |r: usize| {
        let c = at.old_of[r] as usize;
        q.offsets[c + 1] - q.offsets[c]
    };
    let mut failed = None;
    let mut next = 0_usize;
    let mut pending = Vec::new().into_iter();
    // Pays for the window's buffers until its events have been taken.
    let mut held = None;
    let weights = std::iter::from_fn(|| {
        loop {
            if let Some(x) = pending.next() {
                return Some(x);
            }
            if next == count as usize || failed.is_some() {
                return None;
            }
            let start = next;
            let mut size = 0;
            while next < count as usize && (next == start || size < window) {
                size += degree(next);
                next += 1;
            }
            let bytes = size * (16 + 8 + record_bound(1)) + (next - start) as u64 * 16;
            drop(held.take());
            let mut step = || -> Result<Vec<u64>, HierarchyError> {
                held = Some(
                    memory
                        .reserve("quotient window", MemoryKind::Anonymous, bytes)
                        .map_err(Error::from)?,
                );
                let grain = NonZeroUsize::new((next - start).div_ceil(64)).expect("a row");
                let pieces = par::map_ranges(next - start, grain, |r| {
                    encode(q, at, start + r.start..start + r.end)
                });
                let mut out = Vec::new();
                for piece in pieces {
                    let piece = piece?;
                    data.write(&piece.records)?;
                    for (d, b) in piece.lists {
                        index.push(d, b)?;
                    }
                    out.extend(piece.events);
                }
                Ok(out)
            };
            match step() {
                Ok(w) => pending = w.into_iter(),
                Err(e) => failed = Some(e),
            }
        }
    });
    let mut events_held = None;
    let events_bytes = if events > entries {
        let bytes = events_section_bytes(entries, events)
            .ok_or_else(|| HierarchyError::Invalid("a quotient too large".into()))?;
        // The sequence, then its bytes beside it, as `encode_events` builds them.
        let reserved = memory
            .reserve(
                "quotient events",
                MemoryKind::Anonymous,
                bytes.saturating_mul(2),
            )
            .map_err(Error::from);
        let reserved = events_held.insert(reserved?);
        let section = encode_events(entries, events, weights);
        reserved.shrink(bytes);
        section.map(Some)
    } else {
        weights.for_each(drop);
        Ok(None)
    };
    if let Some(e) = failed {
        return Err(e);
    }
    let events_bytes = events_bytes?;
    drop(held);
    data.finish()?;
    let (index_bytes, _held) = index.finish(events_bytes.is_some())?;
    let name = |suffix: &str| part(level, &format!("{QUOTIENT}.{suffix}"));
    let mut section =
        writer.begin_result_part(key, &name("index"), Kind::RESULT_ADJACENCY_INDEX)?;
    section.write(&index_bytes)?;
    section.finish()?;
    if let Some(bytes) = events_bytes {
        let mut section =
            writer.begin_result_part(key, &name("events"), Kind::RESULT_ADJACENCY_EVENTS)?;
        section.write(&bytes)?;
        section.finish()?;
    }
    Ok(())
}
