// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Errors and refusals (`docs/ux-contract.md` § 6.1): the text on standard error, or
//! with `--json` the error document on standard output, and the exit status (§ 6.4);
//! and the failures built from what went wrong in the library crates.

use std::fmt::Write as _;
use std::io::{self, Write};
use std::path::Path;

use everygraph_core::InsufficientBudget;
use everygraph_ingest::build::BuildError;
use everygraph_ingest::text::TextError;
use serde_json::{Map, Value, json};

use crate::render::{Style, count, counted, rough_size, size, typed, wrap};

/// Exit status 1: failed for a reason not listed below (§ 6.4).
pub(crate) const FAILED: u8 = 1;
/// Exit status 2: the command line cannot be understood or honoured.
pub(crate) const USAGE: u8 = 2;
/// Exit status 3: the input is not valid.
pub(crate) const INVALID: u8 = 3;
/// Exit status 4: refused for lack of memory or disk.
pub(crate) const NO_MEMORY: u8 = 4;
/// Exit status 5: a graph file is damaged.
pub(crate) const DAMAGED: u8 = 5;
/// Exit status 6: no such node.
pub(crate) const UNKNOWN_NODE: u8 = 6;

/// An error, with its class (§ 6.1), its text, the further keys of its document and its
/// exit status.
#[derive(Debug)]
pub(crate) struct Failure(Box<Inner>);

#[derive(Debug)]
struct Inner {
    class: &'static str,
    headline: String,
    why: Vec<String>,
    ways: Vec<(String, String)>,
    status: u8,
    keys: Map<String, Value>,
    /// The last line: `Nothing was written.` when a new file was to be made, `Nothing
    /// was changed.` when an existing one was involved (M-1).
    changed_file: bool,
}

impl Failure {
    pub(crate) fn new(class: &'static str, headline: String, status: u8) -> Self {
        Self(Box::new(Inner {
            class,
            headline,
            why: Vec::new(),
            ways: Vec::new(),
            status,
            keys: Map::new(),
            changed_file: false,
        }))
    }

    /// The lines that say why.
    pub(crate) fn why(mut self, why: Vec<String>) -> Self {
        self.0.why = why;
        self
    }

    /// The ways forward: what can be typed, and what it does.
    pub(crate) fn ways(mut self, ways: Vec<(String, String)>) -> Self {
        self.0.ways = ways;
        self
    }

    /// A further key of the document (`path`, `holds`, `needs_bytes`, …).
    pub(crate) fn key(mut self, name: &str, value: impl Into<Value>) -> Self {
        self.0.keys.insert(name.to_owned(), value.into());
        self
    }

    /// The operation was on an existing file: the last line is `Nothing was changed.`
    pub(crate) fn on_existing_file(mut self) -> Self {
        self.0.changed_file = true;
        self
    }

    /// The exit status.
    pub(crate) fn status(&self) -> u8 {
        self.0.status
    }

    /// Prints the error, and with `json` its document, and returns the exit status.
    pub(crate) fn report(&self, json: bool) -> u8 {
        self.print_text();
        if json {
            let _ = writeln!(io::stdout(), "{}", self.document());
        }
        self.0.status
    }

    /// Prints the text form on standard error (M-1, M-2).
    pub(crate) fn print_text(&self) {
        let style = Style::stderr();
        let mut text = format!("{} {}\n", style.error(), style.bold(&self.0.headline));
        if !self.0.why.is_empty() {
            text.push('\n');
            for line in &self.0.why {
                let _ = writeln!(text, "{}", wrap("  ", 2, line));
            }
        }
        text.push_str(&ways_text(style, &self.0.ways));
        let left = if self.0.changed_file {
            "Nothing was changed."
        } else {
            "Nothing was written."
        };
        let _ = writeln!(text, "\n{left}");
        let _ = io::stderr().write_all(style.symbols(&text).as_bytes());
    }

    /// The error document (M-5).
    pub(crate) fn document(&self) -> Value {
        let ways: Vec<Value> = self
            .0
            .ways
            .iter()
            .map(|(command, note)| json!({"command": command, "python": "", "note": note}))
            .collect();
        let mut doc = json!({
            "schema": 1,
            "error": self.0.class,
            "message": self.0.headline,
        });
        let map = doc.as_object_mut().expect("an object");
        if !self.0.why.is_empty() {
            map.insert("why".into(), self.0.why.join("\n").into());
        }
        map.extend(self.0.keys.clone());
        map.insert("ways_forward".into(), ways.into());
        map.insert("changed".into(), false.into());
        doc
    }
}

