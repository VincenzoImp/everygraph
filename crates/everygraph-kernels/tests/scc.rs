// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Strong components (#223) on generated directed graphs, against the oracle's Tarjan:
//!
//! - **Same answer everywhere.** Labels and work counts are identical at 1, 3 and 8
//!   threads, with chunks of 16 and 65,536 entries, over raw arrays, the compressed
//!   format in the heap and raw targets streamed. `tests/oracle.rs` adds the memory map,
//!   the in-lists that `everygraph-ingest` derives, and the Python references.
//! - **Relabel.** Under a permutation of the nodes, two nodes share a component exactly
//!   when their images do.
//! - **Disjoint union.** The labels on `G ⊔ H` are those on `G` followed by those on
//!   `H`, shifted by the nodes of `G`.
//! - **Reversal.** Reversing every arc keeps every component, so every label.
//! - **Refusals.** In-lists of another graph, and a budget too small for the state.

#![forbid(unsafe_code)]
#![allow(clippy::disallowed_methods, clippy::cast_possible_truncation)]

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::AtomicU32;

use everygraph_core::rng::{Purpose, below, draw};
use everygraph_core::{Budget, Memory, MemorySource, par};
use everygraph_format::adjacency::{Adjacency, AdjacencyIndex, encode};
use everygraph_kernels::edge_map::{Compressed, Csr, Error, Plan, Settings, Source, Stream, Work};
use everygraph_kernels::scc::scc;
use everygraph_oracle::projection::Adjacency as Analytic;
use everygraph_oracle::reference;
use proptest::prelude::*;

/// A directed graph as its arcs; repeats and loops allowed.
#[derive(Clone, Debug)]
struct Digraph {
    n: u32,
    arcs: Vec<(u32, u32)>,
}

impl Digraph {
    /// The lists of the arcs, sorted, without repeats: the out-lists, or with `reverse`
    /// the in-lists.
    fn lists(&self, reverse: bool) -> (Vec<u64>, Vec<u32>) {
        let mut arcs: Vec<(u32, u32)> = self
            .arcs
            .iter()
            .map(|&(u, v)| if reverse { (v, u) } else { (u, v) })
            .collect();
        arcs.sort_unstable();
        arcs.dedup();
        let mut offsets = vec![0_u64; self.n as usize + 1];
        for &(u, _) in &arcs {
            offsets[u as usize + 1] += 1;
        }
        for v in 0..self.n as usize {
            offsets[v + 1] += offsets[v];
        }
        (offsets, arcs.into_iter().map(|a| a.1).collect())
    }

    fn reversed(&self) -> Self {
        Self {
            n: self.n,
            arcs: self.arcs.iter().map(|&(u, v)| (v, u)).collect(),
        }
    }

    /// Node `v` becomes `to[v]`.
    fn relabel(&self, to: &[u32]) -> Self {
        Self {
            n: self.n,
            arcs: self
                .arcs
                .iter()
                .map(|&(u, v)| (to[u as usize], to[v as usize]))
                .collect(),
        }
    }

    fn union(&self, other: &Self) -> Self {
        let shifted = other.arcs.iter().map(|&(u, v)| (u + self.n, v + self.n));
        Self {
            n: self.n + other.n,
            arcs: self.arcs.iter().copied().chain(shifted).collect(),
        }
    }

    /// The oracle's Tarjan, labelled by the smallest node of each component.
    fn tarjan(&self) -> Vec<u32> {
        let (offsets, targets) = self.lists(false);
        let ones = vec![1; targets.len()];
        reference::scc(&Analytic {
            offsets,
            multiplicity: ones.clone(),
            weight_sum: ones.iter().map(|&x| x as f64).collect(),
            weight_min: ones.iter().map(|&x| x as f64).collect(),
            targets,
        })
    }
}

fn labels<S: Source>(out: &S, inn: &S, threads: usize, chunk: u64) -> (Vec<u32>, Work, Work) {
    let memory = Memory::unlimited();
    let settings = Settings {
        chunk_entries: chunk,
        batch_entries: 7,
        ..Settings::default()
    };
    let threads = NonZeroUsize::new(threads).unwrap();
    // One type for both lists, as `scc` takes them.
    fn both<S: Source>(
        out: &S,
        inn: &S,
        memory: &Memory,
        settings: Settings,
    ) -> (Vec<u32>, Work, Work) {
        Plan::with_pair(out, inn, memory, settings, |po, pi| {
            let (l, _held) = scc(&po, &pi, memory).unwrap();
            let l = l.into_iter().map(AtomicU32::into_inner).collect();
            (l, po.work(), pi.work())
        })
        .unwrap()
    }
    par::with_threads(threads, || both(out, inn, &memory, settings)).unwrap()
}

