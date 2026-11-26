// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `everygraph verify FILE [--deep] [--strict] [--json]` (issue #41;
//! `docs/api-sketch.md` § 5.1).
//!
//! The report is the command's result, intact or not, so it goes to standard output;
//! a damaged file exits with status 5. A file that cannot be read at all is an
//! `IoError` on standard error, status 1 (`design/DECISIONS.md`, "`everygraph verify`
//! reports on standard output"). A newest commit that cannot be read is a warning on
//! standard error, status 0, or 5 with `--strict` ("A lost newest commit").

use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use everygraph_core::{Budget, InsufficientBudget, Memory, MemoryProbe, ReadError};
use everygraph_format::container::Container;
use everygraph_format::verify::{Depth, Problem, Report, verify};
use everygraph_ingest::ids::{Dictionary, DictionaryError};
use everygraph_io::{MmapSource, ProcMemoryProbe};
use serde_json::{Value, json};

use crate::args::{self as cli, Arg};
use crate::failure::{self, Failure};
use crate::render::{Style, count, printable, wrap};

const HELP: &str = "\
Check that a graph file (.eg) is intact.

Usage
  everygraph verify FILE [--deep] [--strict] [--json]

Options
  --deep                every byte and every invariant; without it, the header,
                        the table of contents and the sections up to 1 MiB
  --strict              exit with status 5 also when the newest commit cannot
                        be read and the file reads as the one before
  --json                print the report as JSON, with nothing around it
  -h, --help            this text
  --                    what follows is the file, even if it starts with -

Exit status: 0 intact, 5 damaged, 1 the file cannot be read, 4 not enough
memory.
";

/// What the command was asked.
#[derive(Default)]
struct Options {
    deep: bool,
    strict: bool,
    json: bool,
}

/// Why the check could not be finished.
enum Stop {
    Io(String),
    Memory(InsufficientBudget),
}

/// Runs the command; returns the exit status.
pub(crate) fn main(raw: &[OsString]) -> u8 {
    let json = raw.iter().any(|a| a == "--json");
    let mut options = Options::default();
    let mut file = None;
    let mut it = cli::Args::new(raw, "verify");
    let parsed = (|| {
        while let Some(arg) = it.next()? {
            match arg {
                Arg::Option(help) if cli::is_help(help) => {
                    print!("{HELP}");
                    return Ok(false);
                }
                Arg::Option("--deep") => options.deep = true,
                Arg::Option("--strict") => options.strict = true,
                Arg::Option("--json") => options.json = true,
                Arg::Option(other) => return Err(it.unknown(other)),
                Arg::Plain(_) if file.is_some() => {
                    return Err(it.usage("verify takes one file.".into()));
                }
                Arg::Plain(path) => file = Some(PathBuf::from(path)),
            }
        }
        Ok(true)
    })();
    match parsed {
        Ok(false) => 0,
        Ok(true) => match file {
            Some(file) => run(&file, &options),
            None => it.usage("verify needs a FILE.".into()).report(json),
        },
        Err(failure) => failure.report(json),
    }
}

fn run(path: &Path, options: &Options) -> u8 {
    let shown = path.display().to_string();
    // The structural check and the dictionary share half of the free memory.
    let free = ProcMemoryProbe::new()
        .probe()
        .map_or(u64::MAX, |m| m.available() / 2);
    let budget = Arc::new(Budget::new(free));
    let outcome = if path.is_dir() {
        Err(Stop::Io("it is a directory".to_owned()))
    } else {
        check(path, options, &budget)
    };
    let report = match outcome {
        Ok(report) => report,
        Err(Stop::Io(reason)) => {
            return Failure::new(
                "IoError",
                format!("Cannot read {shown}: {reason}."),
                failure::FAILED,
            )
            .key("path", shown)
            .ways(vec![(
                "check the path and the permissions of the file".into(),
                String::new(),
            )])
            .on_existing_file()
            .report(options.json);
        }
        Err(Stop::Memory(e)) => {
            return failure::memory("check", budget.limit(), None, &e)
                .on_existing_file()
                .report(options.json);
        }
    };
    if let (Some(lost), Some(generation)) = (report.lost_generation, report.generation) {
        failure::warning(
            &format!("The newest commit, generation {lost}, could not be read."),
            &format!(
                "The file reads as generation {generation}, which is whole. Generation {lost} \
                 is damaged, or its commit was cut short."
            ),
            &[(
                "restore a copy of the file to get it back".into(),
                String::new(),
            )],
            "Nothing was changed.",
        );
    }
    if options.json {
        println!("{}", document(&shown, options.deep, &report));
    } else {
        print!("{}", text(&shown, options.deep, &report, Style::stdout()));
    }
    let lost = options.strict && report.lost_generation.is_some();
    if report.is_intact() && !lost {
        0
    } else {
        failure::DAMAGED
    }
}

fn check(path: &Path, options: &Options, budget: &Arc<Budget>) -> Result<Report, Stop> {
    let io = |e: &dyn std::fmt::Display| Stop::Io(e.to_string());
    let source = MmapSource::open(path).map_err(|e| io(&e))?;
    let depth = if options.deep {
        Depth::Deep
    } else {
        Depth::Fast
    };
    let memory = Memory::from(Arc::clone(budget));
    let mut report = verify(&source, depth, &memory).map_err(|e| io(&e))?;
    if options.deep {
        check_dictionary(&source, budget, &mut report)?;
    }
    Ok(report)
}

