// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Views against their definition (`everygraph_query::views`): random graphs with
//! node, pair and event columns, random predicates, and a model written from the
//! definition. Then hostile predicates, and the budget a view holds.

#![forbid(unsafe_code)]
#![allow(
    clippy::disallowed_methods,
    clippy::cast_possible_truncation,
    clippy::float_cmp,
    clippy::cast_possible_wrap
)]

use std::sync::{Arc, Mutex};

use everygraph_core::{Budget, ByteSource, Memory, ReadError};
use everygraph_format::adjacency::{Adjacency, AdjacencyIndex, encode};
use everygraph_kernels::edge_map::{
    Compressed, Csr, Error, Plan, Settings, Source, Stream, all, ids,
};
use everygraph_kernels::planner::{self, ViewForm};
use everygraph_query::{Column, Predicate, Scalar, Test, Values, ViewSpec};
use proptest::prelude::*;

/// A 64-bit generator for test data (`SplitMix64`).
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        (z ^ (z >> 31)) % n
    }
}

/// A directed graph with loops, its event offsets and columns.
struct Graph {
    offsets: Vec<u64>,
    targets: Vec<u32>,
    events: Vec<u64>,
    label: Vec<u32>,
    score: Vec<f64>,
    value: Vec<f64>,
    time: Vec<i64>,
    amount: Vec<f64>,
}

impl Graph {
    /// A graph of `n` nodes; with `ring`, each node also lists the next, with a pair
    /// value of 0, so that a pair predicate can keep every node.
    fn random(rng: &mut Rng, n: u32, ring: bool) -> Self {
        let (mut offsets, mut targets, mut events) = (vec![0], Vec::new(), vec![0]);
        for v in 0..n {
            let mut list: Vec<u32> = (0..rng.below(6))
                .map(|_| rng.below(n.into()) as u32)
                .chain(ring.then_some((v + 1) % n))
                .collect();
            list.sort_unstable();
            list.dedup();
            for _ in &list {
                events.push(events.last().unwrap() + 1 + rng.below(3));
            }
            targets.extend(list);
            offsets.push(targets.len() as u64);
        }
        let rows = *events.last().unwrap();
        let score = |rng: &mut Rng| match rng.below(8) {
            0 => f64::NAN,
            k => k as f64 / 4.0,
        };
        let mut value: Vec<f64> = targets.iter().map(|_| score(rng)).collect();
        for v in (0..n).filter(|_| ring) {
            let at = offsets[v as usize] as usize;
            let next = targets[at..offsets[v as usize + 1] as usize].binary_search(&((v + 1) % n));
            value[at + next.unwrap()] = 0.0;
        }
        Self {
            label: (0..n).map(|_| rng.below(4) as u32).collect(),
            score: (0..n).map(|_| score(rng)).collect(),
            value,
            time: (0..rows).map(|_| rng.below(100) as i64 - 50).collect(),
            amount: (0..rows)
                .map(|_| match rng.below(6) {
                    0 => 1e16,
                    1 => -1e16,
                    k => (k * rng.below(64)) as f64 / 8.0,
                })
                .collect(),
            offsets,
            targets,
            events,
        }
    }

    fn n(&self) -> u32 {
        (self.offsets.len() - 1) as u32
    }

    fn list(&self, v: u32) -> std::ops::Range<usize> {
        self.offsets[v as usize] as usize..self.offsets[v as usize + 1] as usize
    }
}

/// What a random view asks for: each part optional.
#[derive(Clone, Debug)]
struct Ask {
    labels: Option<Vec<u32>>,
    score_at_least: Option<f64>,
    set: Option<Vec<u32>>,
    value_below: Option<f64>,
    window: Option<(i64, i64)>,
}

