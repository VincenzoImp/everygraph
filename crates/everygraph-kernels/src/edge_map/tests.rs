// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The drivers over every source built in memory, for Miri; and the parts of
//! [`Plan::push_with`].

use everygraph_core::MemorySource;
use everygraph_format::adjacency::{Adjacency, AdjacencyIndex, encode};

use super::*;

#[test]
fn blocks_follow_the_node_count_only() {
    assert_eq!(block_nodes(0), MIN_BLOCK);
    assert_eq!(block_nodes(1000), MIN_BLOCK);
    assert_eq!(block_nodes(64 * 4096 + 1), 128);
    assert_eq!(block_nodes(3_072_441), 1024);
    assert_eq!(block_nodes(u32::MAX), MAX_BLOCK);
}

/// Tasks are whole blocks, in order, covering every node, whatever the chunk size.
#[test]
fn tasks_are_whole_blocks_covering_the_graph() {
    let n: u32 = 1000;
    let offsets: Vec<u64> = (0..=u64::from(n))
        .scan(0, |sum, v| {
            let at = *sum;
            *sum += v % 7;
            Some(at)
        })
        .collect();
    let targets: Vec<u32> = (0..offsets[ix(n.into())])
        .map(|e| u32::try_from(e % 1000).unwrap())
        .collect();
    let csr = Csr::new(&offsets, &targets).unwrap();
    for chunk in [1, 64, 1000, 1 << 16, u64::MAX] {
        let settings = Settings {
            chunk_entries: chunk,
            ..Settings::default()
        };
        Plan::with(&csr, &Memory::unlimited(), settings, |plan| {
            let mut at = 0;
            for t in &plan.tasks {
                assert_eq!(t.start, at);
                assert!(t.end > t.start && (t.end % plan.block == 0 || t.end == n));
                at = t.end;
            }
            assert_eq!(at, n, "chunk {chunk}");
        });
    }
}

/// A small graph: node `v` lists `v + 1 .. v + 1 + v % 4` (mod `n`), and node `bad`
/// lists `n + 5` if `bad` is given.
fn lists(n: u32, bad: Option<u32>) -> (Vec<u64>, Vec<u32>) {
    let (mut offsets, mut targets) = (vec![0], Vec::new());
    for v in 0..n {
        if bad == Some(v) {
            targets.push(n + 5);
        } else {
            let mut list: Vec<u32> = (1..=v % 4).map(|k| (v + k) % n).collect();
            list.sort_unstable();
            list.dedup();
            targets.extend(list);
        }
        offsets.push(targets.len() as u64);
    }
    (offsets, targets)
}

/// Every driver, over every source built in memory: each target indexes a
/// `PerNode`, so Miri sees every unchecked read the drivers allow.
/// [`lists`] with node 7 listing every node: longer than a run and than a prefix.
fn with_hub(n: u32) -> (Vec<u64>, Vec<u32>) {
    let (offsets, targets) = lists(n, None);
    let (mut o, mut t) = (vec![0], Vec::new());
    for v in 0..ix(n.into()) {
        if v == 7 {
            t.extend(0..n);
        } else {
            t.extend_from_slice(&targets[ix(offsets[v])..ix(offsets[v + 1])]);
        }
        o.push(t.len() as u64);
    }
    (o, t)
}

