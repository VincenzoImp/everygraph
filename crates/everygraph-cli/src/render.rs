// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Text for people (`docs/ux-contract.md` § 2): the rendering of a stream, symbols and
//! their ASCII forms, and numbers, sizes and durations.

use std::io::IsTerminal;

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
