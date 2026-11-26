// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Running implementations of the six algorithms on a data set and checking them against
//! its reference outputs.

use std::fmt;
use std::io;
use std::time::Instant;

use super::read::{Csr, Dataset, Ids, LdbcGraph, merge_directions};
use super::validate::{self, Outcome};
use super::{Algorithm, UNREACHABLE, Values, spec};
use crate::graph::Graph;
use crate::projection::Projection;
use crate::{ix, reference};

/// What computes a result.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Implementation {
    /// The algorithms of [`spec`], as the specification defines them.
    Spec,
    /// [`spec::cdlp`] with initial labels in order of first appearance in the edge file:
    /// what label propagation gives if graph order is input order, not id order.
    FirstSeen,
    /// The oracle's own references ([`crate::reference`]), which follow Everygraph's
    /// semantics (`docs/spec/graph-semantics.md`), not LDBC's.
    Oracle,
}

impl Implementation {
    /// Every implementation.
    pub const ALL: [Self; 3] = [Self::Spec, Self::FirstSeen, Self::Oracle];

    /// The name on the command line and in reports.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Spec => "spec",
            Self::FirstSeen => "first-seen",
            Self::Oracle => "oracle",
        }
    }

    /// The implementation named `name`.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|i| i.name() == name)
    }
}

impl fmt::Display for Implementation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// One checked (graph, algorithm, implementation).
#[derive(Clone, Debug)]
pub struct Row {
    /// The data set.
    pub graph: String,
    /// The algorithm.
    pub algorithm: Algorithm,
    /// Who computed the result: an [`Implementation`] name or a caller's own.
    pub implementation: String,
    /// The comparison, or why there was none (`skipped: …` when it does not apply).
    pub outcome: Result<Outcome, String>,
    /// Seconds spent computing the result (exploratory: not a measurement).
    pub seconds: f64,
}

impl Row {
    /// `match`, `mismatch`, `skipped` or `error`.
    #[must_use]
    pub fn status(&self) -> &'static str {
        match &self.outcome {
            Ok(o) if o.matches() => "match",
            Ok(_) => "mismatch",
            Err(e) if e.starts_with("skipped") => "skipped",
            Err(_) => "error",
        }
    }

    /// One line for a report.
    #[must_use]
    pub fn describe(&self) -> String {
        let detail = match &self.outcome {
            Ok(o) => {
                let mut s = format!("{} of {} differ", o.mismatched, o.checked);
                if o.max_rel_error > 0.0 {
                    s += &format!(", max relative error {:.2e}", o.max_rel_error);
                }
                if !o.examples.is_empty() {
                    s += &format!(" (e.g. {})", o.examples.join("; "));
                }
                s
            }
            Err(e) => e.clone(),
        };
        format!(
            "{:<22} {:<5} {:<12} {:<9} {:>8.2}s  {detail}",
            self.graph,
            self.algorithm.name(),
            self.implementation,
            self.status(),
            self.seconds
        )
    }

    /// The row as JSON.
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        let mut v = serde_json::json!({
            "graph": self.graph,
            "algorithm": self.algorithm.name(),
            "implementation": self.implementation,
            "status": self.status(),
            "seconds": self.seconds,
        });
        match &self.outcome {
            Ok(o) => {
                v["rule"] = format!("{:?}", o.rule).to_lowercase().into();
                v["checked"] = o.checked.into();
                v["mismatched"] = o.mismatched.into();
                v["max_rel_error"] = o.max_rel_error.into();
                v["examples"] = o.examples.clone().into();
            }
            Err(e) => v["error"] = e.clone().into(),
        }
        v
    }
}

/// The graph's ids written in place of dense labels.
fn labels_to_ids(g: &LdbcGraph, labels: &[u32]) -> Values {
    Values::Ints(labels.iter().map(|&l| g.ids.ids[ix(l)]).collect())
}

fn source(ds: &Dataset, g: &LdbcGraph, a: Algorithm) -> Result<u32, String> {
    let id = ds.source(a).map_err(|e| e.to_string())?;
    g.ids
        .index(id)
        .ok_or_else(|| format!("source vertex {id} is not a vertex"))
}