fn drive<S: Source>(source: &S, offsets: &[u64]) -> Result<[u64; 5], Error> {
    let memory = Memory::unlimited();
    let settings = Settings {
        chunk_entries: 16,
        batch_entries: 5,
        prefetch: true,
    };
    Plan::with(source, &memory, settings, |plan| {
        let values: Vec<u64> = (0..u64::from(plan.nodes())).collect();
        let per = plan.per_node(&values)?;
        let mut out: Vec<u64> = values.iter().map(|_| 0).collect();
        let scanned: u64 = plan.scan(&mut out[..], all, |acc: &mut u64, out, i, row, w| {
            out[i] = row.list.iter().map(|&u| per[u]).sum();
            *acc += out[i];
            if let Some(&u) = row.list.first() {
                let side = w.lists.get(u).expect("a good list");
                *acc += side.list.iter().map(|&t| per[t]).sum::<u64>();
            }
            assert_eq!(row.first_entry, offsets[row.node.index()]);
        })?;
        let frontier: Vec<Node<'_>> = plan.all_nodes().filter(|v| v.get() % 3 != 0).collect();
        let pushed: u64 = plan.frontier(&frontier, |acc: &mut u64, row, _| {
            *acc += row.list.iter().map(|&u| per[u]).sum::<u64>();
        })?;
        let degrees: u64 = plan.vertices((), |acc: &mut u64, (), _, v, d| {
            *acc += per[v] * u64::from(d);
        })?;
        let parts: u64 = plan.push_with(
            &frontier,
            || (),
            |acc: &mut u64, row, w| {
                *acc += row.list.iter().map(|&u| per[u]).sum::<u64>() + row.first_entry;
                if let Some(&u) = row.list.last() {
                    *acc += w.lists.get(u).expect("a good list").list.len() as u64;
                }
            },
        )?;
        let until: u64 = plan.scan_until((), all, |acc: &mut u64, (), _, row| {
            *acc += row.list.iter().map(|&u| per[u]).sum::<u64>() + row.first_entry;
            if row.node.get() % 2 == 1 {
                ControlFlow::Continue(())
            } else {
                ControlFlow::Break(())
            }
        })?;
        Ok([scanned, pushed, degrees, parts, until])
    })
}

#[test]
fn drivers_over_every_source_in_memory() {
    let n = 1100;
    let (offsets, targets) = with_hub(n);
    let encoded = encode(&offsets, &targets).unwrap();
    let index = AdjacencyIndex::parse(&encoded.index).unwrap();
    let bytes: Vec<u8> = targets.iter().flat_map(|t| t.to_le_bytes()).collect();
    let stream_bytes = MemorySource::new(bytes);
    let data = MemorySource::new(encoded.data.clone());
    let csr = drive(&Csr::new(&offsets, &targets).unwrap(), &offsets).unwrap();
    let heap = Adjacency::new(&index, &encoded.data).unwrap();
    assert_eq!(drive(&Compressed::new(heap), &offsets).unwrap(), csr);
    let mapped = Compressed::mapped(&index, &data).unwrap();
    assert_eq!(drive(&mapped, &offsets).unwrap(), csr);
    let stream = Stream::new(&offsets, &stream_bytes).unwrap();
    assert_eq!(drive(&stream, &offsets).unwrap(), csr);
    // In one part, and in parts of at most 1,200 entries, a little more than the hub's
    // list: lists are never cut.
    for part in [u64::MAX >> 8, 1200] {
        let raw = RawCopy::decode_in_parts(&heap, &Memory::unlimited(), part).unwrap();
        assert_eq!(drive(&raw, &offsets).unwrap(), csr, "parts of {part}");
    }
}

/// One corrupt list: the scan and the frontier that reach it fail and hand out no
/// list of it; a scan that skips its block, and a frontier without it, then succeed
/// with the same reader.
#[test]
fn a_corrupt_list_is_refused_and_the_reader_stays_usable() {
    let (n, bad) = (70, 40);
    let (offsets, targets) = lists(n, Some(bad));
    let encoded = encode(&offsets, &targets).unwrap();
    let index = AdjacencyIndex::parse(&encoded.index).unwrap();
    let bytes: Vec<u8> = targets.iter().flat_map(|t| t.to_le_bytes()).collect();
    let stream_bytes = MemorySource::new(bytes);
    let heap = Compressed::new(Adjacency::new(&index, &encoded.data).unwrap());
    let stream = Stream::new(&offsets, &stream_bytes).unwrap();
    check_corrupt(&heap, bad);
    check_corrupt(&stream, bad);
}

