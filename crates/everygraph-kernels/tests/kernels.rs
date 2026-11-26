// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `PageRank`, eigenvector centrality, BFS, multi-source BFS and SSSP on generated
//! graphs: the probe values of `docs/spec/graph-semantics.md` § 5.1, metamorphic
//! relations (relabelling, disjoint union, one traversal against another), the same bits
//! under any threads, chunks, source and budget, and the refusals. The oracle test
//! (`tests/oracle.rs`) compares the same kernels with the references on the corpus.

#![forbid(unsafe_code)]
// Test graphs and their expected values are small buffers outside the budget.
#![allow(clippy::disallowed_methods)]
// Small generated graphs: casts cannot truncate; expected values are exact.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::float_cmp
)]

use std::cmp::Reverse;
use std::collections::{BTreeSet, BinaryHeap};
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64};

use everygraph_core::{Budget, Memory, MemorySource, par};
use everygraph_format::adjacency::{Adjacency, AdjacencyIndex, encode};
use everygraph_kernels::edge_map::{Compressed, Csr, Error, Plan, Settings, Source, Stream};
use everygraph_kernels::pagerank::{
    self, Eigenvector, Graph, PageRank, Real, eigenvector, strength, top,
};
use everygraph_kernels::sssp::{self, Sssp, sssp};
use everygraph_kernels::traversal::{BATCH, Bfs, UNREACHED, bfs, multi_bfs};

fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Lists in node order with one weight per entry.
#[derive(Clone, Debug, PartialEq)]
struct Lists {
    offsets: Vec<u64>,
    targets: Vec<u32>,
    weight: Vec<f64>,
}

impl Lists {
    /// From `(source, target, weight)` arcs, sorted and without repeats.
    fn new(n: u32, mut arcs: Vec<(u32, u32, f64)>) -> Self {
        arcs.sort_by_key(|a| (a.0, a.1));
        arcs.dedup_by_key(|a| (a.0, a.1));
        let mut offsets = vec![0_u64; n as usize + 1];
        for a in &arcs {
            offsets[a.0 as usize + 1] += 1;
        }
        for v in 0..n as usize {
            offsets[v + 1] += offsets[v];
        }
        Self {
            offsets,
            targets: arcs.iter().map(|a| a.1).collect(),
            weight: arcs.iter().map(|a| a.2).collect(),
        }
    }

    fn csr(&self) -> Csr<'_> {
        Csr::new(&self.offsets, &self.targets).unwrap()
    }
}

/// A graph as the kernels read it: out-lists and in-lists, a kept loop being the entry
/// `v` in the list of `v`. Undirected: both are the symmetric lists.
#[derive(Clone, Debug)]
struct G {
    n: u32,
    directed: bool,
    arcs: Vec<(u32, u32, f64)>,
    out: Lists,
    inn: Lists,
}

impl G {
    /// `arcs` are the pairs: each once for an undirected graph.
    fn new(n: u32, directed: bool, mut arcs: Vec<(u32, u32, f64)>) -> Self {
        if !directed {
            arcs = arcs
                .into_iter()
                .map(|(u, v, w)| (u.min(v), u.max(v), w))
                .collect();
        }
        // Stable: of repeated pairs the first one given is kept, whatever the labels.
        arcs.sort_by_key(|a| (a.0, a.1));
        arcs.dedup_by_key(|a| (a.0, a.1));
        let both: Vec<_> = if directed {
            arcs.clone()
        } else {
            arcs.iter()
                .flat_map(|&(u, v, w)| [(u, v, w), (v, u, w)])
                .collect()
        };
        let rev = both.iter().map(|&(u, v, w)| (v, u, w)).collect();
        Self {
            n,
            directed,
            out: Lists::new(n, both),
            inn: Lists::new(n, rev),
            arcs,
        }
    }

