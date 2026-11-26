// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Leiden (Traag, Waltman and van Eck 2019) with the choices of DESIGN D14 and D15: the
//! driver over levels and iterations, and its options and result.
//!
//! One iteration: on each level, local moving, then refinement inside the communities
//! found, then aggregation of the refined communities into the next level, whose nodes
//! start in the community they were refined from. A level stops the iteration when
//! nothing moved, when it is the tenth, or when refinement left more than 80% of its
//! nodes alone (nodes without a neighbour other than themselves are left out of that
//! count). The last level's communities are split into their connected components, so
//! no community of the result is disconnected. A further iteration starts again from
//! level 0, in the communities of the previous result.

use std::num::NonZeroU32;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering::Relaxed;

use everygraph_core::Memory;
use everygraph_kernels::edge_map::{Error, NodeCells, Plan, Settings, Source};

use crate::aggregate::{Dense, Members, Quotient, dense, quotient};
use crate::column::Column;
use crate::level::{
    Ctx, Graph, Moving, Schedule, Strengths, Weights, count, local_move, refine, split, strengths,
};
use crate::quality::tally;
use crate::table::Tables;

/// Iterations of the quality mode.
const QUALITY_ITERATIONS: NonZeroU32 = NonZeroU32::new(3).expect("non-zero");
/// Levels per iteration, at most.
const MAX_LEVELS: u32 = 10;
/// Local moving stops when a sweep gains at most this much modularity: `0.01` on level
/// 0 of iteration 0, ten times less for each level and each iteration after.
const TOLERANCE: [f64; 12] = [
    1e-2, 1e-3, 1e-4, 1e-5, 1e-6, 1e-7, 1e-8, 1e-9, 1e-10, 1e-11, 1e-12, 1e-13,
];

/// Whether the result must not depend on the schedule.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// The same result for the same input and seed, at any thread count, chunking and
    /// source (DESIGN D14). The default.
    #[default]
    Deterministic,
    /// Moves applied as they are found, by every thread at once: faster, and its result
    /// changes from run to run. Opt-in, and labelled in the output; whether it ships
    /// depends on #17.
    Fast,
}

/// How many iterations run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Iterations {
    /// This many. One is the default; three is the quality mode (DESIGN D15).
    Fixed(NonZeroU32),
    /// Until an iteration returns the partition it started from, or `max` iterations.
    Converge {
        /// The most iterations.
        max: NonZeroU32,
    },
}

/// How refinement chooses the refined community a node joins.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Refinement {
    /// The best positive gain (the lab's choice, `lab/b2-leiden`).
    Greedy,
    /// A draw among the gains of at least zero, with probability proportional to
    /// `exp(gain / theta)`, the gain in units of edge weight (Traag et al. 2019; igraph
    /// takes `theta = 0.01`).
    Randomized {
        /// The randomness, above 0.
        theta: f64,
    },
    /// None: Louvain (Blondel et al. 2008), aggregating the communities of local moving,
    /// with no split. An internal baseline, never a default: its communities can be
    /// disconnected (17-94 per run in the lab, FINDINGS § B2).
    Louvain,
}

/// When disconnected communities are split into their components.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SplitAt {
    /// After the last level of each iteration.
    #[default]
    Last,
    /// Also after local moving on every level (GSP-Louvain).
    EveryLevel,
}

/// What [`leiden`] does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Options {
    /// Deterministic (the default) or fast.
    pub mode: Mode,
    /// One iteration by default.
    pub iterations: Iterations,
    /// The resolution `γ` of modularity; 1 by default.
    pub resolution: f64,
    /// Seeds the visiting order, the sub-rounds, the ties and randomized refinement.
    pub seed: u64,
    /// Greedy by default.
    pub refinement: Refinement,
    /// After the last level, go back down: on each finer level, local moving starts from
    /// the communities found above (prolongation, as `NetworKit`'s PLMR). Keeps every
    /// level's quotient graph in memory.
    pub prolong: bool,
    /// After the last level by default.
    pub split: SplitAt,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            mode: Mode::Deterministic,
            iterations: Iterations::Fixed(NonZeroU32::MIN),
            resolution: 1.0,
            seed: 0,
            refinement: Refinement::Greedy,
            prolong: false,
            split: SplitAt::Last,
        }
    }
}

impl Options {
    /// The quality mode: three iterations (DESIGN D15).
    #[must_use]
    pub fn quality() -> Self {
        Self {
            iterations: Iterations::Fixed(QUALITY_ITERATIONS),
            ..Self::default()
        }
    }
}