/// Runs `a` as the specification defines it.
///
/// # Errors
///
/// If a parameter is missing, or SSSP is asked on an unweighted graph.
pub fn run_spec(
    ds: &Dataset,
    g: &LdbcGraph,
    sym: &Symmetric,
    a: Algorithm,
) -> Result<Values, String> {
    let e = |e: io::Error| e.to_string();
    Ok(match a {
        Algorithm::Bfs => Values::Ints(spec::bfs(g.out.lists(), source(ds, g, a)?)),
        Algorithm::Pr => Values::Reals(spec::pagerank(
            g.out.lists(),
            g.inn(),
            ds.damping().map_err(e)?,
            ds.iterations(a).map_err(e)?,
        )),
        Algorithm::Wcc => labels_to_ids(g, &spec::wcc(sym.lists(g))),
        Algorithm::Cdlp => {
            let identity: Vec<u32> = (0u32..).take(g.ids.len()).collect();
            let inn = g.inn.as_ref().map(Csr::lists);
            labels_to_ids(
                g,
                &spec::cdlp(g.out.lists(), inn, &identity, ds.iterations(a).map_err(e)?),
            )
        }
        Algorithm::Lcc => Values::Reals(spec::lcc(sym.lists(g), sym.dirs())),
        Algorithm::Sssp => Values::Reals(
            spec::sssp(g.out.lists(), source(ds, g, a)?)
                .ok_or("skipped: the graph has no weights")?,
        ),
    })
}

/// CDLP with initial labels in order of first appearance; labels reported as the id of
/// the vertex that started with them.
fn run_first_seen(ds: &Dataset, g: &LdbcGraph) -> Result<Values, String> {
    let iterations = ds.iterations(Algorithm::Cdlp).map_err(|e| e.to_string())?;
    let inn = g.inn.as_ref().map(Csr::lists);
    let labels = spec::cdlp(g.out.lists(), inn, &g.first_seen, iterations);
    let mut owner = vec![0u32; g.first_seen.len()];
    for (v, &r) in g.first_seen.iter().enumerate() {
        owner[ix(r)] = u32::try_from(v).expect("u32 ids");
    }
    let by_owner: Vec<u32> = labels.iter().map(|&l| owner[ix(l)]).collect();
    Ok(labels_to_ids(g, &by_owner))
}

/// The graph as the oracle's events: an undirected edge once.
fn oracle_graph(ds: &Dataset, g: &LdbcGraph) -> Result<Graph, String> {
    let n = u32::try_from(g.ids.len()).map_err(|_| "too many vertices")?;
    let mut og = Graph::with_nodes(&ds.name, n, g.directed);
    let out = g.out.lists();
    let mut weight = Vec::new();
    for u in 0..out.n() {
        for e in out.range(u) {
            let v = out.targets[e];
            if g.directed || ix(v) > u {
                og.src.push(u32::try_from(u).expect("u32 ids"));
                og.dst.push(v);
                if let Some(w) = out.weights {
                    weight.push(w[e]);
                }
            }
        }
    }
    if out.weights.is_some() {
        og.weight = Some(weight);
    }
    og.validate()
        .map_err(|e| format!("skipped: the oracle refuses the graph ({e})"))?;
    Ok(og)
}

/// Runs `a` through the oracle's references.
fn run_oracle(ds: &Dataset, g: &LdbcGraph, p: &Projection, a: Algorithm) -> Result<Values, String> {
    let unreached = |d: Vec<Option<u64>>| d.into_iter().map(|x| x.unwrap_or(UNREACHABLE)).collect();
    Ok(match a {
        Algorithm::Bfs => {
            let s = ix(source(ds, g, a)?);
            Values::Ints(unreached(reference::bfs(&p.out, s).ok_or("bad source")?))
        }
        Algorithm::Pr => Values::Reals(reference::pagerank(
            p,
            ds.damping().map_err(|e| e.to_string())?,
            false,
            &[],
        )),
        Algorithm::Wcc => labels_to_ids(g, &reference::wcc(&p.undirected)),
        Algorithm::Cdlp => return Err("skipped: the oracle has no label propagation".into()),
        Algorithm::Lcc => Values::Reals(reference::clustering(&p.undirected)),
        Algorithm::Sssp => {
            if g.out.weights.is_none() {
                return Err("skipped: the graph has no weights".into());
            }
            let s = ix(source(ds, g, a)?);
            let d = reference::sssp(&p.out, s).ok_or("bad source")?;
            Values::Reals(d.into_iter().map(|x| x.unwrap_or(f64::INFINITY)).collect())
        }
    })
}

