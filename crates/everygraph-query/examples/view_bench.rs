// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The benchmark of filtered views (#93, ADR 0010, `bench/specs/views.toml`): a kernel
//! on the whole graph, on a view held as a mask, and on the same view materialized, on
//! a CSR directory of the dataset registry (`offsets.u64`, `targets.u32`), raw arrays in
//! the heap, timed inside the process.
//!
//! ```text
//! view_bench --graph DIR --select edges|nodes --keep PERCENT
//!     --form whole|mask|materialized --kernel pr64|bfs|cc --threads T [--reps R]
//! ```
//!
//! An edge view keeps a pair when a hash of its two ends, the same at both ends, falls
//! below `PERCENT` of its range; a node view keeps a node by a hash of its id. The view
//! is built from a per-pair or per-node column of those hashes through `ViewSpec`, as a
//! user's predicate would be. `pr64` is `PageRank` with f64 state, 20 iterations; `bfs`
//! starts at the view's node 0; `cc` is weak components.
//!
//! One JSON line: the time to evaluate the predicate and make the view (`build_seconds`),
//! to materialize it (`materialize_seconds`, 0 for a mask), every repetition of the
//! kernel and the best, the view's nodes, entries and bytes, the peak resident memory,
//! and a hash of the result, which is the same for a mask and the view materialized.

#![forbid(unsafe_code)]
#![allow(
    clippy::print_stdout,
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use std::collections::HashMap;
use std::io::Read;
use std::num::NonZeroUsize;
use std::path::Path;
use std::time::Instant;

use everygraph_core::{Memory, par};
use everygraph_kernels::edge_map::{Csr, Plan, Settings, Source};
use everygraph_kernels::pagerank::PageRank;
use everygraph_kernels::run::{self, Column, Direction, Lanes, Operation, Precision, Weight};
use everygraph_query::{Predicate, Test, Values, ViewSpec};

/// A little-endian array file of `W`-byte words.
fn read_words<const W: usize, T>(path: &Path, from: fn([u8; W]) -> T) -> Vec<T> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|mut f| f.read_to_end(&mut bytes))
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    bytes
        .chunks_exact(W)
        .map(|c| from(c.try_into().expect("W bytes")))
        .collect()
}

/// A hash of `x` into `0..10_000`.
fn bucket(x: u64) -> u32 {
    let z = x
        .wrapping_add(0x9E37_79B9_7F4A_7C15)
        .wrapping_mul(0xBF58_476D_1CE4_E5B9);
    ((z ^ (z >> 31)) % 10_000) as u32
}

fn fnv(words: impl Iterator<Item = u64>) -> u64 {
    words.fold(0xCBF2_9CE4_8422_2325, |h, w| {
        (h ^ w).wrapping_mul(0x0000_0100_0000_01B3)
    })
}

/// The kernel `name` on `source`, timed; its result's hash.
fn kernel<S: Source>(source: &S, name: &str) -> (f64, u64) {
    let op = match name {
        "pr64" => Operation::PageRank {
            params: PageRank {
                tol: 0.0,
                max_iter: 20,
                ..PageRank::default()
            },
            precision: Precision::Full,
            weight: Weight::Pairs,
        },
        "bfs" => Operation::Bfs {
            source: 0,
            max_hops: None,
            direction: Direction::Out,
        },
        "cc" => Operation::Components { strong: false },
        other => panic!("unknown kernel {other}"),
    };
    let memory = Memory::unlimited();
    let start = Instant::now();
    let out = Plan::with(source, &memory, Settings::default(), |p| {
        run::run(&op, &Lanes::out(&p, false), &memory)
    })
    .expect("the kernel");
    let seconds = start.elapsed().as_secs_f64();
    let hash = match &out.columns[0].1 {
        Column::U32 { values, .. } => fnv(values.iter().map(|&x| x.into())),
        Column::F64(values) => fnv(values.iter().map(|x| x.to_bits())),
        Column::F32(values) => fnv(values.iter().map(|x| x.to_bits().into())),
    };
    (seconds, hash)
}

