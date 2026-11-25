// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A toy kernel with switchable, deliberately seeded bugs: the proof that the oracle
//! catches the mistakes chunked, parallel graph code makes.
//!
//! The toy builds an undirected adjacency from the events (two storages: raw arrays and a
//! delta + LEB128 code), cuts it into chunks of about `chunk_entries` entries the way the
//! lab prototype does, and runs degrees, components (min-label propagation), PageRank and
//! a quotient graph over the chunks on `threads` scoped threads. Without a bug its results
//! are identical under every configuration and pass every check.

use std::borrow::Cow;
use std::collections::BTreeMap;

use crate::graph::{Graph, SelfLoops};
use crate::kernel::{Kernel, Output};
use crate::plugin::{Case, Config, KernelUnderTest, Storage};
use crate::{ix, to_u64, to_usize};

/// Nodes per block of a reduction. A constant, not a function of the configuration: a
/// reduction tree that followed the scan's chunks would change the rounding whenever the
/// chunk size (or the storage block size) changes.
pub const REDUCE_GRAIN: usize = 256;

/// A seeded bug.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Bug {
    /// The last list of every chunk is read one entry short.
    OffByOneLastListOfChunk,
    /// Self-loops are dropped although the semantics keep them.
    DropsKeptSelfLoop,
    /// A row `(u, v)` with `u > v` is stored as `u → v` only, not mirrored.
    UnsymmetrizedEdge,
    /// Reductions cut their input into one block per thread, so the order of the
    /// additions depends on the thread count.
    ThreadDependentFloatSum,
    /// Offsets are kept in a counter of `bits` bits that wraps: the 2^32-entry overflow
    /// of a `u32` offset, simulated at 2^`bits` entries.
    OffsetOverflow {
        /// Width of the simulated counter.
        bits: u32,
    },
}

impl Bug {
    /// Every bug the toy knows, with the small-scale overflow at 2^8 entries.
    pub const ALL: [Self; 5] = [
        Self::OffByOneLastListOfChunk,
        Self::DropsKeptSelfLoop,
        Self::UnsymmetrizedEdge,
        Self::ThreadDependentFloatSum,
        Self::OffsetOverflow { bits: 8 },
    ];

    /// A short name.
    #[must_use]
    pub fn name(&self) -> String {
        match self {
            Self::OffByOneLastListOfChunk => "off-by-one-last-list-of-chunk".into(),
            Self::DropsKeptSelfLoop => "drops-kept-self-loop".into(),
            Self::UnsymmetrizedEdge => "unsymmetrized-edge".into(),
            Self::ThreadDependentFloatSum => "thread-dependent-float-sum".into(),
            Self::OffsetOverflow { bits } => format!("offset-overflow-at-2^{bits}"),
        }
    }

    /// Parses [`Bug::name`].
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|b| b.name() == s)
    }
}

/// The toy kernel, optionally with one bug.
#[derive(Clone, Copy, Debug, Default)]
pub struct Toy {
    /// The seeded bug, if any.
    pub bug: Option<Bug>,
}

struct Built {
    offsets: Vec<u64>,
    targets: Vec<u32>,
    loops: Vec<u64>,
    /// Compressed storage: per-node byte offsets and the code.
    code: Option<(Vec<usize>, Vec<u8>)>,
}

impl Built {
    fn list(&self, v: usize) -> Cow<'_, [u32]> {
        if let Some((at, bytes)) = &self.code {
            return Cow::Owned(decode(&bytes[at[v]..at[v + 1]]));
        }
        let (s, e) = (to_usize(self.offsets[v]), to_usize(self.offsets[v + 1]));
        if s <= e && e <= self.targets.len() {
            Cow::Borrowed(&self.targets[s..e])
        } else {
            Cow::Borrowed(&[])
        }
    }

    fn degree(&self, v: usize) -> u64 {
        self.offsets[v + 1].wrapping_sub(self.offsets[v])
    }
}

fn encode(list: &[u32], out: &mut Vec<u8>) {
    let mut prev = 0u32;
    for (i, &x) in list.iter().enumerate() {
        let mut d = if i == 0 { x } else { x - prev };
        prev = x;
        loop {
            let byte = u8::try_from(d & 0x7F).expect("7 bits");
            d >>= 7;
            if d == 0 {
                out.push(byte);
                break;
            }
            out.push(byte | 0x80);
        }
    }
}

fn decode(bytes: &[u8]) -> Vec<u32> {
    let mut out = Vec::new();
    let (mut acc, mut shift, mut prev) = (0u32, 0u32, 0u32);
    for &b in bytes {
        acc |= u32::from(b & 0x7F) << shift;
        if b & 0x80 == 0 {
            prev = if out.is_empty() { acc } else { prev + acc };
            out.push(prev);
            acc = 0;
            shift = 0;
        } else {
            shift += 7;
        }
    }
    out
}

/// Contiguous node ranges holding about `target` entries each (the lab's `edge_chunks`).
fn chunks(offsets: &[u64], target: u64) -> Vec<(usize, usize)> {
    let n = offsets.len() - 1;
    let mut out = Vec::new();
    let mut a = 0;
    while a < n {
        let limit = offsets[a].saturating_add(target.max(1));
        let mut b = a + 1;
        while b < n && offsets[b] < limit {
            b += 1;
        }
        out.push((a, b));
        a = b;
    }
    out
}