    /// `m` random pairs over `n` nodes, some of them loops, with weights from 0 to 10
    /// over 2^-20 to 2^20 in magnitude.
    fn random(n: u32, m: usize, seed: u64, directed: bool) -> Self {
        let mut s = seed;
        let arcs = (0..m)
            .map(|_| {
                let u = (splitmix(&mut s) % u64::from(n)) as u32;
                let r = splitmix(&mut s);
                // Skewed: low ids get more edges, and a few nodes have none.
                let v = if r.is_multiple_of(11) {
                    u
                } else {
                    ((r >> 8) % u64::from(n / 2 + 1)) as u32
                };
                let w = match splitmix(&mut s) % 9 {
                    0 => 0.0,
                    k => (1 + k) as f64 * 2_f64.powi((splitmix(&mut s) % 41) as i32 - 20),
                };
                (u, v, w)
            })
            .collect();
        Self::new(n, directed, arcs)
    }

    /// The graph with node `v` renamed `perm[v]`.
    fn relabel(&self, perm: &[u32]) -> Self {
        let arcs = self
            .arcs
            .iter()
            .map(|&(u, v, w)| (perm[u as usize], perm[v as usize], w))
            .collect();
        Self::new(self.n, self.directed, arcs)
    }

    /// The disjoint union: `other`'s nodes after this graph's.
    fn union(&self, other: &Self) -> Self {
        let shift = |&(u, v, w): &(u32, u32, f64)| (u + self.n, v + self.n, w);
        let arcs = self
            .arcs
            .iter()
            .copied()
            .chain(other.arcs.iter().map(shift))
            .collect();
        Self::new(self.n + other.n, self.directed, arcs)
    }
}

fn permutation(n: u32, seed: u64) -> Vec<u32> {
    let mut s = seed;
    let mut p: Vec<u32> = (0..n).collect();
    for i in (1..p.len()).rev() {
        p.swap(i, (splitmix(&mut s) % (i as u64 + 1)) as usize);
    }
    p
}

/// What every kernel returns on one graph, bit for bit.
#[derive(Debug, PartialEq)]
struct All {
    pr64: Vec<u64>,
    pr32: Vec<u32>,
    eigen: Vec<u64>,
    depth: Vec<u32>,
    parent: Vec<u32>,
    levels: Vec<u64>,
    sssp: Vec<u64>,
    multi: (Vec<Vec<u64>>, Vec<u64>, Vec<u32>),
}

fn load(cells: Vec<AtomicU32>) -> Vec<u32> {
    cells.into_iter().map(AtomicU32::into_inner).collect()
}

fn run_all<S: Source>(
    out: &S,
    inn: &S,
    g: &G,
    memory: &Memory,
    settings: Settings,
    threads: usize,
) -> All {
    let seeds = [1, 2, 2, g.n - 1];
    Plan::with_pair(out, inn, memory, settings, |po, pi| {
        par::with_threads(NonZeroUsize::new(threads).unwrap(), || {
            let s = strength(&po, Some(&g.out.weight), g.directed, memory).unwrap();
            let graph = Graph {
                lists: &pi,
                weights: Some(&g.inn.weight),
                directed: g.directed,
            };
            let p = PageRank {
                tol: 0.0,
                max_iter: 30,
                personalize: Some(&seeds),
                ..PageRank::default()
            };
            let s = s.as_slice().unwrap();
            let pr64 = pagerank::pagerank::<_, f64>(&graph, s, &p, memory).unwrap();
            let pr32 = pagerank::pagerank::<_, f32>(&graph, s, &p, memory).unwrap();
            let e = Eigenvector {
                tol: 0.0,
                max_iter: 30,
            };
            let eigen = eigenvector(&graph, &e, memory).unwrap();
            let b = Bfs {
                parents: true,
                ..Bfs::default()
            };
            let hops = bfs(&po, Some(&pi), 0, &b, memory).unwrap();
            let d = sssp(&po, &g.out.weight, 0, &Sssp::default(), memory).unwrap();
            let sources: Vec<u32> = (0..70).map(|k| k * 7 % g.n).collect();
            let multi = multi_bfs(&pi, &sources, None, memory).unwrap();
            All {
                pr64: pr64.values.iter().map(|x| x.to_bits()).collect(),
                pr32: pr32.values.iter().map(|x| x.to_bits()).collect(),
                eigen: eigen.values.iter().map(|x| x.to_bits()).collect(),
                depth: load(hops.distance),
                parent: load(hops.parent.unwrap()),
                levels: hops.levels,
                sssp: d.bits.into_iter().map(AtomicU64::into_inner).collect(),
                multi: (
                    multi.levels,
                    multi.distance_sum.iter().copied().collect(),
                    multi.reached.iter().copied().collect(),
                ),
            }
        })
        .unwrap()
    })
    .unwrap()
}