/// The symmetric lists WCC and LCC read: the out-lists of an undirected graph, the
/// merged lists of a directed one (with each entry's arcs).
#[derive(Debug)]
pub enum Symmetric {
    /// Undirected: the out-lists are symmetric already.
    Out,
    /// Directed: merged in- and out-lists.
    Merged(Csr, Vec<u8>),
}

impl Symmetric {
    /// Builds what `g` needs.
    #[must_use]
    pub fn of(g: &LdbcGraph) -> Self {
        if g.directed {
            let (c, d) = merge_directions(g.out.lists(), g.inn());
            Self::Merged(c, d)
        } else {
            Self::Out
        }
    }

    /// The lists.
    #[must_use]
    pub fn lists<'a>(&'a self, g: &'a LdbcGraph) -> super::Lists<'a> {
        match self {
            Self::Out => g.out.lists(),
            Self::Merged(c, _) => c.lists(),
        }
    }

    /// The arcs of each entry; `None` on an undirected graph.
    #[must_use]
    pub fn dirs(&self) -> Option<&[u8]> {
        match self {
            Self::Out => None,
            Self::Merged(_, d) => Some(d),
        }
    }
}

/// Checks `got` against the reference output of `a` and makes a row.
#[must_use]
pub fn row(
    ds: &Dataset,
    ids: &Ids,
    a: Algorithm,
    implementation: &str,
    got: Result<Values, String>,
    seconds: f64,
) -> Row {
    let outcome = got.and_then(|v| {
        let want = ds.reference(a, ids).map_err(|e| e.to_string())?;
        validate::check(a, &v, &want, ids)
    });
    Row {
        graph: ds.name.clone(),
        algorithm: a,
        implementation: implementation.to_owned(),
        outcome,
        seconds,
    }
}

/// Runs the chosen implementations of the chosen algorithms (those with a reference
/// output) on a loaded graph. The oracle's references run only up to `oracle_max_rows`
/// edges: their analytic graph costs about 400 bytes per row.
#[must_use]
pub fn run(
    ds: &Dataset,
    g: &LdbcGraph,
    algorithms: &[Algorithm],
    implementations: &[Implementation],
    oracle_max_rows: usize,
) -> Vec<Row> {
    let wanted: Vec<Algorithm> = ds
        .algorithms()
        .into_iter()
        .filter(|a| algorithms.contains(a))
        .collect();
    let sym = Symmetric::of(g);
    let mut rows = Vec::new();
    let oracle = implementations.contains(&Implementation::Oracle).then(|| {
        if g.edges() > oracle_max_rows {
            return Err(format!(
                "skipped: more than {oracle_max_rows} edges for the oracle"
            ));
        }
        oracle_graph(ds, g).map(|og| Projection::new(&og))
    });
    for &a in &wanted {
        for &imp in implementations {
            let t = Instant::now();
            let got = match imp {
                Implementation::Spec => run_spec(ds, g, &sym, a),
                Implementation::FirstSeen if a == Algorithm::Cdlp => run_first_seen(ds, g),
                Implementation::FirstSeen => continue,
                Implementation::Oracle => match &oracle {
                    Some(Ok(p)) => run_oracle(ds, g, p, a),
                    Some(Err(e)) => Err(e.clone()),
                    None => continue,
                },
            };
            let seconds = t.elapsed().as_secs_f64();
            rows.push(row(ds, &g.ids, a, imp.name(), got, seconds));
        }
    }
    rows
}
