// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Comparators: exact for integers, canonical relabelling for labels, a documented
//! tolerance for reals (`oracle/tolerances.json`), and bit-for-bit identity for results of
//! the same kernel under different configurations (DESIGN D14).

use std::collections::BTreeMap;
use std::sync::OnceLock;

use crate::kernel::{Kernel, Output};

/// `|a - b| <= abs + abs_over_n / n + rel * |b|`, with `b` the reference.
#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize)]
pub struct Tolerance {
    /// Absolute part.
    pub abs: f64,
    /// Absolute part divided by the number of nodes (for values that scale as `1/n`).
    pub abs_over_n: f64,
    /// Relative part.
    pub rel: f64,
}

impl Tolerance {
    /// Exact equality.
    pub const EXACT: Self = Self {
        abs: 0.0,
        abs_over_n: 0.0,
        rel: 0.0,
    };

    /// The bound for reference value `want` on a graph of `n` nodes.
    #[must_use]
    pub fn bound(&self, want: f64, n: u32) -> f64 {
        let per_n = if n == 0 {
            0.0
        } else {
            self.abs_over_n / f64::from(n)
        };
        self.abs + per_n + self.rel * want.abs()
    }

    /// True if `got` is within the bound of `want`. Two non-finite values are close only
    /// if they are the same value (NaN is never close).
    #[must_use]
    pub fn close(&self, got: f64, want: f64, n: u32) -> bool {
        if !(got.is_finite() && want.is_finite()) {
            return got == want;
        }
        (got - want).abs() <= self.bound(want, n)
    }
}

/// The tolerances read from `oracle/tolerances.json` (the file the Python side reads too).
///
/// # Panics
///
/// If the file shipped with the crate does not parse; a test covers it.
#[must_use]
pub fn tolerances() -> &'static BTreeMap<String, Tolerance> {
    static T: OnceLock<BTreeMap<String, Tolerance>> = OnceLock::new();
    T.get_or_init(|| {
        let raw: BTreeMap<String, serde_json::Value> =
            serde_json::from_str(include_str!("../../../oracle/tolerances.json"))
                .expect("tolerances.json parses");
        raw.into_iter()
            .filter(|(k, _)| !k.starts_with('_'))
            .map(|(k, v)| (k, serde_json::from_value(v).expect("tolerance entry")))
            .collect()
    })
}

/// The tolerance for a named quantity (`"pagerank"`, `"pagerank_sum"`, ...); exact if the
/// name has no entry.
#[must_use]
pub fn tolerance(name: &str) -> Tolerance {
    tolerances().get(name).copied().unwrap_or(Tolerance::EXACT)
}

/// Relabels classes by their smallest member: the canonical form of a partition.
///
/// # Panics
///
/// If there are more than 2^32 nodes.
#[must_use]
pub fn canonical_labels(labels: &[u32]) -> Vec<u32> {
    let mut first: BTreeMap<u32, u32> = BTreeMap::new();
    labels
        .iter()
        .enumerate()
        .map(|(v, &l)| {
            *first
                .entry(l)
                .or_insert_with(|| u32::try_from(v).expect("u32 ids"))
        })
        .collect()
}

const SHOW: usize = 5;

fn describe_positions(bad: &[String], total: usize) -> String {
    let shown: Vec<&str> = bad.iter().take(SHOW).map(String::as_str).collect();
    format!("{} of {total} differ: {}", bad.len(), shown.join("; "))
}

fn len_check(what: &str, got: usize, want: usize) -> Result<(), String> {
    if got == want {
        Ok(())
    } else {
        Err(format!("{what}: length {got}, reference {want}"))
    }
}