/// f64 and f32 `PageRank`, eigenvector, BFS with parents, SSSP and multi-source BFS give
/// the same bits on every source, at 1, 4 and 8 threads, with chunks of 64 entries and
/// the default, and under a tight budget; directed and undirected, weighted, with loops.
#[test]
fn results_do_not_depend_on_threads_chunks_sources_or_budget() {
    for directed in [false, true] {
        let g = G::random(3000, 12_000, 5, directed);
        let encoded = [
            encode(&g.out.offsets, &g.out.targets).unwrap(),
            encode(&g.inn.offsets, &g.inn.targets).unwrap(),
        ];
        let index: Vec<AdjacencyIndex> = encoded
            .iter()
            .map(|e| AdjacencyIndex::parse(&e.index).unwrap())
            .collect();
        let heap: Vec<Compressed<'_>> = (0..2)
            .map(|k| Compressed::new(Adjacency::new(&index[k], &encoded[k].data).unwrap()))
            .collect();
        let bytes = |l: &Lists| {
            MemorySource::new(
                l.targets
                    .iter()
                    .flat_map(|t| t.to_le_bytes())
                    .collect::<Vec<u8>>(),
            )
        };
        let (out_bytes, in_bytes) = (bytes(&g.out), bytes(&g.inn));
        let stream = [
            Stream::new(&g.out.offsets, &out_bytes).unwrap(),
            Stream::new(&g.inn.offsets, &in_bytes).unwrap(),
        ];
        let unlimited = Memory::unlimited();
        let expected = run_all(
            &g.out.csr(),
            &g.inn.csr(),
            &g,
            &unlimited,
            Settings::default(),
            1,
        );
        assert!(
            expected.levels.len() > 2
                && expected
                    .sssp
                    .iter()
                    .any(|&d| d != 0 && d != f64::INFINITY.to_bits())
        );
        let tight = Memory::from(Arc::new(Budget::new(4 << 20)));
        for threads in [1, 4, 8] {
            for chunk in [64, 1 << 16] {
                let c = Settings {
                    chunk_entries: chunk,
                    batch_entries: 7,
                    prefetch: false,
                };
                let what = format!("directed {directed}, threads {threads}, chunk {chunk}");
                assert_eq!(
                    run_all(&g.out.csr(), &g.inn.csr(), &g, &tight, c, threads),
                    expected,
                    "csr, {what}"
                );
                assert_eq!(
                    run_all(&heap[0], &heap[1], &g, &unlimited, c, threads),
                    expected,
                    "compressed, {what}"
                );
                assert_eq!(
                    run_all(&stream[0], &stream[1], &g, &unlimited, c, threads),
                    expected,
                    "stream, {what}"
                );
            }
        }
    }
}

