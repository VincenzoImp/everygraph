// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The steps of one level of Leiden: strengths, local moving, refinement, and the split
//! of disconnected communities.
//!
//! # Weights
//!
//! Every weight is an integer in the units of the adjacency matrix `A` of
//! `docs/spec/graph-semantics.md` § 3: an entry to another node holds `A[u][v]`, an
//! entry of a node to itself holds `A[v][v]` = twice the loop's weight. At level 0 the
//! graph is unweighted (1 per entry, 2 per kept loop). At coarser levels each entry
//! holds the sum of `A` between two communities, and the diagonal entry the sum inside
//! one. So every sum here (strengths, community volumes, quotient weights) is exact in
//! any order (`docs/research/README.md` N1), and the deterministic result does not
//! depend on threads, chunking or the source. Gains are `f64` expressions of these
//! exact integers.
//!
//! # Deterministic mode
//!
//! A sweep visits the nodes in sub-rounds. A node's sub-round is drawn from the seed:
//! in local moving at level 0 for groups of 16 consecutive nodes (so that a sub-round
//! reads runs of neighbouring lists), otherwise for each node. The count of sub-rounds
//! per level is `clamp(next_pow2(m / 65,536), 4, 1,024)` (`lab/m1-04-det-leiden-scaling`,
//! rule L64k). In a sub-round every node decides against the state frozen at its start;
//! the moves are then committed with commutative integer updates. A sub-round of fewer
//! than 1,024 nodes runs on the calling thread. Two singletons never swap: one only
//! joins the other if the other's label is smaller.
//!
//! Here `m` is the number of nodes with an entry, and every draw is counted by a node's
//! place among them ([`Ranks`]): a node without an entry never moves and no node joins
//! it. Ties go by the draws of the labels, then by the labels' order, and a community
//! starts named by its smallest node. So the result does not depend on the nodes
//! without an entry, nor on the ids beyond their order: a view as a mask over its graph
//! and the same view materialized give the same communities (DESIGN D14).
//!
//! # Fast mode
//!
//! A sweep visits blocks of 64 consecutive ids in a seeded random order and applies
//! each move at once; the heavy nodes are visited after it by one thread ([`heavy`]).
//! Its result depends on the schedule. A node may also decide on a neighbour's old
//! community and miss the wake-up of that neighbour's move (its flag is cleared while
//! the neighbour writes): like the other races of the mode, a cost in quality, and the
//! sweep can stop while such a node still has a move.

use std::num::NonZeroUsize;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64};

use everygraph_core::rng::{Purpose, draw, unit_f64};
use everygraph_core::{Memory, MemoryKind, Reservation, math, par};
use everygraph_format::adjacency::AdjacencyEvents;
use everygraph_kernels::edge_map::{
    Error, FRONTIER_GRAIN, Node, NodeCells, Plan, Reduce, Row, Source, Worker, all,
};

use crate::column::{Column, GRAIN};
use crate::leiden::{Mode, Options, Refinement};
use crate::table::{Lent, Table, Tables};

/// No community: a decision not to move.
const NONE: u32 = u32::MAX;
/// Sweeps of local moving per level, at most.
const MAX_SWEEPS: u32 = 20;
/// Sub-rounds with fewer nodes run on the calling thread.
const SERIAL: usize = 1024;
/// A node is heavy when its strength is above `2W / HEAVY` ([`heavy`]).
const HEAVY: u64 = 1000;
/// Gains are added as fixed-point integers with this unit, 2^-50: exact in any order.
const GAIN_SCALE: f64 = 1_125_899_906_842_624.0;
/// Ids per block of the fast mode's visiting order (ADR 0015).
const ORDER_BLOCK: u32 = 64;
/// In local moving at level 0, this many low bits of an id are dropped before its
/// sub-round is drawn.
const ROUND_SHIFT: u32 = 4;
/// Nodes per task when a sub-round's nodes are taken or its moves committed: a
/// sub-round of 1,024 nodes or more is spread over every thread.
const ROUND_GRAIN: NonZeroUsize = NonZeroUsize::new(1 << 10).expect("non-zero");

/// The weights of a level's entries, in the units of `A`.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Weights<'a> {
    /// 1 per entry and 2 per loop: level 0, an unweighted graph.
    Unit,
    /// One value per entry: a quotient graph in memory.
    Entries(&'a [u64]),
    /// A stored quotient graph, whose entries hold their weight as events: `w(p)`
    /// events, so `A` is that and twice that for a loop. Checked whole before use.
    Events(&'a AdjacencyEvents<'a>),
}

