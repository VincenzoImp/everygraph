// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Text for people (`docs/ux-contract.md` § 2): the rendering of a stream, symbols and
//! their ASCII forms, and numbers, sizes, times and dates.

use std::io::IsTerminal;

/// How one stream is rendered (§ 2.2), chosen per stream (S-6).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Style {
    /// Unicode symbols, or their ASCII forms.
    pub(crate) unicode: bool,
    /// Colour instructions (S-7).
    pub(crate) colour: bool,
    /// Progress in the piped form: no bars, one line per finished phase (§ 4.5, S-10).
    pub(crate) piped: bool,
}

impl Style {
    /// The rendering of standard output.
    pub(crate) fn stdout() -> Self {
        Self::of(std::io::stdout().is_terminal())
    }

    /// The rendering of standard error.
    pub(crate) fn stderr() -> Self {
        Self::of(std::io::stderr().is_terminal())
    }

    /// A terminal shows Unicode when its locale is UTF-8, and colour unless `NO_COLOR`
    /// or `TERM=dumb`; `TERM=dumb` also turns progress into the piped form (S-10).
    fn of(terminal: bool) -> Self {
        let var = |name: &str| std::env::var(name).unwrap_or_default();
        let dumb = var("TERM") == "dumb";
        let locale = [var("LC_ALL"), var("LC_CTYPE"), var("LANG")]
            .into_iter()
            .find(|v| !v.is_empty())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let utf8 = locale.contains("utf-8") || locale.contains("utf8");
        Self {
            unicode: terminal && utf8,
            colour: terminal && !dumb && var("NO_COLOR").is_empty(),
            piped: !terminal || dumb,
        }
    }

    /// The style of progress lines: the piped form, in ASCII, with `--progress=plain`
    /// (S-10).
    pub(crate) fn progress(self, plain: bool) -> Self {
        if plain {
            Self {
                unicode: false,
                piped: true,
                ..self
            }
        } else {
            self
        }
    }

    fn pick(self, unicode: &'static str, ascii: &'static str) -> &'static str {
        if self.unicode { unicode } else { ascii }
    }

    /// `·`, separating facts on one line, with its spaces.
    pub(crate) fn dot(self) -> &'static str {
        self.pick(" · ", " | ")
    }

    /// `→`.
    pub(crate) fn arrow(self) -> &'static str {
        self.pick("→", "->")
    }

    /// `—`, joining a statement to its reason.
    pub(crate) fn dash(self) -> &'static str {
        self.pick("—", "-")
    }

    /// `×`.
    pub(crate) fn times(self) -> &'static str {
        self.pick("×", "x")
    }

    /// `…`.
    pub(crate) fn ellipsis(self) -> &'static str {
        self.pick("…", "...")
    }

    /// `✓`, green.
    pub(crate) fn ok(self) -> String {
        self.paint("32", self.pick("✓", "ok"))
    }

    /// `✗`, red.
    pub(crate) fn error(self) -> String {
        self.paint("31", self.pick("✗", "error:"))
    }

    /// `!`, yellow.
    pub(crate) fn warning(self) -> String {
        self.paint("33", self.pick("!", "warning:"))
    }

    /// `text` in bold.
    pub(crate) fn bold(self, text: &str) -> String {
        self.paint("1", text)
    }

    /// `text` dim.
    pub(crate) fn dim(self, text: &str) -> String {
        self.paint("2", text)
    }

    fn paint(self, code: &str, text: &str) -> String {
        if self.colour {
            format!("\u{1b}[{code}m{text}\u{1b}[0m")
        } else {
            text.to_owned()
        }
    }

    /// `text` with the symbols of § 2.3 in their ASCII forms when the stream is not
    /// Unicode.
    pub(crate) fn symbols(self, text: &str) -> String {
        if self.unicode {
            return text.to_owned();
        }
        [
            ("—", "-"),
            ("·", "|"),
            ("→", "->"),
            ("…", "..."),
            ("×", "x"),
        ]
        .into_iter()
        .fold(text.to_owned(), |t, (from, to)| t.replace(from, to))
    }

    /// The line of a finished phase (§ 4.2, § 4.5).
    pub(crate) fn phase_done(self, name: &str, detail: &str, seconds: f64) -> String {
        self.phase_line(name, detail, &duration(seconds))
    }

    /// The line of a phase whose result already exists (G-3).
    pub(crate) fn phase_kept(self, name: &str, detail: &str) -> String {
        self.phase_line(name, detail, "kept")
    }

    fn phase_line(self, name: &str, detail: &str, time: &str) -> String {
        if self.piped {
            let width = 45_usize.saturating_sub(detail.chars().count());
            format!("  {name:<13}{detail}{time:>width$}")
        } else {
            let bar = self.dim(&self.pick("█", "#").repeat(16));
            format!("  {name:<13}{bar}  {detail:<32}{time:>13}")
        }
    }
}

