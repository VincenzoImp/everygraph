// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Core numbers (#50), after Liu, Dong, Gu and Sun, "Parallel k-Core Decomposition:
//! Theory and Practice" (SIGMOD 2025). Round `k` peels every node whose induced degree
//! (its neighbours not yet peeled) is at most `k`; such a node has core number `k`.
//!
//! - **Active set.** Nodes wait in [`Buckets`] by induced degree, so a round starts from
//!   the nodes of the smallest degree and no round scans the nodes left (§ 3, § 5).
//! - **Local queues** (vertical granularity control, § 4.2). A task peels its node, then,
//!   in the same subround, the neighbours whose degree that drops to `k`, through a
//!   queue that takes up to 128 nodes. What does not fit waits for the next subround.
//! - **Sampled hubs** (§ 4.1). A node of high degree does not count its peeled
//!   neighbours one by one, which would make every worker update one counter. Each
//!   peeled neighbour adds to a sample counter with a small probability, drawn by a
//!   counter-based hash of the edge. When the samples say the degree may have fallen
//!   near `k`, the node counts its neighbours exactly and leaves sample mode. The answer
//!   is exact (Las Vegas): every node counts exactly before it is peeled, and a count
//!   that shows the sampling missed a round (probability below `1 / n`) restarts the
//!   kernel without sampling.
//!
//! Core numbers are unique, so the result does not depend on the threads, the order in
//! which updates land, or the draws (DESIGN D14). Lists are those of the undirected
//! projection: symmetric, one entry per pair. A node's own entry (a kept loop) is not a
//! neighbour (`docs/spec/graph-semantics.md` § 5.2).

use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};
use std::sync::{Mutex, PoisonError};

use everygraph_core::rng::{Purpose, draw};
use everygraph_core::{Memory, MemoryKind, SegVec, par};

use crate::Atomics;
use crate::edge_map::{Buckets, Error, Node, NodeList, NodeSet, PerNode, Plan, Source, all, ix};

/// A node without a core number yet.
const UNSET: u32 = u32::MAX;

/// Nodes a task takes into its local queue before it leaves the rest to the next
/// subround (Liu et al. § 4.2: 128; time "relatively stable" from hundreds to thousands).
const QUEUE: usize = 128;

/// Nodes per task of the passes outside the drivers.
const GRAIN: NonZeroUsize = NonZeroUsize::new(1 << 14).expect("non-zero");

/// The seed of the sampling's draws. Core numbers do not depend on it.
const SEED: u64 = 0;

/// The sampling parameters of Liu et al. § 4.1, with `r = 1/10`: a node leaves sample
/// mode when its degree may have dropped to a tenth.
#[derive(Clone, Copy, Debug)]
struct Sampler {
    /// Samples to take before an exact count: `μ = 4 (c + 2) ln n` with `c = 1`, taken
    /// as `9 log2 n`, which is above `12 ln n`.
    mu: u32,
    /// Smallest degree in sample mode: 20,000, the authors' code (`sample_threshold =
    /// 2000` on `r d`, `KCore/kcore.h` of github.com/ucrparlay/Parallel-KCore). Below it
    /// the exact counts cost more than the contention they save.
    min_degree: u32,
}

impl Sampler {
    fn for_nodes(n: u32) -> Self {
        let mu = 9 * (32 - n.leading_zeros()).max(1);
        Self {
            mu,
            min_degree: 20_000,
        }
    }

    /// The sample rate for degree `d`, times 2^32, if a node of degree `d` samples in
    /// round `k`: when `r d > k` and `d` is large enough.
    fn rate(self, d: u32, k: u32) -> Option<u32> {
        let rate = (u64::from(self.mu) << 32) * 10 / (9 * u64::from(d.max(1)));
        (d >= self.min_degree && u64::from(d) > 10 * u64::from(k))
            .then(|| u32::try_from(rate).unwrap_or(u32::MAX))
    }