/// The view of `g` that `ask` defines, written from the definition: the parent id of
/// each kept node, and each kept node's kept entries as (parent entry, view target).
fn model(g: &Graph, ask: &Ask) -> (Vec<u32>, Vec<Vec<(u64, u32)>>) {
    let n = g.n();
    let allowed = |v: u32| {
        let v = v as usize;
        ask.labels.as_ref().is_none_or(|l| l.contains(&g.label[v]))
            && ask.score_at_least.is_none_or(|s| g.score[v] >= s)
            && ask.set.as_ref().is_none_or(|s| s.contains(&(v as u32)))
    };
    let event_ok = |e: usize| {
        ask.window.is_none_or(|(a, b)| {
            (g.events[e]..g.events[e + 1]).any(|i| (a..=b).contains(&g.time[i as usize]))
        })
    };
    let kept = |u: u32, e: usize| {
        allowed(u)
            && allowed(g.targets[e])
            && ask.value_below.is_none_or(|x| g.value[e] < x)
            && event_ok(e)
    };
    let edges = ask.value_below.is_some() || ask.window.is_some();
    let mut touched = vec![!edges; n as usize];
    for u in 0..n {
        for e in g.list(u).filter(|&e| kept(u, e)) {
            touched[u as usize] = true;
            touched[g.targets[e] as usize] = true;
        }
    }
    let parents: Vec<u32> = (0..n)
        .filter(|&v| allowed(v) && touched[v as usize])
        .collect();
    let id = |p: u32| parents.binary_search(&p).unwrap() as u32;
    let lists = parents
        .iter()
        .map(|&u| {
            g.list(u)
                .filter(|&e| kept(u, e))
                .map(|e| (e as u64, id(g.targets[e])))
                .collect()
        })
        .collect();
    (parents, lists)
}

fn spec<'a>(g: &'a Graph, ask: &'a Ask) -> ViewSpec<'a> {
    let mut s = ViewSpec::new().with_events(&g.events);
    if let Some(l) = &ask.labels {
        s = s.nodes(Predicate::new(Values::U32(&g.label), Test::In(l)));
    }
    if let Some(x) = ask.score_at_least {
        s = s.nodes(Predicate::new(Values::F64(&g.score), Test::Ge(x.into())));
    }
    if let Some(set) = &ask.set {
        s = s.node_set(set);
    }
    if let Some(x) = ask.value_below {
        s = s.pairs(Predicate::new(Values::F64(&g.value), Test::Lt(x.into())));
    }
    if let Some((a, b)) = ask.window {
        let t = Predicate::new(Values::I64(&g.time), Test::Between(a.into(), b.into()));
        s = s.events(t);
    }
    s
}

/// Every list of `source`: by a scan, by a frontier of every node and by random
/// access, which must agree. Each list with its first entry.
fn lists<S: Source>(source: &S) -> Vec<(u64, Vec<u32>)> {
    let memory = Memory::unlimited();
    let settings = Settings {
        chunk_entries: 8,
        batch_entries: 3,
        ..Settings::default()
    };
    Plan::with(source, &memory, settings, |plan| {
        let rows = Mutex::new(Vec::new());
        plan.scan((), all, |(): &mut (), (), _, row, w| {
            let got = w.lists.get(row.node).unwrap();
            assert_eq!(
                (got.first_entry, ids(got.list)),
                (row.first_entry, ids(row.list))
            );
            rows.lock()
                .unwrap()
                .push((row.node.get(), row.first_entry, ids(row.list).to_vec()));
        })
        .unwrap();
        let mut scanned = rows.into_inner().unwrap();
        scanned.sort_unstable();
        let pushed = Mutex::new(Vec::new());
        let every: Vec<_> = plan.all_nodes().collect();
        plan.frontier(&every, |(): &mut (), row, _| {
            pushed
                .lock()
                .unwrap()
                .push((row.node.get(), row.first_entry, ids(row.list).to_vec()));
        })
        .unwrap();
        let mut pushed = pushed.into_inner().unwrap();
        pushed.sort_unstable();
        assert_eq!(scanned, pushed, "scan and frontier");
        scanned.into_iter().map(|(_, f, l)| (f, l)).collect()
    })
}

/// The view of `g` that `ask` defines, over a raw and a compressed parent, as a mask
/// and materialized, against [`model`]: its nodes, lists, ids and per-entry columns.
fn check(g: &Graph, ask: &Ask) {
    let (parents, expected) = model(g, ask);
    let mut first = 0;
    let want: Vec<(u64, Vec<u32>)> = expected
        .iter()
        .map(|l| {
            let at = first;
            first += l.len() as u64;
            (at, l.iter().map(|&(_, t)| t).collect())
        })
        .collect();
    let csr = Csr::new(&g.offsets, &g.targets).unwrap();
    let encoded = encode(&g.offsets, &g.targets).unwrap();
    let index = AdjacencyIndex::parse(&encoded.index).unwrap();
    let heap = Compressed::new(Adjacency::new(&index, &encoded.data).unwrap());
    check_on(&csr, g, ask, &parents, &expected, &want);
    check_on(&heap, g, ask, &parents, &expected, &want);
}

