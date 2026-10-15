//! Timestamp and date/time formatting utilities.

/// Converts a count of days since UNIX epoch (1970-01-01) to `(year, month, day)`.
///
/// Uses the Euclidean affine algorithm (Fliegel & Van Flandern / Hatcher).
pub fn days_to_ymd(days: i64) -> (i64, i64, i64) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1029 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = (mp as i64) + if mp < 10 { 3 } else { -9 };
    let y = y + if m <= 2 { 1 } else { 0 };
    (y, m, d as i64)
}

/// Formats an epoch timestamp in seconds to a human-readable string with relative duration,
/// e.g. `"2026-10-15 14:30 (5m ago)"`.
pub fn format_timestamp(epoch_secs: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let diff = now.saturating_sub(epoch_secs);
    let rel = if diff < 60 {
        "just now".to_string()
    } else if diff < 3600 {
        format!("{}m ago", diff / 60)
    } else if diff < 86400 {
        format!("{}h ago", diff / 3600)
    } else {
        format!("{}d ago", diff / 86400)
    };

    let secs_per_day = 86400;
    let days = epoch_secs.div_euclid(secs_per_day);
    let time_of_day = epoch_secs.rem_euclid(secs_per_day);
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let (year, month, day) = days_to_ymd(days);

    format!("{year:04}-{month:02}-{day:02} {hours:02}:{minutes:02} ({rel})")
}

/// Formats an epoch timestamp in seconds to a compact month/day hour:minute string,
/// e.g. `"10/15 14:30"`.
pub fn format_time_only(epoch_secs: i64) -> String {
    let secs_per_day = 86400;
    let days = epoch_secs.div_euclid(secs_per_day);
    let time_of_day = epoch_secs.rem_euclid(secs_per_day);
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let (_year, month, day) = days_to_ymd(days);
    format!("{month:02}/{day:02} {hours:02}:{minutes:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_days_to_ymd_epoch() {
        assert_eq!(days_to_ymd(0), (1970, 1, 1));
    }

    #[test]
    fn test_days_to_ymd_known_dates() {
        // 2000-01-01 is 10957 days after epoch
        assert_eq!(days_to_ymd(10957), (2000, 1, 1));
        // Leap year: 2024-02-29 is 19782 days after epoch
        assert_eq!(days_to_ymd(19782), (2024, 2, 29));
    }

    #[test]
    fn test_format_time_only() {
        // 1700000000 = 2023-11-14 22:13:20 UTC
        assert_eq!(format_time_only(1700000000), "11/14 22:13");
    }

    #[test]
    fn test_format_timestamp_structure() {
        let ts = 1700000000;
        let formatted = format_timestamp(ts);
        assert!(formatted.starts_with("2023-11-14 22:13"));
        assert!(formatted.contains("ago"));
    }
}