/// Compares a result with a reference for `kernel` on a graph of `n` nodes.
///
/// # Errors
///
/// A description of the differences (the first few positions and the count).
pub fn compare(kernel: &Kernel, n: u32, got: &Output, want: &Output) -> Result<(), String> {
    match (got, want) {
        (Output::Ints(a), Output::Ints(b)) => {
            len_check("ints", a.len(), b.len())?;
            let bad: Vec<String> = a
                .iter()
                .zip(b)
                .enumerate()
                .filter(|(_, (x, y))| x != y)
                .map(|(v, (x, y))| format!("node {v}: {x:?} vs {y:?}"))
                .collect();
            if bad.is_empty() {
                Ok(())
            } else {
                Err(describe_positions(&bad, a.len()))
            }
        }
        (Output::Reals(a), Output::Reals(b)) => {
            len_check("reals", a.len(), b.len())?;
            let tol = tolerance(kernel.name());
            let bad: Vec<String> = a
                .iter()
                .zip(b)
                .enumerate()
                .filter(|(_, (x, y))| match (x, y) {
                    (Some(x), Some(y)) => !tol.close(*x, *y, n),
                    (None, None) => false,
                    _ => true,
                })
                .map(|(v, (x, y))| format!("node {v}: {x:?} vs {y:?}"))
                .collect();
            if bad.is_empty() {
                Ok(())
            } else {
                Err(format!(
                    "{} (tolerance {tol:?})",
                    describe_positions(&bad, a.len())
                ))
            }
        }
        (Output::Labels(a), Output::Labels(b)) => {
            len_check("labels", a.len(), b.len())?;
            let (ca, cb) = (canonical_labels(a), canonical_labels(b));
            let bad: Vec<String> = ca
                .iter()
                .zip(&cb)
                .enumerate()
                .filter(|(_, (x, y))| x != y)
                .map(|(v, (x, y))| format!("node {v}: class of {x} vs class of {y}"))
                .collect();
            if bad.is_empty() {
                Ok(())
            } else {
                Err(describe_positions(&bad, a.len()))
            }
        }
        (Output::Count(a), Output::Count(b)) => {
            if a == b {
                Ok(())
            } else {
                Err(format!("count {a}, reference {b}"))
            }
        }
        (Output::Real(a), Output::Real(b)) => {
            let tol = tolerance(kernel.name());
            if tol.close(*a, *b, n) {
                Ok(())
            } else {
                Err(format!(
                    "{a:e} vs reference {b:e} (difference {:e}, tolerance {tol:?})",
                    (a - b).abs()
                ))
            }
        }
        (
            Output::Csr {
                offsets: oa,
                targets: ta,
            },
            Output::Csr {
                offsets: ob,
                targets: tb,
            },
        ) => {
            len_check("offsets", oa.len(), ob.len())?;
            if oa == ob && ta == tb {
                return Ok(());
            }
            let nodes = oa.len().saturating_sub(1);
            let list = |o: &[u64], t: &[u32], v: usize| -> Option<Vec<u32>> {
                let (s, e) = (usize::try_from(o[v]).ok()?, usize::try_from(o[v + 1]).ok()?);
                t.get(s..e).map(<[u32]>::to_vec)
            };
            let bad: Vec<String> = (0..nodes)
                .filter_map(|v| {
                    let (x, y) = (list(oa, ta, v), list(ob, tb, v));
                    (x != y).then(|| format!("node {v}: {x:?} vs {y:?}"))
                })
                .collect();
            Err(format!(
                "adjacency differs ({} entries vs {}); {}",
                ta.len(),
                tb.len(),
                describe_positions(&bad, nodes)
            ))
        }
        (
            Output::Quotient {
                sizes: sa,
                edges: ea,
            },
            Output::Quotient {
                sizes: sb,
                edges: eb,
            },
        ) => {
            if sa != sb {
                return Err(format!("community sizes differ: {sa:?} vs {sb:?}"));
            }
            if ea == eb {
                Ok(())
            } else {
                Err(format!(
                    "quotient edges differ: {} vs {} edges",
                    ea.len(),
                    eb.len()
                ))
            }
        }
        (Output::Counts(a), Output::Counts(b)) => {
            let keys: std::collections::BTreeSet<&String> = a.keys().chain(b.keys()).collect();
            let bad: Vec<String> = keys
                .into_iter()
                .filter(|k| a.get(*k) != b.get(*k))
                .map(|k| format!("{k}: {:?} vs {:?}", a.get(k), b.get(k)))
                .collect();
            if bad.is_empty() {
                Ok(())
            } else {
                Err(describe_positions(&bad, a.len()))
            }
        }
        _ => Err(format!(
            "output shape {} where the reference is {}",
            got.shape(),
            want.shape()
        )),
    }
}

