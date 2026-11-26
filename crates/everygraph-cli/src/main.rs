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
use std::io::Write;
use std::process::ExitCode;

use render::Style;

const HELP: &str = "\
Everygraph VERSION — ingest, analyze, query and map a graph on one machine.

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
  -h, --help            this text; after a command, the help for that command
  --version             print the version
";

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    ExitCode::from(run(&args))
}

fn run(args: &[OsString]) -> u8 {
    let help = HELP.replace("VERSION", env!("CARGO_PKG_VERSION"));
    let Some((command, rest)) = args.split_first() else {
        let _ = std::io::stderr().write_all(Style::stderr().symbols(&help).as_bytes());
        return failure::USAGE;
    };
    match command.to_str() {
        Some("-h" | "--help") => {
            print!("{}", Style::stdout().symbols(&help));
            0
        }
        Some("--version") => {
            println!("everygraph {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Some("ingest") => ingest::main(rest),
        Some("info") => info::main(rest),
        Some("export") => export::main(rest),
        Some("compact") => compact::main(rest),
        Some("run") => run::main(rest),
        Some("verify") => verify::main(rest),
        _ => failure::usage(
            format!("Unknown command {}.", command.to_string_lossy()),
            "",
        )
        .report(args.iter().any(|a| a == "--json")),
    }
}
