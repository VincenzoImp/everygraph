// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The idle gate of official runs, and the disturbance check of every repetition.
//!
//! The gate looks at the 1-minute load average and `MemAvailable` once, before the run.
//! After that the load average is useless: it counts the benchmark itself and decays
//! over minutes. So each repetition measures instead how many cores *other* processes
//! kept busy — in a quiet "settle" window just before it, from `/proc/stat`, and while
//! it ran, from `/proc/stat` minus the run's own cgroup CPU time. One busy core skewed
//! lab timings by 10–40%; a repetition above the threshold is kept, marked disturbed,
//! and left out of every summary.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::procfs::{CpuTimes, LoadAvg, USER_HZ};
use super::spec::Gate;
use super::units::human_size;

/// Shorter runs are too short for `/proc/stat` (10 ms ticks summed over all CPUs) to say
/// anything about other processes; for them only the settle window counts.
pub const MIN_DURING_WINDOW: Duration = Duration::from_secs(1);

/// What the gate looks at before an official run.
#[derive(Debug, Clone, Copy, PartialEq)]
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

/// Average number of cores other processes kept busy while a run used `own_cpu` of CPU
/// time over `wall`; `None` for runs shorter than [`MIN_DURING_WINDOW`].
#[must_use]
pub fn others_cores(
    before: CpuTimes,
    after: CpuTimes,
    own_cpu: Duration,
    wall: Duration,
) -> Option<f64> {
    if wall < MIN_DURING_WINDOW {
        return None;
    }
    let all = busy_cores(before, after, wall);
    Some((all - own_cpu.as_secs_f64() / wall.as_secs_f64()).max(0.0))
}

/// The disturbance verdict recorded in each row.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Disturbance {
    /// Busy cores in the settle window before the repetition (nothing of ours ran).
    pub settle_busy_cores: Option<f64>,
    /// Cores used by others while it ran ([`others_cores`]).
    pub others_cores_during: Option<f64>,
    /// The threshold applied.
    pub threshold_cores: f64,
    /// Either reading above the threshold, in an official run (exploratory runs
    /// record the readings but claim no idleness, so they mark nothing).
    pub disturbed: bool,
}

/// Applies the threshold.
#[must_use]
pub fn classify(settle: Option<f64>, during: Option<f64>, threshold: f64) -> Disturbance {
    let above = |x: Option<f64>| x.is_some_and(|v| v > threshold);
    Disturbance {
        settle_busy_cores: settle,
        others_cores_during: during,
        threshold_cores: threshold,
        disturbed: above(settle) || above(during),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate() -> Gate {
        Gate {
            max_load: 1.0,
            disturbed_above_cores: 0.5,
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
        // The threshold can be raised from the command line.
        assert!(check_start(&readings(3.2, 50), &gate(), 8 << 30, 4.0).is_ok());
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
        let t = |busy| CpuTimes { busy, total: 0 };
        // 150 ticks in 1 s = 1.5 cores.
        assert!((busy_cores(t(1000), t(1150), Duration::from_secs(1)) - 1.5).abs() < 1e-9);
        // 32 cores busy for 2 s (6400 ticks) of which our cgroup used 62 s of CPU: 1 core
        // of others.
        let o = others_cores(
            t(0),
            t(6400),
            Duration::from_secs(62),
            Duration::from_secs(2),
        );
        assert!((o.expect("long enough") - 1.0).abs() < 1e-9);
        // Never negative (tick rounding), and nothing for short runs.
        assert_eq!(
            others_cores(t(0), t(100), Duration::from_secs(5), Duration::from_secs(2)),
            Some(0.0)
        );
        assert_eq!(
            others_cores(t(0), t(100), Duration::ZERO, Duration::from_millis(300)),
            None
        );
    }

    #[test]
    fn disturbance_needs_either_reading_above_the_threshold() {
        assert!(!classify(Some(0.1), None, 0.5).disturbed);
        assert!(classify(Some(0.9), None, 0.5).disturbed);
        assert!(classify(Some(0.1), Some(1.2), 0.5).disturbed);
        assert!(!classify(None, None, 0.5).disturbed);
    }
}