impl Weights<'_> {
    /// Calls `f(v, A[u][v])` for each entry of `row`, where `u = row.node`.
    #[inline(always)]
    pub(crate) fn each<'g>(self, row: &Row<'_, 'g>, mut f: impl FnMut(Node<'g>, u64)) {
        match self {
            Self::Unit => {
                for &v in row.list {
                    f(v, if v == row.node { 2 } else { 1 });
                }
            }
            Self::Entries(w) => {
                for (&v, &x) in row.list.iter().zip(&w[row.entries()]) {
                    f(v, x);
                }
            }
            Self::Events(events) => {
                for (&v, e) in row.list.iter().zip(row.first_entry..) {
                    let r = events.event_range(e).expect("the events were checked");
                    let x = r.end - r.start;
                    // A reader refuses a stored quotient of 2^62 events or more
                    // (`hierarchy::check`): its sums never reach 2^64.
                    f(
                        v,
                        if v == row.node {
                            x.saturating_mul(2)
                        } else {
                            x
                        },
                    );
                }
            }
        }
    }
}

/// A level's graph: its plan, its weights and its strengths.
pub(crate) struct Graph<'a, 's, 'g, S: Source> {
    pub(crate) plan: &'a Plan<'s, 'g, S>,
    pub(crate) w: Weights<'a>,
    pub(crate) k: &'a [u64],
}

impl<S: Source> Clone for Graph<'_, '_, '_, S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S: Source> Copy for Graph<'_, '_, '_, S> {}

impl<'g, S: Source> Graph<'_, '_, 'g, S> {
    /// The node of id `v`: a community label is always one.
    fn node(&self, v: u32) -> Node<'g> {
        node(self.plan, v)
    }
}

/// Where a step runs: the options, the graph's total weight and the position of the
/// level in the run. The last three name every random draw (`everygraph_core::rng`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Ctx<'o> {
    pub(crate) options: &'o Options,
    /// The sum of all strengths, `2W`.
    pub(crate) total: u64,
    pub(crate) level: u32,
    pub(crate) iteration: u32,
    /// 0 for the passes of an iteration, 1 for its prolongation.
    pub(crate) stage: u32,
}

impl Ctx<'_> {
    fn det(&self) -> bool {
        self.options.mode == Mode::Deterministic
    }

    fn inv_total(&self) -> f64 {
        1.0 / self.total as f64
    }

    fn counter(&self, entity: u32) -> [u32; 4] {
        [entity, self.level, self.iteration, self.stage]
    }
}

/// Whether label `a` goes before label `b` as a candidate of equal value, for a node
/// whose draw is `mine`: by the labels' draws in `salt`, then by their order. A total
/// order that changes with the seed and not with the ids (module documentation).
fn before(salt: &[u32], mine: u32, a: u32, b: u32) -> bool {
    let key = |x: u32| (salt[x as usize] ^ mine).wrapping_mul(0x9E37_79B1);
    (key(a), a) < (key(b), b)
}

/// Nodes per block of [`Ranks`].
const RANK_BLOCK: NonZeroUsize = NonZeroUsize::new(1 << 16).expect("non-zero");

/// The nodes of a level with an entry (a positive strength), numbered from 0 in id
/// order: the counter of every draw.
pub(crate) struct Ranks<'k> {
    k: &'k [u64],
    /// The number of the first such node of each block of [`RANK_BLOCK`] nodes, and
    /// the count after the last block.
    starts: Vec<u32>,
}

impl<'k> Ranks<'k> {
    pub(crate) fn new(k: &'k [u64]) -> Self {
        let counts = par::map_ranges(k.len(), RANK_BLOCK, |r| {
            let c = k[r].iter().filter(|&&s| s > 0).count();
            u32::try_from(c).expect("a node count")
        });
        let starts = std::iter::once(0)
            .chain(counts.iter().scan(0, |at, &c| {
                *at += c;
                Some(*at)
            }))
            .collect();
        Self { k, starts }
    }

    /// How many nodes have an entry.
    pub(crate) fn count(&self) -> u32 {
        *self.starts.last().expect("never empty")
    }

    fn blocks(&self) -> usize {
        self.starts.len() - 1
    }

    /// Calls `f(v, r)` for each node `v` with an entry in block `b`, in order, where
    /// `r` is its number.
    fn each(&self, b: usize, mut f: impl FnMut(u32, u32)) {
        let first = b * RANK_BLOCK.get();
        let nodes = first..(first + RANK_BLOCK.get()).min(self.k.len());
        let mut r = self.starts[b];
        for (v, &s) in nodes.clone().zip(&self.k[nodes]) {
            if s > 0 {
                f(u32::try_from(v).expect("a node id"), r);
                r += 1;
            }
        }
    }

    /// One value per node: `f(r)` for the node of number `r`, `T::default()` for a node
    /// without an entry.
    fn column<T: Copy + Default + Send + Sync>(
        &self,
        memory: &Memory,
        purpose: &'static str,
        f: impl Fn(u32) -> T + Sync,
    ) -> Result<Column<T>, Error> {
        let mut out = Column::new(memory, purpose, self.k.len(), |_| T::default())?;
        par::for_each_chunk_mut(&mut out[..], RANK_BLOCK, |first, chunk| {
            self.each(first / RANK_BLOCK, |v, r| {
                chunk[v as usize - first] = f(r);
            });
        });
        Ok(out)
    }
}