/// The identifier dictionary, when the file has one whose sections are undamaged: ids
/// and keys a bijection, one key per node of every adjacency.
fn check_dictionary(source: &MmapSource, budget: &Budget, report: &mut Report) -> Result<(), Stop> {
    let Ok(file) = Container::open(source) else {
        return Ok(());
    };
    if file.get("ids.header").is_none() {
        return Ok(());
    }
    if report
        .problems
        .iter()
        .any(|p| p.section.as_deref().is_none_or(|s| s.starts_with("ids.")))
    {
        report
            .unchecked
            .push("ids: the dictionary (a part it depends on is damaged)".to_owned());
        return Ok(());
    }
    let damaged = |report: &mut Report, e: DictionaryError| match e {
        DictionaryError::Budget(e) => Err(Stop::Memory(e)),
        DictionaryError::Read(e @ ReadError::Backend(_)) => Err(Stop::Io(e.to_string())),
        e => {
            report.problem(Some("ids.header"), e.to_string());
            Ok(())
        }
    };
    let dictionary = match Dictionary::open(budget, |name| file.section(name).ok()) {
        Ok(d) => d,
        Err(e) => return damaged(report, e),
    };
    if let Err(e) = dictionary.verify() {
        return damaged(report, e);
    }
    let keys = dictionary.len();
    let mismatched: Vec<String> = report
        .adjacencies
        .iter()
        .filter(|a| a.nodes != keys)
        .map(|a| {
            format!(
                "the dictionary has {keys} keys, but {} has {} nodes",
                a.name, a.nodes
            )
        })
        .collect();
    for what in mismatched {
        report.problem(Some("ids.header"), what);
    }
    Ok(())
}

/// One problem or note as a line: the section, then what is wrong.
fn line(p: &Problem) -> String {
    printable(&match &p.section {
        Some(section) => format!("{section}: {}", p.what),
        None => p.what.clone(),
    })
}

/// The text form: a headline, then labels and values, or the problems.
fn text(path: &str, deep: bool, report: &Report, style: Style) -> String {
    let label = |name: &str| style.dim(&format!("{name:<10}"));
    let path = printable(path);
    let mut t = String::new();
    if report.is_intact() {
        let _ = write!(
            t,
            "{} {}\n\n",
            style.ok(),
            style.bold(&format!("{path} is intact."))
        );
        let checked = if deep {
            format!(
                "every byte of {} sections and their structure",
                report.sections
            )
        } else {
            format!(
                "the header, the table of contents and {} of {} sections; --deep checks every \
                 byte",
                report.sections_hashed, report.sections
            )
        };
        let _ = writeln!(
            t,
            "{}",
            wrap(&format!("  {}", label("checked")), 12, &checked)
        );
        let _ = writeln!(
            t,
            "  {}{} bytes",
            label("hashed"),
            count(report.bytes_hashed)
        );
        if let Some(generation) = report.generation {
            let _ = writeln!(t, "  {}generation {}", label("commit"), count(generation));
        }
        for what in &report.unchecked {
            let _ = writeln!(
                t,
                "{}",
                wrap(&format!("  {}", label("unchecked")), 12, &printable(what))
            );
        }
        for note in &report.notes {
            let _ = writeln!(
                t,
                "{}",
                wrap(&format!("  {}", label("note")), 12, &line(note))
            );
        }
        return t;
    }
    if report.generation.is_none() {
        // Not a container at all: there is nothing to restore or rebuild.
        let headline = format!("{path} is not an Everygraph graph file.");
        let _ = write!(t, "{} {}\n\n", style.error(), style.bold(&headline));
        for p in &report.problems {
            let _ = writeln!(t, "{}", wrap("  ", 4, &line(p)));
        }
        t.push_str("\nNothing was changed.\n");
        return t;
    }
    let n = report.problems.len();
    let headline = format!(
        "{path} is damaged: {} problem{}.",
        count(n as u64),
        if n == 1 { "" } else { "s" }
    );
    let _ = write!(t, "{} {}\n\n", style.error(), style.bold(&headline));
    for p in &report.problems {
        let _ = writeln!(t, "{}", wrap("  ", 4, &line(p)));
    }
    for note in &report.notes {
        let _ = writeln!(t, "{}", wrap("  note: ", 4, &line(note)));
    }
    let _ = write!(
        t,
        "\n  Ways forward:\n    restore a copy of the file, or build it again from its input\n\n\
         Nothing was changed.\n"
    );
    t
}

/// The verify document (`docs/api-sketch.md` § 5.3, `urn:everygraph:schema:verify:1`).
fn document(path: &str, deep: bool, report: &Report) -> Value {
    let list = |problems: &[Problem]| -> Vec<Value> {
        problems
            .iter()
            .map(|p| json!({"section": p.section, "problem": p.what}))
            .collect()
    };
    json!({
        "schema": 1,
        "verify": path,
        "deep": deep,
        "intact": report.is_intact(),
        "generation": report.generation,
        "lost_generation": report.lost_generation,
        "sections": report.sections,
        "sections_hashed": report.sections_hashed,
        "bytes_hashed": report.bytes_hashed,
        "problems": list(&report.problems),
        "notes": list(&report.notes),
        "unchecked": report.unchecked,
    })
}
