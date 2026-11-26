// SPDX-License-Identifier: Apache-2.0 OR MIT

//! One ordering of one graph: permute and re-encode (timed), the codec's bits per entry,
//! the log-gap cost and the locality score, and the proof kernels on the original and
//! the permuted graph as interleaved A/B runs in this process.
//!
//! The kernels are the edge-map proof kernels (`everygraph_kernels::proof`), because the
//! production kernels (#218, #220) are not merged. Their answers on the permuted graph
//! are checked against the original's, relabelled: BFS depths and component partitions
//! exactly, PageRank to a stated tolerance.

use std::path::Path;
use std::sync::atomic::AtomicU32;
use std::time::Instant;

use everygraph_core::Memory;
use everygraph_format::adjacency::{Adjacency, AdjacencyIndex, Encoded, encode};
use everygraph_kernels::edge_map::{Compressed, Csr, Plan, Settings, Source};
use everygraph_kernels::proof;
use rayon::prelude::*;

use crate::{Args, Graph, Targets, get, load_as, read_perm};

/// The graph with node `v` renamed `p[v]`, lists sorted.
pub fn permute(g: &Graph, p: &[u32]) -> Graph {
    let n = g.n();
    let mut inv = vec![0u32; n];
    for (v, &w) in p.iter().enumerate() {
        inv[w as usize] = v as u32;
    }
    let mut offsets = vec![0u64; n + 1];
    for w in 0..n {
        offsets[w + 1] = offsets[w] + u64::from(g.degree(inv[w]));
    }
    let mut targets = vec![0u32; g.targets.len()];
    let mut blocks = Vec::new();
    let mut rest: &mut [u32] = &mut targets;
    let mut w = 0;
    while w < n {
        let start = w;
        while w < n && offsets[w] - offsets[start] < 1 << 18 {
            w += 1;
        }
        let len = (offsets[w] - offsets[start]) as usize;
        let (s, tail) = std::mem::take(&mut rest).split_at_mut(len);
        blocks.push((start..w, s));
        rest = tail;
    }
    blocks.into_par_iter().for_each(|(nodes, out)| {
        let mut at = 0;
        for w in nodes {
            let src = g.list(inv[w]);
            let dst = &mut out[at..at + src.len()];
            for (d, &s) in dst.iter_mut().zip(src) {
                *d = p[s as usize];
            }
            dst.sort_unstable();
            at += src.len();
        }
    });
    Graph { offsets, targets: Targets::Heap(targets) }
}

/// Log-gap cost and codec-word statistics of a graph in its current ids.
#[derive(Default, Clone, Copy)]
struct Gaps {
    log2_sum: f64,
    /// Codec words (`w_0 = zigzag(x_0 − v)`, `w_j = x_j − x_{j−1} − 1`) by size class:
    /// zero, then fitting 1, 2, 3 and 4 bytes.
    words: [u64; 5],
    entries: u64,
}

fn gaps(g: &Graph) -> Gaps {
    let n = g.n() as u32;
    (0..n)
        .into_par_iter()
        .with_min_len(1 << 12)
        .fold(Gaps::default, |mut s, v| {
            let mut prev = None;
            for &x in g.list(v) {
                let (gap, word) = match prev {
                    None => {
                        let d = x.wrapping_sub(v) as i32;
                        (u64::from(x.abs_diff(v)), ((d << 1) ^ (d >> 31)) as u32)
                    }
                    Some(p) => (u64::from(x - p), x - p - 1),
                };
                s.log2_sum += (gap.max(1) as f64).log2();
                let class = match word {
                    0 => 0,
                    1..=0xFF => 1,
                    0x100..=0xFFFF => 2,
                    0x1_0000..=0xFF_FFFF => 3,
                    _ => 4,
                };
                s.words[class] += 1;
                s.entries += 1;
                prev = Some(x);
            }
            s
        })
        .reduce(Gaps::default, |mut a, b| {
            a.log2_sum += b.log2_sum;
            for i in 0..5 {
                a.words[i] += b.words[i];
            }
            a.entries += b.entries;
            a
        })
}

fn bits(e: &Encoded, entries: u64) -> (f64, f64) {
    let m = entries as f64;
    (8.0 * (e.index.len() + e.data.len()) as f64 / m, 8.0 * e.data.len() as f64 / m)
}

/// The result of one kernel run, kept to check A against B.
enum Answer {
    Rank(Vec<f32>),
    Labels(Vec<u32>),
}

fn hub<S: Source>(s: &S) -> u32 {
    (0..s.nodes()).max_by_key(|&v| (s.degree(v), std::cmp::Reverse(v))).unwrap_or(0)
}

/// User plus system CPU time of the whole process: unlike wall time, it does not grow
/// while a worker waits for a core that another process holds.
fn cpu_seconds() -> f64 {
    // SAFETY: `getrusage` only writes the struct it is given.
    let mut u: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut u) };
    let tv = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 * 1e-6;
    tv(u.ru_utime) + tv(u.ru_stime)
}

