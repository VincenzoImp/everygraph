// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The `everygraph` command (`docs/api-sketch.md` § 5, `docs/ux-contract.md`).
//!
//! It has one command so far, `verify` (issue #41); the others come with issue #98.
//! What it owns: reading the command line, choosing the rendering of each stream
//! (ux-contract § 2.2), and printing results, errors and exit statuses (§ 6.4).
//!
//! The command line is read by hand, not with an argument-parsing crate: one command
//! with two flags does not justify a dependency, and #98 chooses the parser for the
//! full command set (`design/DECISIONS.md`, "Reading the command line").

#![forbid(unsafe_code)]
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a command-line program talks to its user on the standard streams"
)]

mod out;
mod verify;

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use out::Style;

/// Exit status 2: the command line could not be understood (ux-contract § 6.4).
const USAGE: u8 = 2;

const HELP: &str = "\
Everygraph VERSION — ingest, analyze, query and map a graph on one machine.

Usage
  everygraph COMMAND [OPTIONS]

Commands
  verify FILE           check that a graph file is intact

Options
  -h, --help            this text; after a command, the help for that command
  --version             print the version
";

const VERIFY_HELP: &str = "\
Check that a graph file (.eg) is intact.

Usage
  everygraph verify FILE [--deep] [--json]

Options
  --deep                every byte and every invariant; without it, the header,
                        the table of contents and the sections up to 1 MiB
  --json                print the report as JSON, with nothing around it
  -h, --help            this text

Exit status: 0 intact, 5 damaged, 1 the file cannot be read.
";

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let json = args.iter().any(|a| a == "--json");
    match run(&args) {
        Ok(code) => code,
        Err(usage) => {
            out::usage_error(&usage, json);
            ExitCode::from(USAGE)
        }
    }
}

/// A usage error: the one thing that is wrong, and the help to read.
struct Usage {
    message: String,
    help: &'static str,
}

fn usage(message: impl Into<String>, help: &'static str) -> Usage {
    Usage {
        message: message.into(),
        help,
    }
}

fn run(args: &[OsString]) -> Result<ExitCode, Usage> {
    let version = |text: &str| text.replace("VERSION", env!("CARGO_PKG_VERSION"));
    let help = |text: &str| {
        print!("{}", Style::of(&std::io::stdout()).symbols(&version(text)));
        Ok(ExitCode::SUCCESS)
    };
    let Some((command, rest)) = args.split_first() else {
        eprint!("{}", Style::of(&std::io::stderr()).symbols(&version(HELP)));
        return Ok(ExitCode::from(USAGE));
    };
    match command.to_str() {
        Some("-h" | "--help") => help(HELP),
        Some("--version") => {
            println!("everygraph {}", env!("CARGO_PKG_VERSION"));
            Ok(ExitCode::SUCCESS)
        }
        Some("verify") => {
            let mut file = None;
            let (mut deep, mut json) = (false, false);
            for arg in rest {
                match arg.to_str() {
                    Some("-h" | "--help") => return help(VERIFY_HELP),
                    Some("--deep") => deep = true,
                    Some("--json") => json = true,
                    Some(option) if option.starts_with('-') => {
                        return Err(usage(
                            format!("Unknown option {option}."),
                            "everygraph verify --help",
                        ));
                    }
                    _ if file.is_some() => {
                        return Err(usage("verify takes one file.", "everygraph verify --help"));
                    }
                    _ => file = Some(PathBuf::from(arg)),
                }
            }
            let file =
                file.ok_or_else(|| usage("verify needs a FILE.", "everygraph verify --help"))?;
            Ok(verify::run(
                &file,
                deep,
                json,
                &Style::of(&std::io::stdout()),
            ))
        }
        _ => Err(usage(
            format!("Unknown command {}.", command.to_string_lossy()),
            "everygraph --help",
        )),
    }
}
