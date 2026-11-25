// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The plug-in trait and the harness that runs a kernel through generators, references,
//! comparators and invariants, under every configuration it supports.
//!
//! # How a kernel is checked
//!
//! For every graph of the corpus and every kernel the plug-in supports on it:
//!
//! 1. it runs under each of the plug-in's [`Config`]s;
//! 2. **cross-configuration identity**: every result must equal the first one bit for bit
//!    (DESIGN D14: same input, same output, whatever the threads, storage or regime);
//! 3. **in-process reference**: every distinct result is compared with
//!    [`crate::reference`];
//! 4. **Python references**: and with each library's answer in `reference.json`
//!    (NetworkX, python-igraph, scipy), when the corpus has been through `oracle-ref run`;
//! 5. **invariants**: and checked against [`crate::invariants`], which need no reference.
//!
//! Each of the four families of checks is independent of the others: an invariant knows
//! nothing of the references, the references know nothing of the configurations. A
//! [`Finding`] names the check that failed.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::compare::{compare, identical};
use crate::corpus::Entry;
use crate::exchange::References;
use crate::graph::Graph;
use crate::invariants;
use crate::kernel::{Kernel, Output};
use crate::projection::Projection;
use crate::reference;

/// How adjacency is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Storage {
    /// Plain arrays of offsets and targets.
    Raw,
    /// The compressed adjacency codec (DESIGN D4).
    Compressed,
}

/// The execution regime (DESIGN D11).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Regime {
    /// Arrays in memory.
    InMemory,
    /// Arrays memory-mapped from a file.
    Mmap,
    /// Adjacency streamed sequentially from disk, O(n) state in memory.
    Stream,
}

/// One configuration a kernel can run under. Results must not depend on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Config {
    /// Worker threads.
    pub threads: usize,
    /// Target chunk size in adjacency entries, for kernels that cut their work by entries.
    pub chunk_entries: u64,
    /// Storage of the adjacency.
    pub storage: Storage,
    /// Execution regime.
    pub regime: Regime,
}

impl Config {
    /// One thread, in memory, raw storage, chunks of 2^16 entries.
    pub const BASELINE: Self = Self {
        threads: 1,
        chunk_entries: 1 << 16,
        storage: Storage::Raw,
        regime: Regime::InMemory,
    };

    /// Every combination of the given values.
    #[must_use]
    pub fn grid(
        threads: &[usize],
        chunks: &[u64],
        storages: &[Storage],
        regimes: &[Regime],
    ) -> Vec<Self> {
        let mut out = Vec::new();
        for &regime in regimes {
            for &storage in storages {
                for &chunk_entries in chunks {
                    for &threads in threads {
                        out.push(Self {
                            threads,
                            chunk_entries,
                            storage,
                            regime,
                        });
                    }
                }
            }
        }
        out
    }
}

impl fmt::Display for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "threads={} chunk={} storage={:?} regime={:?}",
            self.threads, self.chunk_entries, self.storage, self.regime
        )
    }
}

/// What a plug-in receives for one graph.
#[derive(Clone, Copy, Debug)]
pub struct Case<'a> {
    /// The input events.
    pub graph: &'a Graph,
    /// The analytic graph, as the oracle built it.
    pub projection: &'a Projection,
    /// The graph's directory in exchange form, when the corpus is on disk (external
    /// kernels read it from there).
    pub dir: Option<&'a Path>,
}

/// A kernel implementation under test.
///
/// Implement it for an engine kernel (from #45 on), for a prototype behind an adapter
/// (see [`crate::external`]), or for a deliberately broken toy ([`crate::toy`]).
pub trait KernelUnderTest {
    /// A short name for reports.
    fn name(&self) -> String;

    /// Whether this implementation computes `kernel` on this graph. Unsupported pairs are
    /// skipped and counted, not failed.
    fn supports(&self, kernel: &Kernel, case: &Case<'_>) -> bool;

    /// The configurations to run under; at least one.
    fn configurations(&self) -> Vec<Config>;

    /// Computes `kernel` on `case` under `config`, in canonical form.
    ///
    /// # Errors
    ///
    /// A description of why it could not run; reported as a finding.
    fn run(&self, case: &Case<'_>, kernel: &Kernel, config: &Config) -> Result<Output, String>;
}

/// The check a finding comes from.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Check {
    /// The plug-in returned an error or panicked.
    Run,
    /// Results under two configurations differ.
    CrossConfig,
    /// The result differs from the in-process reference.
    RustReference,
    /// The result differs from a Python library's answer.
    PythonReference(String),
    /// An invariant does not hold.
    Invariant(String),
}

impl Check {
    /// The family the check belongs to: `run`, `cross_config`, `reference` (Rust or
    /// Python) or `invariant`. Two checks of different families are independent.
    #[must_use]
    pub const fn family(&self) -> &'static str {
        match self {
            Self::Run => "run",
            Self::CrossConfig => "cross_config",
            Self::RustReference | Self::PythonReference(_) => "reference",
            Self::Invariant(_) => "invariant",
        }
    }
}