fn ways_text(style: Style, ways: &[(String, String)]) -> String {
    let mut text = String::new();
    if !ways.is_empty() {
        text.push_str("\n  Ways forward:\n");
        for (command, note) in ways {
            if note.is_empty() {
                let _ = writeln!(text, "    {}", style.bold(command));
            } else {
                let pad = 24_usize.saturating_sub(command.chars().count()).max(2);
                let _ = writeln!(text, "    {}{}{note}", style.bold(command), " ".repeat(pad));
            }
        }
    }
    text
}

/// A warning in the shape of M-8, on standard error: what, its consequence, the ways to
/// avoid it, and what happens now.
pub(crate) fn warning(what: &str, why: &str, ways: &[(String, String)], now: &str) {
    let style = Style::stderr();
    let mut text = format!("{} {}\n", style.warning(), style.bold(what));
    if !why.is_empty() {
        let _ = write!(text, "\n{}\n", wrap("  ", 2, why));
    }
    text.push_str(&ways_text(style, ways));
    let _ = write!(text, "\n  {now}\n");
    let _ = io::stderr().write_all(style.symbols(&text).as_bytes());
}

/// A usage error, with the help of `command` as the way forward.
pub(crate) fn usage(what: String, command: &str) -> Failure {
    let help = if command.is_empty() {
        "everygraph --help".to_owned()
    } else {
        format!("everygraph {command} --help")
    };
    Failure::new("UsageError", what, USAGE).ways(vec![(help, String::new())])
}

/// The operating system's reason, without its error number.
pub(crate) fn reason(e: &io::Error) -> String {
    let text = e.to_string();
    text.split(" (os error").next().unwrap_or(&text).to_owned()
}

/// An operating-system error on `path`; `verb` is `read` or `write`.
pub(crate) fn io(verb: &str, path: &Path, e: &io::Error) -> Failure {
    let way = if verb == "read" {
        "check the path, and that the file can be read"
    } else {
        "check that the directory exists, can be written and has room"
    };
    Failure::new(
        "IoError",
        format!("Cannot {verb} {}: {}.", path.display(), reason(e)),
        FAILED,
    )
    .key("path", path.display().to_string())
    .ways(vec![(way.into(), String::new())])
}

/// A graph file that is damaged or is not one.
pub(crate) fn corrupt(path: &Path, what: &dyn std::fmt::Display) -> Failure {
    Failure::new(
        "CorruptFile",
        format!("{} is damaged: {what}.", path.display()),
        DAMAGED,
    )
    .ways(vec![(
        format!("everygraph verify {}", path.display()),
        "what is damaged".into(),
    )])
    .on_existing_file()
}

/// An output in the way that `--overwrite` was not given for. `ours` says whether it
/// is a graph file.
pub(crate) fn output_exists(output: &Path, ours: bool) -> Failure {
    let (holds, key) = if ours {
        ("holds another graph", "other graph")
    } else {
        ("was not written by Everygraph", "not ours")
    };
    Failure::new(
        "OutputExists",
        format!("{} exists and {holds}.", output.display()),
        FAILED,
    )
    .key("path", output.display().to_string())
    .key("holds", key)
    .ways(vec![
        ("--out NEW".into(), "write it under another name".into()),
        ("--overwrite".into(), "replace it".into()),
    ])
}

/// An input that changed while it was read.
pub(crate) fn changed(input: &Path) -> Failure {
    Failure::new(
        "InvalidInput",
        format!("{} changed while it was read.", input.display()),
        INVALID,
    )
    .ways(vec![(
        "run the same command again once the file is complete".into(),
        String::new(),
    )])
}

/// A failure of the text reader on `input`.
pub(crate) fn text(input: &Path, e: TextError) -> Failure {
    let file = input.display().to_string();
    match e {
        TextError::Read(cause) => {
            Failure::new("IoError", format!("Cannot read {file}: {cause}."), FAILED)
                .key("path", file)
        }
        TextError::InvalidRows(summary) => {
            let mut why = Vec::new();
            let mut problems = Vec::new();
            for p in &summary.problems {
                why.push(format!("{file}:{:<12}{p}", p.line));
                why.push(format!("  {}", String::from_utf8_lossy(&p.excerpt)));
                problems.push(json!({"file": file, "line": p.line, "problem": p.to_string()}));
            }
            let shown = u64::try_from(summary.problems.len()).unwrap_or(u64::MAX);
            if summary.invalid_rows > shown {
                why.push(format!("and {} more", count(summary.invalid_rows - shown)));
            }
            why.push(format!(
                "The whole file was checked: {}, {} of them invalid.",
                counted(summary.lines, "line", "lines"),
                count(summary.invalid_rows)
            ));
            let fix = if summary.invalid_rows == 1 {
                "correct that line, then run the same command again".to_owned()
            } else {
                format!(
                    "correct those {} lines, then run the same command again",
                    count(summary.invalid_rows)
                )
            };
            let (invalid, lines) = (summary.invalid_rows, summary.lines);
            let headline = format!("Cannot read {file}: {}.", TextError::InvalidRows(summary));
            Failure::new("InvalidInput", headline, INVALID)
                .why(why)
                .key("problems", problems)
                .key("invalid_rows", invalid)
                .key("rows_checked", lines)
                .ways(vec![(fix, String::new())])
        }
        other @ (TextError::UnknownColumn { .. }
        | TextError::NoHeader { .. }
        | TextError::BeyondHeader { .. }) => {
            Failure::new("UsageError", format!("Cannot read {file}: {other}."), USAGE)
                .ways(vec![("everygraph ingest --help".into(), String::new())])
        }
        other => Failure::new(
            "InvalidInput",
            format!("Cannot read {file}: {other}."),
            INVALID,
        )
        .ways(vec![(
            "correct the file, then run the same command again".into(),
            String::new(),
        )]),
    }
}

