// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Renderings, symbols, errors and JSON (`docs/ux-contract.md` §§ 2, 6).

use std::fmt::Write as _;
use std::io::IsTerminal;

use crate::Usage;

/// How one stream is rendered (ux-contract § 2.2).
pub(crate) struct Style {
    unicode: bool,
    colour: bool,
}

impl Style {
    /// The rendering of `stream`: Unicode on a UTF-8 terminal, colour on a terminal
    /// unless `NO_COLOR` or `TERM=dumb`; ASCII without colour when piped.
    pub(crate) fn of(stream: &impl IsTerminal) -> Self {
        let terminal = stream.is_terminal();
        let env = |name| std::env::var(name).ok().filter(|v| !v.is_empty());
        let utf8 = ["LC_ALL", "LC_CTYPE", "LANG"]
            .into_iter()
            .find_map(env)
            .is_some_and(|v| {
                let v = v.to_ascii_uppercase();
                v.contains("UTF-8") || v.contains("UTF8")
            });
        let no_colour = env("NO_COLOR").is_some() || env("TERM").as_deref() == Some("dumb");
        Self {
            unicode: terminal && utf8,
            colour: terminal && !no_colour,
        }
    }

    fn paint(&self, code: &str, text: &str) -> String {
        if self.colour {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_owned()
        }
    }

    /// `✓` in green, or `ok`.
    pub(crate) fn ok(&self) -> String {
        self.paint("32", if self.unicode { "✓" } else { "ok" })
    }

    /// `✗` in red, or `error:`.
    pub(crate) fn error(&self) -> String {
        self.paint("31", if self.unicode { "✗" } else { "error:" })
    }

    /// `text` with the symbols of ux-contract § 2.3 replaced by their ASCII forms when
    /// the stream is not Unicode.
    pub(crate) fn symbols(&self, text: &str) -> String {
        if self.unicode {
            return text.to_owned();
        }
        [("—", "-"), ("·", "|"), ("→", "->"), ("…", "...")]
            .into_iter()
            .fold(text.to_owned(), |t, (from, to)| t.replace(from, to))
    }

    pub(crate) fn bold(&self, text: &str) -> String {
        self.paint("1", text)
    }

    pub(crate) fn dim(&self, text: &str) -> String {
        self.paint("2", text)
    }
}

/// Error `what` (one sentence) in the shape of ux-contract M-1, on standard error.
pub(crate) fn error(what: &str, ways: &[&str], left: &str) {
    let style = Style::of(&std::io::stderr());
    let mut text = format!("{} {}\n", style.error(), style.bold(what));
    if !ways.is_empty() {
        text.push_str("\n  Ways forward:\n");
        for way in ways {
            let _ = writeln!(text, "    {}", style.bold(way));
        }
    }
    let _ = write!(text, "\n{left}\n");
    eprint!("{text}");
}

/// The error document of ux-contract § 6.1 (M-5), on standard output.
pub(crate) fn error_json(class: &str, message: &str, path: Option<&str>, ways: &[&str]) {
    let ways: Vec<String> = ways
        .iter()
        .map(|w| format!(r#"{{"command":{},"python":"","note":""}}"#, json(w)))
        .collect();
    let path = path.map_or_else(String::new, |p| format!(r#","path":{}"#, json(p)));
    println!(
        r#"{{"schema":1,"error":{},"message":{}{path},"ways_forward":[{}],"changed":false}}"#,
        json(class),
        json(message),
        ways.join(",")
    );
}

pub(crate) fn usage_error(usage: &Usage, json: bool) {
    if json {
        error_json("UsageError", &usage.message, None, &[usage.help]);
    } else {
        error(&usage.message, &[usage.help], "Nothing was changed.");
    }
}

/// `text` as a JSON string.
pub(crate) fn json(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `n` with a comma every three digits (ux-contract § 2.6).
pub(crate) fn count(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `text` broken at spaces into lines of at most 80 columns (W-1), the first starting
/// after `first`, the others indented by `indent` spaces.
pub(crate) fn wrap(first: &str, indent: usize, text: &str) -> String {
    let mut out = first.to_owned();
    let mut width = first.chars().count();
    let mut words_on_line = false;
    for word in text.split(' ') {
        let len = word.chars().count();
        if words_on_line && width + 1 + len > 80 {
            out.push('\n');
            out.push_str(&" ".repeat(indent));
            width = indent;
            words_on_line = false;
        }
        if words_on_line {
            out.push(' ');
            width += 1;
        }
        out.push_str(word);
        width += len;
        words_on_line = true;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_json_and_wrapping() {
        assert_eq!(count(0), "0");
        assert_eq!(count(1234), "1,234");
        assert_eq!(count(1_234_567), "1,234,567");
        assert_eq!(json("a\"b\\\n\u{1}é"), r#""a\"b\\\n\u0001é""#);
        let long = "word ".repeat(30);
        let wrapped = wrap("  label     ", 12, long.trim());
        assert!(wrapped.lines().all(|l| l.chars().count() <= 80));
        assert!(wrapped.starts_with("  label     word word"));
        assert!(
            wrapped
                .lines()
                .skip(1)
                .all(|l| l.starts_with("            word"))
        );
    }
}
