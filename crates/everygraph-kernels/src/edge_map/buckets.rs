// SPDX-License-Identifier: Apache-2.0 OR MIT

//! [`Buckets`]: nodes ordered by an integer key whose values only go down while a node
//! is held, for peeling (k-core) and bucketed shortest paths (Δ-stepping).
//!
//! The kernel owns the keys; the buckets read them through a closure when they need
//! them. Bucket 0 holds the smallest key, `base`. Bucket `j ≥ 1` holds the keys whose
//! highest bit that differs from `base` is bit `j − 1`: `base + 1` (for an even base),
//! then ranges that double in width. This is the hierarchical bucketing structure of
//! Liu, Dong, Gu and Sun (SIGMOD 2025, § 5.2): a node moves down at most once per
//! bucket, so it costs `O(log key)` moves, where Julienne's fixed window of buckets costs
//! `O(key / b + b)`.
//!
//! A change of key is not applied when it is made. [`Buckets::decreased`] marks the node
//! when its bucket changes; the marks are applied at the next [`peek`](Buckets::peek)
//! or [`pop`](Buckets::pop). A node may then sit in several buckets at once: each copy
//! is checked against the node's key when its bucket is read, and dropped if it is out
//! of date.
//!
//! **Determinism.** [`pop`](Buckets::pop) returns the nodes of the smallest key in
//! increasing order, each once, whatever the order of the marks and the thread count.
//! The order of the nodes inside a bucket follows the data only.

#![forbid(unsafe_code)]

use std::num::NonZeroUsize;

use everygraph_core::{Memory, MemoryKind, par};

use super::{Error, Node, NodeList, NodeSet, Plan, Source};

/// Buckets 0 to 32: one per bit position of a `u32` key, and one for `base` itself.
const BUCKETS: usize = 33;

/// Nodes per task when buckets are filled in parallel.
const GRAIN: NonZeroUsize = NonZeroUsize::new(1 << 14).expect("non-zero");

/// The bucket of `key` when the smallest key is `base`. Keys below `base` go to bucket 0.
fn bucket(key: u32, base: u32) -> usize {
    if key <= base {
        0
    } else {
        32 - (key ^ base).leading_zeros() as usize
    }
}

/// Nodes of the plan of brand `'g` by key (module documentation). Made by
/// [`Buckets::new`], empty.
#[derive(Debug)]
pub struct Buckets<'g> {
    base: u32,
    open: Vec<NodeList<'g>>,
    /// Nodes whose bucket changed since the last `peek`.
    moved: NodeSet<'g>,
    /// Scratch: the nodes of a bucket, once each.
    found: NodeSet<'g>,
    list: NodeList<'g>,
    memory: Memory,
}

impl<'g> Buckets<'g> {
    /// No nodes, smallest key 0, over the nodes of `plan`.
    ///
    /// # Errors
    ///
    /// [`Error::Memory`] if two node sets do not fit in the budget.
    pub fn new<S: Source>(plan: &Plan<'_, 'g, S>, memory: &Memory) -> Result<Self, Error> {
        Ok(Self {
            base: 0,
            open: (0..BUCKETS).map(|_| NodeList::new(memory)).collect(),
            moved: plan.node_set(memory)?,
            found: plan.node_set(memory)?,
            list: NodeList::new(memory),
            memory: memory.clone(),
        })
    }

    /// The smallest key that [`pop`](Self::pop) may still return. Keys below it are
    /// taken as equal to it.
    #[must_use]
    pub fn base(&self) -> u32 {
        self.base
    }

    /// Adds `v`, or tells that its key changed. Its key is read at the next
    /// [`peek`](Self::peek) or [`pop`](Self::pop). May be called from many workers.
    pub fn insert(&self, v: Node<'g>) {
        self.moved.insert(v);
    }

    /// Tells that the key of `v`, which the buckets hold, went from `old` down to `new`.
    /// Cheap when the bucket stays the same, which is the usual case.
    #[inline]
    pub fn decreased(&self, v: Node<'g>, old: u32, new: u32) {
        if bucket(new, self.base) != bucket(old, self.base) {
            self.moved.insert(v);
        }
    }

