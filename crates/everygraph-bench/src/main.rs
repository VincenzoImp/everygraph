// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The `everygraph-bench` harness: declarative benchmark specs, the idle-machine check,
//! memory-limited runs and result files. It has no commands until issue #4; until then
//! it says so and exits with a non-zero status, so that no script records its output as
//! a measurement.

#![forbid(unsafe_code)]
#![allow(
    clippy::print_stderr,
    reason = "a command-line program talks to its user on the standard streams"
)]

use std::process::ExitCode;

fn main() -> ExitCode {
    eprintln!(
        "everygraph-bench {}: no commands yet (the benchmark harness is issue #4)",
        env!("CARGO_PKG_VERSION")
    );
    ExitCode::from(2)
}
