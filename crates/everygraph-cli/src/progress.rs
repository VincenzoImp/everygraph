// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Progress lines on standard error (`docs/ux-contract.md` § 4): one line per finished
//! phase, with a full bar on a terminal and without one in the piped form, which
//! `TERM=dumb` and `--progress=plain` also give (S-10). Nothing is redrawn.

use std::io::{self, Write};
use std::sync::{Mutex, PoisonError};
use std::time::Instant;

use everygraph_ingest::build::{Phase, Progress};

use crate::render::{Style, counted};

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