    /// Whether a node that entered sample mode at degree `d` with `rate` and has taken
    /// `samples` since may stay in sample mode through round `k` (Liu et al.'s
    /// `Validate`): `r d > k`, and fewer samples than a quarter of those expected if
    /// `d − k` neighbours were peeled.
    fn safe(d: u32, rate: u32, samples: u32, k: u32) -> bool {
        u64::from(d) > 10 * u64::from(k)
            && (u128::from(samples) << 34) < u128::from(d - k) * u128::from(rate)
    }
}

/// Core numbers of the undirected graph whose lists `plan` reads: `core[v]` is the
/// largest `k` such that `v` is in the largest induced subgraph whose nodes all have at
/// least `k` neighbours in it. Returned as the atomics they were computed in, with the
/// reservation that holds them.
///
/// # Errors
///
/// From the plan's source, or the state does not fit in the budget (about 16 bytes per
/// node, 8 without hubs, plus the buckets).
pub fn kcore<S: Source>(plan: &Plan<'_, '_, S>, memory: &Memory) -> Result<Atomics, Error> {
    decompose(plan, memory, Some(Sampler::for_nodes(plan.nodes())))
}

/// [`kcore`] with `sampler`, restarted without it if it missed a round.
fn decompose<S: Source>(
    plan: &Plan<'_, '_, S>,
    memory: &Memory,
    sampler: Option<Sampler>,
) -> Result<Atomics, Error> {
    match peel(plan, memory, sampler)? {
        Some(core) => Ok(core),
        None => peel(plan, memory, None)?.ok_or(Error::Invalid("k-core: an exact peel failed")),
    }
}

/// Set in the degree word of a node in sample mode; the other bits hold its degree when
/// it entered. A peel then reads one word per entry, sampled or not.
const SAMPLED: u32 = 1 << 31;

/// The sampling's per-node state: rate times 2^32, and samples taken.
#[derive(Clone, Copy)]
struct Samples<'a, 'g> {
    sampler: Sampler,
    rate: PerNode<'a, 'g, AtomicU32>,
    taken: PerNode<'a, 'g, AtomicU32>,
}

/// The state of one peel.
struct Peel<'p, 'a, 'g, S: Source> {
    plan: &'p Plan<'p, 'g, S>,
    memory: &'p Memory,
    /// Induced degree, or [`SAMPLED`] and the degree at entry.
    degree: PerNode<'a, 'g, AtomicU32>,
    core: PerNode<'a, 'g, AtomicU32>,
    samples: Option<Samples<'a, 'g>>,
    /// The next subround's frontier.
    next: NodeSet<'g>,
    /// Nodes in sample mode that took their last sample.
    recount: NodeSet<'g>,
    /// An exact count found that the sampling missed a round.
    missed: AtomicBool,
}

impl<'g, S: Source> Peel<'_, '_, 'g, S> {
    /// The key of `v` in the buckets: its degree, while it is neither peeled nor sampled.
    fn key(&self, v: Node<'g>) -> Option<u32> {
        let d = self.degree[v].load(Relaxed);
        (self.core[v].load(Relaxed) == UNSET && d & SAMPLED == 0).then_some(d)
    }

    /// The nodes of `sampled` still in sample mode that may not stay in it through round
    /// `k`, and those still in sample mode.
    fn at_risk(&self, sampled: &NodeList<'g>, k: u32) -> Result<[NodeList<'g>; 2], Error> {
        let mut out = [NodeList::new(self.memory), NodeList::new(self.memory)];
        if let Some(s) = self.samples {
            for &v in sampled.as_slice()? {
                let d = self.degree[v].load(Relaxed);
                if d & SAMPLED != 0 {
                    let (rate, taken) = (s.rate[v].load(Relaxed), s.taken[v].load(Relaxed));
                    let late = !Sampler::safe(d & !SAMPLED, rate, taken, k);
                    out[usize::from(!late)].push(v)?;
                }
            }
        }
        Ok(out)
    }