/// What one level of one iteration did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LevelStats {
    /// The iteration, from 0; prolongation counts as part of its iteration.
    pub iteration: u32,
    /// The level, from 0.
    pub level: u32,
    /// Its nodes and entries.
    pub nodes: u32,
    /// Its entries.
    pub entries: u64,
    /// Sweeps of local moving.
    pub sweeps: u32,
    /// Moves of local moving.
    pub moves: u64,
    /// Sub-rounds run by all threads and on the calling thread (deterministic mode).
    pub parallel_rounds: u64,
    /// See `parallel_rounds`.
    pub serial_rounds: u64,
    /// Communities after refinement, or after the split on the last level.
    pub communities: u32,
    /// Communities the split added.
    pub split_added: u32,
}

/// The result of [`leiden`].
#[derive(Debug)]
pub struct Communities {
    /// The community of each node, numbered from 0 in the order of each community's
    /// smallest node.
    pub membership: Column<u32>,
    /// The number of communities.
    pub count: u32,
    /// The modularity of `membership` at the run's resolution.
    pub modularity: f64,
    /// The levels of the last iteration: `levels[0]` maps the nodes to the nodes of
    /// level 1 (refined communities), and so on; the last maps to the communities.
    /// Without prolongation, `membership` is their composition.
    pub levels: Vec<Column<u32>>,
    /// The mode that produced it.
    pub mode: Mode,
    /// Iterations run.
    pub iterations: u32,
    /// One entry per level run, in order.
    pub stats: Vec<LevelStats>,
}

/// What a level hands to the driver.
struct Level {
    /// Its nodes' places one level up; `None` if the level merged nothing and so ends
    /// the iteration where the level below ended.
    map: Option<Column<u32>>,
    /// The next level, its strengths and its nodes' starting communities (alone for
    /// Louvain), unless this was the last.
    next: Option<(Quotient, Strengths, Option<Column<u32>>)>,
    /// The modularity of the communities of the last level.
    modularity: f64,
    stats: LevelStats,
}

/// The labels of `cells`, as a column.
fn labels<'g, S: Source>(
    plan: &Plan<'_, 'g, S>,
    memory: &Memory,
    cells: &NodeCells<'g>,
) -> Result<Column<u32>, Error> {
    let mut out = Column::new(memory, "labels", plan.nodes() as usize, |_| 0_u32)?;
    plan.vertices(&mut out[..], |(): &mut (), out, i, v, _| {
        out[i] = cells.load(v, Relaxed).get();
    })?;
    Ok(out)
}

/// One level: local moving, refinement, and either the aggregation into the next level
/// or the end of the iteration.
fn level<S: Source>(
    plan: &Plan<'_, '_, S>,
    memory: &Memory,
    w: Weights<'_>,
    k: &Strengths,
    bound: Option<&[u32]>,
    ctx: &Ctx<'_>,
) -> Result<Level, Error> {
    let n = plan.nodes();
    let mut stats = LevelStats {
        iteration: ctx.iteration,
        level: ctx.level,
        nodes: n,
        entries: plan.source().entries(),
        ..LevelStats::default()
    };
    let tables = Tables::new(memory);
    let schedule = Schedule::new(n, memory, ctx, false)?;
    let mut st = Moving::new(plan, memory, &k.of, bound, ctx)?;
    let tol = TOLERANCE[(ctx.iteration + ctx.level).min(11) as usize];
    let g = Graph { plan, w, k: &k.of };
    let moved = local_move(g, &st, &schedule, &tables, ctx, tol)?;
    (stats.sweeps, stats.moves) = (moved.sweeps, moved.moves);

    (stats.parallel_rounds, stats.serial_rounds) = (moved.parallel_rounds, moved.serial_rounds);
    if ctx.options.split == SplitAt::EveryLevel {
        let before = count(plan, memory, &st.comm)?;
        let parts = split(plan, memory, &st.comm)?;
        st.relabel(plan, &k.of, parts)?;
        stats.split_added = count(plan, memory, &st.comm)? - before;
    }
    let louvain = ctx.options.refinement == Refinement::Louvain;
    let refined = if louvain {
        None
    } else {
        let schedule = match schedule {
            Schedule::Rounds { .. } => {
                drop(schedule);
                Schedule::new(n, memory, ctx, true)?
            }
            order @ Schedule::Order { .. } => order,
        };
        Some(refine(g, memory, &st, &schedule, &tables, ctx)?)
    };
    let d = match &refined {
        Some(r) => dense(plan, memory, |v| r.rcomm.load(v, Relaxed).get())?,
        None => dense(plan, memory, |v| st.comm.load(v, Relaxed).get())?,
    };
    stats.communities = d.count;
    let alone = k.alone;
    let first_of_later = ctx.iteration > 0 && ctx.level == 0;
    let last = (stats.moves == 0 && !first_of_later)
        || ctx.level + 1 == MAX_LEVELS
        || u64::from(stats.communities - alone) * 5 > u64::from(n - alone) * 4;
    if last {
        let d = if louvain {
            d
        } else {
            drop((d, refined));
            let before = count(plan, memory, &st.comm)?;
            let parts = split(plan, memory, &st.comm)?;
            drop(st);
            let d = dense(plan, memory, |v| parts.load(v, Relaxed).get())?;
            stats.split_added += d.count - before;
            d
        };
        stats.communities = d.count;
        let modularity =
            tally(plan, memory, w, &d.map, d.count)?.modularity(ctx.options.resolution);
        let map = (d.count < n || ctx.level == 0).then_some(d.map);
        return Ok(Level {
            map,
            next: None,
            modularity,
            stats,
        });
    }
    drop(refined);
    let members = Members::new(memory, &d)?;
    let (q, kq) = quotient(plan, memory, w, &d, &members, &tables)?;
    let bound = if louvain {
        None
    } else {
        Some(next_bound(plan, memory, &st, &d, &members)?)
    };
    Ok(Level {
        map: Some(d.map),
        next: Some((q, kq, bound)),
        modularity: 0.0,
        stats,
    })
}