impl fmt::Display for Check {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Run => f.write_str("run"),
            Self::CrossConfig => f.write_str("cross-config"),
            Self::RustReference => f.write_str("reference:rust"),
            Self::PythonReference(lib) => write!(f, "reference:{lib}"),
            Self::Invariant(name) => write!(f, "invariant:{name}"),
        }
    }
}

/// A failed check.
#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    /// Graph name.
    pub graph: String,
    /// Kernel with parameters.
    pub kernel: Kernel,
    /// The configuration involved, if one.
    pub config: Option<Config>,
    /// Which check.
    pub check: Check,
    /// What was found.
    pub detail: String,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {} {}", self.check, self.graph, self.kernel.label())?;
        if let Some(c) = &self.config {
            write!(f, " ({c})")?;
        }
        write!(f, ": {}", self.detail)
    }
}

/// The outcome of a harness run.
#[derive(Clone, Debug, Default)]
pub struct Report {
    /// Graphs seen.
    pub graphs: usize,
    /// (graph, kernel) pairs checked.
    pub checked: usize,
    /// (graph, kernel) pairs the plug-in does not support.
    pub unsupported: usize,
    /// Kernel runs (pairs × configurations).
    pub runs: usize,
    /// Comparisons with a Python library's answer that were made.
    pub python_comparisons: usize,
    /// (graph, kernel) pairs without any Python answer although the kernel is one the
    /// Python side computes.
    pub python_missing: usize,
    /// Failed checks.
    pub findings: Vec<Finding>,
}

impl Report {
    /// Findings counted by check.
    #[must_use]
    pub fn by_check(&self) -> BTreeMap<String, usize> {
        let mut m = BTreeMap::new();
        for f in &self.findings {
            *m.entry(f.check.to_string()).or_insert(0) += 1;
        }
        m
    }

    /// Distinct checks that fired, grouped by family.
    #[must_use]
    pub fn families(&self) -> BTreeMap<&'static str, Vec<String>> {
        let mut m: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();
        for f in &self.findings {
            let v = m.entry(f.check.family()).or_default();
            let s = f.check.to_string();
            if !v.contains(&s) {
                v.push(s);
            }
        }
        m
    }
}

/// A graph ready to be checked: an entry, its projection, and optionally its directory
/// and the Python references.
#[derive(Clone, Debug)]
pub struct Prepared {
    /// The graph and its kernels.
    pub entry: Entry,
    /// Its analytic graph.
    pub projection: Projection,
    /// Its directory in exchange form.
    pub dir: Option<PathBuf>,
    /// The Python references, if computed.
    pub references: Option<References>,
}

impl Prepared {
    /// Prepares an entry that lives only in memory.
    #[must_use]
    pub fn new(entry: Entry) -> Self {
        let projection = Projection::new(&entry.graph);
        Self {
            entry,
            projection,
            dir: None,
            references: None,
        }
    }

    /// Reads a graph directory and its `reference.json`, if any.
    ///
    /// # Errors
    ///
    /// Errors of [`crate::exchange::read_entry`] and [`crate::exchange::read_references`].
    pub fn load(dir: &Path) -> std::io::Result<Self> {
        let entry = crate::exchange::read_entry(dir)?;
        let references = crate::exchange::read_references(dir)?;
        let mut p = Self::new(entry);
        p.dir = Some(dir.to_path_buf());
        p.references = references;
        Ok(p)
    }
}

/// Kernels the Python side computes (`oracle/src/everygraph_oracle_ref/refs.py`).
#[must_use]
pub const fn python_computes(kernel: &Kernel) -> bool {
    !matches!(kernel, Kernel::Adjacency | Kernel::Quotient)
}

/// Options of a harness run.
#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    /// Skip the in-process reference (used when the plug-in *is* that reference).
    pub skip_rust_reference: bool,
    /// Stop collecting findings after this many (0 = no limit).
    pub max_findings: usize,
}

