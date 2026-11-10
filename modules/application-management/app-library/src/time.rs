//! RFC 3339 UTC timestamps without a date crate.
//!
//! The catalogue stores timestamps as text, and the only shape it needs is
//! `YYYY-MM-DDTHH:MM:SSZ`. That is a fixed number of characters computed from a
//! Unix epoch second and the proleptic Gregorian calendar, so a date crate would
//! add a dependency and a supply-chain surface for a function of two dozen
//! lines. The conversion is Howard Hinnant's `civil_from_days` algorithm, which
//! is exact for every day SQLite can hold.

use std::time::{SystemTime, UNIX_EPOCH};

const SECONDS_PER_DAY: u64 = 86_400;
const SECONDS_PER_HOUR: u64 = 3_600;
const SECONDS_PER_MINUTE: u64 = 60;

/// Returns the current time as an RFC 3339 UTC timestamp: `YYYY-MM-DDTHH:MM:SSZ`.
///
/// The result is rounded down to the second and always carries the `Z` suffix,
/// so callers can store and order it as text. A clock set before the Unix epoch
/// is reported as the epoch itself: the catalogue only sorts and displays this
/// text, and a pre-epoch instant would be a worse lie than an obviously early
/// one.
pub fn now_utc() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    format_utc(seconds)
}

/// Formats an epoch second as an RFC 3339 UTC timestamp.
fn format_utc(seconds_since_epoch: u64) -> String {
    let days = seconds_since_epoch / SECONDS_PER_DAY;
    let remaining = seconds_since_epoch % SECONDS_PER_DAY;
    let (year, month, day) = civil_from_days(days as i64);
    let hour = remaining / SECONDS_PER_HOUR;
    let minute = remaining % SECONDS_PER_HOUR / SECONDS_PER_MINUTE;
    let second = remaining % SECONDS_PER_MINUTE;

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Converts a count of days since 1970-01-01 into a proleptic Gregorian
/// `(year, month, day)`.
fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    // Shift the epoch to 0000-03-01, which puts the leap day at the end of the
    // year and makes the month arithmetic below a linear function.
    let shifted = days_since_epoch + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    let year = if month <= 2 { year + 1 } else { year };

    (year, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_unix_epoch_is_formatted() {
        assert_eq!(format_utc(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn the_first_day_boundary_is_formatted() {
        assert_eq!(format_utc(86_399), "1970-01-01T23:59:59Z");
        assert_eq!(format_utc(86_400), "1970-01-02T00:00:00Z");
    }

    #[test]
    fn a_leap_day_is_formatted() {
        assert_eq!(format_utc(1_709_164_800), "2024-02-29T00:00:00Z");
        assert_eq!(format_utc(951_782_400), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn a_century_that_is_not_a_leap_year_is_formatted() {
        assert_eq!(format_utc(4_107_542_400), "2100-03-01T00:00:00Z");
    }

    #[test]
    fn a_timestamp_with_a_time_of_day_is_formatted() {
        assert_eq!(format_utc(1_709_164_800 + 3_661), "2024-02-29T01:01:01Z");
    }

    #[test]
    fn the_fixed_epoch_of_this_cut_is_formatted() {
        assert_eq!(format_utc(1_794_182_400), "2026-11-09T00:00:00Z");
    }

    #[test]
    fn the_current_time_has_the_documented_shape() {
        let now = now_utc();

        assert_eq!(now.len(), 20, "unexpected timestamp: {now}");
        assert!(now.ends_with('Z'), "unexpected timestamp: {now}");
        assert_eq!(&now[4..5], "-", "unexpected timestamp: {now}");
        assert_eq!(&now[10..11], "T", "unexpected timestamp: {now}");
        assert!(
            now.bytes().enumerate().all(|(index, byte)| index == 4
                || index == 7
                || index == 10
                || index == 13
                || index == 16
                || index == 19
                || byte.is_ascii_digit()),
            "unexpected timestamp: {now}"
        );
    }
}