/// A gain as a fixed-point integer. A move's claimed gain is positive and at most
/// `2(1 + γ)·k_u / 2W`, so a sweep's gains add to at most `2(1 + γ)`, below 2^64 in
/// these units for the resolutions `leiden` accepts.
fn fixed(gain: f64) -> u64 {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "positive, and below 2^64 once scaled (above)"
    )]
    let units = (gain * GAIN_SCALE) as u64;
    units
}

/// `x` as a float. Every weight sum is below 2^62 (`strengths`), so the signed
/// conversion is exact in range, and it is one instruction where the unsigned one is
/// several.
#[inline]
fn real(x: u64) -> f64 {
    #[expect(clippy::cast_possible_wrap, reason = "below 2^62")]
    let x = x as i64;
    x as f64
}

/// The node of id `v` of a plan: a community label is always one.
fn node<'g, S: Source>(plan: &Plan<'_, 'g, S>, v: u32) -> Node<'g> {
    plan.node(v).expect("a community label is a node")
}

/// Strengths `s(v)` of a level, their sum `2W`, and the nodes without a neighbour other
/// than themselves: they stay alone at every level, so the stopping test leaves them out.
pub(crate) struct Strengths {
    pub(crate) of: Column<u64>,
    pub(crate) total: u64,
    pub(crate) alone: u32,
}

pub(crate) fn strengths<S: Source>(
    plan: &Plan<'_, '_, S>,
    memory: &Memory,
    w: Weights<'_>,
) -> Result<Strengths, Error> {
    let n = plan.nodes() as usize;
    let mut of = Column::new(memory, "strengths", n, |_| 0_u64)?;
    let (total, alone): (u64, u32) =
        plan.scan(&mut of[..], all, |acc: &mut (u64, u32), out, i, row, _| {
            let (mut s, mut others) = (0, false);
            w.each(&row, |v, x| {
                s += x;
                others |= v != row.node;
            });
            out[i] = s;
            *acc = acc.add((s, u32::from(!others)));
        })?;
    if total >= 1 << 62 {
        return Err(Error::Invalid("total weight of 2^62 or more"));
    }
    Ok(Strengths { of, total, alone })
}

/// The local-moving state of a level: each node's community, and per community its
/// volume and (deterministic mode) its size.
pub(crate) struct Moving<'g> {
    pub(crate) comm: NodeCells<'g>,
    /// Each node's draw for its ties, made once per level.
    salt: Column<u32>,
    pub(crate) sigma: Column<AtomicU64>,
    size: Column<AtomicU32>,
    active: Column<AtomicBool>,
    /// The decision of each node in the current sub-round (deterministic mode).
    target: Column<AtomicU32>,
}

impl<'g> Moving<'g> {
    /// Every node in the community of `bound` (its own when `None`), named by its
    /// smallest node, and every node active.
    pub(crate) fn new<S: Source>(
        plan: &Plan<'_, 'g, S>,
        memory: &Memory,
        k: &[u64],
        ranks: &Ranks<'_>,
        bound: Option<&[u32]>,
        ctx: &Ctx<'_>,
    ) -> Result<Self, Error> {
        let (n, det) = (plan.nodes() as usize, ctx.det());
        let comm = plan.node_cells(memory)?;
        let salt = ranks.column(memory, "tie draws", |r| {
            let bits = draw(ctx.options.seed, Purpose::LEIDEN_TIE, ctx.counter(r));
            u32::try_from(bits >> 32).expect("32 bits")
        })?;
        let sigma = Column::new(memory, "community volumes", n, |_| AtomicU64::new(0))?;
        let per = if det { n } else { 0 };
        let size = Column::new(memory, "community sizes", per, |_| AtomicU32::new(0))?;
        let target = Column::new(memory, "decisions", per, |_| AtomicU32::new(NONE))?;
        let active = Column::new(memory, "active nodes", n, |_| AtomicBool::new(true))?;
        if let Some(b) = bound {
            let least = Column::new(memory, "least members", n, |_| AtomicU32::new(u32::MAX))?;
            plan.vertices((), |(): &mut (), (), _, v, _| {
                least[b[v.index()] as usize].fetch_min(v.get(), Relaxed);
            })?;
            plan.vertices((), |(): &mut (), (), _, v, _| {
                let first = least[b[v.index()] as usize].load(Relaxed);
                comm.store(v, node(plan, first), Relaxed);
            })?;
        }
        let st = Self {
            comm,
            salt,
            sigma,
            size,
            active,
            target,
        };
        st.count(plan, k)?;
        Ok(st)
    }

    /// Adds each node's strength and size to its community's.
    fn count<S: Source>(&self, plan: &Plan<'_, 'g, S>, k: &[u64]) -> Result<(), Error> {
        let det = !self.size.is_empty();
        plan.vertices((), |(): &mut (), (), _, v, _| {
            let c = self.comm.load(v, Relaxed).index();
            self.sigma[c].fetch_add(k[v.index()], Relaxed);
            if det {
                self.size[c].fetch_add(1, Relaxed);
            }
        })
    }