/// `PageRank` of a graph's lists, to convergence, as `f64`s.
fn ranks(g: &G, weighted: bool, personalize: Option<&[u32]>) -> Vec<f64> {
    let memory = Memory::unlimited();
    let w = |l: &Lists| weighted.then_some(&l.weight[..]).map(<[f64]>::to_vec);
    let (w_out, w_in) = (w(&g.out), w(&g.inn));
    Plan::with_pair(
        &g.out.csr(),
        &g.inn.csr(),
        &memory,
        Settings::default(),
        |po, pi| {
            let s = strength(&po, w_out.as_deref(), g.directed, &memory).unwrap();
            let graph = Graph {
                lists: &pi,
                weights: w_in.as_deref(),
                directed: g.directed,
            };
            let p = PageRank {
                tol: 1e-15,
                max_iter: 2000,
                personalize,
                ..PageRank::default()
            };
            let r =
                pagerank::pagerank::<_, f64>(&graph, s.as_slice().unwrap(), &p, &memory).unwrap();
            assert!(r.change < 1e-15, "converged: {r:?}");
            r.values.iter().copied().collect()
        },
    )
    .unwrap()
}

/// Equal within `tol` at every node: ranks sum to 1, so an absolute bound.
fn close(a: &[f64], b: &[f64], tol: f64) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol)
}

/// The probe of `docs/spec/graph-semantics.md` § 5.6: the triangle 0-1-2 with a loop at
/// 0, undirected (`A[0][0] = 2w`) and with the loop's weight 3; and the directed 3-cycle
/// with a loop at 0.
#[test]
fn pagerank_probe_values_with_a_kept_loop() {
    let tri = |directed, loop_w| {
        G::new(
            3,
            directed,
            vec![(0, 0, loop_w), (0, 1, 1.0), (1, 2, 1.0), (2, 0, 1.0)],
        )
    };
    let got = ranks(&tri(false, 1.0), false, None);
    assert!(close(&got, &[0.475, 0.2625, 0.2625], 1e-12), "{got:?}");
    let got = ranks(&tri(false, 3.0), true, None);
    assert!(
        close(&got, &[38.0 / 63.0, 25.0 / 126.0, 25.0 / 126.0], 1e-12),
        "{got:?}"
    );
    let got = ranks(&tri(true, 1.0), false, None);
    assert!(
        close(&got, &[0.480_056, 0.254_024, 0.265_920], 1e-5),
        "{got:?}"
    );
}

/// Renaming the nodes renames the results: exactly for distances, which do not depend
/// on summation order, and within rounding for the power iterations.
#[test]
fn results_follow_a_relabelling() {
    for directed in [false, true] {
        let g = G::random(500, 2500, 9, directed);
        let perm = permutation(g.n, 3);
        let h = g.relabel(&perm);
        let renamed = |x: &[f64]| {
            (0..g.n as usize)
                .map(|v| x[perm[v] as usize])
                .collect::<Vec<_>>()
        };
        let seeds_h: Vec<u32> = [3, 4].iter().map(|&s| perm[s as usize]).collect();
        for weighted in [false, true] {
            let a = ranks(&g, weighted, Some(&[3, 4]));
            let b = renamed(&ranks(&h, weighted, Some(&seeds_h)));
            assert!(close(&a, &b, 1e-12), "pagerank, directed {directed}");
        }
        let (a, b) = (eigen(&g), renamed(&eigen(&h)));
        assert!(close(&a, &b, 1e-9), "eigenvector, directed {directed}");
        let s = 5;
        let (ta, tb) = (traversals(&g, s), traversals(&h, perm[s as usize]));
        for v in 0..g.n as usize {
            let w = perm[v] as usize;
            assert_eq!(ta.depth[v], tb.depth[w], "bfs, node {v}");
            assert_eq!(ta.sssp[v].to_bits(), tb.sssp[w].to_bits(), "sssp, node {v}");
        }
    }
}