/// The labels of `g`, checked against Tarjan.
fn strong(g: &Digraph) -> Vec<u32> {
    let (oo, ot) = g.lists(false);
    let (io, it) = g.lists(true);
    let got = labels(
        &Csr::new(&oo, &ot).unwrap(),
        &Csr::new(&io, &it).unwrap(),
        1,
        1 << 16,
    );
    assert_eq!(got.0, g.tarjan());
    got.0
}

/// The labels and work of `g` in every configuration, all equal; and equal to Tarjan.
fn everywhere(g: &Digraph) -> Vec<u32> {
    let (oo, ot) = g.lists(false);
    let (io, it) = g.lists(true);
    let (eo, ei) = (encode(&oo, &ot).unwrap(), encode(&io, &it).unwrap());
    let (xo, xi) = (
        AdjacencyIndex::parse(&eo.index).unwrap(),
        AdjacencyIndex::parse(&ei.index).unwrap(),
    );
    let bytes =
        |t: &[u32]| MemorySource::new(t.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>());
    let (bo, bi) = (bytes(&ot), bytes(&it));
    let want = labels(
        &Csr::new(&oo, &ot).unwrap(),
        &Csr::new(&io, &it).unwrap(),
        1,
        1 << 16,
    );
    assert_eq!(want.0, g.tarjan());
    for threads in [1, 3, 8] {
        for chunk in [16, 1 << 16] {
            let csr = labels(
                &Csr::new(&oo, &ot).unwrap(),
                &Csr::new(&io, &it).unwrap(),
                threads,
                chunk,
            );
            let heap = labels(
                &Compressed::new(Adjacency::new(&xo, &eo.data).unwrap()),
                &Compressed::new(Adjacency::new(&xi, &ei.data).unwrap()),
                threads,
                chunk,
            );
            let stream = labels(
                &Stream::new(&oo, &bo).unwrap(),
                &Stream::new(&io, &bi).unwrap(),
                threads,
                chunk,
            );
            for (source, got) in [("raw", csr), ("compressed", heap), ("streamed", stream)] {
                assert_eq!(got, want, "{source}, {threads} threads, chunks of {chunk}");
            }
        }
    }
    want.0
}

/// A permutation of `0..n` from `seed`.
fn permutation(n: u32, seed: u64) -> Vec<u32> {
    let mut p: Vec<u32> = (0..n).collect();
    let mut x = seed | 1;
    for i in (1..p.len()).rev() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        p.swap(i, (x % (i as u64 + 1)) as usize);
    }
    p
}

fn relations(g: &Digraph, seed: u64) {
    let r = everywhere(g);
    let n = g.n as usize;
    let to = permutation(g.n, seed);
    let h = strong(&g.relabel(&to));
    for u in 0..n {
        for v in 0..n {
            let (a, b) = (to[u] as usize, to[v] as usize);
            assert_eq!(h[a] == h[b], r[u] == r[v], "relabelled {u} {v}");
        }
    }
    let shifted = h.iter().map(|&l| l + g.n);
    let union: Vec<u32> = r.iter().copied().chain(shifted).collect();
    assert_eq!(strong(&g.union(&g.relabel(&to))), union);
    assert_eq!(strong(&g.reversed()), r);
}

fn arb_digraph(max_n: u32) -> impl Strategy<Value = Digraph> {
    (0..=max_n, 0..=4_u32).prop_flat_map(|(n, density)| {
        let arc = (0..n.max(1), 0..n.max(1));
        let count = if n == 0 { 0 } else { (n * density) as usize };
        prop::collection::vec(arc, count).prop_map(move |arcs| Digraph { n, arcs })
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(if cfg!(debug_assertions) { 24 } else { 128 }))]

    #[test]
    fn strong_components_keep_their_relations(g in arb_digraph(70), seed in any::<u64>()) {
        relations(&g, seed);
    }
}

