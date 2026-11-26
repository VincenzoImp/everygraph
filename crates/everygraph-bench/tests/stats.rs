// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The statistics of the harness against known distributions, and the order of an
//! interleaved run.

use everygraph_bench::harness::run::schedule;
use everygraph_bench::harness::stats::{
    Rng, median_interval, min_repetitions, rank_confidence, ratio_interval, repetitions_for,
};

/// A standard normal value (Box–Muller).
fn normal(rng: &mut Rng) -> f64 {
    let u = 1.0 - rng.unit();
    let v = rng.unit();
    (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()
}

type Sampler = fn(&mut Rng) -> f64;

/// Distributions with a known median: name, sampler, median.
fn distributions() -> Vec<(&'static str, Sampler, f64)> {
    vec![
        ("normal(10, 1)", |r| 10.0 + normal(r), 10.0),
        ("lognormal(0, 0.5)", |r| (0.5 * normal(r)).exp(), 1.0),
        (
            "exponential(1)",
            |r| -(1.0 - r.unit()).ln(),
            std::f64::consts::LN_2,
        ),
    ]
}

#[test]
fn rank_intervals_have_their_binomial_confidence() {
    assert_eq!(rank_confidence(3, 1), 0.75);
    assert_eq!(rank_confidence(6, 1), 0.968_75);
    assert_eq!(rank_confidence(10, 2), 1.0 - 2.0 * 11.0 / 1024.0);
    assert_eq!(min_repetitions(0.95), 6);
    let ten: Vec<f64> = (1..=10).map(f64::from).collect();
    let i = median_interval(&ten, 0.95).expect("values");
    assert_eq!((i.low, i.high), (2.0, 9.0));
    let three = median_interval(&[3.0, 1.0, 2.0], 0.95).expect("values");
    assert_eq!((three.low, three.high, three.confidence), (1.0, 3.0, 0.75));
    assert!(!three.is_95());
    // Large n: no underflow, and the interval narrows towards the middle.
    let many: Vec<f64> = (0..5000).map(f64::from).collect();
    let i = median_interval(&many, 0.95).expect("values");
    assert!(i.confidence >= 0.95 && i.confidence < 0.96, "{i:?}");
    assert!(i.low > 2400.0 && i.high < 2600.0, "{i:?}");
}

/// The rank interval is exact for any continuous distribution: over many samples it
/// holds the true median as often as its confidence says.
#[test]
fn rank_intervals_cover_the_median_as_often_as_they_claim() {
    let trials = 20_000u32;
    let mut rng = Rng::new(1);
    for (name, draw, true_median) in distributions() {
        for n in [3, 6, 10, 25] {
            let mut confidence = 0.0;
            let mut covered = 0u32;
            for _ in 0..trials {
                let v: Vec<f64> = (0..n).map(|_| draw(&mut rng)).collect();
                let i = median_interval(&v, 0.95).expect("values");
                confidence = i.confidence;
                covered += u32::from(i.low <= true_median && true_median <= i.high);
            }
            let share = f64::from(covered) / f64::from(trials);
            // 4 standard errors of a share of 20,000 trials at worst (p = 0.5).
            assert!(
                (share - confidence).abs() < 0.015,
                "{name}, n = {n}: covered {share}, claimed {confidence}"
            );
        }
    }
}

/// The bootstrap ratio interval of two lognormal samples whose medians differ by 10%.
/// Measured coverage with these seeds and 2,000 trials: 0.956 at n = 6, 0.963 at
/// n = 10, 0.959 at n = 30.
#[test]
fn ratio_intervals_cover_the_true_ratio() {
    let trials = 2000u32;
    let mut rng = Rng::new(2);
    for n in [6, 10, 30] {
        let mut covered = 0u32;
        for t in 0..trials {
            let a: Vec<f64> = (0..n).map(|_| (0.1 * normal(&mut rng)).exp()).collect();
            let b: Vec<f64> = (0..n)
                .map(|_| 1.1 * (0.1 * normal(&mut rng)).exp())
                .collect();
            let (_, i) = ratio_interval(&a, &b, 0.95, u64::from(t)).expect("values");
            covered += u32::from(i.low <= 1.1 && 1.1 <= i.high);
        }
        let share = f64::from(covered) / f64::from(trials);
        assert!((0.935..=0.975).contains(&share), "n = {n}: covered {share}");
    }
}

#[test]
fn a_ratio_interval_is_reproducible_and_scales_with_the_data() {
    let a = [1.0, 1.1, 0.9, 1.05, 0.95, 1.0];
    let b: Vec<f64> = a.iter().map(|x| 2.0 * x).collect();
    let (r, i) = ratio_interval(&a, &b, 0.95, 7).expect("values");
    assert!((r - 2.0).abs() < 1e-12);
    assert_eq!(ratio_interval(&a, &b, 0.95, 7).expect("values").1, i);
    assert!(i.low <= 2.0 && 2.0 <= i.high, "{i:?}");
    // Swapping the sides inverts the ratio.
    let (r2, _) = ratio_interval(&b, &a, 0.95, 7).expect("values");
    assert!((r2 - 0.5).abs() < 1e-12);
    assert!(ratio_interval(&[0.0, 1.0], &b, 0.95, 7).is_none());
    assert!(ratio_interval(&[], &b, 0.95, 7).is_none());
}

/// The repetitions estimated for ±1% of a normal sample with σ = 5% of the median are
/// about (1.96 · 1.2533 · 5)² = 151; at that count the rank interval's half-width is
/// about 1% indeed.
#[test]
fn the_repetitions_for_a_target_width_reach_it() {
    let mut rng = Rng::new(3);
    let pilot: Vec<f64> = (0..400).map(|_| 1.0 + 0.05 * normal(&mut rng)).collect();
    let n = repetitions_for(&pilot, 0.01).expect("estimate");
    assert!((120..=185).contains(&n), "{n}");
    let trials = 400;
    let mut half = 0.0;
    for _ in 0..trials {
        let v: Vec<f64> = (0..n).map(|_| 1.0 + 0.05 * normal(&mut rng)).collect();
        let i = median_interval(&v, 0.95).expect("values");
        half += (i.high - i.low) / 2.0;
    }
    let half = half / f64::from(trials);
    assert!(
        (0.008..=0.0125).contains(&half),
        "mean half-width {half} at n = {n}"
    );
    // No spread: the fewest repetitions with a 95% interval.
    assert_eq!(repetitions_for(&[2.0, 2.0, 2.0], 0.01), Some(6));
    assert_eq!(repetitions_for(&[2.0, 2.0], 0.01), None);
}

#[test]
fn an_interleaved_schedule_runs_every_repetition_in_abba_rounds() {
    let reps = [6, 6, 3, 1];
    let slots = schedule(&reps, Some(42));
    for (case, &n) in reps.iter().enumerate() {
        let mine: Vec<u32> = slots
            .iter()
            .filter(|s| s.case == case)
            .map(|s| s.repetition)
            .collect();
        assert_eq!(mine, (1..=n).collect::<Vec<_>>(), "case {case}");
    }
    let round = |r: u32| -> Vec<usize> {
        let mut v: Vec<_> = slots.iter().filter(|s| s.round == Some(r)).collect();
        v.sort_by_key(|s| s.position);
        assert_eq!(
            v.iter()
                .map(|s| s.position.expect("set"))
                .collect::<Vec<_>>(),
            (1..=u32::try_from(v.len()).expect("small")).collect::<Vec<_>>()
        );
        v.iter().map(|s| s.case).collect()
    };
    for r in [2, 4, 6] {
        let mut previous = round(r - 1);
        previous.reverse();
        previous.retain(|&c| reps[c] >= r);
        assert_eq!(round(r), previous, "round {r} reverses round {}", r - 1);
    }
    assert_eq!(schedule(&reps, Some(42)), slots, "seeded");
    let orders: std::collections::BTreeSet<Vec<usize>> = (0..20)
        .map(|seed| {
            schedule(&[2, 2, 2, 2], Some(seed))
                .iter()
                .map(|s| s.case)
                .collect()
        })
        .collect();
    assert!(orders.len() > 5, "seeds give different orders: {orders:?}");
}

#[test]
fn without_a_seed_each_case_runs_back_to_back() {
    let slots = schedule(&[2, 1], None);
    let got: Vec<(usize, u32, Option<u32>)> = slots
        .iter()
        .map(|s| (s.case, s.repetition, s.round))
        .collect();
    assert_eq!(got, vec![(0, 1, None), (0, 2, None), (1, 1, None)]);
}