fn check_on<S: Source>(
    parent: &S,
    g: &Graph,
    ask: &Ask,
    parents: &[u32],
    expected: &[Vec<(u64, u32)>],
    want: &[(u64, Vec<u32>)],
) {
    let memory = Memory::unlimited();
    let s = spec(g, ask);
    let mut f = s.build(parent, &memory, Settings::default()).unwrap();
    let view = f.view();
    assert_eq!(view.nodes() as usize, parents.len(), "{ask:?}");
    assert_eq!(
        view.entries(),
        want.iter().map(|l| l.1.len() as u64).sum::<u64>()
    );
    for (v, &p) in parents.iter().enumerate() {
        assert_eq!(view.parent_id(v as u32), p);
        assert_eq!(view.view_id(p), Some(v as u32));
    }
    for p in (0..g.n()).filter(|p| parents.binary_search(p).is_err()) {
        assert_eq!(view.view_id(p), None);
    }
    assert_eq!(view.view_id(g.n()), None);
    let kept: Vec<u64> = expected.iter().flatten().map(|&(e, _)| e).collect();
    let in_window = |i: u64| {
        ask.window
            .is_none_or(|(a, b)| (a..=b).contains(&g.time[i as usize]))
    };
    let events =
        |e: u64| (g.events[e as usize]..g.events[e as usize + 1]).filter(|&i| in_window(i));
    let values: Vec<f64> = kept.iter().map(|&e| g.value[e as usize]).collect();
    let counts: Vec<u64> = kept.iter().map(|&e| events(e).count() as u64).collect();
    let sums: Vec<f64> = kept
        .iter()
        .map(|&e| {
            let eighths: i128 = events(e)
                .map(|i| (g.amount[i as usize] * 8.0) as i128)
                .sum();
            eighths as f64 / 8.0
        })
        .collect();
    let bits = |v: &[f64]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
    let pair = f.pair_values(&g.value, &memory).unwrap();
    assert_eq!(bits(pair.as_slice().unwrap()), bits(&values));
    let m = f.multiplicity(&memory).unwrap();
    assert_eq!(m.as_slice().unwrap(), &counts[..]);
    let s = f.event_sums(&g.amount, &memory).unwrap();
    assert_eq!(s.as_slice().unwrap(), &sums[..]);
    assert_eq!(lists(f.view()), want, "mask, {ask:?}");
    assert_eq!(f.hold(2, &memory).unwrap(), ViewForm::Materialized);
    assert!(f.view().is_materialized());
    assert_eq!(lists(f.view()), want, "materialized, {ask:?}");
    assert_eq!(f.hold(1, &memory).unwrap(), ViewForm::Mask);
    assert!(!f.view().is_materialized());
}

