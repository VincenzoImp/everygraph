// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `everygraph verify FILE [--deep] [--json]` (issue #41; `docs/api-sketch.md` § 5.1).
//!
//! The report is the command's result, intact or not, so it goes to standard output;
//! a damaged file exits with status 5. A file that cannot be read at all is an
//! `IoError` on standard error, status 1 (`design/DECISIONS.md`, "`everygraph verify`
//! reports on standard output").

use std::fmt::Write as _;
use std::path::Path;
use std::process::ExitCode;

use everygraph_core::Budget;
use everygraph_core::MemoryProbe;
use everygraph_format::container::Container;
use everygraph_format::verify::{Depth, Problem, Report, verify};
use everygraph_ingest::ids::Dictionary;
use everygraph_io::{MmapSource, ProcMemoryProbe};

use crate::out::{self, Style, count, json, wrap};

/// Exit status 1: a file could not be read (ux-contract § 6.4).
const IO_ERROR: u8 = 1;
/// Exit status 5: a graph file is damaged.
const DAMAGED: u8 = 5;

pub(crate) fn run(path: &Path, deep: bool, as_json: bool, style: &Style) -> ExitCode {
    let shown = path.display().to_string();
    let failed = |reason: String| {
        let message = format!("Cannot read {shown}: {reason}.");
        if as_json {
            out::error_json("IoError", &message, Some(&shown), &[]);
        } else {
            out::error(&message, &[], "Nothing was changed.");
        }
        ExitCode::from(IO_ERROR)
    };
    let source = match MmapSource::open(path) {
        Ok(source) => source,
        Err(e) => return failed(e.to_string()),
    };
    let depth = if deep { Depth::Deep } else { Depth::Fast };
    let mut report = match verify(&source, depth) {
        Ok(report) => report,
        Err(e) => return failed(e.to_string()),
    };
    if deep {
        if let Err(e) = check_dictionary(&source, &mut report) {
            return failed(e);
        }
    }
    if as_json {
        println!("{}", document(&shown, deep, &report));
    } else {
        print!("{}", text(&shown, deep, &report, style));
    }
    if report.is_intact() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(DAMAGED)
    }
}

/// The identifier dictionary, when the file has one whose sections are undamaged: ids
/// and keys a bijection, one key per node of every adjacency.
fn check_dictionary(source: &MmapSource, report: &mut Report) -> Result<(), String> {
    let damaged = report
        .problems
        .iter()
        .any(|p| p.section.as_deref().is_none_or(|s| s.starts_with("ids.")));
    let Ok(file) = Container::open(source) else {
        return Ok(());
    };
    if damaged || file.get("ids.header").is_none() {
        return Ok(());
    }
    // The dictionary holds its hash function in memory: half of what is free.
    let free = ProcMemoryProbe::new()
        .probe()
        .map_or(u64::MAX, |m| m.available() / 2);
    let budget = Budget::new(free);
    let dictionary = match Dictionary::open(&budget, |name| file.section(name).ok()) {
        Ok(d) => d,
        Err(e) => {
            report.problem(Some("ids.header"), e.to_string());
            return Ok(());
        }
    };
    if let Err(e) = dictionary.verify() {
        report.problem(Some("ids.header"), e.to_string());
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

/// The text form: a headline, then labels and values, or the problems.
fn text(path: &str, deep: bool, report: &Report, style: &Style) -> String {
    let label = |name: &str| style.dim(&format!("{name:<10}"));
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
                wrap(&format!("  {}", label("unchecked")), 12, what)
            );
        }
        for note in &report.notes {
            let _ = writeln!(
                t,
                "{}",
                wrap(&format!("  {}", label("note")), 12, &note.what)
            );
        }
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
        let line = match &p.section {
            Some(section) => format!("{section}: {}", p.what),
            None => p.what.clone(),
        };
        let _ = writeln!(t, "{}", wrap("  ", 4, &line));
    }
    for note in &report.notes {
        let _ = writeln!(t, "{}", wrap("  note: ", 4, &note.what));
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
    let generation = report
        .generation
        .map_or_else(|| "null".to_owned(), |g| g.to_string());
    format!(
        r#"{{"schema":1,"verify":{},"deep":{deep},"intact":{},"generation":{generation},"sections":{},"sections_hashed":{},"bytes_hashed":{},"problems":[{}],"notes":[{}],"unchecked":[{}]}}"#,
        json(path),
        report.is_intact(),
        report.sections,
        report.sections_hashed,
        report.bytes_hashed,
        list(&report.problems),
        list(&report.notes),
        unchecked.join(",")
    )
}
