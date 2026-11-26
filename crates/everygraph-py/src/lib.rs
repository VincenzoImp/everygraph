// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The native module of the Python package, `everygraph._native` (`PyO3`, abi3; DESIGN
//! D19). What lives here: graphs opened or built for Python (modules `graph`, `read`),
//! the whole-graph algorithms planned and run on them with the interpreter released
//! (`kernels`, through `everygraph_kernels::planner` and `run::execute`), and results
//! handed over as Arrow C streams without copying their numeric columns (module
//! `table`).
//!
//! The Python package (`python/everygraph`) owns the public API: names, arguments and
//! their defaults, text forms, progress lines and exceptions (`docs/api-sketch.md` § 3).
//! This module raises `NativeError` with the class name and the fields of an error
//! document (`docs/ux-contract.md` § 6.1), which the package turns into its exception.

#![forbid(unsafe_code)]

mod error;
mod graph;
mod kernels;
mod read;
mod run;
mod source;
mod table;

use pyo3::prelude::*;

/// The module `everygraph._native`.
#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add("NativeError", m.py().get_type::<error::NativeError>())?;
    m.add_class::<graph::Graph>()?;
    m.add_class::<table::Table>()?;
    m.add_function(wrap_pyfunction!(graph::open, m)?)?;
    m.add_function(wrap_pyfunction!(read::read, m)?)?;
    m.add_function(wrap_pyfunction!(read::inspect, m)?)?;
    m.add_function(wrap_pyfunction!(machine_memory, m)?)?;
    m.add_function(wrap_pyfunction!(default_threads, m)?)?;
    m.add_function(wrap_pyfunction!(job_id, m)?)?;
    m.add_function(wrap_pyfunction!(about, m)?)?;
    Ok(())
}

/// The machine's memory in all and free now, in bytes, or `None` where it cannot be read
/// (`everygraph_io::ProcMemoryProbe`, Linux only).
#[pyfunction]
fn machine_memory() -> Option<(u64, u64)> {
    use everygraph_core::MemoryProbe;
    let memory = everygraph_io::ProcMemoryProbe::new().probe().ok()?;
    Some((memory.total(), memory.available()))
}

/// The 16 hexadecimal digits that name a job (`docs/ux-contract.md` G-19): the one
/// fingerprint of `everygraph_ingest::native::job_id`.
#[pyfunction]
fn job_id(parts: Vec<String>) -> String {
    let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
    everygraph_ingest::native::job_id(&parts)
}

/// A time whose interval is wide, as the step table writes it: "about" and two
/// significant digits (`docs/ux-contract.md` P-8).
#[pyfunction]
fn about(seconds: f64) -> String {
    everygraph_kernels::planner::about(seconds)
}

/// The threads a call uses at most by default: the physical cores this process may use
/// (`docs/api-sketch.md` § 2.4, DESIGN D13); the planner may take fewer.
#[pyfunction]
fn default_threads() -> usize {
    everygraph_io::physical_cores().get()
}
