// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Times: a number since the epoch, or ISO 8601, to microseconds since
//! 1970-01-01T00:00:00Z.

use super::parse::decimal;
use super::{ProblemKind, TimeUnit};

/// Parses a time field.
pub(super) fn parse(b: &[u8], unit: TimeUnit) -> Result<i64, ProblemKind> {
    if b.is_empty() {
        return Err(ProblemKind::Empty);
    }
    let numeric = b
        .strip_prefix(b"-")
        .unwrap_or(b)
        .iter()
        .all(|&c| c.is_ascii_digit() || c == b'.');
    if numeric { epoch(b, unit) } else { iso(b) }
}

fn epoch(b: &[u8], unit: TimeUnit) -> Result<i64, ProblemKind> {
    let (negative, digits) = match b.strip_prefix(b"-") {
        Some(rest) => (true, rest),
        None => (false, b),
    };
    let (whole, fraction) = match digits.iter().position(|&c| c == b'.') {
        Some(k) => (&digits[..k], &digits[k + 1..]),
        None => (digits, &digits[digits.len()..]),
    };
    if whole.is_empty() || fraction.contains(&b'.') {
        return Err(ProblemKind::NotATime);
    }
    let (scale, places) = match unit {
        TimeUnit::Seconds => (1_000_000, 6),
        TimeUnit::Milliseconds => (1_000, 3),
        TimeUnit::Microseconds => (1, 0),
    };
    let whole = decimal(whole).map_err(|_| ProblemKind::TimeOutOfRange)?;
    // Digits beyond a microsecond are dropped.
    let mut part = 0_i64;
    for k in 0..places {
        let digit = fraction.get(k).map_or(0, |&c| c - b'0');
        part = part * 10 + i64::from(digit);
    }
    let micros = i64::try_from(whole)
        .ok()
        .and_then(|w| w.checked_mul(scale))
        .and_then(|w| w.checked_add(part))
        .ok_or(ProblemKind::TimeOutOfRange)?;
    Ok(if negative { -micros } else { micros })
}

/// `n` ASCII digits at the start of `b`.
fn digits(b: &[u8], n: usize) -> Option<i64> {
    let field = b.get(..n)?;
    field.iter().try_fold(0_i64, |acc, &c| {
        c.is_ascii_digit().then(|| acc * 10 + i64::from(c - b'0'))
    })
}

/// `YYYY-MM-DD`, optionally followed by `T` (or a space) and `hh:mm[:ss[.f…]]`, and then
/// optionally by `Z` or an offset `±hh[[:]mm]`. Without an offset the time is UTC.
fn iso(b: &[u8]) -> Result<i64, ProblemKind> {
    iso_parts(b).ok_or(ProblemKind::NotATime)
}

fn iso_parts(b: &[u8]) -> Option<i64> {
    let year = digits(b, 4)?;
    let month = digits(b.get(5..)?, 2)?;
    let day = digits(b.get(8..)?, 2)?;
    if b[4] != b'-' || b[7] != b'-' || !(1..=12).contains(&month) {
        return None;
    }
    if day < 1 || day > days_in_month(year, month) {
        return None;
    }
    let mut seconds = days_from_civil(year, month, day) * 86_400;
    let mut micros = 0;
    let mut rest = &b[10..];
    if let Some((&sep, time)) = rest.split_first() {
        if !matches!(sep, b'T' | b't' | b' ') {
            return None;
        }
        let hour = digits(time, 2)?;
        let minute = digits(time.get(3..)?, 2)?;
        if time[2] != b':' || hour > 23 || minute > 59 {
            return None;
        }
        rest = &time[5..];
        let mut second = 0;
        if rest.first() == Some(&b':') {
            second = digits(rest.get(1..)?, 2)?;
            if second > 60 {
                return None;
            }
            rest = &rest[3..];
            if let Some(fraction) = rest.strip_prefix(b".").or_else(|| rest.strip_prefix(b",")) {
                let n = fraction.iter().take_while(|c| c.is_ascii_digit()).count();
                if n == 0 {
                    return None;
                }
                for k in 0..6 {
                    micros = micros * 10
                        + fraction
                            .get(k)
                            .filter(|_| k < n)
                            .map_or(0, |&c| i64::from(c - b'0'));
                }
                rest = &fraction[n..];
            }
        }
        seconds += hour * 3600 + minute * 60 + second;
        seconds -= offset(rest)?;
    }
    Some(seconds * 1_000_000 + micros)
}

/// Seconds east of UTC of `Z`, `±hh`, `±hhmm` or `±hh:mm`; nothing means UTC.
fn offset(b: &[u8]) -> Option<i64> {
    let (sign, rest) = match b.split_first() {
        None => return Some(0),
        Some((b'Z' | b'z', [])) => return Some(0),
        Some((b'+', rest)) => (1, rest),
        Some((b'-', rest)) => (-1, rest),
        Some(_) => return None,
    };
    let hours = digits(rest, 2)?;
    let minutes = match &rest[2..] {
        [] => 0,
        [b':', m @ ..] | m if m.len() == 2 => digits(m, 2)?,
        _ => return None,
    };
    (hours <= 23 && minutes <= 59).then_some(sign * (hours * 3600 + minutes * 60))
}

