// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Progress lines on standard error (`docs/ux-contract.md` § 4): one line per finished
//! phase, with a full bar on a terminal and without one in the piped form, which
//! `TERM=dumb` and `--progress=plain` also give (S-10). Nothing is redrawn.

use std::io::{self, Write};
use std::sync::{Mutex, PoisonError};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use everygraph_ingest::build::{Phase, Progress};

use std::path::Path;

use crate::render::{Style, counted, duration};

/// An operation estimated to take at least this long prints its plan and progress
/// (P-3, `T_PLAN`; placeholder). Without an estimate, a phase that took this long
/// prints its line.
pub(crate) const LONG_SECONDS: f64 = 5.0;

/// Now, in nanoseconds since 1970-01-01 UTC, as provenance records hold times.
pub(crate) fn now_ns() -> Option<i64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_nanos()).ok())
}

/// The first line of the result block (G-11), on standard error: `✓`, the name of the
/// file made, `facts`, and the time.
pub(crate) fn made(path: &Path, facts: &[String], seconds: f64) {
    let style = Style::stderr();
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let mut line = format!("{} {}", style.ok(), style.bold(&name));
    for fact in facts.iter().chain([&duration(seconds)]) {
        line.push_str(style.dot());
        line.push_str(fact);
    }
    let _ = writeln!(io::stderr(), "{line}");
}

/// The phase lines of one operation.
#[derive(Debug)]
pub(crate) struct Lines {
    style: Style,
    started: Mutex<Option<(&'static str, Instant)>>,
}

impl Lines {
    pub(crate) fn new(style: Style) -> Self {
        Self {
            style,
            started: Mutex::new(None),
        }
    }

    /// Phase `name` begins.
    pub(crate) fn start(&self, name: &'static str) {
        *self.started.lock().unwrap_or_else(PoisonError::into_inner) = Some((name, Instant::now()));
    }

    /// Phase `name` ended with `detail`: prints its line, and returns its time in
    /// seconds.
    pub(crate) fn done(&self, name: &'static str, detail: &str) -> f64 {
        let seconds = self.seconds(name);
        let line = self.style.phase_done(name, detail, seconds);
        let _ = writeln!(io::stderr(), "{line}");
        seconds
    }

    /// Phase `name` ended with `detail`: prints its line if it was long (P-3), and
    /// returns its time in seconds.
    pub(crate) fn done_if_long(&self, name: &'static str, detail: &str) -> f64 {
        let seconds = self.seconds(name);
        if seconds >= LONG_SECONDS {
            let line = self.style.phase_done(name, detail, seconds);
            let _ = writeln!(io::stderr(), "{line}");
        }
        seconds
    }

    /// Phase `name` was not run: its result exists (G-3).
    pub(crate) fn kept(&self, name: &'static str, detail: &str) {
        let line = self.style.phase_kept(name, detail);
        let _ = writeln!(io::stderr(), "{line}");
    }

    /// The time since phase `name` began, and its end.
    pub(crate) fn seconds(&self, name: &'static str) -> f64 {
        self.started
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .filter(|b| b.0 == name)
            .map_or(0.0, |b| b.1.elapsed().as_secs_f64())
    }
}

impl Progress for Lines {
    fn begin(&self, phase: Phase, _total: Option<u64>) {
        self.start(phase.name());
    }

    fn end(&self, phase: Phase, done: u64) {
        let detail = match phase {
            Phase::Reading => counted(done, "row", "rows"),
            Phase::Identifiers => counted(done, "identifier", "identifiers"),
            Phase::Building => counted(done, "part", "parts"),
            Phase::Checking => format!("{} verified", counted(done, "section", "sections")),
        };
        self.done(phase.name(), &detail);
    }
}
