// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A copy of `everygraph_kernels::sssp` (main 50c15cd) whose cost of entry `e` is a
//! function `cost(e)` instead of a column. The f64 column of twitter-sym (19.2 GB) and
//! Friendster (28.9 GB) does not fit in the brief's limits (16 GB, 24 GB). Only the cost
//! lookup and the weight check differ; `sssp-check` compares this copy with the product
//! kernel on orkut (distance hash, and the per-step counts at one thread).

use std::collections::VecDeque;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Mutex, PoisonError};

use everygraph_core::{Memory, par};
use everygraph_kernels::edge_map::{Error, Node, NodeList, NodeSet, PerNode, Plan, Row, Source, counters};

pub const FUSE: usize = 1024;
const NEAR: f64 = 8.0;
const SAMPLES: u64 = 1024;
const GRAIN: NonZeroUsize = NonZeroUsize::new(4096).expect("non-zero");

/// `sssp::delta`, with `cost(e)` for `costs[e]`.
pub fn delta<S: Source>(plan: &Plan<'_, '_, S>, cost: fn(u64) -> f64) -> f64 {
    let m = plan.source().entries();
    let k = m.min(SAMPLES);
    let at = |i: u64| (u128::from(i) * u128::from(m) / u128::from(k)) as u64;
    let mut sample: Vec<f64> = (0..k).map(|i| cost(at(i))).collect();
    sample.sort_unstable_by(f64::total_cmp);
    let median = match sample.get(sample.len() / 2) {
        Some(&x) if x > 0.0 => x,
        _ => sample.last().copied().unwrap_or(0.0),
    };
    if !(median > 0.0 && median.is_finite()) {
        return 1.0;
    }
    let degree = m as f64 / f64::from(plan.nodes().max(1));
    let width = if degree > 8.0 { median / degree } else { 16.0 * median / degree };
    if width > 0.0 && width.is_finite() { width } else { median }
}

/// `sssp::sssp` with `cost(e)`; each step's frontier call is tagged `sssp.step`.
pub fn sssp<S: Source>(
    out: &Plan<'_, '_, S>,
    cost: fn(u64) -> f64,
    source: u32,
    width: f64,
    memory: &Memory,
) -> Result<Vec<AtomicU64>, Error> {
    let source = out.node(source).ok_or(Error::Argument("not a node"))?;
    let n = out.nodes();
    let infinity = f64::INFINITY.to_bits();
    let bits: Vec<AtomicU64> = par::collect(n as usize, GRAIN, |_| AtomicU64::new(infinity));
    let dist = out.per_node(&bits)?;
    dist[source].store(0.0_f64.to_bits(), Relaxed);
    let (near, far) = (out.node_set(memory)?, out.node_set(memory)?);
    near.insert(source);
    let mut split = bucket_end(0.0, width, NEAR);
    let mut now = out.node_set(memory)?;
    let (mut waiting, mut frontier) = (NodeList::new(memory), NodeList::new(memory));
    let failed = Mutex::new(None);
    let mut step = 0_u64;
    loop {
        near.list_into(&mut waiting)?;
        if waiting.is_empty() {
            far.list_into(&mut waiting)?;
            let list = waiting.as_slice()?;
            if list.is_empty() {
                break;
            }
            split = bucket_end(lowest(list, dist), width, NEAR);
            move_below(list, dist, split, &far, &near);
            continue;
        }
        let list = waiting.as_slice()?;
        let theta = bucket_end(lowest(list, dist), width, 1.0);
        move_below(list, dist, theta, &near, &now);
        now.list_into(&mut frontier)?;
        now.clear();
        let room = if frontier.len() <= FUSE { FUSE } else { 0 };
        counters::tag("sssp.step", step);
        step += 1;
        out.frontier_with(frontier.as_slice()?, VecDeque::new, |(): &mut (), row, w| {
            let s = Step { dist, cost, theta, room, split, near: &near, far: &far };
            s.relax(row, w.scratch);
            while let Some(v) = w.scratch.pop_front() {
                match w.lists.get(v) {
                    Ok(row) => s.relax(row, w.scratch),
                    Err(e) => {
                        failed.lock().unwrap_or_else(PoisonError::into_inner).get_or_insert(e);
                        w.scratch.clear();
                    }
                }
            }
        })?;
        if let Some(e) = failed.lock().unwrap_or_else(PoisonError::into_inner).take() {
            return Err(e);
        }
    }
    Ok(bits)
}

fn lowest<'g>(list: &[Node<'g>], dist: PerNode<'_, 'g, AtomicU64>) -> f64 {
    let parts = par::map_ranges(list.len(), GRAIN, |r| list[r].iter().map(|&v| dist[v].load(Relaxed)).min());
    f64::from_bits(parts.into_iter().flatten().min().unwrap_or(f64::INFINITY.to_bits()))
}

fn bucket_end(lowest: f64, width: f64, k: f64) -> f64 {
    let end = ((lowest / width).floor() + k) * width;
    if end > lowest { end } else { lowest.next_up() }
}

fn move_below<'g>(list: &[Node<'g>], dist: PerNode<'_, 'g, AtomicU64>, bound: f64, from: &NodeSet<'g>, to: &NodeSet<'g>) {
    par::for_each_range(list.len(), GRAIN, |r| {
        for &v in &list[r] {
            if f64::from_bits(dist[v].load(Relaxed)) < bound {
                from.remove(v);
                to.insert(v);
            }
        }
    });
}

struct Step<'a, 'g> {
    dist: PerNode<'a, 'g, AtomicU64>,
    cost: fn(u64) -> f64,
    theta: f64,
    room: usize,
    split: f64,
    near: &'a NodeSet<'g>,
    far: &'a NodeSet<'g>,
}

impl<'g> Step<'_, 'g> {
    fn relax(&self, row: Row<'_, 'g>, queue: &mut VecDeque<Node<'g>>) {
        let d = f64::from_bits(self.dist[row.node].load(Relaxed));
        for (&v, e) in row.list.iter().zip(row.first_entry..) {
            let new = (d + (self.cost)(e)).to_bits();
            let to = &self.dist[v];
            if new < to.load(Relaxed) && new < to.fetch_min(new, Relaxed) {
                if f64::from_bits(new) < self.theta && queue.len() < self.room {
                    queue.push_back(v);
                } else if f64::from_bits(new) < self.split {
                    self.near.insert(v);
                    self.far.remove(v);
                } else {
                    self.far.insert(v);
                }
            }
        }
    }
}
