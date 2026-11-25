// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Sizes, durations and timestamps as the spec files write them and the rows record them.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

const KIB: f64 = 1024.0;

/// Parses a size as `systemd` does for `MemoryMax=`: a number with an optional suffix
/// `K`, `M`, `G` or `T`, each a power of 1024 (`8G` = 8 GiB). A plain number is bytes.
///
/// # Errors
///
/// When the text is not such a size, or is zero.
pub fn parse_size(text: &str) -> Result<u64, String> {
    let t = text.trim();
    let t = t
        .strip_suffix("iB")
        .or_else(|| {
            t.strip_suffix('B')
                .filter(|r| r.ends_with(['K', 'M', 'G', 'T']))
        })
        .unwrap_or(t);
    let (number, factor) = match t.chars().last() {
        Some('K' | 'k') => (&t[..t.len() - 1], KIB),
        Some('M' | 'm') => (&t[..t.len() - 1], KIB * KIB),
        Some('G' | 'g') => (&t[..t.len() - 1], KIB * KIB * KIB),
        Some('T' | 't') => (&t[..t.len() - 1], KIB * KIB * KIB * KIB),
        _ => (t, 1.0),
    };
    let value: f64 = number
        .trim()
        .parse()
        .map_err(|_| format!("{text:?} is not a size; write it like 512M, 8G or 1.5T"))?;
    let bytes = value * factor;
    if !(bytes >= 1.0 && bytes < 2f64.powi(63)) {
        return Err(format!(
            "{text:?} is not a usable size (it must be at least 1 byte)"
        ));
    }
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "checked to lie in [1, 2^63) just above"
    )]
    Ok(bytes.round() as u64)
}

/// Parses a duration: a number followed by `ms`, `s`, `m` or `h` (`500ms`, `2s`, `10m`).
///
/// # Errors
///
/// When the text is not such a duration.
pub fn parse_duration(text: &str) -> Result<Duration, String> {
    let t = text.trim();
    let (number, unit) = if let Some(n) = t.strip_suffix("ms") {
        (n, 0.001)
    } else if let Some(n) = t.strip_suffix('s') {
        (n, 1.0)
    } else if let Some(n) = t.strip_suffix('m') {
        (n, 60.0)
    } else if let Some(n) = t.strip_suffix('h') {
        (n, 3600.0)
    } else {
        return Err(format!(
            "{text:?} is not a duration; write it with a unit, like 500ms, 2s or 10m"
        ));
    };
    let value: f64 = number
        .trim()
        .parse()
        .map_err(|_| format!("{text:?} is not a duration; write it like 500ms, 2s or 10m"))?;
    Duration::try_from_secs_f64(value * unit)
        .map_err(|_| format!("{text:?} is not a usable duration"))
}

/// A size for people: `8.0 GiB`, `742 MiB`, `12 KiB`, `17 B`.
#[must_use]
pub fn human_size(bytes: u64) -> String {
    #[allow(
        clippy::cast_precision_loss,
        reason = "a size shown to three digits does not need 53 bits"
    )]
    let b = bytes as f64;
    if b >= KIB * KIB * KIB {
        format!("{:.1} GiB", b / (KIB * KIB * KIB))
    } else if b >= KIB * KIB {
        format!("{:.0} MiB", b / (KIB * KIB))
    } else if b >= KIB {
        format!("{:.0} KiB", b / KIB)
    } else {
        format!("{bytes} B")
    }
}

/// Seconds since the Unix epoch, now.
#[must_use]
pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The civil date (UTC) of a day count since 1970-01-01 (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "d is in 1..=31 and m in 1..=12 by construction"
    )]
    (y, m as u32, d as u32)
}

fn split(unix: u64) -> (i64, u32, u32, u64, u64, u64) {
    let days = i64::try_from(unix / 86_400).unwrap_or(i64::MAX / 2);
    let (y, m, d) = civil_from_days(days);
    let s = unix % 86_400;
    (y, m, d, s / 3600, s % 3600 / 60, s % 60)
}

/// `2026-11-25T09:06:42Z`.
#[must_use]
pub fn rfc3339(unix: u64) -> String {
    let (y, m, d, hh, mm, ss) = split(unix);
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

/// `20261125T090642Z`, for file names that sort by time.
#[must_use]
pub fn compact_timestamp(unix: u64) -> String {
    let (y, m, d, hh, mm, ss) = split(unix);
    format!("{y:04}{m:02}{d:02}T{hh:02}{mm:02}{ss:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_follow_systemd() {
        assert_eq!(parse_size("8G"), Ok(8 << 30));
        assert_eq!(parse_size("512M"), Ok(512 << 20));
        assert_eq!(parse_size("1.5G"), Ok(3 << 29));
        assert_eq!(parse_size("2GiB"), Ok(2 << 30));
        assert_eq!(parse_size("4096"), Ok(4096));
        assert_eq!(parse_size(" 1T "), Ok(1 << 40));
        for bad in ["", "G", "eight", "0", "-1G", "8X"] {
            assert!(parse_size(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn durations_need_a_unit() {
        assert_eq!(parse_duration("500ms"), Ok(Duration::from_millis(500)));
        assert_eq!(parse_duration("2s"), Ok(Duration::from_secs(2)));
        assert_eq!(parse_duration("1.5m"), Ok(Duration::from_secs(90)));
        assert_eq!(parse_duration("1h"), Ok(Duration::from_secs(3600)));
        for bad in ["10", "s", "-1s", "fast"] {
            assert!(parse_duration(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn human_sizes() {
        assert_eq!(human_size(17), "17 B");
        assert_eq!(human_size(8 << 30), "8.0 GiB");
        assert_eq!(human_size(742 << 20), "742 MiB");
    }

    #[test]
    fn timestamps_are_utc() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        // 2026-11-25T09:06:42Z
        assert_eq!(rfc3339(1_795_597_602), "2026-11-25T09:06:42Z");
        assert_eq!(compact_timestamp(1_795_597_602), "20261125T090642Z");
        // A leap day.
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
    }
}