/// A count with thousands separators: `1,830,397,917`.
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

/// `n` and its noun, singular for one: `1 self-loop`, `4,201 self-loops`.
pub(crate) fn counted(n: u64, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{} {many}", count(n))
    }
}

/// An estimated count in a label-and-value line: four significant digits at most and
/// a suffix (`212 M`).
pub(crate) fn rough_count(n: u64) -> String {
    let (value, suffix) = match n {
        n if n >= 1_000_000_000 => (to_f64(n) / 1e9, " B"),
        n if n >= 1_000_000 => (to_f64(n) / 1e6, " M"),
        n if n >= 1_000 => (to_f64(n) / 1e3, "k"),
        n => return n.to_string(),
    };
    let digits = match value {
        v if v < 10.0 => 2,
        v if v < 100.0 => 1,
        _ => 0,
    };
    format!("{value:.digits$}{suffix}")
}

const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];

fn scaled(bytes: u64) -> (f64, &'static str) {
    let mut value = to_f64(bytes);
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    (value, UNITS[unit])
}

/// An exact size or a budget: powers of 10, one decimal below 100 (§ 2.6).
pub(crate) fn size(bytes: u64) -> String {
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    match scaled(bytes) {
        (v, unit) if v < 100.0 => format!("{v:.1} {unit}"),
        (v, unit) => format!("{v:.0} {unit}"),
    }
}

/// An estimate or what the machine reports: two significant digits.
pub(crate) fn rough_size(bytes: u64) -> String {
    match scaled(bytes) {
        (v, unit) if v < 10.0 && unit != "B" => format!("{v:.1} {unit}"),
        (v, unit) => format!("{v:.0} {unit}"),
    }
}

/// A duration: `ms` below 1 s, whole seconds below 1 min, then `M min S s`, `H h M min`.
pub(crate) fn duration(seconds: f64) -> String {
    if seconds < 1.0 {
        let ms = seconds * 1000.0;
        return if ms < 10.0 {
            format!("{ms:.1} ms")
        } else {
            format!("{ms:.0} ms")
        };
    }
    let whole = whole_seconds(seconds);
    let (h, m, s) = (whole / 3600, whole / 60 % 60, whole % 60);
    match (h, m, s) {
        (0, 0, s) => format!("{s} s"),
        (0, m, 0) => format!("{m} min"),
        (0, m, s) => format!("{m} min {s} s"),
        (h, 0, _) => format!("{h} h"),
        (h, m, _) => format!("{h} h {m} min"),
    }
}

/// An estimated duration, rounded as § 2.6 says, with "about".
pub(crate) fn about(seconds: f64) -> String {
    let step = match seconds {
        s if s < 60.0 => 5.0,
        s if s < 600.0 => 10.0,
        s if s < 3600.0 => 60.0,
        _ => 300.0,
    };
    let rounded = (seconds / step).round().max(1.0) * step;
    format!("about {}", duration(rounded))
}

/// An age, in its largest unit only: `6 min ago`, `3 days ago` (§ 2.6).
pub(crate) fn age(seconds: u64) -> String {
    let (n, one, many) = match seconds {
        s if s < 60 => return "just now".to_owned(),
        s if s < 3600 => (s / 60, "min", "min"),
        s if s < 86_400 => (s / 3600, "h", "h"),
        s => (s / 86_400, "day", "days"),
    };
    format!("{n} {} ago", if n == 1 { one } else { many })
}

