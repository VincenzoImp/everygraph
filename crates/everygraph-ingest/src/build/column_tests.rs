// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The columns the build stores (`docs/spec/eg-columns.md` § 6) against an oracle that
//! groups the input rows by pair itself: on every graph of the CI corpus, under every
//! option, in memory and with spills. And the same bytes at 1, 4 and 16 threads.

use std::collections::BTreeMap;
use std::sync::Arc;

use everygraph_core::{Budget, ByteSource, Memory, MemorySource, par};
use everygraph_format::columns::{Column, ColumnLayout, ColumnStats, SectionNames};
use everygraph_format::container::{Container, Provenance};
use everygraph_format::verify::{Depth, verify};
use everygraph_oracle::corpus::{Tier, specs};
use everygraph_oracle::generators::generate;
use everygraph_oracle::rng::SplitMix64;

use super::tests::{SMALL, SPILLING, all_options, read_back};
use super::*;
use crate::ids::MemoryScratch;
use crate::text::{Delimiter, Header, TextOptions};

/// One input row: its ends, its weight and its time in seconds.
#[derive(Clone, Copy, Debug)]
struct Row {
    u: u32,
    v: u32,
    w: f64,
    t: i64,
}

/// The rows of a corpus graph with weights that are multiples of 1/16 (so that any
/// order of summation is exact), some negative and some −0.0, and times from a small
/// range (so that many events tie on time and are ordered by input position).
fn rows_of(src: &[u32], dst: &[u32], seed: u64) -> Vec<Row> {
    let mut rng = SplitMix64::new(seed);
    src.iter()
        .zip(dst)
        .map(|(&u, &v)| {
            let w = match rng.below(10) {
                0 => -0.0,
                1 => -f64::from(rng.below_u32(64)) / 16.0,
                _ => f64::from(rng.below_u32(64)) / 16.0,
            };
            let t = i64::try_from(rng.below(6)).unwrap();
            Row { u, v, w, t }
        })
        .collect()
}

fn text(rows: &[Row]) -> Vec<u8> {
    let mut out = String::new();
    for r in rows {
        out += &format!("{}\t{}\t{:?}\t{}\n", r.u, r.v, r.w, r.t);
    }
    out.into_bytes()
}

/// Which columns are read, and how the build runs.
#[derive(Clone, Copy)]
struct Case<'t> {
    graph: GraphOptions,
    weight: bool,
    time: bool,
    budget: u64,
    tuning: &'t Tuning,
}

fn build_bytes(case: &Case<'_>, text: &[u8]) -> Result<(Vec<u8>, Report), BuildError> {
    let options = TextOptions {
        delimiter: Delimiter::Byte(b'\t'),
        header: Header::Absent,
        weight: case.weight.then(|| 2.into()),
        time: case.time.then(|| 3.into()),
        ..TextOptions::default()
    };
    let reader = TextReader::open_with_chunk(MemorySource::new(text), &options, 96).unwrap();
    let memory = Memory::from(Arc::new(Budget::new(case.budget)));
    let options = BuildOptions::new(case.graph, NonZeroUsize::new(4).unwrap());
    let sink = MemorySource::new(Vec::new());
    let unfinished = build_tuned(
        &reader,
        &options,
        &memory,
        &MemoryScratch,
        sink,
        &Silent,
        case.tuning,
    )?;
    let (sink, report) = unfinished.commit(Provenance::default())?;
    Ok((sink.into_inner(), report))
}

/// A column read back: its values as `u64` bits, and its statistics.
fn column(
    file: &Container<MemorySource<&[u8]>>,
    scope: &str,
    name: &str,
) -> (Vec<u64>, ColumnStats) {
    let names = SectionNames::new(scope, name);
    let bytes = |n: &str| file.section(n).unwrap().as_slice().unwrap().to_vec();
    let layout = ColumnLayout::parse(&bytes(&names.layout)).unwrap();
    let stats = ColumnStats::parse(&bytes(&names.stats), &layout).unwrap();
    let column = Column::open(&layout, file.section(&names.column).unwrap()).unwrap();
    let mut values = Vec::new();
    let mut left = layout.rows();
    for batch in layout.batches() {
        let rows = left.min(layout.batch_rows());
        left -= rows;
        let mut raw = vec![0; usize::try_from(rows * 8).unwrap()];
        column
            .source()
            .read_at(batch.values.start, &mut raw)
            .unwrap();
        values.extend(
            raw.chunks_exact(8)
                .map(|b| u64::from_le_bytes(b.try_into().unwrap())),
        );
    }
    (values, stats)
}