    /// The smallest key of the nodes held, where `key(v)` is the key of node `v`, or
    /// `None` if `v` is no longer held. `None` if no node is held.
    ///
    /// `key` must be at least [`base`](Self::base) for every node held, and must not
    /// change while this runs. Every change of key since the last call was told by
    /// [`insert`](Self::insert) or [`decreased`](Self::decreased).
    ///
    /// # Errors
    ///
    /// [`Error::Memory`] if the buckets do not fit in the budget.
    pub fn peek(
        &mut self,
        key: impl Fn(Node<'g>) -> Option<u32> + Sync,
    ) -> Result<Option<u32>, Error> {
        let mut list = std::mem::replace(&mut self.list, NodeList::new(&self.memory));
        self.moved.list_into(&mut list)?;
        self.moved.clear();
        self.distribute(list.as_slice()?, &key, None)?;
        self.list = list;
        loop {
            let Some(j) = self.open.iter().position(|b| !b.is_empty()) else {
                return Ok(None);
            };
            let held = std::mem::replace(&mut self.open[j], NodeList::new(&self.memory));
            let nodes = held.as_slice()?;
            if j == 0 {
                let base = self.base;
                let kept = self.distribute(nodes, &|v| key(v).filter(|&k| k <= base), Some(0))?;
                if kept > 0 {
                    return Ok(Some(self.base));
                }
                continue;
            }
            let base = self.base;
            let held_here = |v| key(v).filter(|&k| bucket(k, base) == j);
            let least = par::map_ranges(nodes.len(), GRAIN, |r| {
                nodes[r].iter().filter_map(|&v| held_here(v)).min()
            })
            .into_iter()
            .flatten()
            .min();
            if let Some(least) = least {
                self.base = least;
                self.distribute(nodes, &held_here, None)?;
            }
        }
    }

    /// The nodes of the smallest key, into `out` in increasing order, each once; that
    /// key, or `None` if no node is held. They are no longer held. As [`peek`](Self::peek)
    /// otherwise.
    ///
    /// # Errors
    ///
    /// [`Error::Memory`] if the buckets or `out` do not fit in the budget.
    pub fn pop(
        &mut self,
        key: impl Fn(Node<'g>) -> Option<u32> + Sync,
        out: &mut NodeList<'g>,
    ) -> Result<Option<u32>, Error> {
        let Some(k) = self.peek(key)? else {
            out.clear();
            return Ok(None);
        };
        let nodes = self.open[0].as_slice()?;
        let found = &self.found;
        par::for_each_range(nodes.len(), GRAIN, |r| {
            for &v in &nodes[r] {
                found.insert(v);
            }
        });
        self.found.list_into(out)?;
        self.found.clear();
        self.open[0].clear();
        Ok(Some(k))
    }

    /// Appends each node of `nodes` with a key to the bucket of its key, or to bucket
    /// `to` if given; returns how many were appended. In parallel, in chunk order.
    fn distribute(
        &mut self,
        nodes: &[Node<'g>],
        key: &(impl Fn(Node<'g>) -> Option<u32> + Sync),
        to: Option<usize>,
    ) -> Result<usize, Error> {
        let base = self.base;
        let _transient = self.memory.reserve(
            "buckets: nodes being moved",
            MemoryKind::Anonymous,
            // Twice the ids: the per-chunk vectors grow by doubling.
            nodes.len() as u64 * 8,
        )?;
        let chunks = par::map_ranges(nodes.len(), GRAIN, |r| {
            let mut parts: Vec<Vec<Node<'g>>> = (0..BUCKETS).map(|_| Vec::new()).collect();
            for &v in &nodes[r] {
                if let Some(k) = key(v) {
                    parts[to.unwrap_or_else(|| bucket(k, base))].push(v);
                }
            }
            parts
        });
        let mut added = 0;
        for parts in &chunks {
            for (open, part) in self.open.iter_mut().zip(parts) {
                open.extend_from_slice(part)?;
                added += part.len();
            }
        }
        Ok(added)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

    use super::*;
    use crate::edge_map::{Csr, Settings};
    use everygraph_core::rng::{Purpose, below, draw};

    #[test]
    fn buckets_follow_the_hierarchy_of_liu_et_al() {
        // Figure 4 of the paper, rows k = 0, 2, 4, 8.
        let ids = |base, keys: &[u32]| keys.iter().map(|&k| bucket(k, base)).collect::<Vec<_>>();
        assert_eq!(ids(0, &[0, 1, 2, 3, 4, 7, 8, 15]), [0, 1, 2, 2, 3, 3, 4, 4]);
        assert_eq!(ids(2, &[2, 3, 4, 7, 8, 15]), [0, 1, 3, 3, 4, 4]);
        assert_eq!(ids(4, &[4, 5, 6, 7, 8, 15]), [0, 1, 2, 2, 4, 4]);
        assert_eq!(ids(8, &[8, 9, 10, 11, 12, 15, 16]), [0, 1, 2, 2, 3, 3, 5]);
        assert_eq!(bucket(u32::MAX, 0), 32);
        assert_eq!(bucket(3, 5), 0);
    }

    /// Random decreases of random keys, popped against an ordered set of (key, node).
    #[test]
    #[cfg_attr(
        miri,
        ignore = "safe code; Miri checks the unsafe code, which the framework tests reach"
    )]
    fn pops_match_an_ordered_set() {
        let n = 3000_u32;
        let offsets: Vec<u64> = (0..=n).map(|_| 0).collect();
        let csr = Csr::new(&offsets, &[]).unwrap();
        let memory = Memory::unlimited();
        for seed in 0..4 {
            Plan::with(&csr, &memory, Settings::default(), |plan| {
                let r = |i: u32, j: u32, m: u64| below(draw(seed, Purpose::TEST, [i, j, 0, 0]), m);
                let keys: Vec<AtomicU32> = (0..n)
                    .map(|v| AtomicU32::new(u32::try_from(r(v, 0, 1 << (v % 20))).unwrap()))
                    .collect();
                let done: Vec<AtomicU32> = (0..n).map(|_| AtomicU32::new(0)).collect();
                let key = |v: Node<'_>| {
                    (done[v.index()].load(Relaxed) == 0).then(|| keys[v.index()].load(Relaxed))
                };
                let mut want: BTreeSet<(u32, u32)> = (0..n)
                    .map(|v| (keys[v as usize].load(Relaxed), v))
                    .collect();
                let mut b = Buckets::new(&plan, &memory).unwrap();
                for v in plan.all_nodes() {
                    b.insert(v);
                }
                let mut out = NodeList::new(&memory);
                let mut round = 0;
                while let Some(k) = b.pop(key, &mut out).unwrap() {
                    let expect: Vec<u32> =
                        want.iter().take_while(|e| e.0 == k).map(|e| e.1).collect();
                    let got: Vec<u32> = out.as_slice().unwrap().iter().map(|v| v.get()).collect();
                    assert_eq!(got, expect, "seed {seed}, key {k}");
                    for &v in &expect {
                        want.remove(&(k, v));
                        done[v as usize].store(1, Relaxed);
                    }
                    // Lower some keys, never below the current one.
                    for i in 0..50 {
                        let v = u32::try_from(r(round, i, u64::from(n))).unwrap();
                        let old = keys[v as usize].load(Relaxed);
                        if done[v as usize].load(Relaxed) != 0 || old == k {
                            continue;
                        }
                        let new =
                            k + u32::try_from(r(round, i + 1000, u64::from(old - k))).unwrap();
                        keys[v as usize].store(new, Relaxed);
                        want.remove(&(old, v));
                        want.insert((new, v));
                        b.decreased(plan.node(v).unwrap(), old, new);
                    }
                    round += 1;
                }
                assert!(want.is_empty(), "{} nodes never popped", want.len());
            });
        }
    }

    /// A node put back with the current key comes out again with that key.
    #[test]
    fn a_node_inserted_at_the_base_comes_out_again() {
        let offsets = [0_u64; 5];
        let csr = Csr::new(&offsets, &[]).unwrap();
        let memory = Memory::unlimited();
        Plan::with(&csr, &memory, Settings::default(), |plan| {
            let keys = [5_u32, 9, 9, 40];
            let mut b = Buckets::new(&plan, &memory).unwrap();
            plan.all_nodes().for_each(|v| b.insert(v));
            let mut out = NodeList::new(&memory);
            let live = AtomicU32::new(0b1111);
            let key =
                |v: Node<'_>| (live.load(Relaxed) >> v.get() & 1 == 1).then(|| keys[v.index()]);
            assert_eq!(b.peek(key).unwrap(), Some(5));
            assert_eq!(b.pop(key, &mut out).unwrap(), Some(5));
            b.insert(plan.node(0).unwrap());
            assert_eq!(b.pop(key, &mut out).unwrap(), Some(5));
            live.fetch_and(!1, Relaxed);
            assert_eq!(b.pop(key, &mut out).unwrap(), Some(9));
            assert_eq!(out.as_slice().unwrap().len(), 2);
            live.fetch_and(!0b110, Relaxed);
            assert_eq!(b.pop(key, &mut out).unwrap(), Some(40));
            live.store(0, Relaxed);
            assert_eq!(b.pop(key, &mut out).unwrap(), None);
            assert!(out.is_empty());
        });
    }
}
