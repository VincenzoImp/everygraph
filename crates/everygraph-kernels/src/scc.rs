// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Strongly connected components (#223) of a directed graph, from its out- and in-lists:
//! BGSS, the prefix-doubling algorithm of Blelloch, Gu, Shun and Sun (SPAA 2016), in the
//! form of Wang, Dong, Gu and Sun (SIGMOD 2023; `docs/research/sota-kernels.md` § 3).
//!
//! **Rounds.** Nodes are sources in a random order, in batches that double. A round
//! searches forward and backward from every source of its batch at once, each search
//! inside the part of its source. A node that source `s` reaches both ways is in the
//! component of `s`, which is then whole. Every other node goes to the part given by
//! its old part and the sets of sources that reach it and that it reaches. Two nodes of
//! one component have the same sets, so a part never cuts a component. Every source
//! finishes in its round.
//!
//! Before the rounds, a node with an empty in-list or out-list is a component of its own.
//! The first round has one source, the first node of the order, and searches by
//! direction-optimizing BFS ([`crate::traversal`]).
//!
//! **Pairs.** A search keeps the pairs (node, source) it found in a concurrent set. A
//! step pushes from each node of the frontier only the sources that the node got in the
//! step before.
//!
//! **Labels.** A part is a sum of hashes over the sets, so it does not depend on the
//! order of inserts; parts that collide cost work only, never a wrong answer. A node
//! joins its source in a union-find whose roots are the smallest node of their tree, so
//! `labels[v]` is the smallest node of the component of `v` (DESIGN D14). The sets, the
//! labels and the work do not depend on the threads, the chunking or the source.

use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering::Relaxed};

use everygraph_core::rng::{Purpose, below, draw};
use everygraph_core::{Memory, MemoryKind, Reservation, SegVec, par, usize_of};

use crate::Atomics;
use crate::components::unite;
use crate::edge_map::{
    Error, FRONTIER_GRAIN, Node, NodeCells, NodeList, NodeSet, Plan, Reduce, Source, ix,
    one_segment, one_segment_mut,
};
use crate::traversal::{self, Bfs, UNREACHED};

/// Items per task of the passes outside the drivers.
const GRAIN: NonZeroUsize = NonZeroUsize::new(1 << 14).expect("non-zero");

/// Frontier nodes per bound on the pairs that a step adds.
const RUN: NonZeroUsize = NonZeroUsize::new(FRONTIER_GRAIN).expect("non-zero");

/// The seed of the order of sources. Labels do not depend on it.
const SEED: u64 = 0;

/// Fewest slots of a set of pairs.
const MIN_SLOTS: usize = 1 << 10;

/// The mark of a pair found before the last step.
const OLD: u8 = 0;

/// The mark of the pairs found in `step`.
fn mark(step: u32) -> u8 {
    if step.is_multiple_of(2) { 1 } else { 2 }
}

fn pack(v: u32, s: u32) -> u64 {
    (u64::from(v) << 32 | u64::from(s)) + 1
}

#[expect(clippy::cast_possible_truncation, reason = "the two halves of a pair")]
fn unpack(key: u64) -> (u32, u32) {
    let k = key - 1;
    ((k >> 32) as u32, k as u32)
}