    /// Replaces the communities by `comm`, and their volumes and sizes with them.
    pub(crate) fn relabel<S: Source>(
        &mut self,
        plan: &Plan<'_, 'g, S>,
        k: &[u64],
        comm: NodeCells<'g>,
    ) -> Result<(), Error> {
        self.comm = comm;
        par::for_each_chunk_mut(&mut self.sigma, GRAIN, |_, s| {
            s.iter_mut().for_each(|x| *x.get_mut() = 0);
        });
        par::for_each_chunk_mut(&mut self.size, GRAIN, |_, s| {
            s.iter_mut().for_each(|x| *x.get_mut() = 0);
        });
        self.count(plan, k)
    }
}

/// Which nodes a sweep visits, in which order.
pub(crate) enum Schedule {
    /// Deterministic mode: the nodes of sub-round `r` are `ids[starts[r]..starts[r + 1]]`,
    /// increasing.
    Rounds {
        ids: Column<u32>,
        starts: Vec<usize>,
    },
    /// Fast mode: every node, blocks of [`ORDER_BLOCK`] ids in a drawn order.
    Order { ids: Column<u32> },
}

impl Schedule {
    /// The schedule of local moving or, with `refining`, of refinement. The
    /// deterministic mode leaves out the nodes without an entry.
    pub(crate) fn new(
        ranks: &Ranks<'_>,
        memory: &Memory,
        ctx: &Ctx<'_>,
        refining: bool,
    ) -> Result<Self, Error> {
        let seed = ctx.options.seed;
        if ctx.det() {
            let m = ranks.count();
            let mut ids = Column::new(memory, "schedule", m as usize, |_| 0_u32)?;
            let rounds = (m / 65_536).next_power_of_two().clamp(4, 1024);
            let (purpose, shift) = match (refining, ctx.level) {
                (true, _) => (Purpose::LEIDEN_REFINE_ROUND, 0),
                (false, 0) => (Purpose::LEIDEN_ROUND, ROUND_SHIFT),
                (false, _) => (Purpose::LEIDEN_ROUND, 0),
            };
            let bits = 64 - rounds.trailing_zeros();
            let round = |r: u32| {
                let r = draw(seed, purpose, ctx.counter(r >> shift)) >> bits;
                usize::try_from(r).expect("below the round count")
            };
            let buckets = NonZeroUsize::new(rounds as usize).expect("non-zero");
            let starts = par::scatter(
                memory,
                "sub-rounds",
                ranks.blocks(),
                buckets,
                |b, sink| {
                    ranks.each(b, |v, r| sink.push(round(r), v));
                    Ok::<(), Error>(())
                },
                [&mut ids[..]],
            )?;
            return Ok(Self::Rounds { ids, starts });
        }
        let n = u32::try_from(ranks.k.len()).expect("a node count");
        let mut ids = Column::new(memory, "schedule", n as usize, |_| 0_u32)?;
        let whole = n / ORDER_BLOCK;
        let mut blocks: Vec<(u64, u32)> = (0..whole)
            .map(|b| (draw(seed, Purpose::LEIDEN_ORDER, ctx.counter(b)), b))
            .collect();
        blocks.sort_unstable();
        par::for_each_chunk_mut(&mut ids[..], GRAIN, |first, chunk| {
            for (i, id) in (first..).zip(chunk) {
                let (b, at) = (i / ORDER_BLOCK as usize, i % ORDER_BLOCK as usize);
                *id = match blocks.get(b) {
                    Some(&(_, block)) => block * ORDER_BLOCK + u32::try_from(at).expect("below 64"),
                    None => u32::try_from(i).expect("an id"),
                };
            }
        });
        Ok(Self::Order { ids })
    }
}

/// What local moving did.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Moved {
    pub(crate) sweeps: u32,
    pub(crate) moves: u64,
    pub(crate) parallel_rounds: u64,
    pub(crate) serial_rounds: u64,
    pub(crate) heavy: u32,
}