/// On a disjoint union, `PageRank` seeded inside one part is that part's own `PageRank`
/// and 0 on the other (the dangling mass follows the seeds), and a traversal stays in
/// its part.
#[test]
fn results_on_a_disjoint_union() {
    for directed in [false, true] {
        let (a, b) = (
            G::random(300, 1200, 1, directed),
            G::random(200, 900, 2, directed),
        );
        let u = a.union(&b);
        for weighted in [false, true] {
            let alone = ranks(&a, weighted, Some(&[0, 7]));
            let joint = ranks(&u, weighted, Some(&[0, 7]));
            assert!(close(&joint[..300], &alone, 1e-12), "directed {directed}");
            // The other part's start decays by α per iteration, to the stopping rule.
            assert!(joint[300..].iter().all(|&x| x < 1e-12));
        }
        let (ta, tu) = (traversals(&a, 4), traversals(&u, 4));
        assert_eq!(tu.depth[..300], ta.depth[..]);
        assert!(tu.depth[300..].iter().all(|&d| d == UNREACHED));
        assert_eq!(
            tu.sssp[..300]
                .iter()
                .map(|d| d.to_bits())
                .collect::<Vec<_>>(),
            ta.sssp.iter().map(|d| d.to_bits()).collect::<Vec<_>>()
        );
    }
}

fn eigen(g: &G) -> Vec<f64> {
    let memory = Memory::unlimited();
    Plan::with(&g.inn.csr(), &memory, Settings::default(), |pi| {
        let graph = Graph {
            lists: &pi,
            weights: Some(&g.inn.weight),
            directed: g.directed,
        };
        let p = Eigenvector {
            tol: 1e-15,
            max_iter: 20_000,
        };
        let r = eigenvector(&graph, &p, &memory).unwrap();
        r.values.iter().copied().collect()
    })
}

struct Traversals {
    depth: Vec<u32>,
    sssp: Vec<f64>,
}

fn traversals(g: &G, source: u32) -> Traversals {
    let memory = Memory::unlimited();
    Plan::with_pair(
        &g.out.csr(),
        &g.inn.csr(),
        &memory,
        Settings::default(),
        |po, pi| {
            let hops = bfs(&po, Some(&pi), source, &Bfs::default(), &memory).unwrap();
            let d = sssp(&po, &g.out.weight, source, &Sssp::default(), &memory).unwrap();
            Traversals {
                depth: load(hops.distance),
                sssp: (0..g.n)
                    .map(|v| d.get(v).unwrap_or(f64::INFINITY))
                    .collect(),
            }
        },
    )
    .unwrap()
}

/// Dijkstra over the out-lists, adding costs from the source as the kernel does.
fn dijkstra(l: &Lists, n: u32, source: u32) -> Vec<f64> {
    let mut dist = vec![f64::INFINITY; n as usize];
    dist[source as usize] = 0.0;
    let mut heap = BinaryHeap::from([Reverse((0_u64, source))]);
    while let Some(Reverse((bits, u))) = heap.pop() {
        let d = f64::from_bits(bits);
        if d > dist[u as usize] {
            continue;
        }
        let r = l.offsets[u as usize] as usize..l.offsets[u as usize + 1] as usize;
        for (&v, &c) in l.targets[r.clone()].iter().zip(&l.weight[r]) {
            let nd = d + c;
            if nd < dist[v as usize] {
                dist[v as usize] = nd;
                heap.push(Reverse((nd.to_bits(), v)));
            }
        }
    }
    dist
}

/// SSSP equals Dijkstra bit for bit, whatever the bucket width, with and without bucket
/// fusion's reach (a width far below every cost leaves nothing to fuse).
#[test]
fn sssp_equals_dijkstra_for_every_bucket_width() {
    for directed in [false, true] {
        let g = G::random(2000, 9000, 4, directed);
        let want = dijkstra(&g.out, g.n, 0);
        let memory = Memory::unlimited();
        Plan::with(&g.out.csr(), &memory, Settings::default(), |plan| {
            let rule = sssp::delta(&plan, &g.out.weight);
            assert!(rule > 0.0 && rule.is_finite());
            for width in [None, Some(1e-12), Some(rule / 8.0), Some(1e12)] {
                let p = Sssp { delta: width };
                let d = sssp(&plan, &g.out.weight, 0, &p, &memory).unwrap();
                let got: Vec<f64> = (0..g.n)
                    .map(|v| d.get(v).unwrap_or(f64::INFINITY))
                    .collect();
                let bits = |x: &[f64]| x.iter().map(|d| d.to_bits()).collect::<Vec<_>>();
                assert_eq!(
                    bits(&got),
                    bits(&want),
                    "directed {directed}, width {width:?}"
                );
            }
        });
    }
}