fn ask(rng: &mut Rng, n: u32) -> Ask {
    let some = |rng: &mut Rng| rng.below(2) == 0;
    Ask {
        labels: some(rng).then(|| {
            let mut l: Vec<u32> = (0..rng.below(4)).map(|_| rng.below(4) as u32).collect();
            l.sort_unstable();
            l.dedup();
            l
        }),
        score_at_least: some(rng).then(|| rng.below(9) as f64 / 4.0),
        set: some(rng).then(|| {
            (0..rng.below(u64::from(n) + 1))
                .map(|_| rng.below(n.max(1).into()) as u32)
                .collect()
        }),
        value_below: some(rng).then(|| rng.below(9) as f64 / 4.0),
        window: some(rng).then(|| {
            let a = rng.below(100) as i64 - 50;
            (a, a + rng.below(60) as i64)
        }),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    #[test]
    fn a_view_keeps_what_its_definition_says(seed: u64, n in 0_u32..60) {
        let mut rng = Rng(seed);
        let ring = rng.below(2) == 0;
        let g = Graph::random(&mut rng, n, ring);
        let ask = ask(&mut rng, n);
        check(&g, &ask);
    }
}

/// Graphs past the rank directory's words and the tasks of a materialization: 3,000
/// nodes, every kind of view.
#[test]
fn larger_views_keep_what_their_definition_says() {
    let mut rng = Rng(7);
    for ring in [false, true] {
        let g = Graph::random(&mut rng, 3000, ring);
        for _ in 0..8 {
            let ask = ask(&mut rng, 3000);
            check(&g, &ask);
        }
        let pairs = Ask {
            labels: None,
            score_at_least: None,
            set: None,
            value_below: Some(1.0),
            window: None,
        };
        check(&g, &pairs);
    }
}

/// A small graph: 0–1, 1–2, 2–3, 3–0 and 3–3, both ways, one event per entry.
fn square() -> Graph {
    let offsets = vec![0, 2, 4, 6, 9];
    let targets = vec![1, 3, 0, 2, 1, 3, 0, 2, 3];
    let events = (0..=targets.len() as u64).collect();
    Graph {
        label: vec![0, 1, 1, 0],
        score: vec![0.0, -0.0, f64::NAN, f64::INFINITY],
        value: vec![1.0, f64::NAN, 1.0, 2.0, 2.0, -0.0, f64::NAN, -0.0, 3.0],
        time: vec![i64::MIN, 0, 0, 5, 5, i64::MAX, i64::MAX, 7, 7],
        amount: vec![1.0; targets.len()],
        offsets,
        targets,
        events,
    }
}

/// The nodes and entries of the view of `square` that `s` defines.
fn counts(s: &ViewSpec<'_>, g: &Graph) -> Result<(u32, u64), Error> {
    let csr = Csr::new(&g.offsets, &g.targets).unwrap();
    let f = s.build(&csr, &Memory::unlimited(), Settings::default())?;
    Ok((f.view().nodes(), f.view().entries()))
}

/// Thresholds and values that break naive comparisons: NaN, signed zeros, infinities,
/// integers beyond 2^53 against floats, the ends of `i64`, empty and inverted windows,
/// nulls.
#[test]
fn hostile_predicates_keep_what_the_comparison_rules_say() {
    let g = square();
    let node = |test| ViewSpec::new().nodes(Predicate::new(Values::F64(&g.score), test));
    let pair = |test| ViewSpec::new().pairs(Predicate::new(Values::F64(&g.value), test));
    let f = |x: f64| Scalar::Float(x);
    // A NaN threshold keeps nothing; a NaN value is never kept.
    assert_eq!(counts(&node(Test::Ge(f(f64::NAN))), &g).unwrap(), (0, 0));
    assert_eq!(counts(&pair(Test::Lt(f(f64::NAN))), &g).unwrap(), (0, 0));
    assert_eq!(
        counts(&node(Test::Le(f(f64::INFINITY))), &g).unwrap(),
        (3, 5)
    );
    // −0.0 equals 0.0; two nodes score a zero.
    assert_eq!(counts(&node(Test::Eq(f(-0.0))), &g).unwrap(), (2, 2));
    assert_eq!(counts(&pair(Test::Eq(f(0.0))), &g).unwrap().1, 2);
    // A window with its ends swapped is empty; one at the ends of i64 keeps every time.
    let time = |a: i64, b: i64| {
        let t = Predicate::new(Values::I64(&g.time), Test::Between(a.into(), b.into()));
        ViewSpec::new().with_events(&g.events).events(t)
    };
    assert_eq!(counts(&time(5, 0), &g).unwrap(), (0, 0));
    assert_eq!(counts(&time(i64::MIN, i64::MAX), &g).unwrap(), (4, 9));
    assert_eq!(counts(&time(i64::MAX, i64::MAX), &g).unwrap(), (3, 2));
    // Integers compare exactly with floats: 2^53 + 1 is above the float 2^53, and
    // u64::MAX below the float 2^64 it rounds to.
    let big = [(1_u64 << 53) + 1, u64::MAX, 0, 1];
    let ints = |test| ViewSpec::new().nodes(Predicate::new(Values::U64(&big), test));
    assert_eq!(
        counts(&ints(Test::Gt(f(9_007_199_254_740_992.0))), &g)
            .unwrap()
            .0,
        2
    );
    assert_eq!(
        counts(&ints(Test::Lt(f(18_446_744_073_709_551_616.0))), &g)
            .unwrap()
            .0,
        4
    );
    assert_eq!(
        counts(&ints(Test::Eq(f(18_446_744_073_709_551_616.0))), &g)
            .unwrap()
            .0,
        0
    );
    assert_eq!(counts(&ints(Test::Eq(u64::MAX.into())), &g).unwrap().0, 1);
    assert_eq!(counts(&ints(Test::Gt(f(0.5))), &g).unwrap().0, 3);
    // Where only the fraction decides: 0 and 1 against 0.5, −1 and 0 against −0.5, and
    // the integer 2 against floats around it.
    let small = [0_u64, 1, 0, 1];
    let small = |test| ViewSpec::new().nodes(Predicate::new(Values::U64(&small), test));
    assert_eq!(counts(&small(Test::Ge(f(0.5))), &g).unwrap().0, 2);
    assert_eq!(counts(&small(Test::Gt(f(0.5))), &g).unwrap().0, 2);
    assert_eq!(counts(&small(Test::Lt(f(0.5))), &g).unwrap().0, 2);
    assert_eq!(counts(&small(Test::Le(f(0.5))), &g).unwrap().0, 2);
    assert_eq!(counts(&small(Test::Eq(f(0.5))), &g).unwrap().0, 0);
    let negative = [-1_i64, 0, -1, 0];
    let negative = |test| ViewSpec::new().nodes(Predicate::new(Values::I64(&negative), test));
    assert_eq!(counts(&negative(Test::Ge(f(-0.5))), &g).unwrap().0, 2);
    assert_eq!(counts(&negative(Test::Le(f(-0.5))), &g).unwrap().0, 2);
    assert_eq!(counts(&negative(Test::Lt(f(-0.5))), &g).unwrap().0, 2);
    let around = [2.5, 1.5, 2.0, 1.0];
    let around = |test| ViewSpec::new().nodes(Predicate::new(Values::F64(&around), test));
    assert_eq!(counts(&around(Test::Gt(2_i64.into())), &g).unwrap().0, 1);
    assert_eq!(counts(&around(Test::Ge(2_i64.into())), &g).unwrap().0, 2);
    assert_eq!(counts(&around(Test::Lt(2_i64.into())), &g).unwrap().0, 2);
    assert_eq!(counts(&around(Test::Le(2_i64.into())), &g).unwrap().0, 3);
    assert_eq!(counts(&around(Test::Eq(2_i64.into())), &g).unwrap().0, 1);
    // A null is never kept, whatever the test: node 1 is null.
    let valid = [0b1101_u8];
    let column = Column {
        values: Values::F64(&g.score),
        valid: Some(&valid),
    };
    let nulls = ViewSpec::new().nodes(Predicate::new(column, Test::Le(f(f64::INFINITY))));
    assert_eq!(counts(&nulls, &g).unwrap().0, 2);
    // Every predicate that holds everywhere keeps the graph whole.
    let everything = ViewSpec::new()
        .nodes(Predicate::new(Values::U32(&g.label), Test::In(&[0, 1])))
        .node_set(&[3, 2, 1, 0, 0]);
    assert_eq!(counts(&everything, &g).unwrap(), (4, 9));
    assert_eq!(counts(&ViewSpec::new().node_set(&[]), &g).unwrap(), (0, 0));
}

/// Definitions a view refuses, before any pass over the lists.
#[test]
fn hostile_definitions_are_refused() {
    let g = square();
    let refused = |s: ViewSpec<'_>| match counts(&s, &g) {
        Err(Error::Argument(why)) => why,
        other => panic!("not refused: {other:?}"),
    };
    let short = [1.0, 2.0];
    let label = |set| Predicate::new(Values::U32(&g.label), Test::In(set));
    refused(ViewSpec::new().nodes(Predicate::new(Values::F64(&short), Test::Lt(1.0.into()))));
    refused(ViewSpec::new().pairs(Predicate::new(Values::F64(&g.score), Test::Lt(1.0.into()))));
    refused(ViewSpec::new().nodes(label(&[1, 0])));
    refused(ViewSpec::new().nodes(label(&[1, 1])));
    refused(ViewSpec::new().nodes(Predicate::new(Values::F64(&g.score), Test::In(&[0]))));
    refused(ViewSpec::new().node_set(&[4]));
    refused(ViewSpec::new().node_set(&[u32::MAX]));
    let column = Column {
        values: Values::F64(&g.value),
        valid: Some(&[0xFF]),
    };
    refused(ViewSpec::new().pairs(Predicate::new(column, Test::Lt(1.0.into()))));
    let mut decreasing = g.events.clone();
    decreasing.swap(3, 4);
    let mut late = g.events.clone();
    late[0] = 1;
    for bad in [&g.events[1..], &decreasing[..], &late[..]] {
        let p = Predicate::new(Values::I64(&g.time), Test::Ge(0_i64.into()));
        refused(ViewSpec::new().with_events(bad).events(p));
    }
    let copy = g.events.clone();
    let p = Predicate::new(Values::I64(&g.time), Test::Ge(0_i64.into()));
    refused(ViewSpec::new().events(p));
    refused(
        ViewSpec::new()
            .with_events(&g.events)
            .with_events(&copy)
            .events(p),
    );
    // A view without event offsets has no event counts or sums.
    let csr = Csr::new(&g.offsets, &g.targets).unwrap();
    let memory = Memory::unlimited();
    let f = ViewSpec::new()
        .build(&csr, &memory, Settings::default())
        .unwrap();
    assert!(matches!(f.multiplicity(&memory), Err(Error::Argument(_))));
    assert!(matches!(
        f.event_sums(&g.amount, &memory),
        Err(Error::Argument(_))
    ));
    let f = ViewSpec::new()
        .with_events(&g.events)
        .build(&csr, &memory, Settings::default());
    let short = f.unwrap().event_sums(&g.amount[1..], &memory);
    assert!(matches!(short, Err(Error::Argument(_))));
}

