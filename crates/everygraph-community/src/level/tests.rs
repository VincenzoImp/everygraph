// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tests of the private steps of a level.

// Small generated graphs and draws: casts cannot truncate, and counts are exact floats.
#![allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]

use everygraph_kernels::edge_map::{Csr, Settings};

use super::*;

/// The heavy nodes of a star with `leaves` leaves, and of a cycle of `n` nodes.
fn heavy_ids(offsets: &[u64], targets: &[u32]) -> Vec<u32> {
    let csr = Csr::new(offsets, targets).unwrap();
    let memory = Memory::unlimited();
    Plan::with(&csr, &memory, Settings::default(), |plan| {
        let k = strengths(&plan, &memory, Weights::Unit, &Options::default()).unwrap();
        let g = Graph::new(&plan, Weights::Unit, &k);
        heavy(g, k.total).iter().map(|v| v.get()).collect()
    })
}

#[test]
fn heavy_nodes_hold_more_than_a_thousandth_of_the_volume() {
    // A star: the hub holds half of 2W, a leaf 1 / (2 * leaves).
    for leaves in [10_u32, 600, 3000] {
        let offsets: Vec<u64> = std::iter::once(0)
            .chain((0..=leaves).map(|i| u64::from(leaves + i)))
            .collect();
        let targets: Vec<u32> = (1..=leaves)
            .chain(std::iter::repeat_n(0, leaves as usize))
            .collect();
        let heavy = heavy_ids(&offsets, &targets);
        let expected: Vec<u32> = if leaves < 500 {
            (0..=leaves).collect()
        } else {
            vec![0]
        };
        assert_eq!(heavy, expected, "{leaves} leaves");
    }
    // A cycle of n nodes: each holds 1 / n of 2W, heavy only below 1,000 nodes.
    for (n, count) in [(999_u32, 999), (1000, 0), (4000, 0)] {
        let offsets: Vec<u64> = (0..=u64::from(n)).map(|i| 2 * i).collect();
        let targets: Vec<u32> = (0..n)
            .flat_map(|v| {
                let mut l = [(v + n - 1) % n, (v + 1) % n];
                l.sort_unstable();
                l
            })
            .collect();
        assert_eq!(heavy_ids(&offsets, &targets).len(), count, "cycle of {n}");
    }
}

/// A 64-bit linear congruential step, for test data.
fn step(x: &mut u64) -> u64 {
    *x = x
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    *x >> 33
}

#[test]
fn ranks_count_the_nodes_with_an_entry_across_blocks() {
    // Five and a half blocks: block 2 has no entry at all, the others random holes.
    let n = 5 * RANK_BLOCK.get() + RANK_BLOCK.get() / 2;
    let mut x = 7;
    let k: Vec<u64> = (0..n)
        .map(|v| {
            let draw = step(&mut x);
            if (120_000..270_000).contains(&v) || draw.is_multiple_of(5) {
                0
            } else {
                1 + draw % 3
            }
        })
        .collect();
    let mut next = 0;
    let expected: Vec<Option<u32>> = k
        .iter()
        .map(|&s| {
            (s > 0).then(|| {
                next += 1;
                next - 1
            })
        })
        .collect();
    let ranks = Ranks::new(&k);
    assert_eq!(ranks.count(), next);
    assert_eq!(ranks.blocks(), n.div_ceil(RANK_BLOCK.get()));
    let mut seen: Vec<Option<u32>> = (0..n).map(|_| None).collect();
    for b in 0..ranks.blocks() {
        ranks.each(b, |v, r| {
            assert!(seen[v as usize].replace(r).is_none(), "node {v} twice");
        });
    }
    assert_eq!(seen, expected);
    let column = ranks
        .column(&Memory::unlimited(), "ranks", |r| r + 1)
        .unwrap();
    let plus_one: Vec<u32> = expected.iter().map(|r| r.map_or(0, |r| r + 1)).collect();
    assert_eq!(column[..], plus_one[..]);
}