/// The nodes of `ids` that pass `keep`, in order, filtered in parallel.
fn take<'g, S: Source>(
    plan: &Plan<'_, 'g, S>,
    tables: &Tables,
    ids: &[u32],
    keep: impl Fn(Node<'g>) -> bool + Send + Sync,
) -> Result<(Vec<Node<'g>>, Reservation), Error> {
    // The parts grow by doubling, each with its old block beside the new one while it
    // moves (3×), then are joined into one copy (1×), which alone stays held.
    let mut held = tables.memory().reserve(
        "frontier",
        MemoryKind::Anonymous,
        (ids.len() as u64).saturating_mul(4 * size_of::<Node<'g>>() as u64),
    )?;
    let parts = par::map_ranges(ids.len(), ROUND_GRAIN, |r| {
        ids[r]
            .iter()
            .filter_map(|&v| plan.node(v))
            .filter(|&v| keep(v))
            .collect::<Vec<_>>()
    });
    let frontier = parts.concat();
    drop(parts);
    held.shrink(held.bytes() - (frontier.len() * size_of::<Node<'g>>()) as u64);
    Ok((frontier, held))
}

/// Runs `body` on the rows of `frontier` and adds up its values. Fewer than [`SERIAL`]
/// nodes run on the calling thread, in runs the framework does not split.
fn drive<'g, 't, S: Source, A: Reduce>(
    plan: &Plan<'_, 'g, S>,
    frontier: &[Node<'g>],
    tables: &'t Tables,
    body: impl Fn(&mut A, Row<'_, 'g>, &mut Worker<'_, 'g, S, Lent<'t>>) + Sync,
) -> Result<A, Error> {
    let mut sum = A::ZERO;
    if frontier.len() < SERIAL {
        for run in frontier.chunks(FRONTIER_GRAIN) {
            sum = sum.add(plan.frontier_with(run, || tables.lend(), &body)?);
        }
    } else {
        sum = plan.frontier_with(frontier, || tables.lend(), &body)?;
    }
    tables.check()?;
    Ok(sum)
}

/// Calls `f` on each node of `frontier`, in parallel unless it is small.
fn each<'g>(frontier: &[Node<'g>], f: impl Fn(Node<'g>) + Send + Sync) {
    if frontier.len() < SERIAL {
        frontier.iter().copied().for_each(f);
    } else {
        par::for_each_range(frontier.len(), ROUND_GRAIN, |r| {
            frontier[r].iter().copied().for_each(&f);
        });
    }
}

/// Marks the neighbours of a node that moved as active.
#[inline]
fn wake(active: &[AtomicBool], list: &[Node<'_>]) {
    for &v in list {
        let a = &active[v.index()];
        if !a.load(Relaxed) {
            a.store(true, Relaxed);
        }
    }
}

/// The best move of `row.node` against the current state, and its gain in modularity
/// on that state; `None` to stay.
#[inline]
fn decide<'g, S: Source>(
    g: Graph<'_, '_, 'g, S>,
    row: &Row<'_, 'g>,
    st: &Moving<'g>,
    table: &mut Table,
    ctx: &Ctx<'_>,
) -> Option<(Node<'g>, f64)> {
    let u = row.node;
    let (w, k) = (g.w, g.k[u.index()]);
    let c = st.comm.load(u, Relaxed).get();
    w.each(row, |v, x| {
        if v != u {
            table.add(st.comm.load(v, Relaxed).get(), x);
        }
    });
    let f = ctx.options.resolution * real(k) * ctx.inv_total();
    let sigma = |d: u32| st.sigma[d as usize].load(Relaxed);
    let salt = st.salt[u.index()];
    let (mut stay, mut best) = (0, None::<(f64, u32)>);
    for (d, kud) in table.entries() {
        if d == c {
            stay = kud;
            continue;
        }
        let value = real(kud) - f * real(sigma(d));
        let better =
            best.is_none_or(|(b, bd)| value > b || (value == b && before(&st.salt, salt, d, bd)));
        if better {
            best = Some((value, d));
        }
    }
    let (value, d) = best?;
    let stay = real(stay) - f * real(sigma(c).saturating_sub(k));
    if value <= stay {
        return None;
    }
    if ctx.det() && d > c {
        let size = |x: u32| st.size[x as usize].load(Relaxed);
        if size(c) == 1 && size(d) == 1 {
            return None;
        }
    }
    Some((g.node(d), 2.0 * (value - stay) * ctx.inv_total()))
}

/// Moves `u` from community `c` to `d`.
fn relocate<'g>(st: &Moving<'g>, u: Node<'g>, d: Node<'g>, k: u64, det: bool) {
    let c = st.comm.load(u, Relaxed);
    st.sigma[c.index()].fetch_sub(k, Relaxed);
    st.sigma[d.index()].fetch_add(k, Relaxed);
    if det {
        st.size[c.index()].fetch_sub(1, Relaxed);
        st.size[d.index()].fetch_add(1, Relaxed);
    }
    st.comm.store(u, d, Relaxed);
}

/// The heavy nodes of a level, in id order: a strength above [`HEAVY`] of `2W`, so at
/// most 1,000. Two of them deciding at once can each join a community that does not yet
/// hold the other, and lose `2·k_u·k_v / (2W)²` together; on Friendster's coarse levels
/// such pairs cycled until the sweep cap and made the asynchronous result collapse
/// (#17, `lab/m1-03-louvain-collapse`).
fn heavy<'g, S: Source>(g: Graph<'_, '_, 'g, S>, total: u64) -> Vec<Node<'g>> {
    let n = g.plan.nodes() as usize;
    let parts = par::map_ranges(n, GRAIN, |r| {
        r.filter_map(|v| u32::try_from(v).ok().and_then(|v| g.plan.node(v)))
            .filter(|&v| g.k[v.index()] > total / HEAVY)
            .collect::<Vec<_>>()
    });
    parts.concat()
}

/// Local moving until a sweep gains at most `tol` in modularity, or [`MAX_SWEEPS`].
///
/// In the fast mode the heavy nodes ([`heavy`]) are left out of the parallel part of a
/// sweep and visited after it, one at a time in the sweep's order (on a small graph most
/// nodes are heavy, and the order carries the quality, ADR 0015), each move applied
/// before the next decision.
pub(crate) fn local_move<'g, S: Source>(
    g: Graph<'_, '_, 'g, S>,
    st: &Moving<'g>,
    schedule: &Schedule,
    tables: &Tables,
    ctx: &Ctx<'_>,
    tol: f64,
) -> Result<Moved, Error> {
    let (plan, k, det) = (g.plan, g.k, ctx.det());
    let heavy = match schedule {
        Schedule::Rounds { .. } => Vec::new(),
        Schedule::Order { .. } => heavy(g, ctx.total),
    };
    let light = |v: Node<'g>| heavy.binary_search(&v).is_err();
    let (heavy_in_order, _heavy_held) = match schedule {
        Schedule::Order { ids } if !heavy.is_empty() => take(plan, tables, ids, |v| !light(v))?,
        _ => (
            Vec::new(),
            Reservation::empty(tables.memory(), "frontier", MemoryKind::Anonymous),
        ),
    };
    // A sequential visit: each move is applied before the next node decides.
    let visit =
        |acc: &mut (u64, u64), row: Row<'_, 'g>, worker: &mut Worker<'_, 'g, S, Lent<'_>>| {
            let u = row.node;
            st.active[u.index()].store(false, Relaxed);
            let Some(table) = worker.scratch.begin(row.list.len()) else {
                return;
            };
            if let Some((d, gain)) = decide(g, &row, st, table, ctx) {
                relocate(st, u, d, k[u.index()], false);
                wake(&st.active, row.list);
                *acc = acc.add((fixed(gain), 1));
            }
        };
    let mut done = Moved {
        heavy: u32::try_from(heavy.len()).expect("at most 1,000"),
        ..Moved::default()
    };
    for _ in 0..MAX_SWEEPS {
        done.sweeps += 1;
        let (mut gain, mut processed) = (0_u64, 0_usize);
        match schedule {
            Schedule::Rounds { ids, starts } => {
                for r in starts.windows(2) {
                    let (frontier, _held) = take(plan, tables, &ids[r[0]..r[1]], |v| {
                        st.active[v.index()].swap(false, Relaxed)
                    })?;
                    if frontier.is_empty() {
                        continue;
                    }
                    processed += frontier.len();
                    if frontier.len() < SERIAL {
                        done.serial_rounds += 1;
                    } else {
                        done.parallel_rounds += 1;
                    }
                    let (sum, m) = drive(
                        plan,
                        &frontier,
                        tables,
                        |acc: &mut (u64, u64), row, worker| {
                            let u = row.node;
                            let target = &st.target[u.index()];
                            target.store(NONE, Relaxed);
                            let Some(table) = worker.scratch.begin(row.list.len()) else {
                                return;
                            };
                            if let Some((d, gain)) = decide(g, &row, st, table, ctx) {
                                target.store(d.get(), Relaxed);
                                wake(&st.active, row.list);
                                *acc = acc.add((fixed(gain), 1));
                            }
                        },
                    )?;
                    gain += sum;
                    done.moves += m;
                    each(&frontier, |u| {
                        let d = st.target[u.index()].load(Relaxed);
                        if d != NONE {
                            relocate(st, u, node(plan, d), k[u.index()], det);
                        }
                    });
                }
            }
            Schedule::Order { ids } => {
                let (frontier, _held) = take(plan, tables, ids, |v| {
                    light(v) && st.active[v.index()].load(Relaxed)
                })?;
                processed = frontier.len();
                let (sum, m) = drive(plan, &frontier, tables, visit)?;
                gain += sum;
                done.moves += m;
            }
        }
        let alone: Vec<Node<'g>> = heavy_in_order
            .iter()
            .copied()
            .filter(|v| st.active[v.index()].load(Relaxed))
            .collect();
        for &u in &alone {
            let (sum, m) = drive(plan, &[u], tables, visit)?;
            gain += sum;
            done.moves += m;
        }
        processed += alone.len();
        if processed == 0 || gain as f64 <= tol * GAIN_SCALE {
            break;
        }
    }
    Ok(done)
}