/// Bit-parallel BFS from 70 sources (two batches, one source twice) equals 70 BFS runs:
/// the level sizes of each source, and each node's summed distance and sources.
#[test]
fn multi_source_bfs_equals_one_bfs_per_source() {
    for directed in [false, true] {
        let g = G::random(1500, 5000, 8, directed);
        let mut sources: Vec<u32> = (0..70).map(|k| k * 13 % g.n).collect();
        sources[69] = sources[3];
        assert!(sources.len() > BATCH);
        let memory = Memory::unlimited();
        Plan::with_pair(
            &g.out.csr(),
            &g.inn.csr(),
            &memory,
            Settings::default(),
            |po, pi| {
                let multi = multi_bfs(&pi, &sources, None, &memory).unwrap();
                let mut sum = vec![0_u64; g.n as usize];
                let mut reached = vec![0_u32; g.n as usize];
                for (k, &s) in sources.iter().enumerate() {
                    let hops = bfs(&po, Some(&pi), s, &Bfs::default(), &memory).unwrap();
                    for (d, &count) in hops.levels.iter().enumerate() {
                        assert_eq!(multi.levels[d][k], count, "source {k}, level {d}");
                    }
                    assert!(multi.levels[hops.levels.len()..].iter().all(|l| l[k] == 0));
                    for (v, d) in load(hops.distance).into_iter().enumerate() {
                        if d != UNREACHED {
                            sum[v] += u64::from(d);
                            reached[v] += 1;
                        }
                    }
                }
                assert_eq!(multi.distance_sum.iter().copied().collect::<Vec<_>>(), sum);
                assert_eq!(multi.reached.iter().copied().collect::<Vec<_>>(), reached);
                let two = multi_bfs(&pi, &sources, Some(2), &memory).unwrap();
                assert_eq!(two.levels[..], multi.levels[..3]);
            },
        )
        .unwrap();
    }
}

/// BFS without in-lists (top-down only) equals direction-optimizing BFS; `max_hops`
/// cuts the distances and the levels; parents follow the rule of `docs/api-sketch.md`
/// § 2.12.
#[test]
fn bfs_variants_agree() {
    for directed in [false, true] {
        let g = G::random(4000, 30_000, 6, directed);
        let memory = Memory::unlimited();
        Plan::with_pair(
            &g.out.csr(),
            &g.inn.csr(),
            &memory,
            Settings::default(),
            |po, pi| {
                let p = Bfs {
                    parents: true,
                    ..Bfs::default()
                };
                let full = bfs(&po, Some(&pi), 0, &p, &memory).unwrap();
                let push = bfs(&po, None, 0, &p, &memory).unwrap();
                let (depth, parent) = (load(full.distance), load(full.parent.unwrap()));
                assert_eq!(load(push.distance), depth);
                assert_eq!(load(push.parent.unwrap()), parent);
                assert_eq!(push.levels, full.levels);
                assert_eq!(
                    full.levels.iter().sum::<u64>(),
                    depth.iter().filter(|&&d| d != UNREACHED).count() as u64
                );
                for v in 0..g.n as usize {
                    let r = g.inn.offsets[v] as usize..g.inn.offsets[v + 1] as usize;
                    let want = match depth[v] {
                        0 | UNREACHED => UNREACHED,
                        d => g.inn.targets[r]
                            .iter()
                            .copied()
                            .filter(|&u| depth[u as usize] == d - 1)
                            .min()
                            .unwrap(),
                    };
                    assert_eq!(parent[v], want, "node {v}");
                }
                let cut = Bfs {
                    max_hops: Some(2),
                    parents: false,
                };
                let near = bfs(&po, Some(&pi), 0, &cut, &memory).unwrap();
                let want: Vec<u32> = depth
                    .iter()
                    .map(|&d| if d <= 2 { d } else { UNREACHED })
                    .collect();
                assert_eq!(load(near.distance), want);
                assert_eq!(near.levels[..], full.levels[..3]);
            },
        )
        .unwrap();
    }
}