/// The `SplitMix64` finalizer (Steele, Lea and Flood, OOPSLA 2014) of `s` and a direction:
/// the term that source `s` adds to the part of a node it reaches, or that reaches it.
fn spread(s: u32, forward: bool) -> u64 {
    let mut z = (u64::from(s) << 1 | u64::from(forward)).wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A concurrent set of pairs (node, source), by linear probing on the node alone: the
/// pairs of a node are between its home slot and the next empty slot. It grows only
/// between calls of the drivers.
struct Pairs {
    /// 0 for an empty slot, else [`pack`] of the pair.
    slots: Vec<AtomicU64>,
    /// When each pair was found: [`OLD`], or the [`mark`] of its step.
    marks: Vec<AtomicU8>,
    len: u64,
    /// The pairs there can be at most: nodes times sources.
    most: u64,
    shift: u32,
    _held: Reservation,
}

impl Pairs {
    /// An empty set of at least `slots` slots, a power of two.
    fn new(memory: &Memory, slots: usize, most: u64) -> Result<Self, Error> {
        let slots = slots
            .max(MIN_SLOTS)
            .checked_next_power_of_two()
            .ok_or(Error::Invalid(
                "strong components: more pairs than the address space",
            ))?;
        let held = memory.reserve(
            "strong components: pairs",
            MemoryKind::Anonymous,
            slots as u64 * 9,
        )?;
        Ok(Self {
            slots: par::collect(slots, GRAIN, |_| AtomicU64::new(0)),
            marks: par::collect(slots, GRAIN, |_| AtomicU8::new(OLD)),
            len: 0,
            most,
            shift: 64 - slots.trailing_zeros(),
            _held: held,
        })
    }

    /// Pairs that fit below the load of one half.
    fn room(&self) -> u64 {
        (self.slots.len() as u64 / 2).saturating_sub(self.len)
    }

    /// The slot where the pairs of `v` start: Fibonacci hashing.
    fn home(&self, v: u32) -> usize {
        ix(u64::from(v).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> self.shift)
    }

    fn pair(&self, slot: usize) -> Option<(u32, u32)> {
        let key = self.slots[slot].load(Relaxed);
        (key != 0).then(|| unpack(key))
    }

    /// The pairs of `v`: their slots and sources.
    fn of(&self, v: u32) -> impl Iterator<Item = (usize, u32)> + '_ {
        let mask = self.slots.len() - 1;
        let mut i = self.home(v);
        std::iter::from_fn(move || {
            loop {
                let (u, s) = self.pair(i)?;
                let at = i;
                i = (i + 1) & mask;
                if u == v {
                    return Some((at, s));
                }
            }
        })
    }

    fn contains(&self, v: u32, s: u32) -> bool {
        self.of(v).any(|(_, t)| t == s)
    }

    /// Adds `(v, s)` with `mark`; true if it was not in the set. The caller keeps the
    /// load at most one half, so an empty slot is always ahead.
    fn insert(&self, v: u32, s: u32, mark: u8) -> bool {
        let key = pack(v, s);
        let mask = self.slots.len() - 1;
        let mut i = self.home(v);
        loop {
            match self.slots[i].load(Relaxed) {
                k if k == key => return false,
                0 => match self.slots[i].compare_exchange(0, key, Relaxed, Relaxed) {
                    Ok(_) => {
                        self.marks[i].store(mark, Relaxed);
                        return true;
                    }
                    Err(k) if k == key => return false,
                    Err(_) => continue,
                },
                _ => i = (i + 1) & mask,
            }
        }
    }

    /// The number of pairs of `v` with `mark`.
    fn fresh(&self, v: u32, mark: u8) -> u64 {
        self.of(v)
            .filter(|&(i, _)| self.marks[i].load(Relaxed) == mark)
            .count() as u64
    }

    /// Moves the sources of the pairs of `v` with `mark` into `out`, and marks those
    /// pairs [`OLD`].
    fn take(&self, v: u32, mark: u8, out: &mut Vec<u32>) {
        out.clear();
        for (i, s) in self.of(v) {
            if self.marks[i].load(Relaxed) == mark {
                self.marks[i].store(OLD, Relaxed);
                out.push(s);
            }
        }
    }

    /// Grows until `more` pairs fit, or as many as there can still be.
    fn make_room(&mut self, more: u64, memory: &Memory) -> Result<(), Error> {
        let more = more.min(self.most.saturating_sub(self.len));
        let mut slots = self.slots.len();
        while (slots as u64 / 2).saturating_sub(self.len) < more {
            slots = slots.checked_mul(2).ok_or(Error::Invalid(
                "strong components: more pairs than the address space",
            ))?;
        }
        if slots == self.slots.len() {
            return Ok(());
        }
        let mut bigger = Self::new(memory, slots, self.most)?;
        par::for_each_range(self.slots.len(), GRAIN, |r| {
            for i in r {
                if let Some((v, s)) = self.pair(i) {
                    bigger.insert(v, s, self.marks[i].load(Relaxed));
                }
            }
        });
        bigger.len = self.len;
        *self = bigger;
        Ok(())
    }
}

/// What the rounds change, per node.
struct State<'g> {
    /// The part of each node.
    part: Vec<AtomicU64>,
    /// The nodes whose component is found.
    done: NodeSet<'g>,
    /// A union-find of the components found.
    parent: NodeCells<'g>,
}

