// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Graphs for the search: a [`Recipe`] of components (random edges, hubs, a long chain,
//! many small components, repeated rows, isolated nodes, sparse or text identifiers;
//! Singularity's generator components, `docs/research/sota-testing.md` challenge 3)
//! written as a text edge list. [`FAMILIES`] are the fixed recipes every operation is
//! checked on.

use std::fmt::Write as _;

/// How a graph is made. Its parts are shares of [`n`](Self::n), so that one recipe
/// gives the same kind of graph at every size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recipe {
    /// Name, for reports.
    pub name: &'static str,
    /// Nodes.
    pub n: u32,
    /// Random edges per node, in sixteenths.
    pub degree16: u32,
    /// Hubs: nodes 0, 1, … joined to [`hub_share`](Self::hub_share) of all nodes each.
    pub hubs: u32,
    /// The share of all nodes a hub is joined to, in 1/1024.
    pub hub_share: u32,
    /// The share of nodes on one long path, in 1/1024.
    pub chain_share: u32,
    /// Nodes per component (0: one component); random edges stay inside one.
    pub component: u32,
    /// Times each row is written.
    pub repeats: u32,
    /// The share of nodes that have only a loop row, dropped at ingest, in 1/1024.
    pub isolated_share: u32,
    /// Node `v` is written as `v × stride + 1`.
    pub stride: u64,
    /// Identifiers written as text (`n` and hexadecimal) instead of integers.
    pub text: bool,
    /// Seed of the random edges.
    pub seed: u64,
}

impl Recipe {
    /// The same recipe at `n` nodes.
    #[must_use]
    pub const fn at(self, n: u32) -> Self {
        Self { n, ..self }
    }

    /// The rows of the edge list, in order.
    #[must_use]
    pub fn rows(&self) -> Vec<(u32, u32)> {
        let n = self.n.max(2);
        let share = |s: u32| u32::try_from(u64::from(n) * u64::from(s) / 1024).unwrap_or(n);
        let isolated = share(self.isolated_share).min(n - 2);
        let linked = n - isolated;
        let mut rng = self.seed;
        let mut below =
            |m: u32| u32::try_from(splitmix(&mut rng) % u64::from(m.max(1))).unwrap_or(0);
        let mut rows = Vec::new();
        let random = u64::from(linked) * u64::from(self.degree16) / 16;
        let size = if self.component == 0 {
            linked
        } else {
            self.component.clamp(2, linked)
        };
        for i in 0..random {
            let u = u32::try_from(i % u64::from(linked)).unwrap_or(0);
            let start = u - u % size;
            let v = start + below(size.min(linked - start));
            rows.push((u, v));
        }
        for v in 1..share(self.chain_share).min(linked) {
            rows.push((v - 1, v));
        }
        for h in 0..self.hubs.min(linked) {
            for _ in 0..share(self.hub_share) {
                rows.push((h, below(linked)));
            }
        }
        rows.extend((linked..n).map(|v| (v, v)));
        if rows.is_empty() {
            // A recipe of no parts still makes a graph: an empty input has no dictionary.
            rows.push((0, 1));
        }
        let repeats = self.repeats.max(1) as usize;
        rows.into_iter()
            .flat_map(|r| std::iter::repeat_n(r, repeats))
            .collect()
    }

    /// The edge list as text: one row per line, the two identifiers separated by a tab.
    #[must_use]
    pub fn text(&self) -> Vec<u8> {
        let mut out = String::new();
        for (u, v) in self.rows() {
            let (u, v) = (self.key(u), self.key(v));
            let _ = if self.text {
                writeln!(out, "n{u:x}\tn{v:x}")
            } else {
                writeln!(out, "{u}\t{v}")
            };
        }
        out.into_bytes()
    }

    fn key(&self, v: u32) -> u64 {
        u64::from(v) * self.stride.max(1) + 1
    }
}

const BASE: Recipe = Recipe {
    name: "random",
    n: 1 << 10,
    degree16: 64,
    hubs: 0,
    hub_share: 0,
    chain_share: 0,
    component: 0,
    repeats: 1,
    isolated_share: 0,
    stride: 1,
    text: false,
    seed: 1,
};

/// The fixed families: each stresses another kind of state.
pub const FAMILIES: [Recipe; 8] = [
    BASE,
    Recipe {
        name: "hubs",
        degree16: 16,
        hubs: 4,
        hub_share: 512,
        ..BASE
    },
    Recipe {
        name: "chain",
        degree16: 4,
        chain_share: 1024,
        ..BASE
    },
    Recipe {
        name: "path",
        degree16: 0,
        chain_share: 1024,
        ..BASE
    },
    Recipe {
        name: "small components",
        degree16: 48,
        component: 3,
        ..BASE
    },
    Recipe {
        name: "repeated rows",
        degree16: 32,
        repeats: 3,
        ..BASE
    },
    Recipe {
        name: "isolated nodes",
        isolated_share: 512,
        ..BASE
    },
    Recipe {
        name: "sparse text ids",
        stride: 1 << 40,
        text: true,
        ..BASE
    },
];

/// The `SplitMix64` step.
pub(crate) fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}
