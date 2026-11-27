// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The point queries through the correctness oracle: `neighbors` of every node give the
//! adjacency, `degrees` the three degrees, and `khop` without a limit the BFS distances.
//! Each runs on the CI corpus on five sources (raw arrays, the compressed lists in the
//! heap and memory-mapped, raw targets and the compressed lists streamed), whose answers
//! must be identical, and against the in-process references and the invariants.
//!
//! With `EVERYGRAPH_ORACLE_CORPUS` set to a corpus directory that `oracle-ref run` has
//! filled (`oracle/run-ci.sh` does), the graphs are read from there and the answers are
//! compared with NetworkX (the adjacency, degrees and distances), python-igraph and
//! scipy as well.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::print_stderr,
    clippy::disallowed_macros,
    clippy::doc_markdown,
    reason = "test code: small graphs, and a summary line"
)]

use everygraph_core::Memory;
use everygraph_kernels::edge_map::Source;
use everygraph_oracle::corpus::{Tier, entries};
use everygraph_oracle::plugin::{
    Case, Config, KernelUnderTest, Options, Prepared, Regime, Storage, run,
};
use everygraph_oracle::projection::{Adjacency as Analytic, Projection};
use everygraph_oracle::{Kernel, Output};
use everygraph_query::{Direction, Graph, Never};
use support::{Lists, Stored};

mod support;

/// One adjacency of the analytic graph, a kept loop written in as the entry `v` of `v`.
fn lists(adj: &Analytic, p: &Projection) -> Lists {
    let mut l = Lists {
        offsets: vec![0],
        ..Lists::default()
    };
    for v in 0..adj.n() {
        let mut list = adj.list(v).to_vec();
        if p.kept_loops(v) == 1 {
            let at = list.partition_point(|&u| (u as usize) < v);
            list.insert(at, v as u32);
        }
        l.targets.extend(&list);
        l.offsets.push(l.targets.len() as u64);
    }
    l
}

fn answer<S: Source>(g: &Graph<'_, S>, n: u32, kernel: &Kernel) -> Result<Output, String> {
    let e = |e: everygraph_query::QueryError| e.to_string();
    let mut r = g.reader(&Memory::unlimited()).map_err(e)?;
    let all: Vec<u32> = (0..n).collect();
    match kernel {
        Kernel::Adjacency => {
            let rows = r.neighbors(&all, Direction::Out, u64::MAX).map_err(e)?;
            let mut offsets = vec![0_u64; n as usize + 1];
            for &v in rows.source.iter() {
                offsets[v as usize + 1] += 1;
            }
            for v in 0..n as usize {
                offsets[v + 1] += offsets[v];
            }
            let targets = rows.target.iter().copied().collect();
            Ok(Output::Csr { offsets, targets })
        }
        Kernel::Degree | Kernel::DegreeOut | Kernel::DegreeIn => {
            let d = r.degrees(&all).map_err(e)?;
            let column = match kernel {
                Kernel::DegreeOut => d.out_degree.ok_or("no out_degree")?,
                Kernel::DegreeIn => d.in_degree.ok_or("no in_degree")?,
                _ => d.degree,
            };
            Ok(Output::Ints(
                column.iter().map(|&x| Some(x.into())).collect(),
            ))
        }
        Kernel::Bfs { source } => {
            let h = r
                .khop(*source, u32::MAX, Direction::Out, u64::MAX, &Never)
                .map_err(e)?;
            let mut d = vec![None; n as usize];
            for (&v, &k) in h.node.iter().zip(h.distance.iter()) {
                d[v as usize] = Some(k.into());
            }
            Ok(Output::Ints(d))
        }
        _ => Err("unsupported".into()),
    }
}

fn on<S: Source>(
    out: &S,
    inn: Option<&S>,
    directed: bool,
    n: u32,
    k: &Kernel,
) -> Result<Output, String> {
    let g = if directed {
        Graph::directed(out, inn).map_err(|e| e.to_string())?
    } else {
        Graph::undirected(out)
    };
    answer(&g, n, k)
}

struct Queries;

impl KernelUnderTest for Queries {
    fn name(&self) -> String {
        "everygraph-query".into()
    }

    fn supports(&self, kernel: &Kernel, _: &Case<'_>) -> bool {
        matches!(
            kernel,
            Kernel::Adjacency
                | Kernel::Degree
                | Kernel::DegreeOut
                | Kernel::DegreeIn
                | Kernel::Bfs { .. }
        )
    }

    fn configurations(&self) -> Vec<Config> {
        let sources = [
            (Storage::Raw, Regime::InMemory),
            (Storage::Compressed, Regime::InMemory),
            (Storage::Compressed, Regime::Mmap),
            (Storage::Raw, Regime::Stream),
            (Storage::Compressed, Regime::Stream),
        ];
        Config::grid(
            &[1],
            &[1 << 16],
            &[Storage::Raw, Storage::Compressed],
            &[Regime::InMemory, Regime::Mmap, Regime::Stream],
        )
        .into_iter()
        .filter(|c| sources.contains(&(c.storage, c.regime)))
        .collect()
    }

    fn run(&self, case: &Case<'_>, k: &Kernel, config: &Config) -> Result<Output, String> {
        let p = case.projection;
        let out = Stored::new(lists(&p.out, p));
        let inn = p.directed.then(|| Stored::new(lists(&p.inn, p)));
        let (n, directed, inn) = (p.n, p.directed, inn.as_ref());
        match (config.storage, config.regime) {
            (Storage::Raw, Regime::InMemory) => {
                on(&out.csr(), inn.map(Stored::csr).as_ref(), directed, n, k)
            }
            (Storage::Compressed, Regime::Mmap) => on(
                &out.mapped(),
                inn.map(Stored::mapped).as_ref(),
                directed,
                n,
                k,
            ),
            (Storage::Compressed, Regime::Stream) => on(
                &out.streamed(),
                inn.map(Stored::streamed).as_ref(),
                directed,
                n,
                k,
            ),
            (Storage::Compressed, _) => {
                on(&out.heap(), inn.map(Stored::heap).as_ref(), directed, n, k)
            }
            (Storage::Raw, _) => on(
                &out.stream(),
                inn.map(Stored::stream).as_ref(),
                directed,
                n,
                k,
            ),
        }
    }
}

#[test]
fn queries_pass_the_oracle_on_every_source() {
    let (graphs, python) = match std::env::var_os("EVERYGRAPH_ORACLE_CORPUS") {
        Some(dir) => {
            let dirs = everygraph_oracle::exchange::list_corpus(dir.as_ref()).unwrap();
            (
                dirs.iter()
                    .map(|d| Prepared::load(d).unwrap())
                    .collect::<Vec<_>>(),
                true,
            )
        }
        None => (
            entries(Tier::Ci).into_iter().map(Prepared::new).collect(),
            false,
        ),
    };
    let report = run(&Queries, &graphs, Options::default());
    let shown: Vec<String> = report
        .findings
        .iter()
        .take(20)
        .map(ToString::to_string)
        .collect();
    assert!(
        report.findings.is_empty(),
        "{} findings:\n{}",
        report.findings.len(),
        shown.join("\n")
    );
    assert_eq!(report.runs, report.checked * 5);
    if python {
        assert!(
            report.python_comparisons > 0 && report.python_missing == 0,
            "{report:?}"
        );
    }
    eprintln!(
        "{} graphs, {} (graph, kernel) pairs, {} runs, {} Python comparisons, 0 findings",
        report.graphs, report.checked, report.runs, report.python_comparisons
    );
}