fn f(bits: u64) -> f64 {
    f64::from_bits(bits)
}

fn t(bits: u64) -> i64 {
    i64::from_le_bytes(bits.to_le_bytes())
}

/// The bits a float is stored as: −0.0 as +0.0 (`docs/spec/eg-columns.md` § 1).
fn stored(x: f64) -> u64 {
    if x == 0.0 { 0 } else { x.to_bits() }
}

/// The statistics of `values` computed here: sums of multiples of 1/16 and of small
/// integers are exact in any order.
fn check_stats(stats: &ColumnStats, values: &[u64], float: bool, what: &str) {
    assert_eq!(stats.count, u64::try_from(values.len()).unwrap(), "{what}");
    assert_eq!((stats.null_count, stats.nan_count), (0, 0), "{what}");
    if float {
        let sum: f64 = values.iter().map(|&b| f(b)).sum();
        assert_eq!(stats.sum, sum + 0.0, "{what}");
        let lo = values.iter().map(|&b| f(b)).min_by(f64::total_cmp);
        let hi = values.iter().map(|&b| f(b)).max_by(f64::total_cmp);
        let range = stats.range.map(|(a, b)| {
            (
                f64::from_le_bytes(a[..8].try_into().unwrap()),
                f64::from_le_bytes(b[..8].try_into().unwrap()),
            )
        });
        assert_eq!(range, lo.zip(hi), "{what}");
    } else {
        let sum: i128 = values.iter().map(|&b| i128::from(t(b))).sum();
        #[expect(clippy::cast_precision_loss, reason = "small sums, exact")]
        let sum = sum as f64;
        assert_eq!(stats.sum, sum, "{what}");
        let lo = values.iter().map(|&b| t(b)).min();
        let hi = values.iter().map(|&b| t(b)).max();
        let range = stats.range.map(|(a, b)| {
            (
                i64::from_le_bytes(a[..8].try_into().unwrap()),
                i64::from_le_bytes(b[..8].try_into().unwrap()),
            )
        });
        assert_eq!(range, lo.zip(hi), "{what}");
    }
}

