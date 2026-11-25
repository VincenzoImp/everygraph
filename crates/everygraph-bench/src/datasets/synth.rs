// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tiny deterministic graphs generated locally, for unit tests. Their output is pinned by
//! SHA-256 in the registry like any downloaded file, so a change to a generator shows up
//! as a checksum failure rather than as silently different test inputs.
//!
//! Output format: two `#` comment lines, then one undirected edge per line as
//! `u<TAB>v` with `u < v`, nodes numbered from 0 — the layout of a SNAP edge list.

use std::collections::HashSet;
use std::io::{self, Write};

use serde::Deserialize;

/// A deterministic generator of a small undirected simple graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Generator {
    /// `0 - 1 - … - (n-1)`.
    Path {
        /// Number of nodes.
        n: u32,
    },
    /// A path closed into a ring (needs `n >= 3`).
    Cycle {
        /// Number of nodes.
        n: u32,
    },
    /// Node 0 joined to every other node.
    Star {
        /// Number of nodes, centre included.
        n: u32,
    },
    /// Every pair of nodes joined.
    Complete {
        /// Number of nodes.
        n: u32,
    },
    /// A `rows × cols` grid with 4-neighbourhoods; node `r * cols + c`.
    Grid {
        /// Number of rows.
        rows: u32,
        /// Number of columns.
        cols: u32,
    },
    /// Two cliques of `k` nodes joined by the single edge `(k-1, k)`: a graph with two
    /// obvious communities.
    Barbell {
        /// Size of each clique.
        k: u32,
    },
    /// `m` distinct edges drawn uniformly among `n` nodes without self-loops (G(n, m)),
    /// from a `SplitMix64` stream seeded with `seed`.
    Gnm {
        /// Number of nodes.
        n: u32,
        /// Number of edges; at most `n (n - 1) / 2`.
        m: u64,
        /// Seed of the random stream.
        seed: u64,
    },
}

impl Generator {
    /// Number of nodes of the generated graph.
    #[must_use]
    pub fn nodes(self) -> u64 {
        match self {
            Generator::Path { n }
            | Generator::Cycle { n }
            | Generator::Star { n }
            | Generator::Complete { n }
            | Generator::Gnm { n, .. } => u64::from(n),
            Generator::Grid { rows, cols } => u64::from(rows) * u64::from(cols),
            Generator::Barbell { k } => 2 * u64::from(k),
        }
    }

    /// Number of edges of the generated graph.
    #[must_use]
    pub fn edges(self) -> u64 {
        let pairs = |n: u64| n * n.saturating_sub(1) / 2;
        match self {
            Generator::Path { n } | Generator::Star { n } => u64::from(n.saturating_sub(1)),
            Generator::Cycle { n } => u64::from(n),
            Generator::Complete { n } => pairs(u64::from(n)),
            Generator::Grid { rows, cols } => {
                let (r, c) = (u64::from(rows), u64::from(cols));
                r * c.saturating_sub(1) + c * r.saturating_sub(1)
            }
            Generator::Barbell { k } => 2 * pairs(u64::from(k)) + 1,
            Generator::Gnm { m, .. } => m,
        }
    }

    /// The generator and its parameters as written in the registry, for example
    /// `grid rows=30 cols=30`. Part of the pinned output, so spelled out here rather
    /// than left to `Debug`.
    #[must_use]
    pub fn describe(self) -> String {
        match self {
            Generator::Path { n } => format!("path n={n}"),
            Generator::Cycle { n } => format!("cycle n={n}"),
            Generator::Star { n } => format!("star n={n}"),
            Generator::Complete { n } => format!("complete n={n}"),
            Generator::Grid { rows, cols } => format!("grid rows={rows} cols={cols}"),
            Generator::Barbell { k } => format!("barbell k={k}"),
            Generator::Gnm { n, m, seed } => format!("gnm n={n} m={m} seed={seed}"),
        }
    }

