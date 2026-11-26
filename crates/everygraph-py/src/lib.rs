// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The native module of the Python package, `everygraph._native` (`PyO3`, abi3; DESIGN
//! D19). What lives here: graphs opened or built for Python ([`graph`], [`read`]),
//! the kernels called on them with the interpreter released, and results handed over
//! as Arrow C streams without copying their numeric columns ([`table`]).
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

/// The threads a call uses by default: the hardware threads this process may use.
#[pyfunction]
fn default_threads() -> usize {
    std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
}
