// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Reading the command line with `clap` (`design/DECISIONS.md`, "Reading the command
//! line"): the commands and the options that mean the same on each
//! (`docs/api-sketch.md` § 5.2). The help texts are the commands' own, laid out as
//! `docs/api-sketch.md` § 5.3 shows; `clap`'s errors become usage errors in the shape
//! of every other error (`docs/ux-contract.md` § 6.1), exit status 2.

use std::ffi::OsString;
use std::num::NonZeroUsize;
use std::path::PathBuf;

use clap::error::{ContextKind, ErrorKind};
use clap::{Parser, Subcommand, ValueEnum};

use crate::failure::{Failure, usage};
use crate::render::{Style, parse_size};
use crate::{compact, export, info, ingest, run, verify};

/// The command line.
#[derive(Debug, Parser)]
#[command(
    name = "everygraph",
    version,
    disable_version_flag = true,
    disable_help_subcommand = true,
    override_help = crate::HELP
)]
struct Everygraph {
    /// Prints the version.
    #[arg(long, action = clap::ArgAction::Version)]
    version: Option<bool>,
    #[command(subcommand)]
    command: Command,
}

/// A command.
#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    Ingest(ingest::Args),
    Run(run::Args),
    Info(info::Args),
    Export(export::Args),
    Verify(verify::Args),
    Compact(compact::Args),
}

/// How progress is shown (S-10).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub(crate) enum Progress {
    /// Bars on a terminal, the piped form elsewhere.
    #[default]
    Auto,
    /// The piped form everywhere: one line per finished phase, never redrawn.
    Plain,
}

/// The options shared by the commands that compute (`docs/api-sketch.md` § 5.2).
#[derive(Clone, Debug, Default, clap::Args)]
pub(crate) struct Common {
    /// The memory budget: the bytes, and the text as typed.
    #[arg(long, short = 'm', value_name = "SIZE", value_parser = memory)]
    pub(crate) memory: Option<(u64, String)>,
    #[arg(long, short = 't', value_name = "N", value_parser = threads)]
    pub(crate) threads: Option<NonZeroUsize>,
    #[arg(long)]
    pub(crate) json: bool,
    #[arg(long, short = 'o', value_name = "FILE")]
    pub(crate) out: Option<PathBuf>,
    #[arg(long)]
    pub(crate) overwrite: bool,
    #[arg(long, value_enum, default_value_t)]
    pub(crate) progress: Progress,
}

impl Common {
    /// `--progress=plain`.
    pub(crate) fn plain(&self) -> bool {
        self.progress == Progress::Plain
    }
}

fn memory(text: &str) -> Result<(u64, String), String> {
    parse_size(text)
        .map(|bytes| (bytes, text.to_owned()))
        .ok_or_else(|| "a size, such as 8GB".to_owned())
}

fn threads(text: &str) -> Result<NonZeroUsize, String> {
    text.parse().map_err(|_| "a positive number".to_owned())
}

/// The command asked for, or `None` when the help or the version was printed; the
/// exit status of that.
///
/// # Errors
///
/// A usage error for a command line that cannot be understood.
pub(crate) fn parse(args: &[OsString]) -> Result<Result<Command, u8>, Failure> {
    let all = std::iter::once(OsString::from("everygraph")).chain(args.iter().cloned());
    match Everygraph::try_parse_from(all) {
        Ok(cli) => Ok(Ok(cli.command)),
        Err(e) => match e.kind() {
            ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
                print!("{}", Style::stdout().symbols(&e.to_string()));
                Ok(Err(0))
            }
            ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand if args.is_empty() => {
                eprint!("{}", Style::stderr().symbols(&e.to_string()));
                Ok(Err(2))
            }
            _ => Err(from_clap(&e, args)),
        },
    }
}