    /// Counts the neighbours of each node of `nodes` (all in sample mode) exactly, and
    /// takes it out of sample mode. During round `k` (`peeling`) a node whose degree is
    /// at most `k` goes to the next subround. Otherwise it samples again if its degree
    /// allows it in round `k`, or goes to the buckets. A node with fewer than `floor`
    /// neighbours not peeled before round `floor` should have been peeled already: that
    /// is a miss.
    fn exact(
        &self,
        buckets: &Buckets<'g>,
        nodes: &[Node<'g>],
        floor: u32,
        k: u32,
        peeling: bool,
    ) -> Result<(), Error> {
        let Some(s) = self.samples else {
            return Ok(());
        };
        self.plan.frontier(nodes, |(): &mut (), row, _| {
            let v = row.node;
            // Hub lists: a few nodes of millions of entries are split between workers.
            let counts = par::map_ranges(row.list.len(), GRAIN, |r| {
                let (mut alive, mut kept) = (0_u32, 0_u32);
                for &u in row.list[r].iter().filter(|&&u| u != v) {
                    let c = self.core[u].load(Relaxed);
                    alive += u32::from(c == UNSET);
                    kept += u32::from(c == UNSET || c >= floor);
                }
                (alive, kept)
            });
            let (alive, kept) = counts
                .into_iter()
                .fold((0, 0), |s, c| (s.0 + c.0, s.1 + c.1));
            if kept < floor {
                self.missed.store(true, Relaxed);
            }
            s.taken[v].store(0, Relaxed);
            if peeling && alive <= k {
                self.degree[v].store(alive, Relaxed);
                self.next.insert(v);
            } else if let Some(rate) = s.sampler.rate(alive, k) {
                s.rate[v].store(rate, Relaxed);
                self.degree[v].store(SAMPLED | alive, Relaxed);
            } else {
                self.degree[v].store(alive, Relaxed);
                buckets.insert(v);
            }
        })
    }

    /// Peels `frontier` in round `k`, each task with its local queue.
    fn subround(&self, buckets: &Buckets<'g>, frontier: &[Node<'g>], k: u32) -> Result<(), Error> {
        let failure: Mutex<Option<Error>> = Mutex::new(None);
        let peel_one = |v: Node<'g>, list: &[Node<'g>], queue: &mut Vec<Node<'g>>| {
            self.core[v].store(k, Relaxed);
            for &u in list {
                let d = &self.degree[u];
                let now = d.load(Relaxed);
                if now <= k {
                    continue;
                }
                if now & SAMPLED != 0 {
                    if let Some(s) = self.samples {
                        let coin = draw(SEED, Purpose::CORE_SAMPLE, [u.get(), v.get(), 0, 0]) >> 32;
                        let hit = coin < u64::from(s.rate[u].load(Relaxed));
                        if hit && s.taken[u].fetch_add(1, Relaxed) + 1 == s.sampler.mu {
                            self.recount.insert(u);
                        }
                    }
                    continue;
                }
                let old = d.fetch_sub(1, Relaxed);
                if old == k + 1 {
                    if queue.len() < QUEUE {
                        queue.push(u);
                    } else {
                        self.next.insert(u);
                    }
                } else if old > k + 1 {
                    buckets.decreased(u, old, old - 1);
                }
            }
        };
        self.plan
            .frontier_with(frontier, Vec::new, |(): &mut (), row, w| {
                // A FIFO that takes at most `QUEUE` nodes in all, so that one task's
                // work stays bounded (Liu et al.'s `rear < local_queue_size`).
                w.scratch.clear();
                peel_one(row.node, row.list, w.scratch);
                let mut at = 0;
                while let Some(&u) = w.scratch.get(at) {
                    at += 1;
                    match w.lists.get(u) {
                        Ok(r) => peel_one(u, r.list, w.scratch),
                        Err(e) => {
                            let mut f = failure.lock().unwrap_or_else(PoisonError::into_inner);
                            f.get_or_insert(e);
                            return;
                        }
                    }
                }
            })?;
        failure
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner)
            .map_or(Ok(()), Err)
    }
}

