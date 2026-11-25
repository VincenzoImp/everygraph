// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A kernel in another process: how prototypes (the lab binaries), other languages and,
//! later, the Python binding are put through the oracle without linking them.
//!
//! # Protocol
//!
//! For each (graph, kernel, configuration) the program is started as
//!
//! ```text
//! <program> <fixed args...> --graph <dir> --kernel '<kernel JSON>' --config '<config JSON>'
//! ```
//!
//! where `<dir>` is the graph in exchange form ([`crate::exchange`]), `<kernel JSON>` a
//! [`Kernel`] (for example `{"kernel":"bfs","source":3}`) and `<config JSON>` a
//! [`Config`] (`{"threads":4,"chunk_entries":64,"storage":"raw","regime":"in_memory"}`).
//! It prints one [`Output`] as JSON on standard output (for example `{"ints":[0,1,null]}`)
//! and exits with status 0, or exits non-zero with a message on standard error.

use std::path::PathBuf;
use std::process::Command;

use crate::graph::SelfLoops;
use crate::kernel::{Kernel, Output};
use crate::plugin::{Case, Config, KernelUnderTest};

/// An external kernel program.
#[derive(Clone, Debug)]
pub struct External {
    /// Name for reports.
    pub name: String,
    /// The program.
    pub program: PathBuf,
    /// Arguments placed before the protocol's own.
    pub args: Vec<String>,
    /// Names of the kernels it computes ([`Kernel::name`]).
    pub kernels: Vec<String>,
    /// Whether it accepts directed graphs.
    pub directed: bool,
    /// Whether it implements kept self-loops.
    pub keeps_self_loops: bool,
    /// Whether it implements the weighted variants (weighted PageRank; weighted distances
    /// are a kernel of their own and are listed by name).
    pub weighted: bool,
    /// The configurations to run it under.
    pub configs: Vec<Config>,
}

impl KernelUnderTest for External {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn supports(&self, kernel: &Kernel, case: &Case<'_>) -> bool {
        case.dir.is_some()
            && self.kernels.iter().any(|k| k == kernel.name())
            && (self.directed || !case.graph.directed)
            && (self.keeps_self_loops || case.graph.semantics.self_loops == SelfLoops::Drop)
            && (self.weighted || !matches!(kernel, Kernel::PageRank { weighted: true, .. }))
    }

    fn configurations(&self) -> Vec<Config> {
        self.configs.clone()
    }

    fn run(&self, case: &Case<'_>, kernel: &Kernel, config: &Config) -> Result<Output, String> {
        let dir = case.dir.ok_or("graph is not on disk")?;
        let kernel_json = serde_json::to_string(kernel).map_err(|e| e.to_string())?;
        let config_json = serde_json::to_string(config).map_err(|e| e.to_string())?;
        let out = Command::new(&self.program)
            .args(&self.args)
            .arg("--graph")
            .arg(dir)
            .arg("--kernel")
            .arg(kernel_json)
            .arg("--config")
            .arg(config_json)
            .output()
            .map_err(|e| format!("cannot start {}: {e}", self.program.display()))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(format!(
                "{} exited with {}: {}",
                self.program.display(),
                out.status,
                err.trim()
            ));
        }
        serde_json::from_slice(&out.stdout).map_err(|e| format!("output is not an Output: {e}"))
    }
}