/// The usage error of a `clap` error, in the words of the contract.
fn from_clap(e: &clap::Error, args: &[OsString]) -> Failure {
    let words: Vec<String> = args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let command = match words.first().map(String::as_str) {
        Some("run") => words
            .get(1)
            .filter(|w| run::NAMES.contains(&w.as_str()))
            .map_or_else(|| "run".to_owned(), |name| format!("run {name}")),
        Some(c @ ("ingest" | "info" | "export" | "verify" | "compact")) => c.to_owned(),
        _ => String::new(),
    };
    let context = |kind| e.get(kind).map(ToString::to_string).unwrap_or_default();
    let arg = context(ContextKind::InvalidArg);
    let option = arg.split(['=', ' ']).next().unwrap_or_default().to_owned();
    let value = context(ContextKind::InvalidValue);
    let name = if command.is_empty() {
        "everygraph"
    } else {
        &command
    };
    let what = match e.kind() {
        ErrorKind::UnknownArgument if arg.starts_with('-') => format!("Unknown option {option}."),
        ErrorKind::UnknownArgument => {
            format!("{name} takes no {arg}; it is one argument too many.")
        }
        ErrorKind::InvalidSubcommand
            if command.starts_with("run") || words.first().is_some_and(|w| w == "run") =>
        {
            format!(
                "Unknown algorithm {}.",
                context(ContextKind::InvalidSubcommand)
            )
        }
        ErrorKind::InvalidSubcommand => {
            format!(
                "Unknown command {}.",
                context(ContextKind::InvalidSubcommand)
            )
        }
        ErrorKind::InvalidValue if value.is_empty() => format!("{option} needs a value."),
        ErrorKind::InvalidValue => {
            let valid = match e.get(ContextKind::ValidValue) {
                Some(clap::error::ContextValue::Strings(v)) => v.join(" or "),
                _ => String::new(),
            };
            format!("{option} takes {valid}, not {value}.")
        }
        ErrorKind::ValueValidation => {
            let reason = std::error::Error::source(e)
                .map(ToString::to_string)
                .unwrap_or_default();
            format!("{option} {value} is not {reason}.")
        }
        ErrorKind::MissingRequiredArgument => {
            let missing = arg.trim_matches(['<', '>']).to_owned();
            format!("{name} needs a {missing}.")
        }
        ErrorKind::MissingSubcommand | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
            format!("{name} needs a NAME: {}.", run::NAMES.join(", "))
        }
        ErrorKind::TooManyValues | ErrorKind::NoEquals => format!("{option} takes no value."),
        ErrorKind::ArgumentConflict => {
            let other = context(ContextKind::PriorArg);
            format!(
                "{option} cannot be used with {}.",
                other.split(['=', ' ']).next().unwrap_or_default()
            )
        }
        _ => {
            let text = e.to_string();
            let first = text.lines().next().unwrap_or_default();
            let first = first.strip_prefix("error: ").unwrap_or(first);
            let mut first = first.to_owned();
            if let Some(c) = first.get_mut(..1) {
                c.make_ascii_uppercase();
            }
            format!("{}.", first.trim_end_matches('.'))
        }
    };
    let failure = usage(what, &command);
    if e.kind() == ErrorKind::InvalidSubcommand && words.first().is_some_and(|w| w == "run") {
        return failure.why(vec![format!("run takes {}.", run::NAMES.join(", "))]);
    }
    failure
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(args: &[&str]) -> String {
        let args: Vec<OsString> = args.iter().map(OsString::from).collect();
        match parse(&args) {
            Err(failure) => failure.document()["message"].as_str().unwrap().to_owned(),
            Ok(_) => String::new(),
        }
    }

    #[test]
    fn errors_are_said_in_the_words_of_the_contract() {
        assert_eq!(
            message(&["verify", "--deeep", "a.eg"]),
            "Unknown option --deeep."
        );
        assert_eq!(message(&["frobnicate"]), "Unknown command frobnicate.");
        assert_eq!(
            message(&["run", "frobnicate", "g.eg"]),
            "Unknown algorithm frobnicate."
        );
        assert_eq!(
            message(&["ingest", "a.txt", "--duplicates", "some"]),
            "--duplicates takes events or collapse or sum, not some."
        );
        assert_eq!(
            message(&["ingest", "a.txt", "--memory", "8 parsecs"]),
            "--memory 8 parsecs is not a size, such as 8GB."
        );
        assert_eq!(message(&["run", "bfs", "g.eg"]), "run bfs needs a NODE.");
        assert_eq!(message(&["info"]), "info needs a GRAPH.");
        assert_eq!(
            message(&["info", "a.eg", "--json=yes"]),
            "--json takes no value."
        );
        assert_eq!(
            message(&["run", "pagerank", "g.eg", "--max-hops", "2"]),
            "Unknown option --max-hops."
        );
        assert_eq!(
            message(&["ingest", "a.txt", "--directed", "--undirected"]),
            "--directed cannot be used with --undirected."
        );
    }

    #[test]
    fn the_short_options_are_the_same_on_every_command() {
        let args: Vec<OsString> = ["ingest", "a.txt", "-o", "b.eg", "-m", "2GB", "-t", "3"]
            .iter()
            .map(OsString::from)
            .collect();
        let Ok(Ok(Command::Ingest(ingest))) = parse(&args) else {
            panic!("ingest parses");
        };
        assert_eq!(ingest.common.out, Some(PathBuf::from("b.eg")));
        assert_eq!(ingest.common.memory.map(|m| m.0), Some(2_000_000_000));
        assert_eq!(ingest.common.threads.map(NonZeroUsize::get), Some(3));
    }
}
