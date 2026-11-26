// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The point-query benchmark of #90, on a CSR directory of the dataset registry
//! (`offsets.u64`, `targets.u32`; `--in DIR` for the in-lists of a directed graph). The
//! graph is read into the heap and encoded there, then every query runs on the raw
//! arrays (`Csr`, the hot tier) and on the compressed lists (`Compressed`, the
//! default), in interleaved rounds, timed inside the process. Prints one JSON line per
//! measurement.
//!
//! ```text
//! pq_bench --graph DIR [--in DIR] [--queries 20000] [--rounds 3] [--threads 16]
//! ```
//!
//! - `neighbors`: random nodes, `limit` 100,000; each call timed (p50, p99).
//! - `common`: a random node and a random neighbour of it; each call timed.
//! - `khop3`: 3 hops from a random node with a `limit` of 10,000; each call timed.
//! - `throughput`: neighbours and 3-hop queries per second on `--threads` threads, one
//!   reader each.
//! - `hub2`: 2 hops from the largest hub with a 1 ms timeout: the time, the rows and the
//!   cut.
//! - `hub_limit`: 2 hops from the largest hub at limits 10^2 … 10^5: the time grows with
//!   the limit, not with the hop.

#![forbid(unsafe_code)]
#![allow(
    clippy::print_stdout,
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a benchmark: files, the clock and threads"
)]

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::path::Path;
use std::time::{Duration, Instant};

use everygraph_core::{Memory, par};
use everygraph_format::adjacency::{Adjacency, AdjacencyIndex, encode};
use everygraph_kernels::edge_map::{Compressed, Csr, Source};
use everygraph_query::{Direction, Graph, Hops, Never};

fn read_words<const W: usize, T>(path: &Path, from: fn([u8; W]) -> T) -> Vec<T> {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    bytes
        .chunks_exact(W)
        .map(|c| from(c.try_into().expect("a word")))
        .collect()
}

/// A deterministic stream of numbers (`SplitMix64`).
struct Mix(u64);

impl Mix {
    fn below(&mut self, n: u64) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        (z ^ (z >> 31)) % n
    }
}

fn percentile(times: &mut [f64], p: f64) -> f64 {
    times.sort_by(f64::total_cmp);
    times[((times.len() - 1) as f64 * p) as usize]
}

fn timed(times: &mut Vec<f64>, f: impl FnOnce() -> usize) -> usize {
    let t = Instant::now();
    let rows = f();
    times.push(t.elapsed().as_secs_f64() * 1e6);
    rows
}

fn graph<'a, S: Source>(out: &'a S, inn: Option<&'a S>) -> Graph<'a, S> {
    match inn {
        Some(_) => Graph::directed(out, inn).expect("a graph"),
        None => Graph::undirected(out),
    }
}

struct Work {
    nodes: Vec<u32>,
    pairs: Vec<(u32, u32)>,
    hub: u32,
}