/// Bad weights and arguments are refused before any work.
#[test]
fn refusals() {
    let g = G::random(50, 200, 3, true);
    let memory = Memory::unlimited();
    let csr = g.out.csr();
    Plan::with(&csr, &memory, Settings::default(), |plan| {
        for bad in [-1.0, f64::NAN, f64::INFINITY] {
            let mut w = g.out.weight.clone();
            w[17] = bad;
            let e = strength(&plan, Some(&w), true, &memory).unwrap_err();
            assert!(matches!(e, Error::Weight { entry: 17, .. }), "{e}");
            let graph = Graph {
                lists: &plan,
                weights: Some(&w),
                directed: true,
            };
            let s = vec![1.0; 50];
            let e = pagerank::pagerank::<_, f64>(&graph, &s, &PageRank::default(), &memory)
                .unwrap_err();
            assert!(matches!(e, Error::Weight { entry: 17, .. }), "{e}");
            let e = sssp(&plan, &w, 0, &Sssp::default(), &memory).unwrap_err();
            assert!(matches!(e, Error::Weight { entry: 17, .. }), "{e}");
        }
        let graph = Graph {
            lists: &plan,
            weights: None,
            directed: true,
        };
        let s = vec![1.0; 50];
        for seeds in [&[][..], &[50]] {
            let p = PageRank {
                personalize: Some(seeds),
                ..PageRank::default()
            };
            let e = pagerank::pagerank::<_, f64>(&graph, &s, &p, &memory).unwrap_err();
            assert!(matches!(e, Error::Argument(_)), "{e}");
        }
        assert!(matches!(
            pagerank::pagerank::<_, f64>(&graph, &s[..49], &PageRank::default(), &memory),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            bfs(&plan, None, 50, &Bfs::default(), &memory),
            Err(Error::Argument(_))
        ));
        assert!(matches!(
            multi_bfs(&plan, &[0, 50], None, &memory),
            Err(Error::Argument(_))
        ));
        let short = &g.out.weight[1..];
        assert!(matches!(
            sssp(&plan, short, 0, &Sssp::default(), &memory),
            Err(Error::Invalid(_))
        ));
    });
}

/// A graph without edges: every node `1/n`, after one iteration as after many.
#[test]
fn pagerank_without_edges() {
    let g = G::new(4, true, Vec::new());
    assert_eq!(ranks(&g, false, None), vec![0.25; 4]);
    let seeded = ranks(&g, false, Some(&[2]));
    assert_eq!(seeded, vec![0.0, 0.0, 1.0, 0.0]);
}

/// `top` returns the largest values first, ties in node order.
#[test]
fn top_breaks_ties_in_node_order() {
    let memory = Memory::unlimited();
    let x = [0.5_f64, 2.0, 0.5, 3.0, 2.0];
    let t = top(&x, 4, &memory).unwrap();
    let got: Vec<(u32, f64)> = t.iter().copied().collect();
    assert_eq!(got, vec![(3, 3.0), (1, 2.0), (4, 2.0), (0, 0.5)]);
    assert_eq!(top(&x, 10, &memory).unwrap().len(), 5);
    assert!(top::<f32>(&[], 3, &memory).unwrap().is_empty());
    let f: Vec<f32> = x.iter().map(|&v| f32::from_f64(v)).collect();
    assert_eq!(
        top(&f, 1, &memory).unwrap().iter().next(),
        Some(&(3, 3.0_f32))
    );
    let distinct: BTreeSet<u32> = top(&x, 5, &memory).unwrap().iter().map(|p| p.0).collect();
    assert_eq!(distinct.len(), 5);
}