/// The refinement state: the refined partition inside the communities of local moving.
pub(crate) struct Refining<'g> {
    pub(crate) rcomm: NodeCells<'g>,
    rsigma: Column<AtomicU64>,
    /// Deterministic mode: `sub-round << 32 | target` of the nodes deciding to merge.
    proposal: Column<AtomicU64>,
    /// Randomized refinement: the weight between each refined community and the rest
    /// of its community, and what a merging node adds to it.
    cut: Column<AtomicI64>,
    cut_change: Column<AtomicI64>,
    /// Randomized refinement: each node's draw.
    draws: Column<u64>,
}

/// The refinement of `st`'s partition: every node starts alone and may join, once, a
/// refined community inside its own community. Greedy: the best positive gain.
/// Randomized: a draw among the non-negative gains weighted by `exp(gain / θ)`, over the
/// well-connected candidates (Traag, Waltman and van Eck 2019, Algorithm A.2). Joins
/// follow edges, so every refined community is connected.
///
/// Randomized refinement updates a refined community's cut per joining node, so it
/// counts twice the weight between two nodes that join it in the same sub-round
/// (deterministic mode) or at the same time (fast mode).
pub(crate) fn refine<'g, S: Source>(
    g: Graph<'_, '_, 'g, S>,
    memory: &Memory,
    ranks: &Ranks<'_>,
    st: &Moving<'g>,
    schedule: &Schedule,
    tables: &Tables,
    ctx: &Ctx<'_>,
) -> Result<Refining<'g>, Error> {
    let (plan, w, k) = (g.plan, g.w, g.k);
    let n = plan.nodes() as usize;
    let det = ctx.det();
    let theta = match ctx.options.refinement {
        Refinement::Greedy | Refinement::Louvain => None,
        Refinement::Randomized { theta } => Some(theta),
    };
    let per = |on: bool| if on { n } else { 0 };
    let r = Refining {
        rcomm: plan.node_cells(memory)?,
        rsigma: Column::new(memory, "refined volumes", n, |v| AtomicU64::new(k[v]))?,
        proposal: Column::new(memory, "proposals", per(det), |_| AtomicU64::new(u64::MAX))?,
        cut: Column::new(memory, "refined cuts", per(theta.is_some()), |_| {
            AtomicI64::new(0)
        })?,
        cut_change: Column::new(memory, "cut changes", per(theta.is_some() && det), |_| {
            AtomicI64::new(0)
        })?,
        draws: if theta.is_some() {
            ranks.column(memory, "refinement draws", |r| {
                draw(ctx.options.seed, Purpose::LEIDEN_REFINE, ctx.counter(r))
            })?
        } else {
            Column::new(memory, "refinement draws", 0, |_| 0)?
        },
    };
    if theta.is_some() {
        plan.scan((), all, |(): &mut (), (), _, row, _| {
            let (u, b) = (row.node, st.comm.load(row.node, Relaxed));
            let mut e = 0;
            w.each(&row, |v, x| {
                if v != u && st.comm.load(v, Relaxed) == b {
                    e += x;
                }
            });
            r.cut[u.index()].store(i64::try_from(e).expect("below 2^62"), Relaxed);
        })?;
    }
    let join = |u: Node<'g>, d: Node<'g>, change: i64| {
        let ku = k[u.index()];
        r.rcomm.store(u, d, Relaxed);
        r.rsigma[d.index()].fetch_add(ku, Relaxed);
        r.rsigma[u.index()].fetch_sub(ku, Relaxed);
        if theta.is_some() {
            r.cut[d.index()].fetch_add(change, Relaxed);
        }
    };
    let alive = |v: Node<'g>| k[v.index()] > 0;
    match schedule {
        Schedule::Rounds { ids, starts } => {
            for (stamp, b) in (0_u64..).zip(starts.windows(2)) {
                let (frontier, _held) = take(plan, tables, &ids[b[0]..b[1]], alive)?;
                if frontier.is_empty() {
                    continue;
                }
                let (): () = drive(plan, &frontier, tables, |(), row, worker| {
                    let u = row.node;
                    let Some(table) = worker.scratch.begin(row.list.len()) else {
                        return;
                    };
                    if let Some((d, change)) = pick(g, &row, st, &r, table, ctx, theta) {
                        r.proposal[u.index()].store(stamp << 32 | u64::from(d.get()), Relaxed);
                        if theta.is_some() {
                            r.cut_change[u.index()].store(change, Relaxed);
                        }
                    }
                })?;
                each(&frontier, |u| {
                    let p = r.proposal[u.index()].load(Relaxed);
                    if p >> 32 != stamp {
                        return;
                    }
                    let d = node(plan, u32::try_from(p & u64::from(u32::MAX)).expect("an id"));
                    if r.proposal[d.index()].load(Relaxed) >> 32 != stamp {
                        let change = if theta.is_some() {
                            r.cut_change[u.index()].load(Relaxed)
                        } else {
                            0
                        };
                        join(u, d, change);
                    }
                });
            }
        }
        Schedule::Order { ids } => {
            let (frontier, _held) = take(plan, tables, ids, alive)?;
            let (): () = drive(plan, &frontier, tables, |(), row, worker| {
                let u = row.node;
                let Some(table) = worker.scratch.begin(row.list.len()) else {
                    return;
                };
                let Some((d, change)) = pick(g, &row, st, &r, table, ctx, theta) else {
                    return;
                };
                // `rsigma[x] == 0` exactly while `x` is claimed (strength-0 nodes never
                // refine): a claimed node takes no member, and a node with a member
                // cannot be claimed, so every join follows an edge into a community
                // that holds its seed.
                let ku = k[u.index()];
                let claimed = r.rsigma[u.index()].compare_exchange(ku, 0, Relaxed, Relaxed);
                if claimed.is_err() {
                    return;
                }
                let joined =
                    r.rsigma[d.index()].try_update(Relaxed, Relaxed, |s| (s != 0).then(|| s + ku));
                if joined.is_ok() {
                    r.rcomm.store(u, d, Relaxed);
                    if theta.is_some() {
                        r.cut[d.index()].fetch_add(change, Relaxed);
                    }
                } else {
                    r.rsigma[u.index()].store(ku, Relaxed);
                }
            })?;
        }
    }
    Ok(r)
}