/// Runs `plugin` on every prepared graph.
///
/// # Panics
///
/// If the plug-in declares no configuration.
#[must_use]
pub fn run(plugin: &dyn KernelUnderTest, graphs: &[Prepared], options: Options) -> Report {
    let mut report = Report::default();
    let configs = plugin.configurations();
    assert!(
        !configs.is_empty(),
        "a plug-in needs at least one configuration"
    );
    for prepared in graphs {
        report.graphs += 1;
        let g = &prepared.entry.graph;
        let case = Case {
            graph: g,
            projection: &prepared.projection,
            dir: prepared.dir.as_deref(),
        };
        for kernel in &prepared.entry.kernels {
            if !plugin.supports(kernel, &case) {
                report.unsupported += 1;
                continue;
            }
            report.checked += 1;
            check_one(
                plugin,
                &case,
                prepared,
                kernel,
                &configs,
                options,
                &mut report,
            );
            if options.max_findings > 0 && report.findings.len() >= options.max_findings {
                return report;
            }
        }
    }
    report
}

fn check_one(
    plugin: &dyn KernelUnderTest,
    case: &Case<'_>,
    prepared: &Prepared,
    kernel: &Kernel,
    configs: &[Config],
    options: Options,
    report: &mut Report,
) {
    let g = case.graph;
    let finding = |config: Option<Config>, check: Check, detail: String| Finding {
        graph: g.name.clone(),
        kernel: kernel.clone(),
        config,
        check,
        detail,
    };
    // Every distinct result, in configuration order; the first is the baseline of the
    // cross-configuration check. References and invariants run on each distinct result, so
    // a bug that shows in only one configuration is seen by every family, not only by
    // cross-configuration identity; identical results are not checked twice.
    let mut distinct: Vec<(Config, Output)> = Vec::new();
    for config in configs {
        report.runs += 1;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            plugin.run(case, kernel, config)
        }));
        let out = match result {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => {
                report.findings.push(finding(Some(*config), Check::Run, e));
                continue;
            }
            Err(panic) => {
                let msg = panic
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                    .unwrap_or_default();
                report.findings.push(finding(
                    Some(*config),
                    Check::Run,
                    format!("panicked: {msg}"),
                ));
                continue;
            }
        };
        if let Some((c0, o0)) = distinct.first() {
            if let Err(e) = identical(&out, o0) {
                report.findings.push(finding(
                    Some(*config),
                    Check::CrossConfig,
                    format!("vs ({c0}): {e}"),
                ));
            }
        }
        if !distinct.iter().any(|(_, o)| identical(&out, o).is_ok()) {
            distinct.push((*config, out));
        }
    }
    if distinct.is_empty() {
        return;
    }
    let p = case.projection;
    let want = if options.skip_rust_reference {
        None
    } else {
        reference::compute(g, p, kernel)
    };
    let python = prepared
        .references
        .as_ref()
        .and_then(|r| r.results.iter().find(|(k, _)| k == kernel).map(|(_, v)| v))
        .filter(|values| !values.is_empty());
    if python.is_none()
        && python_computes(kernel)
        && prepared.references.is_some()
        && g.n <= crate::corpus::PYTHON_MAX_NODES
    {
        report.python_missing += 1;
    }
    for (config, out) in &distinct {
        if let Some(want) = &want {
            if let Err(e) = compare(kernel, g.n, out, want) {
                report
                    .findings
                    .push(finding(Some(*config), Check::RustReference, e));
            }
        }
        for (lib, want) in python.into_iter().flatten() {
            report.python_comparisons += 1;
            if let Err(e) = compare(kernel, g.n, out, want) {
                report.findings.push(finding(
                    Some(*config),
                    Check::PythonReference(lib.clone()),
                    e,
                ));
            }
        }
        for v in invariants::check(kernel, p, g.partition.as_deref(), out) {
            report.findings.push(finding(
                Some(*config),
                Check::Invariant(v.invariant.to_owned()),
                v.detail,
            ));
        }
    }
}

/// The in-process reference as a plug-in, so that it can be checked against the Python
/// references and the invariants like any other implementation.
#[derive(Clone, Copy, Debug, Default)]
pub struct RustReference;

impl KernelUnderTest for RustReference {
    fn name(&self) -> String {
        "reference".into()
    }

    fn supports(&self, kernel: &Kernel, case: &Case<'_>) -> bool {
        reference::compute(case.graph, case.projection, kernel).is_some()
    }

    fn configurations(&self) -> Vec<Config> {
        vec![Config::BASELINE]
    }

    fn run(&self, case: &Case<'_>, kernel: &Kernel, _: &Config) -> Result<Output, String> {
        reference::compute(case.graph, case.projection, kernel).ok_or_else(|| "no reference".into())
    }
}