/// The starting community of each node of the next level: the community of local
/// moving its members were in, named by the smallest next-level node inside it.
fn next_bound<'g, S: Source>(
    plan: &Plan<'_, 'g, S>,
    memory: &Memory,
    st: &Moving<'g>,
    d: &Dense,
    members: &Members,
) -> Result<Column<u32>, Error> {
    let n = plan.nodes() as usize;
    let least = Column::new(memory, "least refined", n, |_| AtomicU32::new(u32::MAX))?;
    plan.vertices((), |(): &mut (), (), _, v, _| {
        least[st.comm.load(v, Relaxed).index()].fetch_min(d.map[v.index()], Relaxed);
    })?;
    let bound = Column::new(memory, "starting communities", d.count as usize, |c| {
        let first = members.of(u32::try_from(c).expect("a community"))[0];
        let v = plan.node(first).expect("a member is a node");
        least[st.comm.load(v, Relaxed).index()].load(Relaxed)
    })?;
    Ok(bound)
}

/// Local moving on a level from the communities `start`, and the split of the result:
/// one step of prolongation.
fn polish<S: Source>(
    plan: &Plan<'_, '_, S>,
    memory: &Memory,
    w: Weights<'_>,
    k: &Strengths,
    start: &[u32],
    ctx: &Ctx<'_>,
) -> Result<(Column<u32>, LevelStats), Error> {
    let n = plan.nodes();
    let tables = Tables::new(memory);
    let schedule = Schedule::new(n, memory, ctx, false)?;
    let st = Moving::new(plan, memory, &k.of, Some(start), ctx)?;
    let tol = TOLERANCE[(ctx.iteration + ctx.level).min(11) as usize];
    let moved = local_move(
        Graph { plan, w, k: &k.of },
        &st,
        &schedule,
        &tables,
        ctx,
        tol,
    )?;
    let parts = if ctx.level == 0 {
        split(plan, memory, &st.comm)?
    } else {
        st.comm
    };
    let out = labels(plan, memory, &parts)?;
    let stats = LevelStats {
        iteration: ctx.iteration,
        level: ctx.level,
        nodes: n,
        entries: plan.source().entries(),
        sweeps: moved.sweeps,
        moves: moved.moves,
        parallel_rounds: moved.parallel_rounds,
        serial_rounds: moved.serial_rounds,
        ..LevelStats::default()
    };
    Ok((out, stats))
}

/// `labels` of a level's nodes taken through `map` one level up: `out[v] =
/// above[map[v]]`.
fn project(memory: &Memory, map: &[u32], above: &[u32]) -> Result<Column<u32>, Error> {
    Ok(Column::new(memory, "membership", map.len(), |v| {
        above[map[v] as usize]
    })?)
}

/// Checks the options.
fn check(options: &Options) -> Result<(), Error> {
    if !(options.resolution.is_finite() && options.resolution >= 0.0) {
        return Err(Error::Invalid(
            "resolution: not a finite number of at least 0",
        ));
    }
    if let Refinement::Randomized { theta } = options.refinement {
        if !(theta.is_finite() && theta > 0.0) {
            return Err(Error::Invalid("theta: not a finite number above 0"));
        }
    }
    Ok(())
}

