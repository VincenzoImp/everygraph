// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `everygraph verify FILE [--deep] [--strict] [--json]` (issue #41;
//! `docs/api-sketch.md` § 5.1).
//!
//! The report is the command's result, intact or not, so it goes to standard output;
//! a damaged file exits with status 5. A file that cannot be read at all is an
//! `IoError` on standard error, status 1 (`design/DECISIONS.md`, "`everygraph verify`
//! reports on standard output"). A newest commit that cannot be read is a warning on
//! standard error, status 0, or 5 with `--strict` ("A lost newest commit").

use std::fmt::Write as _;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

use everygraph_core::{Budget, InsufficientBudget, Memory, MemoryProbe, ReadError};
use everygraph_format::container::Container;
use everygraph_format::verify::{Depth, Problem, Report, verify};
use everygraph_ingest::ids::{Dictionary, DictionaryError};
use everygraph_io::{MmapSource, ProcMemoryProbe};

use crate::out::{self, Style, count, json, printable, wrap};

/// Exit status 1: a file could not be read (ux-contract § 6.4).
const IO_ERROR: u8 = 1;
/// Exit status 4: refused for lack of memory.
const NO_MEMORY: u8 = 4;
/// Exit status 5: a graph file is damaged.
const DAMAGED: u8 = 5;

/// What the command was asked.
pub(crate) struct Options {
    pub(crate) deep: bool,
    pub(crate) strict: bool,
    pub(crate) json: bool,
}

/// Why the check could not be finished.
enum Failure {
    Io(String),
    Memory(InsufficientBudget),
}

pub(crate) fn run(path: &Path, options: &Options, style: &Style) -> ExitCode {
    let shown = path.display().to_string();
    // The structural check and the dictionary share half of the free memory.
    let free = ProcMemoryProbe::new()
        .probe()
        .map_or(u64::MAX, |m| m.available() / 2);
    let budget = Arc::new(Budget::new(free));
    let outcome = if path.is_dir() {
        Err(Failure::Io("it is a directory".to_owned()))
    } else {
        check(path, options, &budget)
    };
    let report = match outcome {
        Ok(report) => report,
        Err(Failure::Io(reason)) => {
            let message = format!("Cannot read {shown}: {reason}.");
            if options.json {
                out::error_json(
                    "IoError",
                    &message,
                    &format!(r#","path":{}"#, json(&shown)),
                    &[],
                );
            } else {
                out::error(&message, &[], "Nothing was changed.");
            }
            return ExitCode::from(IO_ERROR);
        }
        Err(Failure::Memory(e)) => {
            let message = "Not enough memory for this step.";
            let smallest = budget
                .limit()
                .saturating_sub(e.available)
                .saturating_add(e.needed);
            if options.json {
                let extra = format!(
                    r#","why":{},"step":"verify","needs_bytes":{},"budget_bytes":{},"smallest_budget_bytes":{smallest}"#,
                    json(&e.to_string()),
                    e.needed,
                    budget.limit()
                );
                out::error_json("InsufficientBudget", message, &extra, &[]);
            } else {
                out::error(message, &[], "Nothing was changed.");
            }
            return ExitCode::from(NO_MEMORY);
        }
    };
    if let (Some(lost), Some(generation)) = (report.lost_generation, report.generation) {
        out::warning(
            &format!("The newest commit, generation {lost}, could not be read."),
            &format!(
                "The file reads as generation {generation}, which is whole. Generation {lost} \
                 is damaged, or its commit was cut short."
            ),
            &["restore a copy of the file to get it back"],
            "Nothing was changed.",
        );
    }
    if options.json {
        println!("{}", document(&shown, options.deep, &report));
    } else {
        print!("{}", text(&shown, options.deep, &report, style));
    }
    let lost = options.strict && report.lost_generation.is_some();
    if report.is_intact() && !lost {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(DAMAGED)
    }
}

fn check(path: &Path, options: &Options, budget: &Arc<Budget>) -> Result<Report, Failure> {
    let io = |e: &dyn std::fmt::Display| Failure::Io(e.to_string());
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
fn check_dictionary(
    source: &MmapSource,
    budget: &Budget,
    report: &mut Report,
) -> Result<(), Failure> {
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
        DictionaryError::Budget(e) => Err(Failure::Memory(e)),
        DictionaryError::Read(e @ ReadError::Backend(_)) => Err(Failure::Io(e.to_string())),
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
fn text(path: &str, deep: bool, report: &Report, style: &Style) -> String {
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
fn document(path: &str, deep: bool, report: &Report) -> String {
    let list = |problems: &[Problem]| {
        let items: Vec<String> = problems
            .iter()
            .map(|p| {
                let section = p.section.as_deref().map_or_else(|| "null".to_owned(), json);
                format!(r#"{{"section":{section},"problem":{}}}"#, json(&p.what))
            })
            .collect();
        items.join(",")
    };
    let unchecked: Vec<String> = report.unchecked.iter().map(|u| json(u)).collect();
    let number = |n: Option<u64>| n.map_or_else(|| "null".to_owned(), |g| g.to_string());
    format!(
        r#"{{"schema":1,"verify":{},"deep":{deep},"intact":{},"generation":{},"lost_generation":{},"sections":{},"sections_hashed":{},"bytes_hashed":{},"problems":[{}],"notes":[{}],"unchecked":[{}]}}"#,
        json(path),
        report.is_intact(),
        number(report.generation),
        number(report.lost_generation),
        report.sections,
        report.sections_hashed,
        report.bytes_hashed,
        list(&report.problems),
        list(&report.notes),
        unchecked.join(",")
    )
}
