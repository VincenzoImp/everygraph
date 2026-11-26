// SPDX-License-Identifier: Apache-2.0 OR MIT

//! How a long call runs (`docs/api-sketch.md` § 2.9): the engine works on a pool of its
//! own threads with the interpreter released, while the calling thread waits, hands
//! each progress event to the package on that thread, and asks Python every 100 ms
//! whether a signal has arrived.
//!
//! The engine has no cancellation yet (`Run::cancel` of § 4), so a `KeyboardInterrupt`,
//! or an exception of the progress function, is raised once the work has ended, and its
//! result is dropped: no file is written by a stopped build.

use std::num::NonZeroUsize;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use everygraph_core::par;
use everygraph_ingest::build::{Phase, Progress};
use pyo3::prelude::*;

use crate::error::Failure;

/// How often the waiting thread asks Python for signals.
const TICK: Duration = Duration::from_millis(100);

/// One progress event of the engine.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Event {
    /// A phase starts; its count, when known.
    Begin(&'static str, Option<u64>),
    /// More of a phase's count is done.
    Advance(&'static str, u64),
    /// A phase ended, having made this many of its unit.
    End(&'static str, u64),
}

/// Where the engine sends its events, from any thread.
#[derive(Debug)]
pub(crate) struct Events(Sender<(Event, f64)>, Instant);

impl Events {
    pub(crate) fn send(&self, event: Event) {
        let _ = self.0.send((event, self.1.elapsed().as_secs_f64()));
    }
}

impl Progress for Events {
    fn begin(&self, phase: Phase, total: Option<u64>) {
        self.send(Event::Begin(phase.name(), total));
    }

    fn advance(&self, phase: Phase, done: u64) {
        self.send(Event::Advance(phase.name(), done));
    }

    fn end(&self, phase: Phase, made: u64) {
        self.send(Event::End(phase.name(), made));
    }
}

/// Runs `work` on a pool of `threads` threads. `on_event(kind, phase, value, t)` is
/// called on this thread for each event, `kind` being `begin`, `advance` or `end`,
/// `value` the count (or `None`), `t` the seconds since the start.
///
/// # Errors
///
/// `KeyboardInterrupt` or the exception of `on_event`, after the work has ended; an
/// engine error if the threads cannot be started.
pub(crate) fn run<T: Send>(
    py: Python<'_>,
    threads: NonZeroUsize,
    on_event: &Bound<'_, PyAny>,
    work: impl FnOnce(&Events) -> T + Send,
) -> PyResult<T> {
    let (tx, rx) = mpsc::channel();
    let rx = Mutex::new(rx);
    let recv = || {
        rx.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .recv_timeout(TICK)
    };
    let events = Events(tx, Instant::now());
    let mut stop: Option<PyErr> = None;
    let outcome = std::thread::scope(|s| {
        let worker = s.spawn(move || {
            let events = events;
            par::with_threads(threads, || work(&events))
        });
        loop {
            match py.detach(recv) {
                Ok((event, t)) if stop.is_none() => {
                    let (kind, phase, value) = match event {
                        Event::Begin(p, total) => ("begin", p, total),
                        Event::Advance(p, done) => ("advance", p, Some(done)),
                        Event::End(p, made) => ("end", p, Some(made)),
                    };
                    if let Err(e) = on_event.call1((kind, phase, value, t)) {
                        stop = Some(e);
                    }
                }
                Ok(_) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            if stop.is_none() {
                stop = py.check_signals().err();
            }
        }
        py.detach(|| worker.join())
    });
    if let Some(e) = stop {
        return Err(e);
    }
    match outcome {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) => Err(Failure::new("EverygraphError")
            .text("message", format!("Cannot start the threads: {e}."))
            .into_err(py)),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}