/// One kernel run: (wall seconds, CPU seconds, answer, list entries handed to the kernel's
/// bodies: a measure of its work that does not depend on the load).
fn run<S: Source>(plan: &Plan<'_, '_, S>, memory: &Memory, kernel: &str, source: u32) -> (f64, f64, Answer, u64) {
    let work = plan.work().entries;
    let (t, c) = (Instant::now(), cpu_seconds());
    let stop = || (t.elapsed().as_secs_f64(), cpu_seconds() - c);
    let into = |v: Vec<AtomicU32>| v.into_iter().map(AtomicU32::into_inner).collect();
    let ((w, c), a) = match kernel {
        "pr" => {
            let r = proof::pagerank::<_, f32>(plan, memory, 0.85, 20).expect("pagerank");
            (stop(), Answer::Rank(r.iter().copied().collect()))
        }
        "bfs" => {
            let (d, _held) = proof::bfs(plan, memory, source, true).expect("bfs");
            (stop(), Answer::Labels(into(d)))
        }
        "wcc" => {
            let (l, _held) = proof::wcc(plan, memory).expect("wcc");
            (stop(), Answer::Labels(into(l)))
        }
        other => panic!("unknown kernel {other}"),
    };
    (w, c, a, plan.work().entries - work)
}

/// Whether B's answer is A's relabelled by `p`, as a JSON fragment.
fn check(kernel: &str, a: &Answer, b: &Answer, p: &[u32]) -> String {
    match (a, b) {
        (Answer::Rank(a), Answer::Rank(b)) => {
            let top = a.iter().copied().fold(0.0f32, f32::max);
            let diff = a
                .iter()
                .zip(p)
                .map(|(&x, &w)| (x - b[w as usize]).abs())
                .fold(0.0f32, f32::max);
            format!("\"max_abs_diff_over_max\": {:e}, \"ok\": {}", diff / top, diff / top < 1e-3)
        }
        (Answer::Labels(a), Answer::Labels(b)) if kernel == "bfs" => {
            let ok = a.iter().zip(p).all(|(&x, &w)| b[w as usize] == x);
            let reached = a.iter().filter(|&&d| d != proof::UNREACHED).count();
            format!("\"reached\": {reached}, \"ok\": {ok}")
        }
        (Answer::Labels(a), Answer::Labels(b)) => {
            let mut f = vec![u32::MAX; a.len()];
            let mut ok = true;
            for (v, &x) in a.iter().enumerate() {
                let y = b[p[v] as usize];
                let slot = &mut f[x as usize];
                if *slot == u32::MAX {
                    *slot = y;
                }
                ok &= *slot == y;
            }
            let ca = a.iter().enumerate().filter(|&(v, &l)| l as usize == v).count();
            let cb = b.iter().enumerate().filter(|&(v, &l)| l as usize == v).count();
            format!("\"components\": {ca}, \"ok\": {}", ok && ca == cb)
        }
        _ => unreachable!(),
    }
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    let k = v.len();
    if k % 2 == 1 { v[k / 2] } else { (v[k / 2 - 1] + v[k / 2]) / 2.0 }
}

/// Interleaved runs of each kernel on A and B, alternating which goes first.
fn ab<SA: Source, SB: Source>(
    label: &str,
    a: &SA,
    b: &SB,
    p: &[u32],
    kernels: &[&str],
    reps: usize,
) -> Vec<String> {
    let memory = Memory::unlimited();
    let settings = Settings::default();
    let (src_a, src_b) = (hub(a), p[hub(a) as usize]);
    Plan::with(a, &memory, settings, |pa| {
        Plan::with(b, &memory, settings, |pb| {
            let mut rows = Vec::new();
            for &k in kernels {
                let (mut ta, mut tb, mut ca, mut cb) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
                let mut last = (None, None);
                let mut work = (0, 0);
                // One untimed run of each first: page faults, allocator and caches.
                run(&pa, &memory, k, src_a);
                run(&pb, &memory, k, src_b);
                for rep in 0..reps {
                    for first_a in [rep % 2 == 0, rep % 2 == 1] {
                        if first_a {
                            let (s, c, r, e) = run(&pa, &memory, k, src_a);
                            ta.push(s);
                            ca.push(c);
                            last.0 = Some(r);
                            work.0 = e;
                        } else {
                            let (s, c, r, e) = run(&pb, &memory, k, src_b);
                            tb.push(s);
                            cb.push(c);
                            last.1 = Some(r);
                            work.1 = e;
                        }
                    }
                }
                let checked = check(k, last.0.as_ref().unwrap(), last.1.as_ref().unwrap(), p);
                let fmt = |v: &[f64]| v.iter().map(|x| format!("{x:.4}")).collect::<Vec<_>>().join(", ");
                let (ma, mb) = (median(ta.clone()), median(tb.clone()));
                let (mca, mcb) = (median(ca.clone()), median(cb.clone()));
                rows.push(format!(
                    "{{\"source\": \"{label}\", \"kernel\": \"{k}\", \"a_s\": [{}], \"b_s\": [{}], \
                     \"a_median\": {ma:.4}, \"b_median\": {mb:.4}, \"ratio_b_over_a\": {:.4}, \
                     \"a_cpu_s\": [{}], \"b_cpu_s\": [{}], \"cpu_ratio_b_over_a\": {:.4}, \
                     \"a_entries\": {}, \"b_entries\": {}, {checked}}}",
                    fmt(&ta),
                    fmt(&tb),
                    mb / ma,
                    fmt(&ca),
                    fmt(&cb),
                    mcb / mca,
                    work.0,
                    work.1
                ));
            }
            rows
        })
    })
}