fn main() {
    let mut it = std::env::args().skip(1);
    let mut a = HashMap::new();
    while let (Some(k), Some(v)) = (it.next(), it.next()) {
        a.insert(k.trim_start_matches("--").to_owned(), v);
    }
    let get = |k: &str| {
        a.get(k)
            .unwrap_or_else(|| panic!("--{k} is needed"))
            .clone()
    };
    let dir = Path::new(&a["graph"]).to_owned();
    let (select, form, name) = (get("select"), get("form"), get("kernel"));
    let keep = (get("keep").parse::<f64>().expect("a percentage") * 100.0) as u32;
    let threads = NonZeroUsize::new(get("threads").parse().expect("threads")).expect("threads");
    let reps: usize = a.get("reps").map_or(3, |r| r.parse().expect("reps"));

    let offsets: Vec<u64> = read_words(&dir.join("offsets.u64"), u64::from_le_bytes);
    let targets: Vec<u32> = read_words(&dir.join("targets.u32"), u32::from_le_bytes);
    let line = par::with_threads(threads, || {
        let csr = Csr::new(&offsets, &targets).expect("a CSR directory");
        let n = csr.nodes();
        if form == "whole" {
            let runs: Vec<_> = (0..reps).map(|_| kernel(&csr, &name)).collect();
            return report(&runs, (0.0, 0.0), (n, csr.entries(), 0));
        }
        let start = Instant::now();
        let column: Vec<u32> = match select.as_str() {
            "edges" => {
                let mut column = vec![0; targets.len()];
                let grain = NonZeroUsize::new(1024).expect("non-zero");
                par::for_each_range_mut(&mut column[..], &offsets, grain, |u, out| {
                    let u = u as u32;
                    let list = &targets[offsets[u as usize] as usize..][..out.len()];
                    for (b, &v) in out.iter_mut().zip(list) {
                        *b = bucket(u64::from(u.min(v)) << 32 | u64::from(u.max(v)));
                    }
                });
                column
            }
            "nodes" => (0..n).map(|v| bucket(v.into())).collect(),
            other => panic!("unknown selection {other}"),
        };
        let column_seconds = start.elapsed().as_secs_f64();
        let memory = Memory::unlimited();
        let test = Predicate::new(Values::U32(&column), Test::Lt(keep.into()));
        let spec = match select.as_str() {
            "edges" => ViewSpec::new().pairs(test),
            _ => ViewSpec::new().nodes(test),
        };
        let start = Instant::now();
        let mut view = spec
            .build(&csr, &memory, Settings::default())
            .expect("the view");
        let build = start.elapsed().as_secs_f64();
        let start = Instant::now();
        if form == "materialized" {
            view.hold(u32::MAX, &memory).expect("the materialized view");
        }
        let materialize = start.elapsed().as_secs_f64();
        let v = view.view();
        let runs: Vec<_> = (0..reps).map(|_| kernel(v, &name)).collect();
        let bytes = v.mask_bytes() + v.materialized_bytes();
        let line = report(&runs, (build, materialize), (v.nodes(), v.entries(), bytes));
        format!("{line}, \"column_seconds\": {column_seconds}")
    })
    .expect("a pool");
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let peak: u64 = status
        .lines()
        .find_map(|l| l.strip_prefix("VmHWM:"))
        .and_then(|v| v.split_whitespace().next()?.parse().ok())
        .unwrap_or(0);
    println!(
        "{{\"select\": \"{select}\", \"keep_percent\": {}, \"form\": \"{form}\", \"kernel\": \"{name}\", \
         \"threads\": {threads}, {line}, \"peak_rss_bytes\": {}}}",
        f64::from(keep) / 100.0,
        peak * 1024
    );
}

/// The fields of the JSON line for the kernel's runs, the build and materialize times,
/// and the view's nodes, entries and bytes.
fn report(
    runs: &[(f64, u64)],
    (build, materialize): (f64, f64),
    (n, m, bytes): (u32, u64, u64),
) -> String {
    let seconds: Vec<String> = runs.iter().map(|r| r.0.to_string()).collect();
    let best = runs.iter().map(|r| r.0).fold(f64::INFINITY, f64::min);
    let hash = runs.last().map_or(0, |r| r.1);
    format!(
        "\"build_seconds\": {build}, \"materialize_seconds\": {materialize}, \
         \"seconds_all\": [{}], \"seconds\": {best}, \"nodes\": {n}, \"entries\": {m}, \
         \"view_bytes\": {bytes}, \"hash\": \"{hash:016x}\"",
        seconds.join(", ")
    )
}
