// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The super level above the partition (ADR 0016; `design/DECISIONS.md`, "The map's
//! super level"). The rule that decides it is [`wanted`], and only it.

use std::cmp::Reverse;
use std::num::NonZeroUsize;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering::Relaxed;

use everygraph_core::{Memory, par};
use everygraph_kernels::edge_map::{Error, Settings, Source};

use super::SuperLevel;
use crate::aggregate::Members;
use crate::column::Column;
use crate::leiden::{Coarsening, leiden_with};
use crate::{Communities, Options};

/// Resolutions tried, `2^-1` to `2^-16` (ADR 0016).
const TRIES: u32 = 16;

/// The smallest number of the `count` sizes `size(i)`, largest first, that add up to
/// 90% of their total or more.
pub(super) fn cover(
    memory: &Memory,
    count: usize,
    size: impl Fn(usize) -> u64 + Send + Sync,
) -> Result<u32, Error> {
    let mut sizes = Column::new(memory, "cover sizes", count, size)?;
    sizes.sort_unstable_by_key(|&s| Reverse(s));
    let total: u64 = sizes.iter().sum();
    let mut held = 0;
    for (i, &s) in sizes.iter().enumerate() {
        if 10 * held >= 9 * total {
            return Ok(u32::try_from(i).expect("a count"));
        }
        held += s;
    }
    Ok(u32::try_from(count).expect("a count"))
}

/// Whether the map gets a super level, given the cover of the partition. A first map
/// gets one above `F − 1`; a rebuild keeps the previous decision while the cover stays
/// within `F − F/8 ..= F + F/8` (56 to 72 at `F = 64`). The band is a heuristic with
/// no prior art (ADR 0016).
pub(super) fn wanted(cover: u32, fan_out: u32, mode: SuperLevel) -> bool {
    match mode {
        SuperLevel::Always => true,
        SuperLevel::Never => false,
        SuperLevel::Auto { previous: None } => cover >= fan_out,
        SuperLevel::Auto {
            previous: Some(before),
        } => {
            if cover > fan_out + fan_out / 8 {
                true
            } else if cover < fan_out - fan_out / 8 {
                false
            } else {
                before
            }
        }
    }
}

/// The group of every community of the partition.
pub(super) struct Groups {
    /// The group of each community, `0..count`; any value for an empty community.
    pub(super) of: Column<u32>,
    pub(super) count: u32,
    /// The resolution of the Leiden run that found them.
    pub(super) resolution: f64,
}

/// The super level of the partition whose communities hold `t`: [`coarse_leiden`] at
/// resolution `2^-j` for `j` from 1 until its groups need at most `F − 1`
/// communities to cover 90% of the nodes (or `j = 16`). Each community goes to the
/// community of that run that holds most of its nodes, the smallest on a tie, so the
/// levels nest.
pub(super) fn groups<S: Source>(
    source: &S,
    memory: &Memory,
    settings: Settings,
    options: &Options,
    t: &Members,
    count: u32,
    fan_out: u32,
) -> Result<Groups, Error> {
    let mut resolution = 1.0;
    let mut tries = 0;
    loop {
        resolution /= 2.0;
        tries += 1;
        let found = coarse_leiden(source, memory, settings, options, resolution)?;
        let vote = vote(memory, t, count, &found.membership)?;
        drop(found);
        // Number the groups densely, in the order of their first community.
        let mut dense = Column::new(memory, "super groups", t.ids.len(), |_| u32::MAX)?;
        let mut next = 0_u32;
        let mut of = Column::new(memory, "super groups", count as usize, |_| 0_u32)?;
        for c in 0..count as usize {
            let g = vote[c];
            if g != u32::MAX {
                let d = &mut dense[g as usize];
                if *d == u32::MAX {
                    *d = next;
                    next += 1;
                }
                of[c] = *d;
            }
        }
        drop(dense);
        let mut nodes = Column::new(memory, "super sizes", next as usize, |_| 0_u64)?;
        for c in 0..count {
            if vote[c as usize] != u32::MAX {
                nodes[of[c as usize] as usize] += t.of(c).len() as u64;
            }
        }
        if tries == TRIES || cover(memory, nodes.len(), |g| nodes[g])? < fan_out {
            return Ok(Groups {
                of,
                count: next,
                resolution,
            });
        }
    }
}

/// The Leiden run of the super level: Leiden with `options` at `resolution` on the
/// whole graph, its levels coarsened while they shrink the graph by 1% or more, up to
/// 60 levels. The usual rule of [`leiden`](crate::leiden) stops early at a low
/// resolution on road networks: roadnet-ca at `2^-7` gave 122 groups to cover 90% of
/// the nodes against 16 (#19, `lab/m1-05-hierarchy-shaping/shape.py`, `upward`).
///
/// # Errors
///
/// As [`leiden`](crate::leiden).
pub fn coarse_leiden<S: Source>(
    source: &S,
    memory: &Memory,
    settings: Settings,
    options: &Options,
    resolution: f64,
) -> Result<Communities, Error> {
    let run = Options {
        resolution,
        ..*options
    };
    leiden_with(source, memory, settings, &run, Coarsening::DEEP)
}

/// For each community of `t`, the label of `found` that most of its nodes have, the
/// smallest on a tie; `u32::MAX` for an empty community.
fn vote(memory: &Memory, t: &Members, count: u32, found: &[u32]) -> Result<Column<u32>, Error> {
    let mut labels = Column::new(memory, "super votes", t.ids.len(), |i| {
        found[t.ids[i] as usize]
    })?;
    let out = Column::new(memory, "super votes", count as usize, |_| {
        AtomicU32::new(u32::MAX)
    })?;
    par::for_each_range_mut(
        &mut labels[..],
        &t.starts,
        NonZeroUsize::new(64).expect("non-zero"),
        |c, mine| {
            mine.sort_unstable();
            let mut best = (0, u32::MAX);
            let mut run = 0;
            for (i, &x) in mine.iter().enumerate() {
                run = if i > 0 && mine[i - 1] == x {
                    run + 1
                } else {
                    1
                };
                if run > best.0 {
                    best = (run, x);
                }
            }
            out[c].store(best.1, Relaxed);
        },
    );
    Ok(out.map(AtomicU32::into_inner))
}