/// Builds the rows and checks every stored column, the largest multiplicity and the
/// degree histogram against the rows grouped by pair here.
fn check_case(name: &str, rows: &[Row], case: &Case<'_>) -> Report {
    let graph = case.graph;
    let (bytes, report) = build_bytes(case, &text(rows)).unwrap_or_else(|e| panic!("{name}: {e}"));
    let verified = verify(
        &MemorySource::new(&bytes[..]),
        Depth::Deep,
        &Memory::unlimited(),
    )
    .unwrap();
    assert!(
        verified.problems.is_empty(),
        "{name}: {:?}",
        verified.problems
    );
    assert!(
        verified.unchecked.iter().all(|u| !u.contains("values")),
        "{name}"
    );
    let back = read_back(&bytes);
    let file = Container::open(MemorySource::new(&bytes[..])).unwrap();

    // The oracle: each pair's rows in input order, keyed by its ends as the list of `x`
    // holds it.
    let mut pairs: BTreeMap<(u32, u32), Vec<Row>> = BTreeMap::new();
    let mut nodes = std::collections::BTreeSet::new();
    for r in rows {
        nodes.extend([r.u, r.v]);
        if r.u == r.v && graph.self_loops == SelfLoops::Drop {
            continue;
        }
        let key = if graph.directed {
            (r.u, r.v)
        } else {
            (r.u.min(r.v), r.u.max(r.v))
        };
        pairs.entry(key).or_default().push(*r);
    }
    let pair_of = |x: u32, y: u32| {
        let key = if graph.directed {
            (x, y)
        } else {
            (x.min(y), x.max(y))
        };
        &pairs[&key]
    };
    let us = |r: &Row| r.t * 1_000_000;

    let scope = |s: &str| format!("adjacency.out.{s}");
    let get = |s: &str, n: &str| column(&file, &scope(s), n);
    let (wn, tn) = ("column 3", "column 4");
    let mut entry = 0;
    let mut event = 0;
    let mut checked = Vec::new();
    for (id, list) in back.lists.iter().enumerate() {
        let x: u32 = back.keys[id].parse().unwrap();
        for &(target, m) in list {
            let y: u32 = back.keys[usize::try_from(target).unwrap()].parse().unwrap();
            let rows = pair_of(x, y);
            checked.push((entry, event, rows.clone(), m));
            entry += 1;
            event += usize::try_from(m).unwrap();
        }
    }
    let at = |values: &[u64], i: usize| values[i];
    match graph.duplicates {
        Duplicates::Events => {
            let weights = case.weight.then(|| get("event", wn));
            let sums = case.weight.then(|| get("pair", wn));
            let times = case.time.then(|| get("event", tn));
            for (e, ev, rows, m) in &checked {
                let mut order = rows.clone();
                if case.time {
                    order.sort_by_key(|r| r.t);
                }
                assert_eq!(*m, u64::try_from(rows.len()).unwrap(), "{name}");
                for (k, r) in order.iter().enumerate() {
                    if let Some((w, _)) = &weights {
                        assert_eq!(at(w, ev + k), stored(r.w), "{name}: event weight");
                    }
                    if let Some((ts, _)) = &times {
                        assert_eq!(t(at(ts, ev + k)), us(r), "{name}: event time");
                    }
                }
                if let Some((s, _)) = &sums {
                    let sum: f64 = rows.iter().map(|r| r.w).sum();
                    assert_eq!(at(s, *e), stored(sum), "{name}: pair sum");
                }
            }
            for (column, float) in [(weights, true), (sums, true), (times, false)] {
                if let Some((values, stats)) = column {
                    check_stats(&stats, &values, float, name);
                }
            }
        }
        Duplicates::Collapse => {
            let kept = case.weight.then(|| get("pair", wn));
            let times = case.time.then(|| get("pair", tn));
            for (e, _, rows, _) in &checked {
                if let Some((w, _)) = &kept {
                    assert_eq!(at(w, *e), stored(rows[0].w), "{name}: kept weight");
                }
                if let Some((ts, _)) = &times {
                    assert_eq!(t(at(ts, *e)), us(&rows[0]), "{name}: kept time");
                }
            }
            for (column, float) in [(kept, true), (times, false)] {
                if let Some((values, stats)) = column {
                    check_stats(&stats, &values, float, name);
                }
            }
        }
        Duplicates::Sum => {
            let sums = case.weight.then(|| get("pair", wn));
            let edges = get("pair", "edges");
            let first = case.time.then(|| get("pair", "first"));
            let last = case.time.then(|| get("pair", "last"));
            for (e, _, rows, _) in &checked {
                assert_eq!(
                    at(&edges.0, *e),
                    u64::try_from(rows.len()).unwrap(),
                    "{name}"
                );
                if let Some((s, _)) = &sums {
                    let sum: f64 = rows.iter().map(|r| r.w).sum();
                    assert_eq!(at(s, *e), stored(sum), "{name}: summed weight");
                }
                let times = rows.iter().map(us);
                if let (Some((a, _)), Some((b, _))) = (&first, &last) {
                    assert_eq!(t(at(a, *e)), times.clone().min().unwrap(), "{name}: first");
                    assert_eq!(t(at(b, *e)), times.max().unwrap(), "{name}: last");
                }
            }
            check_stats(&edges.1, &edges.0, false, name);
            for (column, float) in [(sums, true), (first, false), (last, false)] {
                if let Some((values, stats)) = column {
                    check_stats(&stats, &values, float, name);
                }
            }
        }
    }

    // The largest multiplicity, and the degree of § 3 by value, over every node read.
    let most = pairs.values().map(Vec::len).max().unwrap_or(0);
    let most = if graph.duplicates == Duplicates::Collapse {
        most.min(1)
    } else {
        most
    };
    assert_eq!(
        back.parameters["multiplicity.max"],
        most.to_string(),
        "{name}"
    );
    assert_eq!(report.most_events, u64::try_from(most).unwrap(), "{name}");
    let mut degree: BTreeMap<u32, u64> = nodes.iter().map(|&v| (v, 0)).collect();
    for &(a, b) in pairs.keys() {
        *degree.get_mut(&a).unwrap() += 1;
        if !graph.directed {
            *degree.get_mut(&b).unwrap() += 1;
        }
    }
    let mut histogram = BTreeMap::<u64, u64>::new();
    for d in degree.values() {
        *histogram.entry(*d).or_default() += 1;
    }
    let (values, _) = get("degrees", "degree");
    let (counts, _) = get("degrees", "nodes");
    let stored: BTreeMap<u64, u64> = values.into_iter().zip(counts).collect();
    assert_eq!(stored, histogram, "{name}: degree histogram");
    report
}