/// The sum of a pair's kept events is exact: `1e16 + 1 − 1e16` is 1, where a running
/// float sum gives 0, and dropping the 1 gives 0.
#[test]
fn event_sums_are_exact() {
    let (offsets, targets) = ([0, 1, 2], [1, 0]);
    let events = [0, 3, 6];
    let amount = [1e16, 1.0, -1e16, 1e16, 1.0, -1e16];
    let time = [0_i64, 1, 2, 0, 1, 2];
    let csr = Csr::new(&offsets, &targets).unwrap();
    let memory = Memory::unlimited();
    for (window, sum) in [
        ((0, 2), 1.0),
        ((0, 0), 1e16),
        ((1, 1), 1.0),
        ((2, 2), -1e16),
    ] {
        let t = Predicate::new(
            Values::I64(&time),
            Test::Between(window.0.into(), window.1.into()),
        );
        let spec = ViewSpec::new().with_events(&events).events(t);
        let f = spec.build(&csr, &memory, Settings::default()).unwrap();
        let sums = f.event_sums(&amount, &memory).unwrap();
        assert_eq!(sums.as_slice().unwrap(), &[sum, sum], "{window:?}");
    }
}

/// What a view holds is reserved: the budget's use is the view's bytes, a budget one
/// byte below the peak of the build refuses it with nothing left reserved, and the
/// planner's rule materializes only what fits.
#[test]
fn a_view_holds_what_it_reserves() {
    let mut rng = Rng(3);
    let g = Graph::random(&mut rng, 5000, false);
    let ask = Ask {
        labels: Some(vec![0, 1, 2]),
        score_at_least: None,
        set: None,
        value_below: Some(1.5),
        window: Some((-40, 30)),
    };
    let csr = Csr::new(&g.offsets, &g.targets).unwrap();
    let build = |limit: u64| {
        let budget = Arc::new(Budget::new(limit));
        let memory = Memory::from(Arc::clone(&budget));
        let built = spec(&g, &ask).build(&csr, &memory, Settings::default());
        (budget, memory, built)
    };
    let (budget, memory, built) = build(u64::MAX);
    let mut f = built.unwrap();
    let peak = budget.peak();
    let event_bits = g.events.last().unwrap().div_ceil(64) * 8;
    assert_eq!(budget.used(), f.view().mask_bytes() + event_bits);
    f.hold(2, &memory).unwrap();
    assert_eq!(
        f.view().materialized_bytes(),
        8 * (u64::from(f.view().nodes()) + 1) + 4 * f.view().entries()
    );
    assert_eq!(
        budget.used(),
        f.view().mask_bytes() + event_bits + f.view().materialized_bytes()
    );
    drop(f);
    assert_eq!(budget.used(), 0);

    let (budget, _, built) = build(peak);
    assert!(built.is_ok());
    drop(built);
    assert_eq!(budget.used(), 0);
    let (budget, _, built) = build(peak - 1);
    assert!(matches!(built, Err(Error::Memory(_))), "{built:?}");
    assert_eq!(budget.used(), 0);

    // With no more free than the copy takes, the rule keeps the mask.
    let (_, memory, built) = build(u64::MAX);
    let mut f = built.unwrap();
    let copy = planner::materialized_bytes(f.view().nodes(), f.view().entries());
    let tight = Memory::from(Arc::new(Budget::new(copy)));
    assert_eq!(f.hold(20, &tight).unwrap(), ViewForm::Mask);
    assert_eq!(f.hold(20, &memory).unwrap(), ViewForm::Materialized);
    assert_eq!(f.hold(1, &memory).unwrap(), ViewForm::Mask);
}

