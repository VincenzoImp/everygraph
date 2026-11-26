// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Errors and refusals (`docs/ux-contract.md` § 6.1): the text on standard error, or
//! with `--json` the error document on standard output, and the exit status (§ 6.4).

use std::io::{self, Write};

use crate::render::Style;

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
    bytes: Option<(u64, u64)>,
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

    /// One more way forward.
    pub(crate) fn more_way(mut self, command: String, note: String) -> Self {
        self.ways.push((command, note));
        self
    }

    pub(crate) fn left(mut self, left: Left) -> Self {
        self.left = left;
        self
    }

    /// What a step needs, and the budget, for the error document.
    pub(crate) fn bytes(mut self, needs: u64, budget: u64) -> Self {
        self.bytes = Some((needs, budget));
        self
    }

    /// Prints the error and returns the exit status.
    pub(crate) fn report(&self, json: bool) -> u8 {
        let style = Style::of(true);
        let mut text = format!("{} {}\n", style.error(), style.bold(&self.headline));
        if !self.why.is_empty() {
            text.push('\n');
            for line in &self.why {
                text.push_str("  ");
                text.push_str(line);
                text.push('\n');
            }
        }
        if !self.ways.is_empty() {
            text.push_str("\n  Ways forward:\n");
            for (command, note) in &self.ways {
                let line = if note.is_empty() {
                    format!("    {}", style.bold(command))
                } else {
                    let pad = 24_usize.saturating_sub(command.chars().count()).max(2);
                    format!("    {}{}{note}", style.bold(command), " ".repeat(pad))
                };
                text.push_str(&line);
                text.push('\n');
            }
        }
        if let Left::Nothing = self.left {
            text.push_str("\nNothing was written.\n");
        }
        let _ = io::stderr().write_all(text.as_bytes());
        if json {
            let _ = writeln!(io::stdout(), "{}", self.document());
        }
        self.status
    }

    /// The error document (M-5).
    fn document(&self) -> String {
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
            "{{\"schema\":1,\"error\":\"{}\",\"message\":{},\"why\":{},\"ways_forward\":[{}],\"changed\":false",
            self.class,
            json_string(&self.headline),
            json_string(&self.why.join("\n")),
            ways.join(",")
        );
        if let Some((needs, budget)) = self.bytes {
            out.push_str(&format!(
                ",\"needs_bytes\":{needs},\"budget_bytes\":{budget}"
            ));
        }
        out.push('}');
        out
    }
}

/// `text` as a JSON string.
pub(crate) fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if u32::from(c) < 0x20 => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