impl<'g> State<'g> {
    /// Source `s` reaches `v` (`forward`), or `v` reaches `s`.
    fn found(&self, v: Node<'g>, s: Node<'g>, forward: bool) {
        self.part[v.index()].fetch_add(spread(s.get(), forward), Relaxed);
    }

    /// `v` is in the component of source `s`.
    fn join(&self, v: Node<'g>, s: Node<'g>) {
        unite(&self.parent, v, s);
        self.done.insert(v);
    }
}

/// The smallest order and its node: the first source.
#[derive(Clone, Copy)]
struct First(u64, u32);

impl Reduce for First {
    const ZERO: Self = Self(u64::MAX, u32::MAX);
    fn add(self, other: Self) -> Self {
        if (other.0, other.1) < (self.0, self.1) {
            other
        } else {
            self
        }
    }
}

/// Strongly connected components of the directed graph whose out-lists `out` reads and
/// whose in-lists `inn` reads: `labels[v]` is the smallest node of the component of
/// `v`. Returned as the atomics they were computed in, with the reservation that holds
/// them.
///
/// Lists may hold the node itself (a kept loop), which changes nothing.
///
/// # Errors
///
/// From the plans' sources, or the state does not fit in the budget;
/// [`Error::Invalid`] if `inn` has another number of entries than `out`, so is not the
/// in-lists of the same graph.
pub fn scc<'g, S: Source>(
    out: &Plan<'_, 'g, S>,
    inn: &Plan<'_, 'g, S>,
    memory: &Memory,
) -> Result<Atomics, Error> {
    if inn.source().entries() != out.source().entries() {
        return Err(Error::Invalid(
            "strong components: in-lists with another number of entries",
        ));
    }
    let n = out.nodes();
    let _parts = memory.reserve(
        "strong components: parts",
        MemoryKind::Anonymous,
        u64::from(n) * 8,
    )?;
    let state = State {
        part: par::collect(usize_of(n), GRAIN, |_| AtomicU64::new(0)),
        done: out.node_set(memory)?,
        parent: out.node_cells(memory)?,
    };
    for plan in [out, inn] {
        plan.vertices((), |(): &mut (), (), _, v, degree| {
            if degree == 0 {
                state.done.insert(v);
            }
        })?;
    }
    // Batch `b` holds the nodes whose place in the order, out of `n`, has `b` bits.
    let mut batch_of =
        SegVec::try_with_len(memory, "strong components: batches", usize_of(n), 0_u8)?;
    let first = out.vertices(
        one_segment_mut(&mut batch_of)?,
        |first: &mut First, batch_of, i, v, _| {
            let order = draw(SEED, Purpose::SCC_ORDER, [v.get(), 0, 0, 0]);
            let place = below(order, u64::from(n));
            #[expect(clippy::cast_possible_truncation, reason = "at most 64")]
            let bits = (64 - place.leading_zeros()) as u8;
            batch_of[i] = bits;
            if !state.done.contains(v) {
                *first = first.add(First(order, v.get()));
            }
        },
    )?;
    if let Some(first) = out.node(first.1) {
        let ahead = traversal::bfs(out, Some(inn), first.get(), &Bfs::default(), memory)?;
        let back = traversal::bfs(inn, Some(out), first.get(), &Bfs::default(), memory)?;
        let (ahead, back) = (
            out.per_node(&ahead.distance)?,
            out.per_node(&back.distance)?,
        );
        out.vertices((), |(): &mut (), (), _, v, _| {
            let f = ahead[v].load(Relaxed) != UNREACHED;
            let b = back[v].load(Relaxed) != UNREACHED;
            if f {
                state.found(v, first, true);
            }
            if b {
                state.found(v, first, false);
            }
            if f && b {
                state.join(v, first);
            }
        })?;
    }
    let batch_of = out.per_node(one_segment(&batch_of)?)?;
    let mut batch = out.node_set(memory)?;
    let mut sources = NodeList::new(memory);
    for b in 0..=64 {
        let (taken, left) = out.vertices((), |count: &mut (u64, u64), (), _, v, _| {
            if !state.done.contains(v) {
                count.1 += 1;
                if batch_of[v] == b {
                    batch.insert(v);
                    count.0 += 1;
                }
            }
        })?;
        if left == 0 {
            break;
        }
        if taken == 0 {
            continue;
        }
        batch.list_into(&mut sources)?;
        batch.clear();
        let most = u64::from(n).saturating_mul(taken);
        let ahead = reach(out, &state, sources.as_slice()?, most, memory)?;
        let back = reach(inn, &state, sources.as_slice()?, most, memory)?;
        for (pairs, forward) in [(&ahead, true), (&back, false)] {
            par::for_each_range(pairs.slots.len(), GRAIN, |r| {
                for i in r {
                    let Some((v, s)) = pairs.pair(i) else {
                        continue;
                    };
                    let (Some(v), Some(s)) = (out.node(v), out.node(s)) else {
                        continue;
                    };
                    state.found(v, s, forward);
                    if forward && back.contains(v.get(), s.get()) {
                        state.join(v, s);
                    }
                }
            });
        }
    }
    state.parent.flatten();
    Ok(state.parent.into_ids())
}

