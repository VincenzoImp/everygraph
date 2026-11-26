// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Metamorphic relations of the components, core-number and degree kernels (#48, #50),
//! on generated undirected graphs with kept loops, at 1 and 4 threads:
//!
//! - **Relabel.** Under a permutation of the nodes, every per-node result moves with its
//!   node, and the component of `v` holds the same nodes.
//! - **Disjoint union.** The result on `G ⊔ H` is the result on `G` followed by the
//!   result on `H`, its labels shifted by the nodes of `G`.
//! - **Induced subgraph.** On the nodes of the `k`-core, core numbers are those of the
//!   whole graph; on a union of components, the components are the same.
//! - **Regimes.** Sampled and one-pass components are equal; numbering by size agrees
//!   with the labels.

#![forbid(unsafe_code)]
#![allow(clippy::disallowed_methods, clippy::cast_possible_truncation)]

use std::num::NonZeroUsize;
use std::sync::atomic::AtomicU32;

use everygraph_core::{Memory, par};
use everygraph_kernels::components::{WccOptions, number, wcc};
use everygraph_kernels::degrees::{Loop, degrees};
use everygraph_kernels::edge_map::{Csr, Plan, Settings};
use everygraph_kernels::kcore::{kcore, shells};
use everygraph_oracle::projection::Projection;
use everygraph_oracle::strategies::arb_undirected_graph;
use proptest::prelude::*;

/// Symmetric lists, sorted, with a node's own entry where it has a kept loop.
#[derive(Clone, Debug)]
struct Lists {
    offsets: Vec<u64>,
    targets: Vec<u32>,
}

impl Lists {
    fn n(&self) -> u32 {
        self.offsets.len() as u32 - 1
    }

    fn list(&self, v: u32) -> &[u32] {
        &self.targets[self.offsets[v as usize] as usize..self.offsets[v as usize + 1] as usize]
    }

    fn from_lists(lists: impl Iterator<Item = Vec<u32>>) -> Self {
        let mut out = Self {
            offsets: Vec::from([0]),
            targets: Vec::new(),
        };
        for mut l in lists {
            l.sort_unstable();
            out.targets.extend(l);
            out.offsets.push(out.targets.len() as u64);
        }
        out
    }

    fn of(p: &Projection) -> Self {
        Self::from_lists((0..p.n).map(|v| {
            let mut l = p.undirected.list(v as usize).to_vec();
            if p.keep_loops && p.loops[v as usize] > 0 {
                l.push(v);
            }
            l
        }))
    }

    /// Node `v` becomes `to[v]`.
    fn relabel(&self, to: &[u32]) -> Self {
        let mut from: Vec<u32> = (0..self.n()).collect();
        from.sort_by_key(|&v| to[v as usize]);
        Self::from_lists(
            from.iter()
                .map(|&v| self.list(v).iter().map(|&u| to[u as usize]).collect()),
        )
    }

    fn union(&self, other: &Self) -> Self {
        let n = self.n();
        Self::from_lists(
            (0..n)
                .map(|v| self.list(v).to_vec())
                .chain((0..other.n()).map(|v| other.list(v).iter().map(|&u| u + n).collect())),
        )
    }

    /// The subgraph induced by the nodes with `keep`, renumbered in order.
    fn induced(&self, keep: &[bool]) -> Self {
        let mut new = Vec::new();
        let mut next = 0;
        for &k in keep {
            new.push(next);
            next += u32::from(k);
        }
        Self::from_lists((0..self.n()).filter(|&v| keep[v as usize]).map(|v| {
            self.list(v)
                .iter()
                .filter(|&&u| keep[u as usize])
                .map(|&u| new[u as usize])
                .collect()
        }))
    }
}

/// The results of every kernel on one graph.
#[derive(Debug, PartialEq)]
struct Results {
    labels: Vec<u32>,
    one_pass: Vec<u32>,
    component: Vec<u32>,
    sizes: Vec<u32>,
    core: Vec<u32>,
    shells: Vec<u64>,
    degree: Vec<u32>,
}