impl Toy {
    fn has(&self, bug: Bug) -> bool {
        self.bug == Some(bug)
    }

    fn build(&self, g: &Graph, storage: Storage) -> Built {
        let n = ix(g.n);
        let keep = g.semantics.self_loops == SelfLoops::Keep && !self.has(Bug::DropsKeptSelfLoop);
        let mut loops = vec![0u64; n];
        let mut entries: Vec<(u32, u32)> = Vec::with_capacity(2 * g.rows());
        for (&u, &v) in g.src.iter().zip(&g.dst) {
            if u == v {
                if keep {
                    loops[ix(u)] = 1;
                }
                continue;
            }
            entries.push((u, v));
            if !(self.has(Bug::UnsymmetrizedEdge) && u > v) {
                entries.push((v, u));
            }
        }
        entries.sort_unstable();
        entries.dedup();
        let mask = match self.bug {
            Some(Bug::OffsetOverflow { bits }) => (1u64 << bits) - 1,
            _ => u64::MAX,
        };
        let mut offsets = vec![0u64; n + 1];
        for &(u, _) in &entries {
            offsets[ix(u) + 1] += 1;
        }
        for v in 0..n {
            offsets[v + 1] = offsets[v].wrapping_add(offsets[v + 1]) & mask;
        }
        let targets: Vec<u32> = entries.iter().map(|e| e.1).collect();
        let mut built = Built {
            offsets,
            targets,
            loops,
            code: None,
        };
        if storage == Storage::Compressed {
            let mut at = vec![0usize; n + 1];
            let mut bytes = Vec::new();
            for v in 0..n {
                let list = built.list(v).into_owned();
                encode(&list, &mut bytes);
                at[v + 1] = bytes.len();
            }
            built.code = Some((at, bytes));
        }
        built
    }

