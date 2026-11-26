// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The memory bound of every query: at any budget, a query is either refused before it
//! reads a list or it returns the whole answer. It never fails half-way for lack of
//! memory, which would mean that its bound missed something it holds.

#![forbid(unsafe_code)]
#![allow(clippy::cast_possible_truncation, reason = "test code on small graphs")]

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use everygraph_core::{InsufficientBudget, Memory, MemoryAccountant, MemoryKind};
use everygraph_kernels::edge_map::Source;
use everygraph_query::{Direction, Graph, Never, QueryError};
use support::{Fixture, Model, Stored};

mod support;

/// An accountant whose limit the test sets.
#[derive(Default)]
struct Ledger {
    used: AtomicU64,
    limit: AtomicU64,
}

impl MemoryAccountant for Ledger {
    fn try_reserve(
        &self,
        purpose: &'static str,
        kind: MemoryKind,
        bytes: u64,
    ) -> Result<(), InsufficientBudget> {
        if kind == MemoryKind::FileBacked {
            return Ok(());
        }
        let limit = self.limit.load(Ordering::Relaxed);
        self.used
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |u| {
                u.checked_add(bytes).filter(|&t| t <= limit)
            })
            .map(|_| ())
            .map_err(|u| InsufficientBudget::new(purpose, bytes, limit.saturating_sub(u)))
    }

    fn release(&self, _: &'static str, kind: MemoryKind, bytes: u64) {
        if kind != MemoryKind::FileBacked {
            self.used.fetch_sub(bytes, Ordering::Relaxed);
        }
    }

    fn available(&self) -> u64 {
        self.limit
            .load(Ordering::Relaxed)
            .saturating_sub(self.used.load(Ordering::Relaxed))
    }
}

/// A graph with a hub (node 0, linked to every node and from every node), repeated
/// pairs and loops. The hub's record, about 60 KB, is longer than a stream reader's
/// usual buffer (its smallest window, 4 KiB, and a batch of 8,192 entries' records).
fn fixture(directed: bool) -> Fixture {
    let n = 48_000;
    let mut rows = Vec::new();
    for v in 0..n {
        rows.push((0, v, 1.0, i64::from(v)));
        rows.push((v, 0, 0.5, 1));
    }
    for v in 0..300 {
        rows.push((v, (v * 7 + 3) % 300, 2.0, 2));
        rows.push((v, (v * 7 + 3) % 300, 2.0, 5));
        if v % 9 == 0 {
            rows.push((v, v, 1.5, 3));
        }
    }
    Fixture::new(Model::new(n, directed, &rows))
}

/// Every query, as one outcome: rows as text, so that answers compare.
fn every_query<S: Source>(g: &Graph<'_, S>, memory: &Memory) -> Vec<Result<String, QueryError>> {
    let Ok(mut r) = g.reader(memory) else {
        return vec![Err(QueryError::Invalid("no reader"))];
    };
    let hubs = [0, 3, 0, 150];
    let mut out = Vec::new();
    for d in [Direction::Out, Direction::In, Direction::Both] {
        out.push(
            r.neighbors(&hubs, d, 200)
                .map(|e| format!("{:?}", e.target)),
        );
        out.push(
            r.khop(3, 3, d, 250, &Never)
                .map(|h| format!("{:?}", h.node)),
        );
    }
    out.push(
        r.degrees(&hubs)
            .map(|d| format!("{:?} {:?}", d.degree, d.weighted)),
    );
    out.push(r.has_edges(&hubs, &[3, 0, 0, 1]).map(|h| format!("{h:?}")));
    out.push(
        r.common_neighbors(0, 3, 100)
            .map(|c| format!("{:?}", c.neighbor)),
    );
    out.push(
        r.edges_between(&[0, 1, 2], &[0, 22, 9], 100)
            .map(|e| format!("{:?}", e.target)),
    );
    out
}

fn sweep<S: Source>(f: &Fixture, out: &S, inn: Option<&S>) {
    let g = f.graph(out, inn);
    let want: Vec<String> = every_query(&g, &Memory::unlimited())
        .into_iter()
        .map(Result::unwrap)
        .collect();
    let (mut refused, mut done) = (0, 0);
    let mut limit = 64_u64;
    for _ in 0..300 {
        limit = limit * 21 / 20;
        let ledger = Arc::new(Ledger::default());
        ledger.limit.store(limit, Ordering::Relaxed);
        let memory = Memory::new(ledger.clone());
        for (i, got) in every_query(&g, &memory).into_iter().enumerate() {
            match got {
                Ok(rows) => {
                    assert_eq!(rows, want[i], "query {i} at a budget of {limit} bytes");
                    done += 1;
                }
                Err(QueryError::Budget(_) | QueryError::Invalid("no reader")) => refused += 1,
                Err(e) => panic!("query {i} at a budget of {limit} bytes failed half-way: {e}"),
            }
        }
        assert_eq!(
            ledger.used.load(Ordering::Relaxed),
            0,
            "a query left memory held"
        );
    }
    assert!(
        refused > 0 && done > 0,
        "the sweep crosses the bound: {refused} {done}"
    );
}

#[test]
fn a_query_is_refused_before_it_reads_or_it_answers_in_full() {
    for directed in [false, true] {
        let f = fixture(directed);
        let inn = f.inn.as_ref();
        sweep(&f, &f.out.heap(), inn.map(Stored::heap).as_ref());
        sweep(&f, &f.out.csr(), inn.map(Stored::csr).as_ref());
        sweep(&f, &f.out.raw, inn.map(|s| &s.raw));
        sweep(&f, &f.out.mapped(), inn.map(Stored::mapped).as_ref());
        sweep(&f, &f.out.stream(), inn.map(Stored::stream).as_ref());
        sweep(&f, &f.out.streamed(), inn.map(Stored::streamed).as_ref());
    }
}

#[test]
fn a_refused_query_reads_nothing() {
    let f = fixture(true);
    let (out, inn) = (f.out.streamed(), f.inn.as_ref().unwrap().streamed());
    let g = f.graph(&out, Some(&inn));
    let ledger = Arc::new(Ledger::default());
    ledger.limit.store(4 << 10, Ordering::Relaxed);
    let mut r = g.reader(&Memory::new(ledger.clone())).unwrap();
    let before = f.out.file.reads.load(Ordering::Relaxed);
    let refused = r.khop(0, 2, Direction::Both, 1 << 20, &Never);
    assert!(matches!(refused, Err(QueryError::Budget(b)) if b.needed > 4 << 10));
    assert_eq!(f.out.file.reads.load(Ordering::Relaxed), before);
    ledger.limit.store(1 << 20, Ordering::Relaxed);
    let answered = r.khop(0, 2, Direction::Out, 10, &Never).unwrap();
    assert_eq!(answered.node.len(), 10);
    assert!(f.out.file.reads.load(Ordering::Relaxed) > before);
}