/// The date of `ns` nanoseconds after 1970-01-01 UTC, as `2026-11-24` (ISO 8601).
pub(crate) fn date(ns: i64) -> String {
    let days = ns.div_euclid(86_400_000_000_000);
    // Howard Hinnant's `civil_from_days`.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// The instant `ns` nanoseconds after 1970-01-01 UTC, to the second:
/// `2026-11-24T10:41:07Z`.
pub(crate) fn timestamp(ns: i64) -> String {
    let second = ns.div_euclid(1_000_000_000).rem_euclid(86_400);
    format!(
        "{}T{:02}:{:02}:{:02}Z",
        date(ns),
        second / 3600,
        second / 60 % 60,
        second % 60
    )
}

fn whole_seconds(seconds: f64) -> u64 {
    // A duration is finite and far below 2^53 seconds.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "rounded"
    )]
    let whole = seconds.round().max(0.0) as u64;
    whole
}

/// For ratios and sizes in text, where exactness is not asked.
pub(crate) fn to_f64(x: u64) -> f64 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "a printed figure of a few digits"
    )]
    let x = x as f64;
    x
}

/// `8GB`, `512MB`, `1.5GB`, `6G`, `256MiB`, or bytes: powers of 10 for `kB`, `MB`,
/// `GB`, `TB` (§ 2.6) and of 2 for the `i` forms.
pub(crate) fn parse_size(text: &str) -> Option<u64> {
    let at = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(at);
    let number: f64 = number.parse().ok()?;
    let scale: f64 = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1.0,
        "k" | "kb" => 1e3,
        "m" | "mb" => 1e6,
        "g" | "gb" => 1e9,
        "t" | "tb" => 1e12,
        "kib" => 1024.0,
        "mib" => 1024.0 * 1024.0,
        "gib" => 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    let bytes = (number * scale).round();
    // Finite, non-negative and below 2^63: exact enough for a budget.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "checked range"
    )]
    (bytes.is_finite() && (0.0..9.2e18).contains(&bytes)).then_some(bytes as u64)
}

/// A budget as it can be typed: rounded up to 100 MB below 1 GB, 0.5 GB from 1 GB,
/// 1 GB from 4 GB (P-14).
pub(crate) fn typed(bytes: u64) -> String {
    let step = match bytes {
        b if b < 1_000_000_000 => 100_000_000,
        b if b < 4_000_000_000 => 500_000_000,
        _ => 1_000_000_000,
    };
    let up = bytes.div_ceil(step).max(1) * step;
    if up % 1_000_000_000 == 0 {
        format!("{}GB", up / 1_000_000_000)
    } else if up >= 1_000_000_000 {
        format!("{:.1}GB", to_f64(up) / 1e9)
    } else {
        format!("{}MB", up / 1_000_000)
    }
}

/// `text` as a JSON string.
pub(crate) fn json_string(text: &str) -> String {
    serde_json::Value::from(text).to_string()
}

