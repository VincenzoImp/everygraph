// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Errors and refusals (`docs/ux-contract.md` § 6.1): the text on standard error, or
//! with `--json` the error document on standard output, and the exit status (§ 6.4);
//! and the failures of `ingest`, built from what went wrong.

use std::fmt::Write as _;
use std::io::{self, Write};
use std::path::Path;

use everygraph_core::InsufficientBudget;
use everygraph_ingest::build::BuildError;
use everygraph_ingest::text::TextError;

use crate::render::{Style, count, counted, json_string, rough_size, size, typed};

/// What the last line of an error says was left.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Left {
    /// `Nothing was written.`: a new file was to be made.
    Nothing,
    /// No last line: nothing was to be written (a usage error of the command line).
    Unsaid,
}

/// An error of the command, with its class (§ 6.1), its text and its exit status.
#[derive(Debug)]
pub(crate) struct Failure {
    class: &'static str,
    headline: String,
    why: Vec<String>,
    ways: Vec<(String, String)>,
    left: Left,
    status: u8,
    bytes: Option<Box<Refused>>,
}

/// The keys an `InsufficientBudget` document must hold (§ 6.1).
#[derive(Debug)]
struct Refused {
    step: &'static str,
    needs: u64,
    budget: u64,
    /// `None` when no budget on this machine would work.
    smallest: Option<u64>,
}

impl Failure {
    pub(crate) fn new(class: &'static str, headline: String, status: u8) -> Self {
        Self {
            class,
            headline,
            why: Vec::new(),
            ways: Vec::new(),
            left: Left::Nothing,
            status,
            bytes: None,
        }
    }

    /// The lines that say why.
    pub(crate) fn why(mut self, why: Vec<String>) -> Self {
        self.why = why;
        self
    }

    /// The ways forward: what can be typed, and what it does.
    pub(crate) fn ways(mut self, ways: Vec<(String, String)>) -> Self {
        self.ways = ways;
        self
    }

    pub(crate) fn left(mut self, left: Left) -> Self {
        self.left = left;
        self
    }

    /// The exit status.
    pub(crate) fn status(&self) -> u8 {
        self.status
    }

    /// Prints the error, and with `json` its document, and returns the exit status.
    pub(crate) fn report(&self, json: bool) -> u8 {
        self.print_text();
        if json {
            let _ = writeln!(io::stdout(), "{}", self.document());
        }
        self.status
    }

    /// Prints the text form on standard error.
    pub(crate) fn print_text(&self) {
        let style = Style::of(true);
        let mut text = format!("{} {}\n", style.error(), style.bold(&self.headline));
        if !self.why.is_empty() {
            text.push('\n');
            for line in &self.why {
                let _ = writeln!(text, "  {line}");
            }
        }
        if !self.ways.is_empty() {
            text.push_str("\n  Ways forward:\n");
            for (command, note) in &self.ways {
                if note.is_empty() {
                    let _ = writeln!(text, "    {}", style.bold(command));
                } else {
                    let pad = 24_usize.saturating_sub(command.chars().count()).max(2);
                    let _ = writeln!(text, "    {}{}{note}", style.bold(command), " ".repeat(pad));
                }
            }
        }
        if let Left::Nothing = self.left {
            text.push_str("\nNothing was written.\n");
        }
        let _ = io::stderr().write_all(text.as_bytes());
    }

    /// The error document (M-5).
    pub(crate) fn document(&self) -> String {
        let ways: Vec<String> = self
            .ways
            .iter()
            .map(|(command, note)| {
                format!(
                    "{{\"command\":{},\"python\":\"\",\"note\":{}}}",
                    json_string(command),
                    json_string(note)
                )
            })
            .collect();
        let mut out = format!(
            "{{\"schema\":1,\"error\":\"{}\",\"message\":{},\"why\":{},\"ways_forward\":[{}],\
             \"changed\":false",
            self.class,
            json_string(&self.headline),
            json_string(&self.why.join("\n")),
            ways.join(",")
        );
        if let Some(refused) = self.bytes.as_deref() {
            let Refused {
                step,
                needs,
                budget,
                smallest,
            } = refused;
            let smallest = smallest.map_or_else(|| "null".to_owned(), |s| s.to_string());
            let _ = write!(
                out,
                ",\"step\":{},\"needs_bytes\":{needs},\"budget_bytes\":{budget},\
                 \"smallest_budget_bytes\":{smallest}",
                json_string(step)
            );
        }
        out.push('}');
        out
    }
}