pub fn main(a: &Args, threads: usize) {
    let dir = Path::new(&a["graph"]);
    let name = a.get("name").map_or("?", String::as_str);
    let reps: usize = get(a, "reps", 3);
    let kernels: Vec<&str> = a.get("kernels").map_or("pr,bfs,wcc", String::as_str).split(',').filter(|s| !s.is_empty()).collect();
    let sources: Vec<&str> = a.get("sources").map_or("raw", String::as_str).split(',').filter(|s| !s.is_empty()).collect();
    let t = Instant::now();
    let g = load_as(dir, get(a, "mmap", 0) == 1);
    let load_s = t.elapsed().as_secs_f64();
    let n = g.n();
    let m = g.targets.len() as u64;
    let p: Vec<u32> = match a["perm"].as_str() {
        "identity" => (0..n as u32).collect(),
        path => read_perm(Path::new(path), n),
    };
    let t = Instant::now();
    let h = permute(&g, &p);
    let permute_s = t.elapsed().as_secs_f64();
    let s = gaps(&h);
    let t = Instant::now();
    let eh = encode(&h.offsets, &h.targets).expect("encode");
    let encode_s = t.elapsed().as_secs_f64();
    let (bits_all, bits_data) = bits(&eh, m);
    let mean_degree = m as f64 / n as f64;
    let bound = 2.0 + (n as f64 / mean_degree).log2();
    let loggap = s.log2_sum / s.entries as f64;
    let w = s.words;
    // Stream VByte data bytes per word: 1234 (codec 0) and 0124 (block codec 2, #m1-16);
    // control bits are 2 per word in both.
    let c0 = (w[0] + w[1] + 2 * w[2] + 3 * w[3] + 4 * w[4]) as f64 * 8.0 + 2.0 * m as f64;
    let c2 = (w[1] + 2 * w[2] + 4 * (w[3] + w[4])) as f64 * 8.0 + 2.0 * m as f64;
    let (index_bytes, data_bytes) = (eh.index.len(), eh.data.len());
    let eh = sources.contains(&"compressed").then_some(eh);
    let h = sources.contains(&"raw").then_some(h);
    let mut rows = Vec::new();
    for src in &sources {
        match *src {
            "raw" => {
                let ca = Csr::new(&g.offsets, &g.targets).expect("csr a");
                let h = h.as_ref().unwrap();
                let cb = Csr::new(&h.offsets, &h.targets).expect("csr b");
                rows.extend(ab("raw", &ca, &cb, &p, &kernels, reps));
            }
            "compressed" => {
                let eg = encode(&g.offsets, &g.targets).expect("encode a");
                let eh = eh.as_ref().unwrap();
                let (ia, ib) = (AdjacencyIndex::parse(&eg.index).unwrap(), AdjacencyIndex::parse(&eh.index).unwrap());
                let ca = Compressed::new(Adjacency::new(&ia, &eg.data).unwrap());
                let cb = Compressed::new(Adjacency::new(&ib, &eh.data).unwrap());
                rows.extend(ab("compressed", &ca, &cb, &p, &kernels, reps));
            }
            "none" => {}
            other => panic!("unknown source {other}"),
        }
    }
    println!(
        "{{\"graph\": \"{}\", \"order\": \"{name}\", \"threads\": {threads}, \"n\": {n}, \"entries\": {m}, \
         \"load_s\": {load_s:.3}, \"permute_s\": {permute_s:.3}, \"encode_s\": {encode_s:.3}, \
         \"index_bytes\": {}, \"data_bytes\": {}, \"bits_per_entry\": {bits_all:.4}, \"data_bits_per_entry\": {bits_data:.4}, \
         \"loggap\": {loggap:.4}, \"random_bound\": {bound:.4}, \"locality_ratio\": {:.4}, \
         \"words_zero_1b_2b_3b_4b\": [{}, {}, {}, {}, {}], \"svb1234_data_bits_est\": {:.4}, \"svb0124_data_bits_est\": {:.4}, \
         \"kernels\": [{}]}}",
        dir.file_name().unwrap().to_string_lossy(),
        index_bytes,
        data_bytes,
        bits_all / bound,
        w[0], w[1], w[2], w[3], w[4],
        c0 / m as f64,
        c2 / m as f64,
        rows.join(", "),
    );
}