/// Every whole-graph kernel runs on an empty view and on a view of one node.
#[test]
fn kernels_run_on_empty_and_tiny_views() {
    use everygraph_kernels::run::{self, Lanes, Operation};
    let g = square();
    let csr = Csr::new(&g.offsets, &g.targets).unwrap();
    let memory = Memory::unlimited();
    for set in [&[][..], &[3][..]] {
        let f = ViewSpec::new()
            .node_set(set)
            .build(&csr, &memory, Settings::default())
            .unwrap();
        Plan::with(f.view(), &memory, Settings::default(), |plan| {
            let components = Operation::Components { strong: false };
            for op in [components, Operation::KCore, Operation::Degrees] {
                let out = run::run(&op, &Lanes::out(&plan, false), &memory).unwrap();
                assert_eq!(out.columns.len(), 1);
            }
        });
    }
}

/// The in-lists of `g` with their own per-entry and per-event columns: what a graph
/// file stores as its in-adjacency.
fn transpose(g: &Graph) -> Graph {
    let mut rows: Vec<(u32, u32, usize)> = Vec::new();
    for u in 0..g.n() {
        for e in g.list(u) {
            rows.push((g.targets[e], u, e));
        }
    }
    rows.sort_unstable();
    let (mut offsets, mut events) = (vec![0; g.n() as usize + 1], vec![0]);
    let (mut time, mut amount) = (Vec::new(), Vec::new());
    for &(v, _, e) in &rows {
        offsets[v as usize + 1] += 1;
        let r = g.events[e] as usize..g.events[e + 1] as usize;
        time.extend_from_slice(&g.time[r.clone()]);
        amount.extend_from_slice(&g.amount[r]);
        events.push(time.len() as u64);
    }
    for v in 0..g.n() as usize {
        offsets[v + 1] += offsets[v];
    }
    Graph {
        offsets,
        targets: rows.iter().map(|r| r.1).collect(),
        events,
        label: g.label.clone(),
        score: g.score.clone(),
        value: rows.iter().map(|r| g.value[r.2]).collect(),
        time,
        amount,
    }
}