/// Leiden communities of an undirected, unweighted graph: each list holds the node's
/// neighbours, both directions of each pair, a kept loop once. `settings` cut the work
/// and never change a deterministic result.
///
/// # Errors
///
/// [`Error::Invalid`] for options out of range or a total weight of 2^62 or more; the
/// source's errors; [`Error::Memory`] when the budget refuses a buffer, before it is
/// allocated.
///
/// # Panics
///
/// Never for a source that keeps the contract of
/// [`Source`](everygraph_kernels::edge_map::Source).
pub fn leiden<S: Source>(
    source: &S,
    memory: &Memory,
    settings: Settings,
    options: &Options,
) -> Result<Communities, Error> {
    check(options)?;
    let n = source.nodes();
    let k0 = Plan::with(source, memory, settings, |plan| {
        strengths(&plan, memory, Weights(None))
    })?;
    let total = k0.total;
    if total == 0 {
        let single = Column::new(memory, "membership", n as usize, |v| {
            u32::try_from(v).expect("a node id")
        })?;
        return Ok(Communities {
            membership: single,
            count: n,
            modularity: 0.0,
            levels: Vec::new(),
            mode: options.mode,
            iterations: 0,
            stats: Vec::new(),
        });
    }
    let (max, converge) = match options.iterations {
        Iterations::Fixed(k) => (k.get(), false),
        Iterations::Converge { max } => (max.get(), true),
    };
    let mut stats = Vec::new();
    let mut previous: Option<(Column<u32>, u32, f64)> = None;
    let mut levels = Vec::new();
    let mut iterations = 0;
    for iteration in 0..max {
        iterations += 1;
        levels.clear();
        let mut graphs: Vec<(Quotient, Strengths)> = Vec::new();
        let mut ctx = Ctx {
            options,
            total,
            level: 0,
            iteration,
            stage: 0,
        };
        let bound = previous.as_ref().map(|p| &p.0[..]);
        let mut out = Plan::with(source, memory, settings, |plan| {
            level(&plan, memory, Weights(None), &k0, bound, &ctx)
        })?;
        loop {
            stats.push(out.stats);
            levels.extend(out.map.take());
            let Some((q, kq, bound)) = out.next.take() else {
                break;
            };
            ctx.level += 1;
            let csr = q.csr()?;
            let w = Weights(Some(&q.weights));
            out = Plan::with(&csr, memory, settings, |plan| {
                level(&plan, memory, w, &kq, bound.as_deref(), &ctx)
            })?;
            if options.prolong {
                graphs.push((q, kq));
            }
        }
        let mut membership = compose(memory, &levels)?;
        let mut modularity = out.modularity;
        let mut communities = out.stats.communities;
        if options.prolong {
            ctx.stage = 1;
            let top = graphs.len();
            let mut above = match (levels.get(top), graphs.last()) {
                (Some(map), _) => Column::new(memory, "membership", map.len(), |v| map[v])?,
                (None, Some((q, _))) => {
                    Column::new(memory, "membership", q.offsets.len() - 1, |v| {
                        u32::try_from(v).expect("a node id")
                    })?
                }
                (None, None) => unreachable!("level 0 always keeps its map"),
            };
            for at in (1..=top).rev() {
                ctx.level = u32::try_from(at).expect("a level");
                let (q, kq) = &graphs[at - 1];
                let csr = q.csr()?;
                let w = Weights(Some(&q.weights));
                let (labels, s) = Plan::with(&csr, memory, settings, |plan| {
                    polish(&plan, memory, w, kq, &above, &ctx)
                })?;
                stats.push(s);
                above = project(memory, &levels[at - 1], &labels)?;
            }
            ctx.level = 0;
            let (labels, s) = Plan::with(source, memory, settings, |plan| {
                polish(&plan, memory, Weights(None), &k0, &above, &ctx)
            })?;
            stats.push(s);
            let (m, c, q) = Plan::with(source, memory, settings, |plan| {
                let d = dense(&plan, memory, |v| labels[v.index()])?;
                let q = tally(&plan, memory, Weights(None), &d.map, d.count)?
                    .modularity(options.resolution);
                Ok::<_, Error>((d.map, d.count, q))
            })?;
            (membership, communities, modularity) = (m, c, q);
        }
        let stable = converge && previous.as_ref().is_some_and(|p| p.0[..] == membership[..]);
        previous = Some((membership, communities, modularity));
        if stable {
            break;
        }
    }
    let (membership, count, modularity) = previous.expect("at least one iteration");
    Ok(Communities {
        membership,
        count,
        modularity,
        levels,
        mode: options.mode,
        iterations,
        stats,
    })
}

/// The composition of `levels`: the place of each node of the first in the last.
fn compose(memory: &Memory, levels: &[Column<u32>]) -> Result<Column<u32>, Error> {
    let n = levels.first().map_or(0, |l| l.len());
    Ok(Column::new(memory, "membership", n, |v| {
        levels
            .iter()
            .fold(u32::try_from(v).expect("a node id"), |x, l| l[x as usize])
    })?)
}
