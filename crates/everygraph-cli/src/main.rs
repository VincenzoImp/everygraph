// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The `everygraph` command (`docs/api-sketch.md` § 5). It has one command so far,
//! `ingest` (issue #35); the others come with issue #98.

#![forbid(unsafe_code)]
#![allow(
    clippy::print_stderr,
    reason = "a command-line program talks to its user on the standard streams"
)]

mod failure;
mod ingest;
mod render;
mod scratch;

use std::ffi::OsString;
use std::io::Write;
use std::process::ExitCode;

const HELP: &str = "\
Everygraph — ingest, analyze, query and map a graph on one machine.

Usage
  everygraph COMMAND [OPTIONS]

Commands
  ingest FILE           read a file of edges and build a graph (.eg) beside it

Options
  -h, --help            this text; after a command, the help for that command
  --version             print the version

Start here
  everygraph ingest FILE
";

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let status = match args.first().and_then(|a| a.to_str()) {
        Some("ingest") => ingest::main(&args[1..]),
        Some("--version") => {
            let _ = writeln!(
                std::io::stdout(),
                "everygraph {}",
                env!("CARGO_PKG_VERSION")
            );
            0
        }
        Some("-h" | "--help") => {
            let _ = std::io::stdout().write_all(HELP.as_bytes());
            0
        }
        None => {
            let _ = std::io::stderr().write_all(HELP.as_bytes());
            2
        }
        Some(other) => {
            let json = args.iter().any(|a| a == "--json");
            failure::Failure::new("UsageError", format!("Unknown command {other}."), 2)
                .ways(vec![("everygraph --help".into(), "the commands".into())])
                .left(failure::Left::Unsaid)
                .report(json)
        }
    };
    ExitCode::from(status)
}
