// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The idle gate of official runs, and the disturbance check of every repetition.
//!
//! The gate looks at the 1-minute load average and `MemAvailable` once, before the run.
//! After that the load average is useless: it counts the benchmark itself and decays
//! over minutes. So each repetition measures instead what *other* processes did:
//!
//! - in a quiet "settle" window just before it (and, for runs shorter than
//!   [`MIN_DURING_WINDOW`], just after it too): busy cores from `/proc/stat` (user and
//!   system time) and storage traffic from `/proc/diskstats`, nothing of ours running;
//! - while it ran (runs of [`MIN_DURING_WINDOW`] and more): the machine's user time minus
//!   the run's own (its cgroup's `cpu.stat`), and the machine's storage traffic minus
//!   the run's own (`/proc/self/io` of the helper).
//!
//! User time only during the run, because the kernel's work for the run (write-back,
//! reclaim) is system time charged outside its cgroup. One busy core skewed lab timings
//! by 10–40%; in an official run a repetition above a threshold is kept, marked
//! disturbed, and left out of every summary.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::procfs::{CpuTimes, LoadAvg, USER_HZ};
use super::spec::Gate;
use super::units::human_size;

/// Shorter windows are too short for `/proc/stat` (10 ms ticks summed over all CPUs),
/// and for a rate of storage traffic, to say anything about other processes; short runs
/// get a settle window after them instead.
pub const MIN_DURING_WINDOW: Duration = Duration::from_secs(1);

/// What the gate looks at before an official run.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StartReadings {
    /// Load averages.
    pub load: LoadAvg,
    /// `MemAvailable`, bytes.
    pub mem_available: u64,
}

/// Refuses an official run on a busy machine. `need` is the memory the run needs (the
/// spec's `min_mem_available`, else its largest memory limit).
///
/// # Errors
///
/// The reason, with what to do about it.
pub fn check_start(r: &StartReadings, gate: &Gate, need: u64, max_load: f64) -> Result<(), String> {
    if r.load.one > max_load {
        return Err(format!(
            "the machine is not idle: the 1-minute load average is {:.2}, above {max_load:.2}. \
             Official results are measured on an idle machine only (CLAUDE.md § 3): wait \
             until nothing else runs (`uptime`; the average falls by about 63% a minute), \
             or run without --official",
            r.load.one
        ));
    }
    let need = gate.min_mem_available.unwrap_or(need);
    if r.mem_available < need {
        return Err(format!(
            "only {} of memory is available and this run needs {} (its largest memory \
             limit, or [gate] min_mem_available). Free memory or select fewer cases",
            human_size(r.mem_available),
            human_size(need)
        ));
    }
    Ok(())
}

/// Average number of busy cores between two `/proc/stat` readings `elapsed` apart.
#[must_use]
pub fn busy_cores(before: CpuTimes, after: CpuTimes, elapsed: Duration) -> f64 {
    let secs = elapsed.as_secs_f64();
    if secs <= 0.0 {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss, reason = "tick counts far below 2^53")]
    let ticks = after.busy.saturating_sub(before.busy) as f64;
    ticks / USER_HZ / secs
}

/// Storage traffic in MB/s between two `/proc/diskstats` byte totals `elapsed` apart.
#[must_use]
pub fn io_mb_s(before: u64, after: u64, elapsed: Duration) -> f64 {
    let secs = elapsed.as_secs_f64();
    if secs <= 0.0 {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss, reason = "a rate shown to a few digits")]
    let bytes = after.saturating_sub(before) as f64;
    bytes / 1e6 / secs
}

/// Average number of cores other processes kept busy *in user mode* while a run used
/// `own_user` of user CPU time over `wall`; `None` for runs shorter than
/// [`MIN_DURING_WINDOW`].
#[must_use]
pub fn others_cores(
    before: CpuTimes,
    after: CpuTimes,
    own_user: Duration,
    wall: Duration,
) -> Option<f64> {
    if wall < MIN_DURING_WINDOW {
        return None;
    }
    #[allow(clippy::cast_precision_loss, reason = "tick counts far below 2^53")]
    let user_ticks = after.user.saturating_sub(before.user) as f64;
    let all = user_ticks / USER_HZ / wall.as_secs_f64();
    Some((all - own_user.as_secs_f64() / wall.as_secs_f64()).max(0.0))
}