/// The refined community `row.node` joins, and the change of that community's cut;
/// `None` to stay alone.
#[inline]
fn pick<'g, S: Source>(
    g: Graph<'_, '_, 'g, S>,
    row: &Row<'_, 'g>,
    st: &Moving<'g>,
    r: &Refining<'g>,
    table: &mut Table,
    ctx: &Ctx<'_>,
    theta: Option<f64>,
) -> Option<(Node<'g>, i64)> {
    let (u, w) = (row.node, g.w);
    let ku = g.k[u.index()];
    if r.rcomm.load(u, Relaxed) != u || r.rsigma[u.index()].load(Relaxed) != ku {
        return None;
    }
    let b = st.comm.load(u, Relaxed);
    let mut inside = 0;
    w.each(row, |v, x| {
        if v != u && st.comm.load(v, Relaxed) == b {
            table.add(r.rcomm.load(v, Relaxed).get(), x);
            inside += x;
        }
    });
    let gamma = ctx.options.resolution * ctx.inv_total();
    let f = gamma * real(ku);
    let rsigma = |d: u32| real(r.rsigma[d as usize].load(Relaxed));
    let Some(theta) = theta else {
        let salt = st.salt[u.index()];
        let mut best = None::<(f64, u32)>;
        for (d, kud) in table.entries() {
            let value = real(kud) - f * rsigma(d);
            let better = best
                .is_none_or(|(b, bd)| value > b || (value == b && before(&st.salt, salt, d, bd)));
            if value > 0.0 && d != u.get() && better {
                best = Some((value, d));
            }
        }
        return best.map(|(_, d)| (g.node(d), 0));
    };
    // Traag et al.: only a node well connected to its community moves, and only into a
    // refined community that is.
    let total = real(st.sigma[b.index()].load(Relaxed));
    if real(inside) < f * (total - real(ku)) {
        return None;
    }
    table.sort();
    let candidates = || {
        table.sorted.iter().filter_map(|&(d, kud)| {
            let (rs, cut) = (rsigma(d), r.cut[d as usize].load(Relaxed) as f64);
            let value = real(kud) - f * rs;
            (d != u.get() && value >= 0.0 && cut >= gamma * rs * (total - rs))
                .then_some((d, kud, value))
        })
    };
    let top = candidates().fold(0.0_f64, |m, c| if c.2 > m { c.2 } else { m });
    let weight = |value: f64| math::exp((value - top) / theta);
    let sum = candidates().fold(weight(0.0), |s, c| s + weight(c.2));
    let mut at = unit_f64(r.draws[u.index()]) * sum - weight(0.0);
    for (d, kud, value) in candidates() {
        if at < 0.0 {
            break;
        }
        at -= weight(value);
        if at < 0.0 {
            let change = i64::try_from(inside).expect("below 2^62")
                - 2 * i64::try_from(kud).expect("below 2^62");
            return Some((g.node(d), change));
        }
    }
    None
}