/// One peel; `None` if the sampling was found to have missed a round.
fn peel<'g, S: Source>(
    plan: &Plan<'_, 'g, S>,
    memory: &Memory,
    sampler: Option<Sampler>,
) -> Result<Option<Atomics>, Error> {
    let n = ix(plan.nodes().into());
    let atomics = |purpose, value: u32| -> Result<Atomics, Error> {
        let held = memory.reserve(purpose, MemoryKind::Anonymous, n as u64 * 4)?;
        Ok((par::collect(n, GRAIN, |_| AtomicU32::new(value)), held))
    };
    let (mut degree, _d) = atomics("k-core degrees", 0)?;
    plan.scan(&mut degree[..], all, |(): &mut (), out, i, row, _| {
        let own = row.list.iter().filter(|&&u| u == row.node).count();
        *out[i].get_mut() = u32::try_from(row.list.len() - own).expect("a degree below 2^32");
    })?;
    let max = par::map_ranges(n, GRAIN, |r| {
        degree[r].iter().map(|d| d.load(Relaxed)).max()
    })
    .into_iter()
    .flatten()
    .max()
    .unwrap_or(0);
    let sampler = sampler.filter(|s| max >= s.min_degree && max < SAMPLED);
    let sampling = match sampler {
        Some(_) => Some((
            atomics("k-core sample rates", 0)?,
            atomics("k-core samples", 0)?,
        )),
        None => None,
    };
    let core = atomics("core numbers", UNSET)?;
    let samples = match (sampler, &sampling) {
        (Some(sampler), Some((rate, taken))) => Some(Samples {
            sampler,
            rate: plan.per_node(&rate.0)?,
            taken: plan.per_node(&taken.0)?,
        }),
        _ => None,
    };
    let mut p = Peel {
        plan,
        memory,
        degree: plan.per_node(&degree)?,
        core: plan.per_node(&core.0)?,
        samples,
        next: plan.node_set(memory)?,
        recount: plan.node_set(memory)?,
        missed: AtomicBool::new(false),
    };

    let mut buckets = Buckets::new(plan, memory)?;
    plan.vertices((), |(): &mut (), (), _, v, _| {
        if let Some(s) = p.samples {
            let d = p.degree[v].load(Relaxed);
            if let Some(rate) = s.sampler.rate(d, 0) {
                s.rate[v].store(rate, Relaxed);
                p.degree[v].store(SAMPLED | d, Relaxed);
                p.recount.insert(v);
                return;
            }
        }
        buckets.insert(v);
    })?;
    let mut sampled = NodeList::new(memory);
    p.recount.list_into(&mut sampled)?;
    p.recount.clear();

    let mut frontier = NodeList::new(memory);
    // The smallest round not finished: every node not peeled has at least `floor`
    // neighbours not peeled before round `floor`.
    let mut floor = 0_u32;
    loop {
        // The round: the smallest key, once no node in sample mode may be below it.
        let k = loop {
            let k = buckets.peek(|v| p.key(v))?;
            let [late, still] = p.at_risk(&sampled, k.unwrap_or(UNSET))?;
            if late.is_empty() {
                break k;
            }
            p.exact(&buckets, late.as_slice()?, floor, k.unwrap_or(UNSET), false)?;
            sampled = still;
            sampled.extend_from_slice(late.as_slice()?)?;
        };
        if p.missed.load(Relaxed) {
            return Ok(None);
        }
        let Some(k) = k else {
            break;
        };
        buckets.pop(|v| p.key(v), &mut frontier)?;
        loop {
            while !frontier.is_empty() {
                p.subround(&buckets, frontier.as_slice()?, k)?;
                let mut counted = NodeList::new(memory);
                p.recount.list_into(&mut counted)?;
                p.recount.clear();
                p.exact(&buckets, counted.as_slice()?, k, k, true)?;
                p.next.list_into(&mut frontier)?;
                p.next.clear();
            }
            // Nodes in sample mode whose degree may have fallen to `k` in this round.
            let [late, still] = p.at_risk(&sampled, k)?;
            if late.is_empty() {
                break;
            }
            p.exact(&buckets, late.as_slice()?, k, k, true)?;
            sampled = still;
            sampled.extend_from_slice(late.as_slice()?)?;
            p.next.list_into(&mut frontier)?;
            p.next.clear();
        }
        if p.missed.load(Relaxed) {
            return Ok(None);
        }
        floor = k + 1;
    }
    Ok(Some(core))
}