/// Storage traffic of others, in MB/s, while a run moved `own_bytes` (read + written)
/// over `wall`: the machine's physical devices minus the run's own; `None` for runs
/// shorter than [`MIN_DURING_WINDOW`]. The run's writes count when its pages are
/// dirtied, the devices' when they are written back, so this errs toward zero.
#[must_use]
pub fn others_io(disk_before: u64, disk_after: u64, own_bytes: u64, wall: Duration) -> Option<f64> {
    if wall < MIN_DURING_WINDOW {
        return None;
    }
    let machine = disk_after.saturating_sub(disk_before);
    Some(io_mb_s(0, machine.saturating_sub(own_bytes), wall))
}

/// What other processes did around one repetition. Missing readings are `None`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Activity {
    /// Busy cores in the settle window before the repetition (nothing of ours ran).
    pub settle_busy_cores: Option<f64>,
    /// Storage traffic in that window, MB/s.
    pub settle_io_mb_s: Option<f64>,
    /// Busy cores in a settle window after it (runs shorter than [`MIN_DURING_WINDOW`]).
    pub after_busy_cores: Option<f64>,
    /// Storage traffic in that window, MB/s.
    pub after_io_mb_s: Option<f64>,
    /// Others' user-mode cores while it ran ([`others_cores`]).
    pub others_cores_during: Option<f64>,
    /// Others' storage traffic while it ran, MB/s ([`others_io`]).
    pub others_io_mb_s_during: Option<f64>,
}

/// The disturbance verdict recorded in each row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Disturbance {
    /// The readings.
    #[serde(flatten)]
    pub activity: Activity,
    /// Cores above which others disturb.
    pub threshold_cores: f64,
    /// MB/s of storage traffic above which others disturb.
    pub threshold_io_mb_s: f64,
    /// Which readings were above their threshold.
    pub reasons: Vec<String>,
    /// Some reading above its threshold, in an official run. Exploratory runs record the
    /// readings and the reasons but claim no idleness, so they mark nothing.
    pub disturbed: bool,
}

