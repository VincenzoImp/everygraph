// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Property-based tests of the oracle itself: generators, the exchange form, the
//! projection, the references and the invariants. An oracle that raises false alarms or
//! misses its own generators' graphs is worse than none.

#![forbid(unsafe_code)]

use everygraph_oracle::corpus::Entry;
use everygraph_oracle::corpus::kernels_for;
use everygraph_oracle::exchange::{read_entry, write_entry};
use everygraph_oracle::generators::generate;
use everygraph_oracle::invariants;
use everygraph_oracle::plugin::{Options, Prepared, RustReference, run};
use everygraph_oracle::projection::Projection;
use everygraph_oracle::reference;
use everygraph_oracle::strategies::{arb_graph, arb_spec, arb_undirected_graph};
use everygraph_oracle::toy::Toy;
use everygraph_oracle::{Kernel, Output};
use proptest::prelude::*;

/// 96 cases per property on every pull request; the nightly run sets `PROPTEST_CASES`.
fn cases() -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(96)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(cases()))]

    #[test]
    fn generators_are_deterministic_and_valid(spec in arb_spec(60)) {
        let a = generate(&spec);
        prop_assert!(a.validate().is_ok());
        prop_assert_eq!(&a, &generate(&spec));
    }

    #[test]
    fn ingest_counts_add_up_to_the_rows(g in arb_graph(60)) {
        let c = Projection::new(&g).counts;
        prop_assert_eq!(c.self_loop_rows + c.duplicate_rows + c.pairs, c.rows);
    }

    #[test]
    fn exchange_form_round_trips(g in arb_graph(30)) {
        let p = Projection::new(&g);
        let e = Entry { kernels: kernels_for(&g, &p), graph: g };
        let dir = std::env::temp_dir().join(format!(
            "everygraph-oracle-prop-{}-{}",
            std::process::id(),
            e.graph.provenance["spec"]["seed"]
        ));
        write_entry(&dir, &e).expect("write");
        let back = read_entry(&dir).expect("read");
        std::fs::remove_dir_all(&dir).expect("clean up");
        prop_assert_eq!(back, e);
    }

    /// No false alarms: the references satisfy every invariant on every graph.
    #[test]
    fn references_satisfy_every_invariant(g in arb_graph(50)) {
        let p = Projection::new(&g);
        let e = Entry { kernels: kernels_for(&g, &p), graph: g };
        let report = run(&RustReference, &[Prepared::new(e)], Options { skip_rust_reference: true, max_findings: 0 });
        let shown: Vec<String> = report.findings.iter().map(ToString::to_string).collect();
        prop_assert!(report.findings.is_empty(), "{:#?}", shown);
    }

    /// The toy kernel agrees with the references and with itself across configurations.
    #[test]
    fn toy_passes_on_arbitrary_undirected_graphs(g in arb_undirected_graph(50)) {
        let p = Projection::new(&g);
        let e = Entry { kernels: kernels_for(&g, &p), graph: g };
        let report = run(&Toy::default(), &[Prepared::new(e)], Options::default());
        let shown: Vec<String> = report.findings.iter().map(ToString::to_string).collect();
        prop_assert!(report.findings.is_empty(), "{:#?}", shown);
    }

    /// Metamorphic: relabelling the nodes does not change label-free answers.
    #[test]
    fn answers_do_not_depend_on_node_ids(mut spec in arb_spec(50)) {
        spec.decorations.shuffle_ids = false;
        let a = generate(&spec);
        spec.decorations.shuffle_ids = true;
        let b = generate(&spec);
        prop_assume!(a.n == b.n && a.rows() == b.rows());
        let (pa, pb) = (Projection::new(&a), Projection::new(&b));
        // Shuffling consumes random draws only after the structure is built, so `b` is `a`
        // relabelled; the label-free summaries must agree.
        let summary = |g: &everygraph_oracle::graph::Graph, p: &Projection| {
            let mut cores = match reference::compute(g, p, &Kernel::CoreNumber) {
                Some(Output::Ints(v)) => v,
                _ => Vec::new(),
            };
            cores.sort_unstable();
            let tri = reference::compute(g, p, &Kernel::TriangleTotal);
            let mut deg = match reference::compute(g, p, &Kernel::Degree) {
                Some(Output::Ints(v)) => v,
                _ => Vec::new(),
            };
            deg.sort_unstable();
            (cores, tri, deg, p.m())
        };
        prop_assert_eq!(summary(&a, &pa), summary(&b, &pb));
    }
}

#[test]
fn invariants_reject_obviously_wrong_answers() {
    let spec = everygraph_oracle::corpus::specs(everygraph_oracle::corpus::Tier::Ci)
        .into_iter()
        .find(|s| s.name.contains("clique-u"))
        .expect("a clique in the corpus");
    let g = generate(&spec);
    let p = Projection::new(&g);
    let n = p.len();
    let wrong_degrees = Output::Ints(vec![Some(1); n]);
    assert!(!invariants::check(&Kernel::Degree, &p, None, &wrong_degrees).is_empty());
    let wrong_pr = Output::Reals(vec![Some(1.0); n]);
    let pr = Kernel::PageRank {
        damping: 0.85,
        weighted: false,
    };
    assert!(!invariants::check(&pr, &p, None, &wrong_pr).is_empty());
    let split = Output::Labels((0..u32::try_from(n).expect("small")).collect());
    assert!(!invariants::check(&Kernel::Wcc, &p, None, &split).is_empty());
}