/// A directed view from one definition: the out-view and the in-view keep the same
/// nodes under the same ids, the in-view's lists are the transpose of the out-view's,
/// and in-adjacency columns that do not belong to the same graph are refused.
#[test]
fn a_directed_view_has_one_node_map_for_its_out_and_in_lists() {
    let mut rng = Rng(11);
    for _ in 0..20 {
        let g = Graph::random(&mut rng, 300, false);
        let t = transpose(&g);
        let ask = ask(&mut rng, 300);
        let (out_csr, in_csr) = (
            Csr::new(&g.offsets, &g.targets).unwrap(),
            Csr::new(&t.offsets, &t.targets).unwrap(),
        );
        let out_spec = spec(&g, &ask);
        let inn = ViewSpec::new().with_events(&t.events);
        let inn = match ask.value_below {
            Some(x) => inn.pairs(Predicate::new(Values::F64(&t.value), Test::Lt(x.into()))),
            None => inn,
        };
        let inn = match ask.window {
            Some((a, b)) => inn.events(Predicate::new(
                Values::I64(&t.time),
                Test::Between(a.into(), b.into()),
            )),
            None => inn,
        };
        let memory = Memory::unlimited();
        let (o, i) = out_spec
            .build_directed(&inn, (&out_csr, &in_csr), &memory, Settings::default())
            .unwrap();
        assert!(o.view().same_nodes(i.view()));
        let mut back: Vec<Vec<u32>> = vec![Vec::new(); o.view().nodes() as usize];
        for (u, (_, list)) in lists(o.view()).into_iter().enumerate() {
            for v in list {
                back[v as usize].push(u as u32);
            }
        }
        let got: Vec<Vec<u32>> = lists(i.view()).into_iter().map(|l| l.1).collect();
        assert_eq!(got, back, "{ask:?}");
        let sums = |f: &everygraph_query::Filtered<'_, '_, Csr<'_>>, g: &Graph| {
            let mut s: Vec<u64> = f
                .event_sums(&g.amount, &memory)
                .unwrap()
                .iter()
                .map(|x| x.to_bits())
                .collect();
            s.sort_unstable();
            s
        };
        assert_eq!(sums(&o, &g), sums(&i, &t));
    }
    // The in-adjacency's part with another threshold keeps other edges.
    let g = Graph::random(&mut rng, 300, true);
    let t = transpose(&g);
    let (out_csr, in_csr) = (
        Csr::new(&g.offsets, &g.targets).unwrap(),
        Csr::new(&t.offsets, &t.targets).unwrap(),
    );
    let out_spec =
        ViewSpec::new().pairs(Predicate::new(Values::F64(&g.value), Test::Lt(1.0.into())));
    let inn = ViewSpec::new().pairs(Predicate::new(Values::F64(&t.value), Test::Lt(0.5.into())));
    let memory = Memory::unlimited();
    let refused = out_spec.build_directed(&inn, (&out_csr, &in_csr), &memory, Settings::default());
    assert!(matches!(refused, Err(Error::Argument(_))));
    let nodes = ViewSpec::new().node_set(&[0]);
    let refused =
        out_spec.build_directed(&nodes, (&out_csr, &in_csr), &memory, Settings::default());
    assert!(matches!(refused, Err(Error::Argument(_))));
}