/// `text` with control characters escaped, so that a name read from a file cannot
/// move the cursor or change the colours of the terminal.
pub(crate) fn printable(text: &str) -> String {
    text.chars()
        .flat_map(|c| {
            let escaped: Vec<char> = if c.is_control() {
                c.escape_default().collect()
            } else {
                vec![c]
            };
            escaped
        })
        .collect()
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
    fn the_examples_of_the_contract() {
        assert_eq!(count(1_830_397_917), "1,830,397,917");
        assert_eq!(count(999), "999");
        assert_eq!(count(0), "0");
        assert_eq!(size(28), "28 B");
        assert_eq!(size(38_200_000_000), "38.2 GB");
        assert_eq!(size(8_000_000_000), "8.0 GB");
        assert_eq!(size(156_475_392), "156 MB");
        assert_eq!(rough_size(61_000_000_000), "61 GB");
        assert_eq!(rough_size(1_400_000_000_000), "1.4 TB");
        assert_eq!(rough_size(512), "512 B");
        assert_eq!(duration(0.0013), "1.3 ms");
        assert_eq!(duration(38.0), "38 s");
        assert_eq!(duration(101.0), "1 min 41 s");
        assert_eq!(duration(7500.0), "2 h 5 min");
        assert_eq!(about(220.0), "about 3 min 40 s");
        assert_eq!(about(2.0), "about 5 s");
        assert_eq!(age(6 * 60 + 5), "6 min ago");
        assert_eq!(age(3 * 86_400 + 7), "3 days ago");
        assert_eq!(age(86_400), "1 day ago");
        assert_eq!(counted(1, "self-loop", "self-loops"), "1 self-loop");
        assert_eq!(counted(4201, "self-loop", "self-loops"), "4,201 self-loops");
        assert_eq!(rough_count(212_400_000), "212 M");
        assert_eq!(rough_count(45_400_000), "45.4 M");
    }

    #[test]
    fn dates_are_iso_and_utc() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(-1), "1969-12-31");
        // 2026-11-24T23:59:59Z.
        assert_eq!(date(1_795_564_799_000_000_000), "2026-11-24");
        assert_eq!(date(951_782_400_000_000_000), "2000-02-29");
        assert_eq!(timestamp(1_795_564_799_000_000_000), "2026-11-24T23:59:59Z");
        assert_eq!(timestamp(-1), "1969-12-31T23:59:59Z");
    }

    #[test]
    fn sizes_are_read_as_the_contract_writes_them() {
        assert_eq!(parse_size("8GB"), Some(8_000_000_000));
        assert_eq!(parse_size("1.5GB"), Some(1_500_000_000));
        assert_eq!(parse_size("512MB"), Some(512_000_000));
        assert_eq!(parse_size("256MiB"), Some(256 << 20));
        assert_eq!(parse_size("6G"), Some(6_000_000_000));
        assert_eq!(parse_size("100"), Some(100));
        assert_eq!(parse_size("GB"), None);
        assert_eq!(parse_size("8 parsecs"), None);
    }

    #[test]
    fn budgets_are_rounded_up_to_a_value_that_can_be_typed() {
        assert_eq!(typed(5_598_053_768), "6GB");
        assert_eq!(typed(1_200_000_000), "1.5GB");
        assert_eq!(typed(230_000_000), "300MB");
        assert_eq!(typed(4_000_000_000), "4GB");
        assert_eq!(typed(0), "100MB");
    }

    #[test]
    fn text_is_escaped_and_wrapped() {
        assert_eq!(json_string("a\"b\\c\n\u{1}é"), r#""a\"b\\c\n\u0001é""#);
        assert_eq!(printable("a\u{1b}[31mb\u{9b}é"), "a\\u{1b}[31mb\\u{9b}é");
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

    #[test]
    fn plain_progress_has_no_bar_and_no_unicode() {
        let terminal = Style {
            unicode: true,
            colour: true,
            piped: false,
        };
        let line = terminal.phase_done("identifiers", "212,418,907 addresses", 38.0);
        assert!(line.contains('█'));
        let plain = terminal.progress(true);
        let line = plain.phase_done("reading", "1,830,402,118 rows", 101.0);
        assert_eq!(
            line,
            "  reading      1,830,402,118 rows                 1 min 41 s"
        );
        assert!(line.is_ascii());
        assert_eq!(plain.dot(), " | ");
        let unchanged = terminal.progress(false);
        assert!(unchanged.unicode && !unchanged.piped);
    }

    #[test]
    fn phase_lines_keep_their_columns() {
        let piped = Style {
            unicode: false,
            colour: false,
            piped: true,
        };
        let line = piped.phase_done("reading", "1,830,402,118 rows", 101.0);
        assert_eq!(line.len(), 60);
        let terminal = Style {
            piped: false,
            ..piped
        };
        let line = terminal.phase_done("identifiers", "212,418,907 addresses", 38.0);
        assert_eq!(line.len(), 78);
        assert!(line.ends_with("38 s"));
    }
}
