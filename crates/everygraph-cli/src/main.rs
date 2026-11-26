// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The `everygraph` command (`docs/api-sketch.md` § 5, `docs/ux-contract.md`).
//!
//! What it owns: reading the command line (by hand: `design/DECISIONS.md`, "Reading the
//! command line"), choosing the rendering of each stream (ux-contract § 2.2), and
//! printing plans, progress, results, errors and exit statuses (§ 6.4). The work is
//! done by the library crates.

#![forbid(unsafe_code)]
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a command-line program talks to its user on the standard streams"
)]

mod args;
mod compact;
mod export;
mod failure;
mod graph;
mod info;
mod ingest;
mod machine;
mod plan;
mod progress;
mod render;
mod run;
mod scratch;
mod table;
mod verify;

use std::ffi::OsString;
use std::process::ExitCode;

use args::Command;

/// The help of `everygraph`: `docs/api-sketch.md` § 5.3, with the commands of this
/// version.
const HELP: &str = concat!(
    "Everygraph ",
    env!("CARGO_PKG_VERSION"),
    " — ingest, analyze, query and map a graph on one machine.

Usage
  everygraph COMMAND [OPTIONS]

Commands
  ingest FILE           read a file of edges and build a graph (.eg) beside it
  run NAME GRAPH        analyze the whole graph: pagerank, components, …
  info GRAPH            what a graph file holds and what was computed so far
  export GRAPH          write the graph's edges in another format
  verify FILE           check that a graph file is intact
  compact FILE          reclaim the bytes of replaced results

Options
  --memory SIZE         the memory budget, for example 8GB
  --threads N           the most threads to use
  --json                print the result as JSON, with nothing around it
  --out FILE            write the result to a file, for example pr.parquet
  --overwrite           replace a file that is in the way
  -h, --help            this text; after a command, the help for that command
  --version             print the version

Short forms: -o for --out, -m for --memory, -t for --threads.

Start here
  everygraph ingest FILE
"
);

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let json = args.iter().any(|a| a == "--json");
    let status = match args::parse(&args) {
        Err(failure) => failure.report(json),
        Ok(Err(status)) => status,
        Ok(Ok(command)) => match command {
            Command::Ingest(a) => ingest::main(&a),
            Command::Run(a) => run::main(a),
            Command::Info(a) => info::main(&a),
            Command::Export(a) => export::main(&a),
            Command::Verify(a) => verify::main(&a),
            Command::Compact(a) => compact::main(&a),
        },
    };
    ExitCode::from(status)
}