fn results(g: &Lists, threads: usize) -> Results {
    let csr = Csr::new(&g.offsets, &g.targets).unwrap();
    let memory = Memory::unlimited();
    let settings = Settings {
        chunk_entries: 16,
        ..Settings::default()
    };
    let plain = |a: Vec<AtomicU32>| a.into_iter().map(AtomicU32::into_inner).collect();
    par::with_threads(NonZeroUsize::new(threads).unwrap(), || {
        Plan::with(&csr, &memory, settings, |plan| {
            let (labels, _l) = wcc(&plan, &memory, WccOptions::default()).unwrap();
            let one_pass = WccOptions {
                sample: false,
                ..WccOptions::default()
            };
            let (single, _s) = wcc(&plan, &memory, one_pass).unwrap();
            let numbered = number(&labels, &memory).unwrap();
            let (core, _c) = kcore(&plan, &memory).unwrap();
            let shells = shells(&core, &memory).unwrap();
            let degree = degrees(&plan, &memory, Loop::Twice).unwrap();
            Results {
                labels: plain(labels),
                one_pass: plain(single),
                component: numbered.component.iter().copied().collect(),
                sizes: numbered.sizes.iter().copied().collect(),
                core: plain(core),
                shells: shells.iter().copied().collect(),
                degree: degree.iter().copied().collect(),
            }
        })
    })
    .unwrap()
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

fn check(g: &Lists, seed: u64) {
    let r = results(g, 1);
    assert_eq!(results(g, 4), r, "threads");
    let n = g.n() as usize;

    // Regimes and numbering.
    assert_eq!(r.one_pass, r.labels);
    let mut by_label = std::collections::BTreeMap::new();
    for v in 0..n {
        assert!(r.labels[v] as usize <= v && r.labels[r.labels[v] as usize] == r.labels[v]);
        *by_label.entry(r.labels[v]).or_insert(0_u32) += 1;
    }
    let mut order: Vec<(u32, u32)> = by_label.iter().map(|(&l, &s)| (s, l)).collect();
    order.sort_by_key(|&(s, l)| (std::cmp::Reverse(s), l));
    for (number, &(size, label)) in order.iter().enumerate() {
        assert_eq!(r.component[label as usize], number as u32);
        assert_eq!(r.sizes[number], size);
    }
    assert_eq!(r.shells.iter().sum::<u64>(), n as u64);

    // Relabel.
    let to = permutation(g.n(), seed);
    let h = results(&g.relabel(&to), 1);
    for v in 0..n {
        let w = to[v] as usize;
        assert_eq!((h.core[w], h.degree[w]), (r.core[v], r.degree[v]));
        for u in 0..n {
            let same = r.labels[u] == r.labels[v];
            assert_eq!(h.labels[to[u] as usize] == h.labels[w], same);
        }
    }
    assert_eq!(
        (h.sizes.clone(), h.shells.clone()),
        (r.sizes.clone(), r.shells.clone())
    );

    // Disjoint union with the relabelled graph.
    let u = results(&g.union(&g.relabel(&to)), 1);
    let shift = |l: &[u32]| l.iter().map(|&x| x + n as u32).collect::<Vec<_>>();
    assert_eq!(u.labels, [r.labels.clone(), shift(&h.labels)].concat());
    assert_eq!(u.core, [r.core.clone(), h.core.clone()].concat());
    assert_eq!(u.degree, [r.degree.clone(), h.degree.clone()].concat());

    // Induced subgraphs: the k-core for each k, and the components of even label.
    for k in 0..=r.core.iter().copied().max().unwrap_or(0) {
        let keep: Vec<bool> = r.core.iter().map(|&c| c >= k).collect();
        let sub = results(&g.induced(&keep), 1);
        let want: Vec<u32> = (0..n).filter(|&v| keep[v]).map(|v| r.core[v]).collect();
        assert_eq!(sub.core, want, "the {k}-core");
    }
    let keep: Vec<bool> = r.labels.iter().map(|&l| l % 2 == 0).collect();
    let sub = results(&g.induced(&keep), 1);
    let rank: Vec<u32> = keep
        .iter()
        .scan(0, |next, &k| {
            let at = *next;
            *next += u32::from(k);
            Some(at)
        })
        .collect();
    let want: Vec<u32> = (0..n)
        .filter(|&v| keep[v])
        .map(|v| rank[r.labels[v] as usize])
        .collect();
    assert_eq!(sub.labels, want);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(if cfg!(debug_assertions) { 24 } else { 128 }))]

    #[test]
    fn components_cores_and_degrees_keep_their_relations(g in arb_undirected_graph(60), seed in any::<u64>()) {
        check(&Lists::of(&Projection::new(&g)), seed);
    }
}

/// Dense enough for the sampling (three entries per node), with a hub and isolates.
#[test]
fn relations_hold_on_a_graph_that_samples() {
    let n = 400_u32;
    let lists = (0..n).map(|v| {
        let mut l: Vec<u32> = (1..6)
            .map(|k| (v * 7 + k * 13) % n)
            .filter(|&u| u != v && v % 50 != 0)
            .collect();
        if v == 1 {
            l.extend((2..n).filter(|u| u % 50 != 0));
        }
        l
    });
    // Symmetrize.
    let one: Vec<Vec<u32>> = lists.collect();
    let mut sym: Vec<Vec<u32>> = (0..n).map(|_| Vec::new()).collect();
    for (v, l) in one.iter().enumerate() {
        for &u in l {
            sym[v].push(u);
            sym[u as usize].push(v as u32);
        }
    }
    let g = Lists::from_lists(sym.into_iter().map(|mut l| {
        l.sort_unstable();
        l.dedup();
        l
    }));
    assert!(g.targets.len() >= 3 * n as usize);
    check(&g, 7);
}
