// SPDX-License-Identifier: Apache-2.0 OR MIT
// (temporary measurement branch, not to be merged)

//! The `everygraph` command. It has no subcommands until issue #98; until then it says
//! so and exits with a non-zero status, so that nothing mistakes it for a working tool.

#![forbid(unsafe_code)]
#![allow(
    clippy::print_stderr,
    reason = "a command-line program talks to its user on the standard streams"
)]

use std::process::ExitCode;

fn main() -> ExitCode {
    eprintln!(
        "everygraph {}: no commands yet (the command line is issue #98)",
        env!("CARGO_PKG_VERSION")
    );
    ExitCode::from(2)
}