/// A failure of the build of `input` under `budget` on a machine of `machine` bytes.
pub(crate) fn build(input: &Path, budget: u64, machine: Option<u64>, e: BuildError) -> Failure {
    match e {
        BuildError::Text(e) => text(input, e),
        BuildError::Budget(refused) => memory("build", budget, machine, &refused),
        BuildError::InputChanged => changed(input),
        BuildError::InvalidKeys(keys) => {
            let mut why: Vec<String> = keys
                .first
                .iter()
                .map(|k| format!("{} does not fit the kind of the other identifiers", k.key))
                .collect();
            why.push(format!(
                "{} in all.",
                counted(keys.count, "identifier", "identifiers")
            ));
            Failure::new(
                "InvalidInput",
                format!(
                    "Cannot read {}: some identifiers do not fit their column.",
                    input.display()
                ),
                INVALID,
            )
            .why(why)
            .ways(vec![(
                "--ids string".into(),
                "read every identifier as text".into(),
            )])
        }
        BuildError::Scratch(e) => io("write", Path::new("the scratch files"), &e),
        other => internal(&format!("The build failed: {other}.")),
    }
}

/// An error of the engine that no other class describes.
pub(crate) fn internal(what: &str) -> Failure {
    Failure::new("EverygraphError", what.to_owned(), FAILED).ways(vec![(
        "report it at https://github.com/VincenzoImp/everygraph/issues".into(),
        String::new(),
    )])
}

/// A step that the budget cannot hold (P-13 to P-15). `what` names the work in the
/// first line of why: `The build needs …`.
pub(crate) fn memory(
    what: &str,
    budget: u64,
    machine: Option<u64>,
    refused: &InsufficientBudget,
) -> Failure {
    let smallest = budget
        .saturating_sub(refused.available)
        .saturating_add(refused.needed);
    let fits_machine = machine.is_none_or(|m| smallest <= m);
    let way = if fits_machine {
        (
            format!("--memory {}", typed(smallest)),
            "a budget that holds this step".to_owned(),
        )
    } else {
        (
            format!("a machine with at least {} of memory", rough_size(smallest)),
            String::new(),
        )
    };
    let mut what = what.to_owned();
    if let Some(first) = what.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    Failure::new(
        "InsufficientBudget",
        "Not enough memory for this step.".into(),
        NO_MEMORY,
    )
    .why(vec![
        format!(
            "The {what} needs {} for {}.",
            size(refused.needed),
            refused.purpose
        ),
        format!(
            "The budget is {}, and {} of it was left.",
            size(budget),
            size(refused.available)
        ),
    ])
    .key("step", refused.purpose)
    .key("needs_bytes", refused.needed)
    .key("budget_bytes", budget)
    .key(
        "smallest_budget_bytes",
        if fits_machine {
            json!(smallest)
        } else {
            Value::Null
        },
    )
    .ways(vec![way])
}

/// The refusal before any work of a budget below the build's smallest.
pub(crate) fn refusal(smallest: u64, budget: u64, machine: Option<u64>, threads: usize) -> Failure {
    let refused = InsufficientBudget::new("the blocks read at once", smallest, budget);
    let mut failure = memory("build", budget, machine, &refused).why(vec![format!(
        "{} read blocks at once; they need about {} and the budget is {}.",
        counted(
            u64::try_from(threads).unwrap_or(u64::MAX),
            "thread",
            "threads"
        ),
        rough_size(smallest),
        size(budget)
    )]);
    if threads > 1 {
        failure.0.ways.push((
            "--threads 1".into(),
            "fewer blocks in flight need less memory".into(),
        ));
    }
    failure
}