/// Many batches and parts: chains of small cycles joined by arcs that go forward only,
/// two long cycles (a high diameter), a node that reaches everything, nodes without
/// in- or out-lists, and loops.
fn layered(n: u32) -> Digraph {
    let mut arcs = Vec::new();
    let mut x = 0x2545_F491_4F6C_DD1D_u64;
    let mut next = |below: u32| {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        (x % u64::from(below)) as u32
    };
    let long = n / 4;
    for v in 0..long {
        arcs.push((v, (v + 1) % long));
    }
    for v in long..2 * long {
        arcs.push((v, if v + 1 == 2 * long { long } else { v + 1 }));
    }
    let mut v = 2 * long;
    while v < n - 8 {
        let size = 1 + next(5);
        for k in 0..size {
            arcs.push((v + k, v + (k + 1) % size));
        }
        for _ in 0..2 {
            let to = v + size + next(n - v - size);
            arcs.push((v + next(size), to));
        }
        v += size;
    }
    for t in (0..n).step_by(97) {
        arcs.push((n - 1, t));
        arcs.push((t, t));
    }
    Digraph { n, arcs }
}

#[test]
fn many_batches_parts_and_a_high_diameter() {
    let g = layered(6000);
    let labels = everywhere(&g);
    let components = (0..g.n).filter(|&v| labels[v as usize] == v).count();
    assert!(components > 500, "{components} components");
    relations(&layered(300), 5);
}

#[test]
fn refusals() {
    let g = layered(500);
    let ((oo, ot), (io, it)) = (g.lists(false), g.lists(true));
    let (out, inn) = (Csr::new(&oo, &ot).unwrap(), Csr::new(&io, &it).unwrap());
    let no_offsets = vec![0_u64; oo.len()];
    let none = Csr::new(&no_offsets, &[]).unwrap();
    let tight = Memory::from(Arc::new(Budget::new(4096)));
    let memory = Memory::unlimited();
    Plan::with_pair(&out, &inn, &tight, Settings::default(), |po, pi| {
        let e = scc(&po, &pi, &tight).unwrap_err();
        assert!(matches!(e, Error::Memory(_)), "{e}");
    })
    .unwrap();
    Plan::with_pair(&out, &none, &memory, Settings::default(), |po, pi| {
        let e = scc(&po, &pi, &memory).unwrap_err();
        assert!(matches!(e, Error::Invalid(_)), "{e}");
    })
    .unwrap();
}

/// The review's probe of the bound on the pairs a step adds. Every `a` has a loop and an
/// arc to the hub `h`; `h` has a loop and arcs to every `b`; every `b` has a loop. The ids
/// are chosen against the kernel's order (seed 0, purpose `SCC_ORDER`): the first source
/// is a `b`, which puts `h` and the `a`s in one part, and `h` and the `a`s are all in the
/// last batch. Then `h` gets `k` new sources at once, and none of its targets is in its
/// part: a bound by degree reserved `k × D` pairs.
#[test]
fn a_hub_reached_by_many_sources_reserves_only_the_pairs_it_finds() {
    let (k, d) = (1_000_u32, 50_000_u32);
    let n = k + 1 + d;
    let place = |v: u32| {
        let order = draw(0, Purpose::SCC_ORDER, [v, 0, 0, 0]);
        below(order, u64::from(n))
    };
    let last = 64 - (u64::from(n) - 1).leading_zeros();
    let high: Vec<u32> = (0..n)
        .filter(|&v| 64 - place(v).leading_zeros() == last)
        .take(k as usize + 1)
        .collect();
    assert_eq!(high.len(), k as usize + 1);
    let (h, a) = (high[0], &high[1..]);
    let b: Vec<u32> = (0..n).filter(|v| !high.contains(v)).collect();
    let mut arcs: Vec<(u32, u32)> = (0..n).map(|v| (v, v)).collect();
    arcs.extend(a.iter().map(|&x| (x, h)));
    arcs.extend(b.iter().map(|&y| (h, y)));
    let g = Digraph { n, arcs };
    let ((oo, ot), (io, it)) = (g.lists(false), g.lists(true));
    let (out, inn) = (Csr::new(&oo, &ot).unwrap(), Csr::new(&io, &it).unwrap());
    let budget = Arc::new(Budget::new(u64::MAX));
    let memory = Memory::from(budget.clone());
    let labels = Plan::with_pair(&out, &inn, &memory, Settings::default(), |po, pi| {
        let (l, _held) = scc(&po, &pi, &memory).unwrap();
        l.into_iter().map(AtomicU32::into_inner).collect::<Vec<_>>()
    })
    .unwrap();
    assert_eq!(
        labels,
        (0..n).collect::<Vec<_>>(),
        "every node is its own component"
    );
    // Each node is reached by at most two sources each way here: its own and the hub.
    let pairs = 4 * u64::from(n);
    let model = u64::from(n) * 13 + pairs * 18;
    assert!(
        budget.peak() <= 2 * model,
        "peak {} bytes, model {model}",
        budget.peak()
    );
}
