// SPDX-License-Identifier: Apache-2.0 OR MIT

//! How a long call runs (`docs/api-sketch.md` § 2.9): the engine works on a pool of its
//! own threads with the interpreter released, while the calling thread waits, hands
//! each progress event to the package on that thread, and asks Python every 100 ms
//! whether a signal has arrived.
//!
//! The engine has no cancellation yet (`Run::cancel` of § 4). So the first
//! `KeyboardInterrupt`, or an exception of the progress function, is held: the work
//! sees it through [`Events::stopped`] between its stages and drops its result, and the
//! exception is raised once the work has ended. A second signal while one is held ends
//! the process at once (`docs/ux-contract.md` C-5), which C-1 makes safe: no file is
//! ever half-written.

use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use everygraph_core::{Budget, par};
use everygraph_ingest::build::{Phase, Progress};
use pyo3::prelude::*;

use crate::error::Failure;

/// How often the waiting thread asks Python for signals.
const TICK: Duration = Duration::from_millis(100);

/// The exit status of a process ended by a second `Ctrl-C` (128 + SIGINT).
const HARD_STOP: i32 = 130;

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
pub(crate) struct Events {
    to: Sender<(Event, f64)>,
    start: Instant,
    stop: Arc<AtomicBool>,
}

impl Events {
    pub(crate) fn send(&self, event: Event) {
        let _ = self.to.send((event, self.start.elapsed().as_secs_f64()));
    }

    /// Whether the caller asked to stop: the work drops its result.
    pub(crate) fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
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

/// What stops a call: the first exception, and how many signals came.
#[derive(Default)]
struct Stop {
    first: Option<PyErr>,
    signals: u32,
}

impl Stop {
    fn exception(&mut self, e: PyErr) {
        self.first.get_or_insert(e);
    }

    /// Takes a signal, if one came; a second one ends the process at once.
    fn signals(&mut self, py: Python<'_>) {
        if let Err(e) = py.check_signals() {
            self.signals += 1;
            if self.signals > 1 {
                std::process::exit(HARD_STOP);
            }
            self.exception(e);
        }
    }
}

/// Runs `work` on a pool of `threads` threads. `on_event(kind, phase, value, t, peak)`
/// is called on this thread for each event, `kind` being `begin`, `advance` or `end`,
/// `value` the count (or `None`), `t` the seconds since the start, `peak` the most bytes
/// `budget` has held.
///
/// # Errors
///
/// `KeyboardInterrupt` or the exception of `on_event`, after the work has ended; an
/// engine error if the threads cannot be started.
pub(crate) fn run<T: Send>(
    py: Python<'_>,
    threads: NonZeroUsize,
    budget: &Budget,
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
    let stopping = Arc::new(AtomicBool::new(false));
    let events = Events {
        to: tx,
        start: Instant::now(),
        stop: Arc::clone(&stopping),
    };
    let mut stop = Stop::default();
    let outcome = std::thread::scope(|s| {
        let worker = s.spawn(move || {
            let events = events;
            par::with_threads(threads, || work(&events))
        });
        loop {
            match py.detach(recv) {
                Ok((event, t)) if stop.first.is_none() => {
                    let (kind, phase, value) = match event {
                        Event::Begin(p, total) => ("begin", p, total),
                        Event::Advance(p, done) => ("advance", p, Some(done)),
                        Event::End(p, made) => ("end", p, Some(made)),
                    };
                    if let Err(e) = on_event.call1((kind, phase, value, t, budget.peak())) {
                        stop.exception(e);
                    }
                }
                Ok(_) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            stop.signals(py);
            stopping.store(stop.first.is_some(), Ordering::Relaxed);
        }
        py.detach(|| worker.join())
    });
    // A signal that came after the last check is taken here, not later in the caller's
    // own code.
    stop.signals(py);
    if let Some(e) = stop.first {
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
