// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The match rules of specification v1.0.5 § 2.4.

use std::collections::HashMap;

use super::read::Ids;
use super::{Algorithm, Rule, Values};

/// The relative error the epsilon rule allows.
pub const EPSILON: f64 = 1e-4;

/// The result of one comparison.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    /// The rule applied.
    pub rule: Rule,
    /// Vertices compared.
    pub checked: usize,
    /// Vertices that break the rule.
    pub mismatched: usize,
    /// The first few, as `id: got vs reference`.
    pub examples: Vec<String>,
    /// Largest `|r - s| / |r|` over finite non-zero references (epsilon rule only).
    pub max_rel_error: f64,
}

impl Outcome {
    /// True if no vertex breaks the rule.
    #[must_use]
    pub fn matches(&self) -> bool {
        self.mismatched == 0
    }
}

const SHOW: usize = 5;

/// Compares `got` with the reference `want` under `a`'s rule.
///
/// # Errors
///
/// If the two have different kinds or lengths.
pub fn check(a: Algorithm, got: &Values, want: &Values, ids: &Ids) -> Result<Outcome, String> {
    let mut out = Outcome {
        rule: a.rule(),
        checked: 0,
        mismatched: 0,
        examples: Vec::new(),
        max_rel_error: 0.0,
    };
    let miss = |out: &mut Outcome, v: usize, text: String| {
        out.mismatched += 1;
        if out.examples.len() < SHOW {
            out.examples.push(format!("{}: {text}", ids.ids[v]));
        }
    };
    match (got, want) {
        (Values::Ints(g), Values::Ints(w)) if g.len() == w.len() && a.rule() != Rule::Epsilon => {
            out.checked = g.len();
            let mut fwd: HashMap<u64, u64> = HashMap::new();
            let mut back: HashMap<u64, u64> = HashMap::new();
            for (v, (&s, &r)) in g.iter().zip(w).enumerate() {
                let ok = match a.rule() {
                    Rule::Equivalence => {
                        // A two-way map between labels must exist (§ 2.4).
                        *fwd.entry(s).or_insert(r) == r && *back.entry(r).or_insert(s) == s
                    }
                    _ => s == r,
                };
                if !ok {
                    miss(&mut out, v, format!("{s} vs {r}"));
                }
            }
        }
        (Values::Reals(g), Values::Reals(w)) if g.len() == w.len() && a.rule() == Rule::Epsilon => {
            out.checked = g.len();
            for (v, (&s, &r)) in g.iter().zip(w).enumerate() {
                let ok = if r.is_finite() {
                    (r - s).abs() <= EPSILON * r.abs()
                } else {
                    s == r
                };
                if r.is_finite() && r != 0.0 {
                    out.max_rel_error = out.max_rel_error.max((r - s).abs() / r.abs());
                }
                if !ok {
                    miss(&mut out, v, format!("{s:e} vs {r:e}"));
                }
            }
        }
        _ => {
            return Err(format!(
                "{}: result and reference differ in length, or are not of the kind the algorithm returns",
                a.name()
            ));
        }
    }
    Ok(out)
}