fn latency<S: Source>(g: &Graph<'_, S>, w: &Work) -> HashMap<&'static str, Vec<f64>> {
    let memory = Memory::unlimited();
    let mut r = g.reader(&memory).expect("a reader");
    let mut out: HashMap<&'static str, Vec<f64>> = HashMap::new();
    let mut check = 0;
    let mut rows = r.rows();
    for &v in &w.nodes {
        check += timed(out.entry("neighbors").or_default(), || {
            r.neighbors_into(&[v], Direction::Out, 100_000, &mut rows)
                .expect("neighbors");
            rows.len()
        });
    }
    for &v in &w.nodes {
        check += timed(out.entry("neighbors_owned").or_default(), || {
            r.neighbors(&[v], Direction::Out, 100_000)
                .expect("neighbors")
                .len()
        });
    }
    for &(a, b) in &w.pairs {
        check += timed(out.entry("common").or_default(), || {
            r.common_neighbors(a, b, 100_000)
                .expect("common")
                .neighbor
                .len()
        });
    }
    let mut hops = Hops::new(&memory);
    for &v in w.nodes.iter().take(w.nodes.len() / 4) {
        check += timed(out.entry("khop3").or_default(), || {
            r.khop_into(v, 3, Direction::Out, 10_000, &Never, &mut hops)
                .expect("khop");
            hops.node.len()
        });
    }
    out.entry("rows").or_default().push(check as f64);
    out
}

/// Queries per second on `threads` threads, one reader and one set of rows each: every
/// node of the work `reps` times for neighbours, once for 3-hop.
fn throughput<S: Source>(g: &Graph<'_, S>, w: &Work, threads: NonZeroUsize) -> (f64, f64) {
    const REPS: usize = 20;
    let memory = Memory::unlimited();
    let run = |hop: bool| {
        let reps = if hop { 1 } else { REPS };
        let grain =
            NonZeroUsize::new(w.nodes.len().div_ceil(4 * threads.get()).max(1)).expect("non-zero");
        let t = Instant::now();
        let rows: Vec<usize> = par::with_threads(threads, || {
            par::map_ranges(w.nodes.len(), grain, |range| {
                let mut r = g.reader(&memory).expect("a reader");
                let (mut rows, mut hops, mut check) = (r.rows(), Hops::new(&memory), 0);
                for _ in 0..reps {
                    for &v in &w.nodes[range.clone()] {
                        if hop {
                            r.khop_into(v, 3, Direction::Out, 10_000, &Never, &mut hops)
                                .expect("khop");
                            check += hops.node.len();
                        } else {
                            r.neighbors_into(&[v], Direction::Out, 100_000, &mut rows)
                                .expect("neighbors");
                            check += rows.len();
                        }
                    }
                }
                check
            })
        })
        .expect("threads");
        let seconds = t.elapsed().as_secs_f64();
        assert!(rows.iter().sum::<usize>() > 0);
        (w.nodes.len() * reps) as f64 / seconds
    };
    (run(false), run(true))
}

fn hubs<S: Source>(g: &Graph<'_, S>, hub: u32) -> Vec<String> {
    let memory = Memory::unlimited();
    let mut r = g.reader(&memory).expect("a reader");
    let mut lines = Vec::new();
    let start = Instant::now();
    let budget = Duration::from_millis(1);
    let deadline = || start.elapsed() > budget;
    let h = r
        .khop(hub, 2, Direction::Out, u64::MAX, &deadline)
        .expect("khop");
    lines.push(format!(
        "\"hub2\": {{\"us\": {:.1}, \"rows\": {}, \"cut\": \"{:?}\"}}",
        start.elapsed().as_secs_f64() * 1e6,
        h.node.len(),
        h.truncated.map(|t| t.by)
    ));
    for limit in [100, 1_000, 10_000, 100_000] {
        let mut times = Vec::new();
        let mut hop = None;
        for _ in 0..5 {
            timed(&mut times, || {
                let h = r.khop(hub, 2, Direction::Out, limit, &Never).expect("khop");
                hop = h.truncated.and_then(|t| t.hop);
                h.node.len()
            });
        }
        lines.push(format!(
            "\"hub_limit_{limit}\": {{\"us\": {:.1}, \"stopped_in_hop\": {hop:?}}}",
            percentile(&mut times, 0.5)
        ));
    }
    lines
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let dir = arg("--graph").expect("--graph DIR");
    let queries: usize = arg("--queries").map_or(20_000, |q| q.parse().expect("a count"));
    let rounds: usize = arg("--rounds").map_or(3, |q| q.parse().expect("a count"));
    let threads = NonZeroUsize::new(arg("--threads").map_or(16, |q| q.parse().expect("a count")))
        .expect("threads");
    let load = |d: &str| {
        let d = Path::new(d);
        (
            read_words(&d.join("offsets.u64"), u64::from_le_bytes),
            read_words(&d.join("targets.u32"), u32::from_le_bytes),
        )
    };
    let (offsets, targets) = load(&dir);
    let inn = arg("--in").map(|d| load(&d));
    let encoded = encode(&offsets, &targets).expect("encode");
    let index = AdjacencyIndex::parse(&encoded.index).expect("an index");
    let in_encoded = inn.as_ref().map(|(o, t)| encode(o, t).expect("encode"));
    let in_index = in_encoded
        .as_ref()
        .map(|e| AdjacencyIndex::parse(&e.index).expect("an index"));

    let raw = Csr::new(&offsets, &targets).expect("a CSR");
    let raw_in = inn.as_ref().map(|(o, t)| Csr::new(o, t).expect("a CSR"));
    let packed = Compressed::new(Adjacency::new(&index, &encoded.data).expect("data"));
    let packed_in = in_index
        .as_ref()
        .zip(in_encoded.as_ref())
        .map(|(i, e)| Compressed::new(Adjacency::new(i, &e.data).expect("data")));
    let raw_g = graph(&raw, raw_in.as_ref());
    let packed_g = graph(&packed, packed_in.as_ref());

    let n = offsets.len() as u64 - 1;
    let mut mix = Mix(90);
    let nodes: Vec<u32> = (0..queries).map(|_| mix.below(n) as u32).collect();
    let pairs = nodes
        .iter()
        .filter_map(|&v| {
            let (a, b) = (offsets[v as usize], offsets[v as usize + 1]);
            (b > a).then(|| (v, targets[(a + mix.below(b - a)) as usize]))
        })
        .collect();
    let hub = (0..n as usize)
        .max_by_key(|&v| offsets[v + 1] - offsets[v])
        .unwrap_or(0) as u32;
    let work = Work { nodes, pairs, hub };
    println!(
        "{{\"graph\": \"{dir}\", \"nodes\": {n}, \"entries\": {}, \"hub_degree\": {}, \"data_bytes\": {}}}",
        targets.len(),
        offsets[hub as usize + 1] - offsets[hub as usize],
        encoded.data.len()
    );
    for round in 0..rounds {
        measure("raw", &raw_g, &work, threads, round);
        measure("compressed", &packed_g, &work, threads, round);
    }
}

fn measure<S: Source>(name: &str, g: &Graph<'_, S>, w: &Work, threads: NonZeroUsize, round: usize) {
    let mut l = latency(g, w);
    let mut stats = Vec::new();
    for q in ["neighbors", "neighbors_owned", "common", "khop3"] {
        let t = l.get_mut(q).expect("times");
        let mean = t.iter().sum::<f64>() / t.len() as f64;
        stats.push(format!(
            "\"{q}\": {{\"p50_us\": {:.3}, \"p99_us\": {:.3}, \"mean_us\": {mean:.3}}}",
            percentile(t, 0.5),
            percentile(t, 0.99)
        ));
    }
    let (nq, kq) = throughput(g, w, threads);
    let hub_lines = hubs(g, w.hub);
    println!(
        "{{\"round\": {round}, \"source\": \"{name}\", {}, \"threads\": {threads}, \"neighbors_per_s\": {nq:.0}, \"khop3_per_s\": {kq:.0}, {}, \"rows\": {}}}",
        stats.join(", "),
        hub_lines.join(", "),
        l["rows"][0]
    );
}