/// Applies the thresholds of `gate` to the readings.
#[must_use]
pub fn classify(a: Activity, gate: &Gate, official: bool) -> Disturbance {
    let mut reasons = Vec::new();
    let mut check = |what: &str, value: Option<f64>, limit: f64, unit: &str| {
        if let Some(v) = value.filter(|v| *v > limit) {
            reasons.push(format!("{what} {v:.2} {unit} > {limit:.2}"));
        }
    };
    let (cores, io) = (gate.disturbed_above_cores, gate.disturbed_above_io_mb_s);
    check("busy cores before", a.settle_busy_cores, cores, "cores");
    check("storage traffic before", a.settle_io_mb_s, io, "MB/s");
    check("busy cores after", a.after_busy_cores, cores, "cores");
    check("storage traffic after", a.after_io_mb_s, io, "MB/s");
    check(
        "others' cores during",
        a.others_cores_during,
        cores,
        "cores",
    );
    check(
        "others' storage traffic during",
        a.others_io_mb_s_during,
        io,
        "MB/s",
    );
    Disturbance {
        activity: a,
        threshold_cores: cores,
        threshold_io_mb_s: io,
        disturbed: official && !reasons.is_empty(),
        reasons,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate() -> Gate {
        Gate {
            max_load: 1.0,
            disturbed_above_cores: 0.5,
            disturbed_above_io_mb_s: 50.0,
            min_mem_available: None,
        }
    }

    fn readings(one: f64, mem_gib: u64) -> StartReadings {
        StartReadings {
            load: LoadAvg {
                one,
                five: one,
                fifteen: one,
            },
            mem_available: mem_gib << 30,
        }
    }

    #[test]
    fn the_gate_refuses_load_and_missing_memory() {
        assert!(check_start(&readings(0.4, 50), &gate(), 8 << 30, 1.0).is_ok());
        let busy = check_start(&readings(3.2, 50), &gate(), 8 << 30, 1.0).unwrap_err();
        assert!(busy.contains("3.20, above 1.00"), "{busy}");
        let short = check_start(&readings(0.1, 4), &gate(), 8 << 30, 1.0).unwrap_err();
        assert!(
            short.contains("4.0 GiB") && short.contains("8.0 GiB"),
            "{short}"
        );
        let explicit = Gate {
            min_mem_available: Some(2 << 30),
            ..gate()
        };
        assert!(check_start(&readings(0.1, 4), &explicit, 8 << 30, 1.0).is_ok());
    }

    #[test]
    fn busy_and_others_cores() {
        // busy and user ticks alike; `others_cores` reads `user`.
        let t = |ticks| CpuTimes {
            busy: ticks,
            user: ticks,
            total: 0,
        };
        assert!((busy_cores(t(1000), t(1150), Duration::from_secs(1)) - 1.5).abs() < 1e-9);
        // 32 cores in user mode for 2 s (6400 ticks) of which our cgroup used 62 s: one
        // core of others.
        let o = others_cores(
            t(0),
            t(6400),
            Duration::from_secs(62),
            Duration::from_secs(2),
        );
        assert!((o.expect("long enough") - 1.0).abs() < 1e-9);
        assert_eq!(
            others_cores(t(0), t(100), Duration::from_secs(5), Duration::from_secs(2)),
            Some(0.0)
        );
        assert_eq!(
            others_cores(t(0), t(100), Duration::ZERO, Duration::from_millis(300)),
            None
        );
        // System time outside the cgroup (kernel write-back for the run) is not counted.
        let before = CpuTimes {
            busy: 0,
            user: 0,
            total: 0,
        };
        let after = CpuTimes {
            busy: 6400,
            user: 6200,
            total: 0,
        };
        assert_eq!(
            others_cores(
                before,
                after,
                Duration::from_secs(62),
                Duration::from_secs(2)
            ),
            Some(0.0)
        );
    }

    #[test]
    fn others_storage_traffic_is_the_machine_minus_the_run() {
        // 300 MB on the devices in 2 s, 200 MB of them the run's: 50 MB/s of others.
        let o = others_io(
            1_000,
            1_000 + 300_000_000,
            200_000_000,
            Duration::from_secs(2),
        );
        assert!((o.expect("long enough") - 50.0).abs() < 1e-9);
        // The run's own traffic alone is no disturbance.
        assert_eq!(
            others_io(0, 100_000_000, 100_000_000, Duration::from_secs(2)),
            Some(0.0)
        );
        assert_eq!(others_io(0, 1, 0, Duration::from_millis(10)), None);
        assert!((io_mb_s(0, 5_000_000, Duration::from_millis(500)) - 10.0).abs() < 1e-9);
    }

    #[test]
    fn official_runs_mark_any_reading_above_its_threshold() {
        let quiet = Activity {
            settle_busy_cores: Some(0.1),
            settle_io_mb_s: Some(1.0),
            others_cores_during: Some(0.2),
            others_io_mb_s_during: Some(3.0),
            ..Activity::default()
        };
        let d = classify(quiet, &gate(), true);
        assert!(!d.disturbed && d.reasons.is_empty());
        for busy in [
            Activity {
                settle_busy_cores: Some(0.9),
                ..quiet
            },
            Activity {
                settle_io_mb_s: Some(400.0),
                ..quiet
            },
            Activity {
                after_busy_cores: Some(1.2),
                ..quiet
            },
            Activity {
                after_io_mb_s: Some(90.0),
                ..quiet
            },
            Activity {
                others_cores_during: Some(1.0),
                ..quiet
            },
            Activity {
                others_io_mb_s_during: Some(500.0),
                ..quiet
            },
        ] {
            let d = classify(busy, &gate(), true);
            assert!(d.disturbed, "{busy:?}");
            assert_eq!(d.reasons.len(), 1, "{:?}", d.reasons);
            // An exploratory run records the reason and marks nothing.
            let e = classify(busy, &gate(), false);
            assert!(!e.disturbed);
            assert_eq!(e.reasons, d.reasons);
        }
        assert!(!classify(Activity::default(), &gate(), true).disturbed);
    }
}