/// Sorted lists of a random graph of `n` nodes and about `3n` pairs, loops dropped.
fn random_lists(n: u32, seed: u64) -> (Vec<u64>, Vec<u32>) {
    let mut x = seed;
    let mut lists: Vec<Vec<u32>> = (0..n).map(|_| Vec::new()).collect();
    for _ in 0..3 * n {
        let (u, v) = (step(&mut x) as u32 % n, step(&mut x) as u32 % n);
        if u != v {
            lists[u as usize].push(v);
            lists[v as usize].push(u);
        }
    }
    let (mut offsets, mut targets) = (vec![0], Vec::new());
    for mut l in lists {
        l.sort_unstable();
        l.dedup();
        targets.extend(l);
        offsets.push(targets.len() as u64);
    }
    (offsets, targets)
}

/// Greedy refinement with every node in one community of local moving, against a
/// sequential reference of its sub-rounds: in each, every node still alone picks the
/// refined community of best positive gain against the state at the sub-round's start,
/// and joins it unless that community's node also picked one. Returns the joins and the
/// nodes that stayed alone with a neighbour.
fn refinement_against_a_reference(n: u32, seed: u64, resolution: f64) -> (usize, usize) {
    let (offsets, targets) = random_lists(n, seed);
    let csr = Csr::new(&offsets, &targets).unwrap();
    let memory = Memory::unlimited();
    let options = Options {
        resolution,
        ..Options::default()
    };
    Plan::with(&csr, &memory, Settings::default(), |plan| {
        let k = strengths(&plan, &memory, Weights::Unit, &options).unwrap();
        let ctx = Ctx {
            options: &options,
            total: k.total,
            coefficient: 1.0 / k.total as f64,
            weight_scale: None,
            level: 0,
            iteration: 0,
            stage: 0,
            coarsening: crate::leiden::Coarsening::USUAL,
        };
        let ranks = Ranks::new(&k.of);
        let one: Vec<u32> = (0..n).map(|_| 0).collect();
        let st = Moving::new(&plan, &memory, &k, &ranks, Some(&one), &ctx).unwrap();
        let schedule = Schedule::new(&ranks, &memory, &ctx, true).unwrap();
        let g = Graph::new(&plan, Weights::Unit, &k);
        let tables = Tables::new(&memory);
        let r = refine(g, &memory, &ranks, &st, &schedule, &tables, &ctx).unwrap();
        let got: Vec<u32> = plan
            .all_nodes()
            .map(|v| r.rcomm.load(v, Relaxed).get())
            .collect();

        let Schedule::Rounds { ids, starts } = &schedule else {
            unreachable!("the deterministic mode")
        };
        let list =
            |u: u32| &targets[offsets[u as usize] as usize..offsets[u as usize + 1] as usize];
        let gamma = resolution * (1.0 / k.total as f64);
        let mut rcomm: Vec<u32> = (0..n).collect();
        let mut rsigma: Vec<u64> = k.of.iter().copied().collect();
        for w in starts.windows(2) {
            let picks: Vec<(u32, u32)> = ids[w[0]..w[1]]
                .iter()
                .filter_map(|&u| {
                    let ku = k.of[u as usize];
                    if rcomm[u as usize] != u || rsigma[u as usize] != ku {
                        return None;
                    }
                    let mut sums = std::collections::BTreeMap::new();
                    for &v in list(u) {
                        *sums.entry(rcomm[v as usize]).or_insert(0_u64) += 1;
                    }
                    let f = gamma * ku as f64;
                    let mut best = None::<(f64, u32)>;
                    for (&d, &kud) in &sums {
                        let value = kud as f64 - f * rsigma[d as usize] as f64;
                        let better = best.is_none_or(|(b, bd)| {
                            value > b
                                || (value == b && before(&st.salt, st.salt[u as usize], d, bd))
                        });
                        if value > 0.0 && d != u && better {
                            best = Some((value, d));
                        }
                    }
                    best.map(|(_, d)| (u, d))
                })
                .collect();
            let picked: std::collections::BTreeSet<u32> = picks.iter().map(|p| p.0).collect();
            for (u, d) in picks {
                if !picked.contains(&d) {
                    let ku = k.of[u as usize];
                    rcomm[u as usize] = d;
                    rsigma[d as usize] += ku;
                    rsigma[u as usize] -= ku;
                }
            }
        }
        assert_eq!(
            got, rcomm,
            "{n} nodes, seed {seed}, resolution {resolution}"
        );
        let joins = (0..n).filter(|&v| rcomm[v as usize] != v).count();
        let alone = (0..n)
            .filter(|&v| rcomm[v as usize] == v && !list(v).is_empty())
            .count();
        (joins, alone)
    })
}