fn cases(directed: bool, budget: u64, tuning: &Tuning) -> Vec<Case<'_>> {
    let mut out = Vec::new();
    for graph in all_options(directed) {
        for (weight, time) in [(true, true), (true, false), (false, true), (false, false)] {
            out.push(Case {
                graph,
                weight,
                time,
                budget,
                tuning,
            });
        }
    }
    out
}

/// Every graph of the CI corpus, every option, weight and time, in memory.
#[test]
fn the_stored_columns_are_the_oracle_s() {
    for (i, spec) in specs(Tier::Ci).into_iter().enumerate() {
        let g = generate(&spec);
        let rows = rows_of(&g.src, &g.dst, widen(i));
        for case in cases(g.directed, 1 << 30, &SMALL) {
            check_case(&g.name, &rows, &case);
        }
    }
}

/// The same with tiny buckets and a budget that makes the larger graphs spill.
#[test]
fn the_stored_columns_are_the_oracle_s_with_spills() {
    let mut spilled = 0;
    for (i, spec) in specs(Tier::Ci).into_iter().enumerate().step_by(3) {
        let g = generate(&spec);
        let rows = rows_of(&g.src, &g.dst, widen(i));
        for case in cases(g.directed, 3 << 20, &SPILLING) {
            let report = check_case(&g.name, &rows, &case);
            spilled += u64::from(report.regime == Regime::OnDisk);
        }
    }
    assert!(spilled > 0, "no build spilled");
}

/// A graph of 6,000 rows with repeated pairs, loops and hubs, with both columns: the
/// same bytes at 1, 4 and 16 threads, in memory and with spills, under every option.
#[test]
fn columns_do_not_depend_on_threads_or_budget() {
    let mut rng = SplitMix64::new(7);
    let (mut src, mut dst) = (Vec::new(), Vec::new());
    for i in 0..6_000_u32 {
        let u = if i % 7 == 0 {
            rng.below_u32(5)
        } else {
            rng.below_u32(900)
        };
        src.push(u);
        dst.push(if i % 13 == 0 { u } else { rng.below_u32(900) });
    }
    let rows = rows_of(&src, &dst, 3);
    let text = text(&rows);
    for directed in [false, true] {
        for graph in all_options(directed) {
            let mut seen = Vec::new();
            for (budget, tuning) in [(1 << 30, &TUNING), (4 << 20, &SPILLING)] {
                let case = Case {
                    graph,
                    weight: true,
                    time: true,
                    budget,
                    tuning,
                };
                for threads in [1, 4, 16] {
                    let threads = NonZeroUsize::new(threads).unwrap();
                    let (bytes, report) = par::with_threads(threads, || build_bytes(&case, &text))
                        .unwrap()
                        .unwrap();
                    seen.push((bytes, report.regime));
                }
            }
            assert!(
                seen.iter().any(|s| s.1 == Regime::OnDisk),
                "{graph:?}: nothing spilled"
            );
            assert!(
                seen.windows(2).all(|w| w[0].0 == w[1].0),
                "{graph:?}: the bytes differ"
            );
        }
    }
}