/// The targets of a graph, read through positional reads: it notes how much of the
/// budget was reserved when the first read came.
struct Watched<'b> {
    bytes: Vec<u8>,
    budget: &'b Budget,
    at_first_read: Mutex<Option<u64>>,
}

impl ByteSource for Watched<'_> {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), ReadError> {
        self.at_first_read
            .lock()
            .unwrap()
            .get_or_insert(self.budget.used());
        let at = offset as usize;
        buf.copy_from_slice(&self.bytes[at..at + buf.len()]);
        Ok(())
    }
}

/// Every array of a view that grows with the nodes is reserved before the parent is
/// read: at the first read the budget already holds the view's mask, so a budget too
/// small for it refuses before any list is read.
#[test]
fn a_view_reserves_its_node_state_before_it_reads() {
    let mut rng = Rng(5);
    let g = Graph::random(&mut rng, 4000, false);
    let budget = Arc::new(Budget::unlimited());
    let memory = Memory::from(Arc::clone(&budget));
    let source = Watched {
        bytes: g.targets.iter().flat_map(|t| t.to_le_bytes()).collect(),
        budget: &budget,
        at_first_read: Mutex::new(None),
    };
    let stream = Stream::new(&g.offsets, &source).unwrap();
    let test = Predicate::new(Values::F64(&g.value), Test::Lt(1.0.into()));
    let spec = ViewSpec::new()
        .nodes(Predicate::new(Values::U32(&g.label), Test::In(&[0, 1, 2])))
        .pairs(test);
    let f = spec.build(&stream, &memory, Settings::default()).unwrap();
    let first = source.at_first_read.lock().unwrap().expect("a read");
    assert!(f.view().nodes() > 0);
    assert!(
        first >= f.view().mask_bytes(),
        "{first} < {}",
        f.view().mask_bytes()
    );
}