    /// Writes the edge list.
    ///
    /// # Errors
    /// If the parameters describe no valid graph (a cycle of fewer than 3 nodes, more
    /// G(n, m) edges than node pairs, node ids beyond `u32`), or writing fails.
    pub fn write(self, out: &mut dyn Write) -> io::Result<()> {
        writeln!(out, "# everygraph synthetic graph: {}", self.describe())?;
        writeln!(out, "# nodes {} edges {}", self.nodes(), self.edges())?;
        let mut edge = |u: u32, v: u32| writeln!(out, "{}\t{}", u.min(v), u.max(v));
        match self {
            Generator::Path { n } => {
                for v in 1..n {
                    edge(v - 1, v)?;
                }
            }
            Generator::Cycle { n } => {
                if n < 3 {
                    return Err(invalid("a cycle needs at least 3 nodes"));
                }
                for v in 1..n {
                    edge(v - 1, v)?;
                }
                edge(0, n - 1)?;
            }
            Generator::Star { n } => {
                for v in 1..n {
                    edge(0, v)?;
                }
            }
            Generator::Complete { n } => clique(0, n, &mut edge)?,
            Generator::Grid { rows, cols } => {
                if u64::from(rows) * u64::from(cols) > u64::from(u32::MAX) {
                    return Err(invalid("grid too large for u32 node ids"));
                }
                for r in 0..rows {
                    for c in 0..cols {
                        let v = r * cols + c;
                        if c + 1 < cols {
                            edge(v, v + 1)?;
                        }
                        if r + 1 < rows {
                            edge(v, v + cols)?;
                        }
                    }
                }
            }
            Generator::Barbell { k } => {
                let end = k
                    .checked_mul(2)
                    .ok_or_else(|| invalid("barbell too large for u32 node ids"))?;
                clique(0, k, &mut edge)?;
                clique(k, end, &mut edge)?;
                if k > 0 {
                    edge(k - 1, k)?;
                }
            }
            Generator::Gnm { n, m, seed } => {
                if m > (Generator::Complete { n }).edges() {
                    return Err(invalid("more G(n, m) edges than node pairs"));
                }
                let mut rng = SplitMix64(seed);
                let mut seen = HashSet::new();
                while u64::try_from(seen.len()).unwrap_or(u64::MAX) < m {
                    let (u, v) = (rng.below(n), rng.below(n));
                    if u != v && seen.insert((u.min(v), u.max(v))) {
                        edge(u, v)?;
                    }
                }
            }
        }
        Ok(())
    }
}

fn clique(
    start: u32,
    end: u32,
    edge: &mut impl FnMut(u32, u32) -> io::Result<()>,
) -> io::Result<()> {
    for u in start..end {
        for v in u + 1..end {
            edge(u, v)?;
        }
    }
    Ok(())
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

/// `SplitMix64` (Steele, Lea, Flood 2014): tiny, fixed forever, good enough for test
/// graphs. Not `rand`, whose streams may change between releases.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A value in `0..n` (the modulo bias is irrelevant at test sizes). `n > 0`.
    fn below(&mut self, n: u32) -> u32 {
        u32::try_from(self.next() % u64::from(n.max(1))).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generate(g: Generator) -> String {
        let mut out = Vec::new();
        g.write(&mut out).expect("generates");
        String::from_utf8(out).expect("utf-8")
    }

    fn edges(text: &str) -> Vec<(u32, u32)> {
        text.lines()
            .filter(|l| !l.starts_with('#'))
            .map(|l| {
                let (u, v) = l.split_once('\t').expect("tab");
                (u.parse().expect("u"), v.parse().expect("v"))
            })
            .collect()
    }

    const ALL: [Generator; 7] = [
        Generator::Path { n: 10 },
        Generator::Cycle { n: 10 },
        Generator::Star { n: 10 },
        Generator::Complete { n: 10 },
        Generator::Grid { rows: 4, cols: 5 },
        Generator::Barbell { k: 5 },
        Generator::Gnm {
            n: 50,
            m: 200,
            seed: 7,
        },
    ];

    #[test]
    fn each_generator_writes_the_edge_count_it_announces_without_loops_or_duplicates() {
        for g in ALL {
            let e = edges(&generate(g));
            assert_eq!(u64::try_from(e.len()).expect("fits"), g.edges(), "{g:?}");
            let distinct: HashSet<_> = e.iter().copied().collect();
            assert_eq!(distinct.len(), e.len(), "{g:?} repeats an edge");
            for (u, v) in e {
                assert!(u < v, "{g:?}: {u} {v}");
                assert!(u64::from(v) < g.nodes(), "{g:?}: node {v}");
            }
        }
    }

    #[test]
    fn generators_are_deterministic() {
        for g in ALL {
            assert_eq!(generate(g), generate(g));
        }
        let other = Generator::Gnm {
            n: 50,
            m: 200,
            seed: 8,
        };
        assert_ne!(generate(ALL[6]), generate(other));
    }

    #[test]
    fn splitmix64_matches_its_reference_values() {
        // First outputs of the reference C implementation
        // (https://prng.di.unimi.it/splitmix64.c) for seeds 0 and 1234567.
        let mut r = SplitMix64(0);
        assert_eq!(r.next(), 0xE220_A839_7B1D_CDAF);
        assert_eq!(r.next(), 0x6E78_9E6A_A1B9_65F4);
        let mut r = SplitMix64(1_234_567);
        assert_eq!(r.next(), 6_457_827_717_110_365_317);
        assert_eq!(r.next(), 3_203_168_211_198_807_973);
    }

    #[test]
    fn impossible_parameters_are_errors() {
        let mut sink = Vec::new();
        assert!(Generator::Cycle { n: 2 }.write(&mut sink).is_err());
        assert!(
            Generator::Gnm {
                n: 4,
                m: 7,
                seed: 0
            }
            .write(&mut sink)
            .is_err()
        );
    }
}
