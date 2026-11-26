// SPDX-License-Identifier: Apache-2.0 OR MIT

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
    let t = leaves + 1;
    let edges = (1..=leaves).map(|leaf| (0, leaf));
    symmetric(leaves + 3, edges.chain([(t, t + 1), (t + 1, 0), (0, t)]))
}

/// The symmetric lists of `n` nodes and `edges`.
fn symmetric(n: u32, edges: impl IntoIterator<Item = (u32, u32)>) -> (Vec<u64>, Vec<u32>) {
    let mut lists: Vec<Vec<u32>> = (0..n).map(|_| Vec::new()).collect();
    for (a, b) in edges {
        lists[a as usize].push(b);
        lists[b as usize].push(a);
    }
    let (mut offsets, mut targets) = (Vec::from([0]), Vec::new());
    for mut l in lists {
        l.sort_unstable();
        targets.extend(l);
        offsets.push(targets.len() as u64);
    }
    (offsets, targets)
}

/// The edges of a clique on `nodes`.
fn clique(nodes: &[u32]) -> impl Iterator<Item = (u32, u32)> + '_ {
    nodes
        .iter()
        .enumerate()
        .flat_map(move |(i, &a)| nodes[i + 1..].iter().map(move |&b| (a, b)))
}

/// After `pad` isolated nodes, a hub of degree 20: 15 leaves and 5 links into a
/// clique of `size` nodes. The hub's core is 5, and after the leaves the smallest
/// key is `size − 1`: the hub's exact count falls in the gap between them.
fn hub_by_clique(pad: u32, size: u32) -> (Vec<u64>, Vec<u32>) {
    let h = pad;
    let nodes: Vec<u32> = (pad + 16..pad + 16 + size).collect();
    let leaves = (pad + 1..pad + 16).map(|l| (h, l));
    let links = nodes[..5].iter().map(|&c| (h, c));
    let edges: Vec<(u32, u32)> = leaves.chain(links).chain(clique(&nodes)).collect();
    symmetric(pad + 16 + size, edges)
}

/// After `pad` isolated nodes, a hub joined to every node of 20 cliques of 6. Every
/// core is 6; the hub's degree falls from 120 to 0 in round 6, where, with `μ = 8`,
/// it takes enough samples to be counted in that round.
fn hub_of_cliques(pad: u32) -> (Vec<u64>, Vec<u32>) {
    let h = pad;
    let mut edges = Vec::new();
    for c in 0..20 {
        let nodes: Vec<u32> = (0..6).map(|i| pad + 1 + 6 * c + i).collect();
        edges.extend(clique(&nodes));
        edges.extend(nodes.iter().map(|&v| (h, v)));
    }
    symmetric(pad + 121, edges)
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

/// A hub counted between rounds whose degree is below the smallest key is a miss:
/// peeled in that key's round, it would get a core number too high.
#[test]
#[cfg_attr(
    miri,
    ignore = "safe code; Miri checks the unsafe code, which the framework tests reach"
)]
fn a_hub_counted_below_the_next_key_keeps_its_core() {
    for size in [7, 9, 12] {
        for pad in 0..40 {
            let (offsets, targets) = hub_by_clique(pad, size);
            let want = reference(&offsets, &targets);
            assert_eq!(want[pad as usize], 5);
            for mu in [1, 2, 4] {
                let sampler = Some(Sampler { mu, min_degree: 20 });
                let got = run(&offsets, &targets, sampler, true).unwrap();
                assert_eq!(got, want, "size {size}, pad {pad}, mu {mu}");
            }
        }
    }
}

/// A hub counted in its own round, because its neighbours peeled in that round gave
/// it its samples, is no miss: the peel ends without a restart.
#[test]
#[cfg_attr(
    miri,
    ignore = "safe code; Miri checks the unsafe code, which the framework tests reach"
)]
fn a_hub_counted_in_its_round_does_not_restart() {
    for pad in 0..10 {
        let (offsets, targets) = hub_of_cliques(pad);
        let want = reference(&offsets, &targets);
        let sampler = Some(Sampler {
            mu: 8,
            min_degree: 20,
        });
        assert_eq!(
            run(&offsets, &targets, sampler, false),
            Some(want),
            "pad {pad}"
        );
    }
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