/// A usage error, with the help as the way forward.
pub(crate) fn usage(what: String) -> Failure {
    Failure::new("UsageError", what, 2).ways(vec![(
        "everygraph ingest --help".into(),
        "the options of ingest".into(),
    )])
}

/// An operating-system error on `path`.
pub(crate) fn io(verb: &str, path: &Path, e: &io::Error) -> Failure {
    let text = e.to_string();
    let reason = text.split(" (os error").next().unwrap_or(&text);
    Failure::new(
        "IoError",
        format!("Cannot {verb} {}: {reason}.", path.display()),
        1,
    )
}

/// An input that changed while it was read.
pub(crate) fn changed(input: &Path) -> Failure {
    Failure::new(
        "InvalidInput",
        format!("{} changed while it was read.", input.display()),
        3,
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
            Failure::new("IoError", format!("Cannot read {file}: {cause}."), 1)
        }
        TextError::InvalidRows(summary) => {
            let mut why = Vec::new();
            for p in &summary.problems {
                why.push(format!("{file}:{:<12}{p}", p.line));
                why.push(format!("  {}", String::from_utf8_lossy(&p.excerpt)));
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
            let headline = format!("Cannot read {file}: {}.", TextError::InvalidRows(summary));
            Failure::new("InvalidInput", headline, 3)
                .why(why)
                .ways(vec![(fix, String::new())])
        }
        other @ (TextError::UnknownColumn { .. }
        | TextError::NoHeader { .. }
        | TextError::BeyondHeader { .. }) => {
            Failure::new("UsageError", format!("Cannot read {file}: {other}."), 2)
        }
        other => Failure::new("InvalidInput", format!("Cannot read {file}: {other}."), 3),
    }
}

/// A failure of the build of `input` under `budget` on a machine of `machine` bytes.
pub(crate) fn build(input: &Path, budget: u64, machine: Option<u64>, e: BuildError) -> Failure {
    match e {
        BuildError::Text(e) => text(input, e),
        BuildError::Budget(refused) => memory(budget, machine, &refused),
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
                3,
            )
            .why(why)
            .ways(vec![(
                "--ids string".into(),
                "read every identifier as text".into(),
            )])
        }
        BuildError::Scratch(e) => io("write", Path::new("the scratch files"), &e),
        other => Failure::new("EverygraphError", format!("The build failed: {other}."), 1),
    }
}

/// A step that the budget cannot hold (P-13 to P-15).
pub(crate) fn memory(budget: u64, machine: Option<u64>, refused: &InsufficientBudget) -> Failure {
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
    let mut failure = Failure::new(
        "InsufficientBudget",
        "Not enough memory for this step.".into(),
        4,
    )
    .why(vec![
        format!(
            "The build needs {} for {}.",
            size(refused.needed),
            refused.purpose
        ),
        format!(
            "The budget is {}, and {} of it was left.",
            size(budget),
            size(refused.available)
        ),
    ])
    .ways(vec![way]);
    failure.bytes = Some(Box::new(Refused {
        step: refused.purpose,
        needs: refused.needed,
        budget,
        smallest: fits_machine.then_some(smallest),
    }));
    failure
}

/// The refusal before any work of a budget below the build's smallest.
pub(crate) fn refusal(smallest: u64, budget: u64, machine: Option<u64>, threads: usize) -> Failure {
    let refused = InsufficientBudget::new("the blocks read at once", smallest, budget);
    let mut failure = memory(budget, machine, &refused).why(vec![format!(
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
        failure.ways.push((
            "--threads 1".into(),
            "fewer blocks in flight need less memory".into(),
        ));
    }
    failure
}
