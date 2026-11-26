// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Text for people (`docs/ux-contract.md` § 2): the rendering of a stream, symbols and
//! their ASCII forms, and numbers, sizes and durations.

use std::fmt::Write as _;
use std::io::{IsTerminal, Write as _};
use std::path::Path;

use everygraph_ingest::build::Report;

/// How one stream is rendered (§ 2.2).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Style {
    /// Unicode symbols, or their ASCII forms.
    pub(crate) unicode: bool,
    /// Colour instructions (S-7).
    pub(crate) colour: bool,
    /// Not a terminal: no bars, one line per finished phase.
    pub(crate) piped: bool,
}

impl Style {
    /// The rendering of standard error, or of standard output.
    pub(crate) fn of(stderr: bool) -> Self {
        let terminal = if stderr {
            std::io::stderr().is_terminal()
        } else {
            std::io::stdout().is_terminal()
        };
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

    fn pick(self, unicode: &'static str, ascii: &'static str) -> &'static str {
        if self.unicode { unicode } else { ascii }
    }

    /// `·`, separating facts on one line.
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

    /// `✓`, green.
    pub(crate) fn ok(self) -> String {
        self.paint("32", self.pick("✓", "ok"))
    }

    /// `✗`, red.
    pub(crate) fn error(self) -> String {
        self.paint("31", self.pick("✗", "error:"))
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

    /// The line of a finished phase (§ 4.2, § 4.5).
    pub(crate) fn phase_done(self, name: &str, detail: &str, seconds: f64) -> String {
        let time = duration(seconds);
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
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// An exact size or a budget: powers of 10, one decimal below 100 (§ 2.6).
pub(crate) fn size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut value = to_f64(bytes);
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if value < 100.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

/// An estimate or what the machine reports: two significant digits.
pub(crate) fn rough_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    let mut value = to_f64(bytes);
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if value < 10.0 && unit > 0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
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

/// `n` and its noun, singular for one: `1 self-loop`, `4,201 self-loops`.
pub(crate) fn counted(n: u64, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{} {many}", count(n))
    }
}

/// An estimated count in a label-and-value line: four significant digits at most and
/// a suffix (`~212.4 M`).
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

/// The result block of a graph made (G-11 to G-13), on standard error.
pub(crate) fn result_block(output: &Path, bytes: u64, report: &Report, seconds: f64, style: Style) {
    let dot = style.dot();
    let name = output
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let mut text = format!(
        "\n{} {}{dot}{}{dot}{}{dot}{}{dot}{}\n",
        style.ok(),
        style.bold(&name),
        size(bytes),
        counted(report.nodes, "node", "nodes"),
        counted(report.edges, "edge", "edges"),
        duration(seconds)
    );
    let mut dropped = Vec::new();
    if report.dropped_self_loops > 0 {
        dropped.push(counted(
            report.dropped_self_loops,
            "self-loop",
            "self-loops",
        ));
    }
    if report.dropped_duplicates > 0 {
        dropped.push(counted(
            report.dropped_duplicates,
            "duplicate",
            "duplicates",
        ));
    }
    if report.rejected > 0 {
        dropped.push(counted(report.rejected, "rejected row", "rejected rows"));
    }
    if !dropped.is_empty() {
        let _ = writeln!(text, "  dropped: {}", dropped.join(dot));
    }
    if report.merged > 0 {
        let merged = counted(report.merged, "duplicate", "duplicates");
        let _ = writeln!(text, "  merged: {merged}, summed into their pairs");
    }
    let next = format!("everygraph map {}", output.display());
    let _ = writeln!(text, "  next: {}", style.bold(&next));
    let _ = std::io::stderr().write_all(text.as_bytes());
}

/// The file document of `docs/ux-contract.md` § 4.7.
pub(crate) fn file_document(
    job: &str,
    output: &Path,
    bytes: u64,
    report: &Report,
    seconds: f64,
) -> String {
    let keys = report.counts();
    let counts: Vec<String> = keys
        .iter()
        .filter_map(|(k, v)| k.strip_prefix("counts.").map(|k| format!("\"{k}\":{v}")))
        .collect();
    let dropped: Vec<String> = keys
        .iter()
        .filter(|(_, v)| *v > 0)
        .filter_map(|(k, v)| k.strip_prefix("dropped.").map(|k| format!("\"{k}\":{v}")))
        .collect();
    format!(
        "{{\"schema\":1,\"job\":\"{job}\",\"operation\":\"ingest\",\"made\":{{\"path\":{},\
         \"kind\":\"graph\",\"bytes\":{bytes}}},\"counts\":{{{}}},\"dropped\":{{{}}},\
         \"kept\":[],\"elapsed_seconds\":{seconds:.3}}}",
        json_string(&output.display().to_string()),
        counts.join(","),
        dropped.join(",")
    )
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
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
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
        assert_eq!(duration(0.0013), "1.3 ms");
        assert_eq!(duration(38.0), "38 s");
        assert_eq!(duration(101.0), "1 min 41 s");
        assert_eq!(duration(7500.0), "2 h 5 min");
        assert_eq!(about(220.0), "about 3 min 40 s");
        assert_eq!(about(2.0), "about 5 s");
        assert_eq!(counted(1, "self-loop", "self-loops"), "1 self-loop");
        assert_eq!(counted(4201, "self-loop", "self-loops"), "4,201 self-loops");
        assert_eq!(rough_count(212_400_000), "212 M");
        assert_eq!(rough_count(45_400_000), "45.4 M");
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
    fn json_strings_are_escaped() {
        assert_eq!(json_string("a\"b\\c\n\u{1}"), r#""a\"b\\c\n\u0001""#);
    }

    #[test]
    fn phase_lines_keep_their_columns() {
        let piped = Style {
            unicode: false,
            colour: false,
            piped: true,
        };
        let line = piped.phase_done("reading", "1,830,402,118 rows", 101.0);
        assert_eq!(
            line,
            "  reading      1,830,402,118 rows                 1 min 41 s"
        );
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
