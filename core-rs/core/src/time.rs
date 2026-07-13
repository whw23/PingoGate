//! Minimal UTC time formatting for status/observability output.
//!
//! The locked dependency stack (constitution III) carries no date library, and
//! the gateway only ever needs a coarse, RFC 3339 wall-clock stamp for the
//! reload status and structured logs. This module formats a Unix timestamp into
//! `YYYY-MM-DDTHH:MM:SSZ` with pure integer arithmetic - no dependency, no
//! `unsafe`, and no panic path. Date conversion uses Howard Hinnant's exact
//! `civil_from_days` algorithm (valid across the entire `u64`-seconds range).

use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds since the Unix epoch, or `0` if the clock is before the epoch.
pub fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The current UTC wall-clock as an RFC 3339 / ISO 8601 string.
pub fn now_rfc3339() -> String {
    format_rfc3339(now_unix_secs())
}

/// Format `unix_secs` (seconds since the epoch, UTC) as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn format_rfc3339(unix_secs: u64) -> String {
    let days = (unix_secs / 86_400) as i64;
    let secs_of_day = unix_secs % 86_400;
    let (hour, minute, second) = (
        secs_of_day / 3_600,
        (secs_of_day % 3_600) / 60,
        secs_of_day % 60,
    );
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Convert a count of days since 1970-01-01 into a `(year, month, day)` UTC date.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // day of era, [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day of year, [0, 365]
    let mp = (5 * doy + 2) / 153; // month index, [0, 11]
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_formats_as_unix_zero() {
        assert_eq!(format_rfc3339(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn known_timestamp_formats_correctly() {
        // 1_700_000_000 == 2023-11-14T22:13:20Z
        assert_eq!(format_rfc3339(1_700_000_000), "2023-11-14T22:13:20Z");
    }

    #[test]
    fn leap_day_is_handled() {
        // 1_582_934_400 == 2020-02-29T00:00:00Z
        assert_eq!(format_rfc3339(1_582_934_400), "2020-02-29T00:00:00Z");
    }

    #[test]
    fn now_is_well_formed_and_after_2020() {
        let s = now_rfc3339();
        assert_eq!(s.len(), 20, "RFC3339 seconds-precision Z form");
        assert!(s.ends_with('Z'));
        assert!(&s[..4] >= "2020");
    }
}