#[test]
fn greedy_refinement_follows_its_sequential_reference() {
    for (n, seed, resolution) in [(300, 1, 1.0), (300, 2, 4.0), (2000, 3, 1.0), (2000, 4, 8.0)] {
        let (joins, alone) = refinement_against_a_reference(n, seed, resolution);
        assert!(
            joins > 0 && alone > 0,
            "{n}, {seed}: {joins} joins, {alone} alone"
        );
    }
}

/// Lists and per-entry weights in the units of `A` from weighted pairs `u < v`.
fn weighted_lists(n: u32, pairs: &[(u32, u32, u64)]) -> (Vec<u64>, Vec<u32>, Vec<u64>) {
    let mut lists: Vec<Vec<(u32, u64)>> = (0..n).map(|_| Vec::new()).collect();
    for &(u, v, w) in pairs {
        lists[u as usize].push((v, w));
        lists[v as usize].push((u, w));
    }
    let (mut offsets, mut targets, mut weights) = (vec![0], Vec::new(), Vec::new());
    for mut l in lists {
        l.sort_unstable();
        targets.extend(l.iter().map(|e| e.0));
        weights.extend(l.iter().map(|e| e.1));
        offsets.push(targets.len() as u64);
    }
    (offsets, targets, weights)
}

/// Two triangles joined by a pair of weight 0, in one community: the split follows
/// only entries of positive weight, so it gives the two triangles.
#[test]
fn the_split_does_not_follow_zero_weights() {
    let pairs = [
        (0, 1, 1),
        (0, 2, 1),
        (1, 2, 1),
        (2, 3, 0),
        (3, 4, 1),
        (3, 5, 1),
        (4, 5, 1),
    ];
    let (offsets, targets, weights) = weighted_lists(6, &pairs);
    let csr = Csr::new(&offsets, &targets).unwrap();
    let memory = Memory::unlimited();
    Plan::with(&csr, &memory, Settings::default(), |plan| {
        let one = plan.node_cells(&memory).unwrap();
        plan.vertices((), |(): &mut (), (), _, v, _| {
            one.store(v, plan.node(0).unwrap(), Relaxed);
        })
        .unwrap();
        let parts = split(&plan, &memory, Weights::Entries(&weights), &one).unwrap();
        let got: Vec<u32> = plan
            .all_nodes()
            .map(|v| parts.load(v, Relaxed).get())
            .collect();
        assert_eq!(got, [0, 0, 0, 3, 3, 3]);
    });
}

/// Node 0 is in a community it gains nothing from (at resolution 2 its value there is
/// negative), and its only other neighbour, node 6, is joined by weight 0: a community
/// of mass 0 and value 0 to it. Local moving skips that candidate, so node 0 stays.
#[test]
fn local_moving_skips_a_candidate_with_no_weight() {
    let mut pairs = vec![(0, 1, 1), (0, 6, 0)];
    for a in 1..6 {
        for b in a + 1..6 {
            pairs.push((a, b, 1));
        }
    }
    let (offsets, targets, weights) = weighted_lists(7, &pairs);
    let csr = Csr::new(&offsets, &targets).unwrap();
    let memory = Memory::unlimited();
    let options = Options {
        resolution: 2.0,
        ..Options::default()
    };
    Plan::with(&csr, &memory, Settings::default(), |plan| {
        let w = Weights::Entries(&weights);
        let k = strengths(&plan, &memory, w, &options).unwrap();
        assert_eq!(k.of[6], 0);
        let ctx = Ctx {
            options: &options,
            total: k.total,
            coefficient: 1.0 / k.total as f64,
            weight_scale: None,
            level: 0,
            iteration: 0,
            stage: 0,
            coarsening: crate::leiden::Coarsening::USUAL,
        };
        let ranks = Ranks::new(&k.of);
        let bound = [0, 0, 0, 0, 0, 0, 6];
        let st = Moving::new(&plan, &memory, &k, &ranks, Some(&bound), &ctx).unwrap();
        let schedule = Schedule::new(&ranks, &memory, &ctx, false).unwrap();
        let tables = Tables::new(&memory);
        local_move(Graph::new(&plan, w, &k), &st, &schedule, &tables, &ctx, 0.0).unwrap();
        let zero = plan.node(0).unwrap();
        assert_ne!(st.comm.load(zero, Relaxed).get(), 6);
    });
}