/// Joins the sets of `x` and `y` in a union-find whose parents only go down, so each
/// root is the smallest node of its set (Rem's algorithm with splicing, as
/// `everygraph_kernels` components).
fn unite<'g>(parent: &NodeCells<'g>, mut x: Node<'g>, mut y: Node<'g>) {
    loop {
        let (mut px, mut py) = (parent.load(x, Relaxed), parent.load(y, Relaxed));
        if px == py {
            return;
        }
        if px < py {
            (x, y, px, py) = (y, x, py, px);
        }
        if x == px {
            if parent.compare_exchange(x, x, py, Relaxed).is_ok() {
                return;
            }
        } else {
            let _ = parent.compare_exchange(x, px, py, Relaxed);
            x = px;
        }
    }
}

/// The connected components inside the communities of `label`: each node's label
/// becomes the smallest node of its component, whatever order the joins land in.
pub(crate) fn split<'g, S: Source>(
    plan: &Plan<'_, 'g, S>,
    memory: &Memory,
    label: &NodeCells<'g>,
) -> Result<NodeCells<'g>, Error> {
    let parent = plan.node_cells(memory)?;
    plan.scan((), all, |(): &mut (), (), _, row, _| {
        let (u, c) = (row.node, label.load(row.node, Relaxed));
        for &v in row.list {
            if v < u && label.load(v, Relaxed) == c {
                unite(&parent, u, v);
            }
        }
    })?;
    parent.flatten();
    Ok(parent)
}

#[cfg(test)]
mod tests;