/// Bit-for-bit identity of two results (reals compared by their bits, labels as given).
///
/// # Errors
///
/// A description of the first differences.
pub fn identical(a: &Output, b: &Output) -> Result<(), String> {
    let same = match (a, b) {
        (Output::Reals(x), Output::Reals(y)) => {
            x.len() == y.len()
                && x.iter()
                    .zip(y)
                    .all(|(p, q)| p.map(f64::to_bits) == q.map(f64::to_bits))
        }
        (Output::Real(x), Output::Real(y)) => x.to_bits() == y.to_bits(),
        (
            Output::Quotient {
                sizes: sa,
                edges: ea,
            },
            Output::Quotient {
                sizes: sb,
                edges: eb,
            },
        ) => {
            sa == sb
                && ea.len() == eb.len()
                && ea
                    .iter()
                    .zip(eb)
                    .all(|(p, q)| p.0 == q.0 && p.1 == q.1 && p.2.to_bits() == q.2.to_bits())
        }
        _ => a == b,
    };
    if same {
        return Ok(());
    }
    if let (Output::Reals(x), Output::Reals(y)) = (a, b) {
        if x.len() == y.len() {
            let diff: Vec<String> = x
                .iter()
                .zip(y)
                .enumerate()
                .filter(|(_, (p, q))| p.map(f64::to_bits) != q.map(f64::to_bits))
                .map(|(v, (p, q))| format!("node {v}: {p:?} vs {q:?}"))
                .collect();
            return Err(format!(
                "not bit-identical; {}",
                describe_positions(&diff, x.len())
            ));
        }
    }
    let exact = compare(&Kernel::Adjacency, 0, a, b)
        .err()
        .unwrap_or_default();
    Err(format!("not identical; {exact}"))
}

/// The node with the largest number of entries in `offsets` (smallest id on ties).
///
/// # Panics
///
/// If there are more than 2^32 nodes.
#[must_use]
pub fn max_degree_node(offsets: &[u64]) -> Option<u32> {
    let n = offsets.len().checked_sub(1)?;
    (0..n)
        .max_by_key(|&v| (offsets[v + 1] - offsets[v], std::cmp::Reverse(v)))
        .map(|v| u32::try_from(v).expect("u32 ids"))
}

#[cfg(test)]
mod tests {
    use super::{canonical_labels, compare, identical, tolerance};
    use crate::kernel::{Kernel, Output};

    #[test]
    fn tolerances_parse_and_have_reasons() {
        assert!(tolerance("pagerank").rel > 0.0);
        assert_eq!(tolerance("degree"), super::Tolerance::EXACT);
    }

    #[test]
    fn labels_compare_by_classes() {
        assert_eq!(canonical_labels(&[7, 7, 3, 7, 3]), vec![0, 0, 2, 0, 2]);
        let k = Kernel::Wcc;
        assert!(
            compare(
                &k,
                3,
                &Output::Labels(vec![5, 5, 9]),
                &Output::Labels(vec![0, 0, 2])
            )
            .is_ok()
        );
        assert!(
            compare(
                &k,
                3,
                &Output::Labels(vec![5, 9, 9]),
                &Output::Labels(vec![0, 0, 2])
            )
            .is_err()
        );
    }

    #[test]
    fn reals_use_tolerance_but_identity_uses_bits() {
        let k = Kernel::PageRank {
            damping: 0.85,
            weighted: false,
            personalize: Vec::new(),
        };
        let a = Output::Reals(vec![Some(0.5), Some(0.5)]);
        let b = Output::Reals(vec![Some(0.5 + 1e-15), Some(0.5)]);
        assert!(compare(&k, 2, &a, &b).is_ok());
        assert!(identical(&a, &b).is_err());
    }
}
