// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The oracle catches seeded bugs (issue #6, acceptance: the off-by-one in the last list
//! of a block is caught by at least two independent checks).
//!
//! Runs on the CI corpus in memory, without the Python references. With
//! `EVERYGRAPH_ORACLE_CORPUS` pointing at a corpus that `oracle-ref run` has filled, the
//! same test also requires the Python references to catch what the Rust one catches.

#![forbid(unsafe_code)]
#![allow(clippy::print_stdout, reason = "the matrix is printed for the record")]

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::OnceLock;

use everygraph_oracle::corpus::{Tier, entries};
use everygraph_oracle::exchange::list_corpus;
use everygraph_oracle::plugin::{Options, Prepared, Report, run};
use everygraph_oracle::toy::{Bug, Toy};

fn graphs() -> &'static (Vec<Prepared>, bool) {
    static G: OnceLock<(Vec<Prepared>, bool)> = OnceLock::new();
    G.get_or_init(|| match std::env::var_os("EVERYGRAPH_ORACLE_CORPUS") {
        Some(dir) => {
            let dirs = list_corpus(Path::new(&dir)).expect("corpus directory");
            let g: Vec<Prepared> = dirs
                .iter()
                .map(|d| Prepared::load(d).expect("graph"))
                .collect();
            let with_python = g.iter().all(|p| p.references.is_some());
            (g, with_python)
        }
        None => (
            entries(Tier::Ci).into_iter().map(Prepared::new).collect(),
            false,
        ),
    })
}

fn checks(report: &Report) -> BTreeSet<String> {
    report
        .findings
        .iter()
        .map(|f| f.check.to_string())
        .collect()
}

fn families(report: &Report) -> BTreeSet<&'static str> {
    report
        .findings
        .iter()
        .map(|f| f.check.family())
        .filter(|&f| f != "run")
        .collect()
}

#[test]
fn the_toy_without_a_bug_passes_every_check() {
    let report = run(&Toy::default(), &graphs().0, Options::default());
    assert!(
        report.checked > 500,
        "only {} pairs checked",
        report.checked
    );
    let shown: Vec<String> = report
        .findings
        .iter()
        .take(5)
        .map(ToString::to_string)
        .collect();
    assert!(report.findings.is_empty(), "{shown:#?}");
}

fn caught(bug: Bug, required: &[&str], min_families: usize) {
    let (g, with_python) = graphs();
    let report = run(&Toy { bug: Some(bug) }, g, Options::default());
    let fired = checks(&report);
    println!(
        "{}: {:?} (families {:?})",
        bug.name(),
        fired,
        families(&report)
    );
    for check in required {
        assert!(
            fired.contains(*check),
            "{} not caught by {check}; fired: {fired:?}",
            bug.name()
        );
    }
    if *with_python && fired.contains("reference:rust") {
        for lib in ["reference:networkx", "reference:igraph"] {
            assert!(
                fired.contains(lib),
                "{}: Rust reference fired but {lib} did not",
                bug.name()
            );
        }
    }
    assert!(
        families(&report).len() >= min_families,
        "{}: caught by {:?}, need {min_families} independent families",
        bug.name(),
        families(&report)
    );
}

#[test]
fn off_by_one_in_the_last_list_of_a_chunk_is_caught_by_three_independent_checks() {
    caught(
        Bug::OffByOneLastListOfChunk,
        &["invariant:degree_sum", "reference:rust", "cross-config"],
        3,
    );
}

#[test]
fn dropped_self_loop_is_caught() {
    caught(
        Bug::DropsKeptSelfLoop,
        &["invariant:degree_sum", "reference:rust"],
        2,
    );
}

#[test]
fn unsymmetrized_edge_is_caught() {
    caught(
        Bug::UnsymmetrizedEdge,
        &[
            "invariant:adjacency_symmetric",
            "invariant:degree_sum",
            "reference:rust",
        ],
        2,
    );
}

#[test]
fn thread_dependent_float_sum_is_caught_by_cross_config_identity() {
    // The drift is a few ulps, far inside every reference tolerance: only the bit-for-bit
    // comparison across configurations can see it, which is why that check exists.
    caught(Bug::ThreadDependentFloatSum, &["cross-config"], 1);
}

#[test]
fn offset_overflow_is_caught() {
    caught(
        Bug::OffsetOverflow { bits: 8 },
        &[
            "invariant:offsets_monotone",
            "invariant:degree_sum",
            "reference:rust",
        ],
        2,
    );
}