/// The number of nodes of each core number `0..=max`: the shells.
///
/// # Errors
///
/// [`Error::Memory`] if the counts do not fit in the budget.
///
/// # Panics
///
/// Never: the counts go up to the largest core number.
pub fn shells(core: &[AtomicU32], memory: &Memory) -> Result<SegVec<u64>, Error> {
    const SHELL_GRAIN: NonZeroUsize = NonZeroUsize::new(1 << 16).expect("non-zero");
    let n = core.len();
    let value = |v: usize| ix(core[v].load(Relaxed).into());
    let Some(max) = par::map_ranges(n, GRAIN, |r| r.map(value).max())
        .into_iter()
        .flatten()
        .max()
    else {
        return Ok(SegVec::new(memory, "shells"));
    };
    let parts = n.div_ceil(SHELL_GRAIN.get()) as u64;
    let _counts = memory.reserve(
        "shell counts",
        MemoryKind::Anonymous,
        parts * (max as u64 + 1) * 4,
    )?;
    let counts = par::map_ranges(n, SHELL_GRAIN, |r| {
        let mut c: Vec<u32> = (0..=max).map(|_| 0).collect();
        for v in r {
            c[value(v)] += 1;
        }
        c
    });
    let mut out = SegVec::try_with_len(memory, "shells", max + 1, 0_u64)?;
    for c in &counts {
        for (s, &x) in c.iter().enumerate() {
            *out.get_mut(s).expect("a core number up to the largest") += u64::from(x);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    //! The sampling and its restart, with parameters small enough for test graphs.

    use std::collections::BTreeSet;

    use super::*;
    use crate::edge_map::{Csr, Settings};
    use everygraph_core::rng::below;

    /// Core numbers by removing a node of least degree, one at a time.
    fn reference(offsets: &[u64], targets: &[u32]) -> Vec<u32> {
        let n = offsets.len() - 1;
        let list = |v: usize| &targets[ix(offsets[v])..ix(offsets[v + 1])];
        let mut deg: Vec<u32> = (0..n)
            .map(|v| u32::try_from(list(v).iter().filter(|&&u| ix(u.into()) != v).count()).unwrap())
            .collect();
        let mut queue: BTreeSet<(u32, usize)> = (0..n).map(|v| (deg[v], v)).collect();
        let (mut core, mut k): (Vec<u32>, u32) = ((0..n).map(|_| 0).collect(), 0);
        while let Some((d, v)) = queue.pop_first() {
            k = k.max(d);
            core[v] = k;
            for &u in list(v) {
                let u = ix(u.into());
                if queue.remove(&(deg[u], u)) {
                    deg[u] -= 1;
                    queue.insert((deg[u], u));
                }
            }
        }
        core
    }

    /// A few dense clusters of different sizes, joined sparsely, with kept loops: hubs
    /// whose degree falls through many rounds.
    fn graph(seed: u64, n: u32) -> (Vec<u64>, Vec<u32>) {
        let mut pairs = BTreeSet::new();
        for v in 0..n {
            let cluster = v % 5;
            let tries = 4 + 6 * cluster;
            for i in 0..tries {
                let u = u32::try_from(below(draw(seed, Purpose::TEST, [v, i, 0, 0]), u64::from(n)))
                    .unwrap();
                if u % 5 == cluster || i == 0 {
                    pairs.insert((v.min(u), v.max(u)));
                }
            }
        }
        let mut lists: Vec<Vec<u32>> = (0..n).map(|_| Vec::new()).collect();
        for &(a, b) in &pairs {
            lists[a as usize].push(b);
            if a != b {
                lists[b as usize].push(a);
            }
        }
        let mut offsets = Vec::from([0]);
        let mut targets = Vec::new();
        for mut l in lists {
            l.sort_unstable();
            targets.extend(l);
            offsets.push(targets.len() as u64);
        }
        (offsets, targets)
    }

    /// A star of `leaves` leaves and a triangle: the hub samples, and its leaves are
    /// peeled in round 1, so a hub that draws no sample misses its round.
    fn star(leaves: u32) -> (Vec<u64>, Vec<u32>) {
        let n = leaves + 3;
        let mut lists: Vec<Vec<u32>> = (0..n).map(|_| Vec::new()).collect();
        let mut join = |a: u32, b: u32| {
            lists[a as usize].push(b);
            lists[b as usize].push(a);
        };
        (1..=leaves).for_each(|leaf| join(0, leaf));
        let t = leaves + 1;
        join(t, t + 1);
        join(t + 1, 0);
        join(0, t);
        let mut offsets = Vec::from([0]);
        let mut targets = Vec::new();
        for mut l in lists {
            l.sort_unstable();
            targets.extend(l);
            offsets.push(targets.len() as u64);
        }
        (offsets, targets)
    }

    fn run(
        offsets: &[u64],
        targets: &[u32],
        sampler: Option<Sampler>,
        restart: bool,
    ) -> Option<Vec<u32>> {
        let csr = Csr::new(offsets, targets).unwrap();
        let memory = Memory::unlimited();
        par::with_threads(NonZeroUsize::new(3).unwrap(), || {
            Plan::with(
                &csr,
                &memory,
                Settings {
                    chunk_entries: 64,
                    ..Settings::default()
                },
                |plan| {
                    let core = if restart {
                        Some(decompose(&plan, &memory, sampler).unwrap())
                    } else {
                        peel(&plan, &memory, sampler).unwrap()
                    };
                    core.map(|(c, _)| c.into_iter().map(AtomicU32::into_inner).collect())
                },
            )
        })
        .unwrap()
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "safe code; Miri checks the unsafe code, which the framework tests reach"
    )]
    fn sampled_peels_are_exact_or_restart() {
        let graphs = (0..6)
            .map(|seed| graph(seed, 600))
            .chain((400..420).map(star));
        let (mut exact, mut restarts) = (0, 0);
        for (offsets, targets) in graphs {
            let want = reference(&offsets, &targets);
            assert_eq!(run(&offsets, &targets, None, false).unwrap(), want);
            for (mu, min_degree) in [(1, 2), (2, 8), (4, 20), (8, 40)] {
                let sampler = Some(Sampler { mu, min_degree });
                match run(&offsets, &targets, sampler, false) {
                    Some(core) => {
                        assert_eq!(core, want, "mu {mu}");
                        exact += 1;
                    }
                    None => restarts += 1,
                }
                assert_eq!(run(&offsets, &targets, sampler, true).unwrap(), want);
            }
        }
        assert!(
            exact > 0 && restarts > 0,
            "{exact} exact, {restarts} restarts"
        );
    }

    #[test]
    fn the_sampler_follows_the_paper() {
        let s = Sampler::for_nodes(65_608_366);
        assert_eq!((s.mu, s.min_degree), (234, 20_000));
        // 12 ln(65,608,366) = 216.0.
        assert!(s.mu > 216);
        assert_eq!(s.rate(19_999, 0), None);
        assert_eq!(s.rate(30_000, 3000), None);
        let r = s.rate(30_000, 2999).unwrap();
        assert!(u64::from(r) < (1 << 32) / 8);
        assert!(Sampler::safe(30_000, r, 0, 2999));
        assert!(!Sampler::safe(30_000, r, 0, 3000));
        assert!(!Sampler::safe(30_000, r, 230, 0));
    }
}