const fn is_leap(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

const fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days from 1970-01-01 to a date of the proleptic Gregorian calendar (H. Hinnant,
/// "chrono-compatible low-level date algorithms").
const fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: i64 = 1_000_000;

    #[test]
    fn epoch_numbers_in_each_unit() {
        assert_eq!(parse(b"0", TimeUnit::Seconds), Ok(0));
        assert_eq!(
            parse(b"1700000000", TimeUnit::Seconds),
            Ok(1_700_000_000 * S)
        );
        assert_eq!(
            parse(b"1289241911.72836", TimeUnit::Seconds),
            Ok(1_289_241_911_728_360)
        );
        assert_eq!(parse(b"-1.5", TimeUnit::Seconds), Ok(-1_500_000));
        assert_eq!(parse(b"1.0000019", TimeUnit::Seconds), Ok(1_000_001));
        assert_eq!(
            parse(b"1700000000123", TimeUnit::Milliseconds),
            Ok(1_700_000_000_123_000)
        );
        assert_eq!(parse(b"12.5", TimeUnit::Milliseconds), Ok(12_500));
        assert_eq!(parse(b"7", TimeUnit::Microseconds), Ok(7));
        assert_eq!(parse(b"1.", TimeUnit::Seconds), Ok(S));
    }

    #[test]
    fn bad_and_out_of_range_numbers() {
        assert_eq!(parse(b"", TimeUnit::Seconds), Err(ProblemKind::Empty));
        assert_eq!(parse(b".5", TimeUnit::Seconds), Err(ProblemKind::NotATime));
        assert_eq!(
            parse(b"1.2.3", TimeUnit::Seconds),
            Err(ProblemKind::NotATime)
        );
        assert_eq!(parse(b"-", TimeUnit::Seconds), Err(ProblemKind::NotATime));
        // 2^63 µs is about 292,000 years.
        assert_eq!(
            parse(b"9300000000000", TimeUnit::Seconds),
            Err(ProblemKind::TimeOutOfRange)
        );
        assert_eq!(
            parse(b"99999999999999999999", TimeUnit::Seconds),
            Err(ProblemKind::TimeOutOfRange)
        );
        assert_eq!(
            parse(b"9223372036854775807", TimeUnit::Microseconds),
            Ok(i64::MAX)
        );
    }

    #[test]
    fn iso_dates_and_times() {
        assert_eq!(parse(b"1970-01-01", TimeUnit::Seconds), Ok(0));
        assert_eq!(
            parse(b"2024-03-02T11:04:35Z", TimeUnit::Seconds),
            Ok(1_709_377_475 * S)
        );
        assert_eq!(
            parse(b"2024-03-02 11:04:35", TimeUnit::Seconds),
            Ok(1_709_377_475 * S)
        );
        assert_eq!(
            parse(b"2024-03-02T12:04:35+01:00", TimeUnit::Seconds),
            Ok(1_709_377_475 * S)
        );
        assert_eq!(
            parse(b"2024-03-02T06:04:35-0500", TimeUnit::Seconds),
            Ok(1_709_377_475 * S)
        );
        assert_eq!(
            parse(b"2024-03-02T13:04:35+02", TimeUnit::Seconds),
            Ok(1_709_377_475 * S)
        );
        assert_eq!(
            parse(b"2024-03-02T11:04:35.25Z", TimeUnit::Seconds),
            Ok(1_709_377_475 * S + 250_000)
        );
        assert_eq!(
            parse(b"2024-03-02T11:04:35,1234567", TimeUnit::Seconds),
            Ok(1_709_377_475 * S + 123_456)
        );
        assert_eq!(
            parse(b"2024-03-02T11:04", TimeUnit::Seconds),
            Ok(1_709_377_440 * S)
        );
        assert_eq!(parse(b"2000-02-29", TimeUnit::Seconds), Ok(951_782_400 * S));
        assert_eq!(parse(b"1969-12-31T23:59:59Z", TimeUnit::Seconds), Ok(-S));
        assert_eq!(
            parse(b"0000-03-01", TimeUnit::Seconds),
            Ok(-62_162_035_200 * S)
        );
    }

    #[test]
    fn malformed_iso_is_not_a_time() {
        for bad in [
            &b"2024-13-01"[..],
            b"2024-02-30",
            b"1900-02-29",
            b"2024-3-02",
            b"2024/03/02",
            b"2024-03-02T",
            b"2024-03-02T24:00",
            b"2024-03-02T11:60",
            b"2024-03-02T11:04:35.",
            b"2024-03-02T11:04:35Q",
            b"2024-03-02T11:04:35+1",
            b"2024-03-02T11:04:35+01:0",
            b"2024-03-02Z",
            b"n/a",
        ] {
            assert_eq!(
                parse(bad, TimeUnit::Seconds),
                Err(ProblemKind::NotATime),
                "{}",
                String::from_utf8_lossy(bad)
            );
        }
    }
}