fn check_corrupt<S: Source>(source: &S, bad: u32) {
    let memory = Memory::unlimited();
    Plan::with(source, &memory, Settings::default(), |plan| {
        let values: Vec<u8> = (0..plan.nodes()).map(|_| 0).collect();
        let per = plan.per_node(&values).unwrap();
        fn sum<'g>(per: PerNode<'_, 'g, u8>, acc: &mut u64, row: Row<'_, 'g>) {
            *acc += row.list.iter().map(|&u| u64::from(per[u])).sum::<u64>();
        }
        let good: Vec<Node<'_>> = (0..10).filter_map(|v| plan.node(v)).collect();
        let with_bad: Vec<Node<'_>> = (bad - 2..bad + 2).filter_map(|v| plan.node(v)).collect();
        for _ in 0..2 {
            assert!(
                plan.frontier(&good, |a: &mut u64, r, _| sum(per, a, r))
                    .is_ok()
            );
            let e = plan
                .frontier(&with_bad, |a: &mut u64, r, _| sum(per, a, r))
                .unwrap_err();
            assert!(
                matches!(e, Error::Target { node, .. } if node == bad),
                "{e}"
            );
            let e = plan
                .scan((), all, |a: &mut u64, (), _, r, _| sum(per, a, r))
                .unwrap_err();
            assert!(
                matches!(e, Error::Target { node, .. } if node == bad),
                "{e}"
            );
            let far = |v: Node<'_>| v.get().abs_diff(bad) > 64;
            assert!(
                plan.scan((), far, |a: &mut u64, (), _, r, _| sum(per, a, r))
                    .is_ok()
            );
        }
    });
}

/// `n` nodes: each node of `hubs` lists `degree` nodes spread over the graph, every
/// other node `v` lists the `v % 5` nodes after it.
fn with_hubs(n: u32, hubs: &[u32], degree: u32) -> (Vec<u64>, Vec<u32>) {
    let (mut offsets, mut targets) = (vec![0], Vec::new());
    for v in 0..n {
        let mut list: Vec<u32> = if hubs.contains(&v) {
            (0..degree)
                .map(|k| (v + 1 + k * (n / degree)) % n)
                .collect()
        } else {
            (1..=v % 5).map(|k| (v + k) % n).collect()
        };
        list.sort_unstable();
        list.dedup();
        targets.extend(list);
        offsets.push(targets.len() as u64);
    }
    (offsets, targets)
}

/// The value of a part: a float whose magnitude changes with it, so that any change of
/// grouping shows in the bits of a sum.
fn value(first: u64, part: &[u32]) -> f64 {
    let x = first ^ part.iter().map(|&t| u64::from(t)).sum::<u64>();
    [1e-9, 1.0, 1e9][(x % 3) as usize] * (1.0 + (x % 7) as f64)
}

/// The sum of `push_with` over `frontier` on `threads` threads.
fn pushed<S: Source>(source: &S, frontier: &[u32], threads: usize) -> f64 {
    par::with_threads(NonZeroUsize::new(threads).unwrap(), || {
        Plan::with(source, &Memory::unlimited(), Settings::default(), |plan| {
            let f: Vec<Node<'_>> = frontier.iter().filter_map(|&v| plan.node(v)).collect();
            plan.push_with(
                &f,
                || (),
                |acc: &mut f64, row, _| {
                    *acc += value(row.first_entry, ids(row.list));
                },
            )
            .unwrap()
        })
    })
    .unwrap()
}