/// The pairs (node, source) of the search from `sources` along the lists of `plan`,
/// each inside the part of its source; at most `most` of them.
fn reach<'g, S: Source>(
    plan: &Plan<'_, 'g, S>,
    state: &State<'g>,
    sources: &[Node<'g>],
    most: u64,
    memory: &Memory,
) -> Result<Pairs, Error> {
    let mut pairs = Pairs::new(memory, sources.len().saturating_mul(4), most)?;
    par::for_each_range(sources.len(), GRAIN, |r| {
        for &s in &sources[r] {
            pairs.insert(s.get(), s.get(), mark(0));
        }
    });
    pairs.len = sources.len() as u64;
    let mut frontier = NodeList::new(memory);
    frontier.extend_from_slice(sources)?;
    let mut next = plan.node_set(memory)?;
    let mut step = 0;
    while !frontier.is_empty() {
        let (now, then) = (mark(step), mark(step + 1));
        let nodes = frontier.as_slice()?;
        // Each run of the frontier adds at most these pairs; a call of the driver takes
        // as many runs as the set has room for.
        let bounds = par::map_ranges(nodes.len(), RUN, |r| {
            nodes[r]
                .iter()
                .map(|&v| u64::from(plan.degree(v)) * pairs.fresh(v.get(), now))
                .sum::<u64>()
        });
        let mut at = 0;
        while at < bounds.len() {
            pairs.make_room(bounds[at], memory)?;
            let (mut end, mut need) = (at + 1, bounds[at]);
            while end < bounds.len() && need + bounds[end] <= pairs.room() {
                need += bounds[end];
                end += 1;
            }
            let piece = &nodes[at * RUN.get()..nodes.len().min(end * RUN.get())];
            let set = &pairs;
            let added =
                plan.frontier_with(piece, Vec::<u32>::new, |added: &mut u64, row, worker| {
                    let news = &mut *worker.scratch;
                    set.take(row.node.get(), now, news);
                    let home = state.part[row.node.index()].load(Relaxed);
                    for &u in row.list {
                        if state.done.contains(u) || state.part[u.index()].load(Relaxed) != home {
                            continue;
                        }
                        for &s in news.iter() {
                            if set.insert(u.get(), s, then) {
                                *added += 1;
                                next.insert(u);
                            }
                        }
                    }
                })?;
            pairs.len += added;
            at = end;
        }
        next.list_into(&mut frontier)?;
        next.clear();
        step += 1;
    }
    Ok(pairs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_grow_and_keep_their_marks() {
        let memory = Memory::unlimited();
        let mut pairs = Pairs::new(&memory, 1, 1 << 20).unwrap();
        for v in 0..3000 {
            for s in [v, v + 1] {
                assert!(pairs.insert(v, s, mark(v % 2)));
                assert!(!pairs.insert(v, s, mark(0)));
            }
            pairs.len += 2;
            if pairs.room() < 2 {
                pairs.make_room(2, &memory).unwrap();
            }
        }
        assert!(pairs.slots.len() >= 2 * 6000);
        let mut out = Vec::new();
        pairs.take(7, mark(1), &mut out);
        out.sort_unstable();
        assert_eq!(out, [7, 8]);
        assert_eq!(pairs.fresh(7, mark(1)), 0);
        assert_eq!(pairs.fresh(8, mark(0)), 2);
        assert!(pairs.contains(2999, 3000) && !pairs.contains(2999, 2998));
    }
}