    /// Runs `f` on every chunk on `threads` threads; results in chunk order.
    fn per_chunk<T: Send>(threads: usize, n: usize, f: impl Fn(usize) -> T + Sync) -> Vec<T> {
        let threads = threads.clamp(1, n.max(1));
        if threads == 1 {
            return (0..n).map(f).collect();
        }
        let mut slots: Vec<Option<T>> = (0..n).map(|_| None).collect();
        std::thread::scope(|s| {
            let handles: Vec<_> = (0..threads)
                .map(|t| {
                    let f = &f;
                    s.spawn(move || {
                        (t..n)
                            .step_by(threads)
                            .map(|i| (i, f(i)))
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            for h in handles {
                for (i, x) in h.join().expect("toy worker") {
                    slots[i] = Some(x);
                }
            }
        });
        slots
            .into_iter()
            .map(|x| x.expect("every chunk ran"))
            .collect()
    }

    /// Sums one value per node. The reduction tree is fixed by the data alone: partial
    /// sums over blocks of [`REDUCE_GRAIN`] nodes, computed on the threads and added in
    /// block order. The bug is the classic one that `everygraph_core::par` warns about:
    /// a grain of `len / threads`, which moves the block boundaries, and with them the
    /// rounding, whenever the thread count changes.
    fn sum_nodes(&self, threads: usize, values: &[f64]) -> f64 {
        let grain = if self.has(Bug::ThreadDependentFloatSum) {
            values.len().div_ceil(threads.max(1)).max(1)
        } else {
            REDUCE_GRAIN
        };
        let blocks = values.len().div_ceil(grain);
        let block = |i: usize| -> f64 {
            values[i * grain..((i + 1) * grain).min(values.len())]
                .iter()
                .sum()
        };
        Self::per_chunk(threads, blocks, block).iter().sum()
    }

    /// The list of node `v` as seen by the scan of chunk `(a, b)`.
    fn scanned<'a>(&self, b: &'a Built, chunk: (usize, usize), v: usize) -> Cow<'a, [u32]> {
        let list = b.list(v);
        if self.has(Bug::OffByOneLastListOfChunk) && v + 1 == chunk.1 && !list.is_empty() {
            let short = list.len() - 1;
            return Cow::Owned(list[..short].to_vec());
        }
        list
    }

    fn pagerank(&self, b: &Built, cs: &[(usize, usize)], threads: usize, damping: f64) -> Vec<f64> {
        let n = b.offsets.len() - 1;
        if n == 0 {
            return Vec::new();
        }
        let nf = f64::from(u32::try_from(n).expect("u32 ids"));
        let deg: Vec<f64> = (0..n)
            .map(|v| f64::from(u32::try_from(b.degree(v)).unwrap_or(u32::MAX)))
            .collect();
        let mut x = vec![1.0 / nf; n];
        for _ in 0..300 {
            let dangling_mass: Vec<f64> = (0..n)
                .map(|v| if deg[v] == 0.0 { x[v] } else { 0.0 })
                .collect();
            let dangling = self.sum_nodes(threads, &dangling_mass);
            let base = (1.0 - damping) / nf + damping * dangling / nf;
            let next: Vec<Vec<f64>> = Self::per_chunk(threads, cs.len(), |c| {
                (cs[c].0..cs[c].1)
                    .map(|v| {
                        let s: f64 = self
                            .scanned(b, cs[c], v)
                            .iter()
                            .map(|&u| x[ix(u)] / deg[ix(u)])
                            .sum();
                        base + damping * s
                    })
                    .collect()
            });
            x = next.into_iter().flatten().collect();
        }
        let total = self.sum_nodes(threads, &x);
        x.iter().map(|r| r / total).collect()
    }

    fn components(&self, b: &Built, cs: &[(usize, usize)], threads: usize) -> Vec<u32> {
        let n = b.offsets.len() - 1;
        let mut label: Vec<u32> = (0..n).map(|v| u32::try_from(v).expect("u32 ids")).collect();
        loop {
            let next: Vec<Vec<u32>> = Self::per_chunk(threads, cs.len(), |c| {
                (cs[c].0..cs[c].1)
                    .map(|v| {
                        let list = self.scanned(b, cs[c], v);
                        list.iter().map(|&u| label[ix(u)]).fold(label[v], u32::min)
                    })
                    .collect()
            });
            let next: Vec<u32> = next.into_iter().flatten().collect();
            if next == label {
                return label;
            }
            label = next;
        }
    }
}

impl KernelUnderTest for Toy {
    fn name(&self) -> String {
        self.bug
            .map_or_else(|| "toy".into(), |b| format!("toy+{}", b.name()))
    }

    fn supports(&self, kernel: &Kernel, case: &Case<'_>) -> bool {
        let g = case.graph;
        !g.directed
            && match kernel {
                Kernel::Adjacency | Kernel::Degree | Kernel::Wcc => true,
                Kernel::PageRank { weighted, .. } => {
                    !weighted && g.semantics.self_loops == SelfLoops::Drop
                }
                Kernel::Quotient => {
                    g.partition.is_some() && g.semantics.self_loops == SelfLoops::Drop
                }
                _ => false,
            }
    }

    fn configurations(&self) -> Vec<Config> {
        Config::grid(
            &[1, 4],
            &[u64::from(crate::corpus::ADVERSARIAL_CHUNK), 1000],
            &[Storage::Raw, Storage::Compressed],
            &[crate::plugin::Regime::InMemory],
        )
    }

    fn run(&self, case: &Case<'_>, kernel: &Kernel, config: &Config) -> Result<Output, String> {
        let g = case.graph;
        let b = self.build(g, config.storage);
        let n = ix(g.n);
        let cs = chunks(&b.offsets, config.chunk_entries);
        let threads = config.threads;
        Ok(match kernel {
            Kernel::Adjacency => Output::Csr {
                offsets: b.offsets.clone(),
                targets: (0..n).flat_map(|v| b.list(v).into_owned()).collect(),
            },
            Kernel::Degree => {
                let parts = Self::per_chunk(threads, cs.len(), |c| {
                    (cs[c].0..cs[c].1)
                        .map(|v| Some(to_u64(self.scanned(&b, cs[c], v).len()) + 2 * b.loops[v]))
                        .collect::<Vec<_>>()
                });
                Output::Ints(parts.into_iter().flatten().collect())
            }
            Kernel::Wcc => Output::Labels(self.components(&b, &cs, threads)),
            Kernel::PageRank { damping, .. } => Output::Reals(
                self.pagerank(&b, &cs, threads, *damping)
                    .into_iter()
                    .map(Some)
                    .collect(),
            ),
            Kernel::Quotient => {
                let part = g.partition.as_ref().ok_or("no partition")?;
                let k = part.iter().map(|&c| ix(c) + 1).max().unwrap_or(0);
                let mut sizes = vec![0u64; k];
                for &c in part {
                    sizes[ix(c)] += 1;
                }
                let maps = Self::per_chunk(threads, cs.len(), |c| {
                    let mut m: BTreeMap<(u32, u32), u64> = BTreeMap::new();
                    for v in cs[c].0..cs[c].1 {
                        for &u in self.scanned(&b, cs[c], v).iter() {
                            if ix(u) > v {
                                let (x, y) = (part[v], part[ix(u)]);
                                *m.entry((x.min(y), x.max(y))).or_default() += 1;
                            }
                        }
                    }
                    m
                });
                let mut total: BTreeMap<(u32, u32), u64> = BTreeMap::new();
                for m in maps {
                    for (key, w) in m {
                        *total.entry(key).or_default() += w;
                    }
                }
                Output::Quotient {
                    sizes,
                    edges: total
                        .into_iter()
                        .map(|((a, c), w)| (a, c, crate::exact_f64(w)))
                        .collect(),
                }
            }
            other => return Err(format!("toy does not compute {}", other.name())),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{decode, encode};

    #[test]
    fn code_round_trips() {
        let list = [0, 1, 127, 128, 300, 70_000, u32::MAX];
        let mut bytes = Vec::new();
        encode(&list, &mut bytes);
        assert_eq!(decode(&bytes), list);
    }
}