/// The parts of `push_with` add as documented, whatever the threads and the source:
/// in each group the runs in order, a long list's parts in order.
#[test]
#[cfg_attr(
    miri,
    ignore = "the drivers' unsafe code is reached by the tests above"
)]
fn push_with_adds_parts_in_order() {
    let n = 6000;
    let (offsets, targets) = with_hubs(n, &[7, 300, 301, 5000], 2500);
    let encoded = encode(&offsets, &targets).unwrap();
    let index = AdjacencyIndex::parse(&encoded.index).unwrap();
    let list = |v: usize| &targets[ix(offsets[v])..ix(offsets[v + 1])];
    let frontier: Vec<u32> = (0..n).filter(|v| v % 3 != 1).collect();
    let mut want = 0.0;
    for group in frontier.chunks(FRONTIER_GRAIN) {
        let (mut sum, mut run, mut entries) = (0.0, 0.0, 0);
        for &v in group {
            let v = ix(v.into());
            let cost = list(v).len() + 1;
            if entries > 0 && entries + cost > RUN_ENTRIES {
                (sum, run, entries) = (sum + run, 0.0, 0);
            }
            entries += cost;
            if list(v).len() > RUN_ENTRIES {
                for (k, part) in list(v).chunks(RUN_ENTRIES).enumerate() {
                    run += value(offsets[v] + (k * RUN_ENTRIES) as u64, part);
                }
            } else {
                run += value(offsets[v], list(v));
            }
        }
        want += sum + run;
    }
    let csr = Csr::new(&offsets, &targets).unwrap();
    let heap = Compressed::new(Adjacency::new(&index, &encoded.data).unwrap());
    for threads in [1, 3, 8] {
        assert_eq!(
            pushed(&csr, &frontier, threads).to_bits(),
            want.to_bits(),
            "csr, {threads}"
        );
        assert_eq!(
            pushed(&heap, &frontier, threads).to_bits(),
            want.to_bits(),
            "compressed, {threads}"
        );
    }
}

/// `push_with` holds no more reader slots than there are workers: its peak stays within
/// a slot (two readers, each with its usual batch and a buffer of the longest list) per
/// thread, the copies of one wave of long lists, and the tasks' results.
#[test]
#[cfg_attr(
    miri,
    ignore = "the drivers' unsafe code is reached by the tests above"
)]
fn push_with_holds_a_reader_slot_per_worker() {
    use everygraph_core::Budget;
    use everygraph_format::adjacency::Batch;
    use std::sync::Arc;
    let (n, degree) = (40_000_u32, 20_000);
    let hubs: Vec<u32> = (0..64).map(|i| i * 600 + 3).collect();
    let (offsets, targets) = with_hubs(n, &hubs, degree);
    let encoded = encode(&offsets, &targets).unwrap();
    let index = AdjacencyIndex::parse(&encoded.index).unwrap();
    let heap = Compressed::new(Adjacency::new(&index, &encoded.data).unwrap());
    let frontier: Vec<u32> = (0..n).filter(|v| v % 2 == 1 || hubs.contains(v)).collect();
    for threads in [1, 8] {
        let budget = Arc::new(Budget::unlimited());
        let memory = Memory::from(budget.clone());
        let peak = par::with_threads(NonZeroUsize::new(threads).unwrap(), || {
            Plan::with(&heap, &memory, Settings::default(), |plan| {
                let f: Vec<Node<'_>> = frontier.iter().filter_map(|&v| plan.node(v)).collect();
                let hub = plan.node(hubs[5]).unwrap();
                plan.push_with(
                    &f,
                    || (),
                    |acc: &mut u64, row, w| {
                        *acc += w.lists.get(hub).unwrap().list.len() as u64 + row.list.len() as u64;
                    },
                )
                .unwrap();
                budget.peak()
            })
        })
        .unwrap();
        let lists = FRONTIER_GRAIN.max(ix(block_nodes(n).into()));
        let longest = ix(degree.into()) + 3;
        let reader = Batch::bytes_for(8192 + 3, lists) + Batch::bytes_for(longest, lists);
        let copies = longest.max(WAVE) * size_of::<Node<'_>>();
        // At most a run per node of the frontier, and a part per 1,024 entries of a hub.
        let runs = frontier.len();
        let parts = hubs.len() * ix(degree.into()).div_ceil(RUN_ENTRIES);
        let results = runs * (size_of::<Option<(u64, Work)>>() + size_of::<Range<usize>>())
            + frontier.len().div_ceil(FRONTIER_GRAIN) * size_of::<Vec<Range<usize>>>()
            + parts * size_of::<Option<(u64, Work)>>();
        let bound = (threads * 2 * reader + copies + results) as u64;
        assert!(
            peak <= bound,
            "{threads} threads: peak {peak}, bound {bound}"
        );
    }
}
