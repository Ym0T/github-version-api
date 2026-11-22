//! Formatting helpers.
use std::time::Instant;

use jiff::Timestamp;

pub fn format_file_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

pub fn format_uptime(start: Instant) -> String {
    let secs = start.elapsed().as_secs();
    let (days, hours, minutes, seconds) = (
        secs / 86_400,
        (secs % 86_400) / 3600,
        (secs % 3600) / 60,
        secs % 60,
    );
    if days > 0 {
        format!("{days}d {hours}h {minutes}m {seconds}s")
    } else if hours > 0 {
        format!("{hours}h {minutes}m {seconds}s")
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}

pub fn current_timestamp() -> String {
    format_timestamp(Timestamp::now())
}

/// RFC 3339 representation in UTC with nanosecond precision.
pub fn format_timestamp(timestamp: Timestamp) -> String {
    timestamp
        .strftime("%Y-%m-%dT%H:%M:%S%.9f+00:00")
        .to_string()
}

/// RFC 3339 representation of a unix timestamp (clamped to a valid range).
pub fn rfc3339(unix_seconds: i64) -> String {
    Timestamp::from_second(unix_seconds)
        .unwrap_or(Timestamp::MAX)
        .strftime("%Y-%m-%dT%H:%M:%S+00:00")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_sizes() {
        assert_eq!(format_file_size(0), "0 B");
        assert_eq!(format_file_size(1023), "1023 B");
        assert_eq!(format_file_size(1024), "1.0 KB");
        assert_eq!(format_file_size(1536), "1.5 KB");
        assert_eq!(format_file_size(1_048_576), "1.0 MB");
    }

    #[test]
    fn timestamps() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00+00:00");
        assert_eq!(rfc3339(1_700_000_000), "2023-11-14T22:13:20+00:00");
        assert_eq!(rfc3339(i64::MAX), "9999-12-30T22:00:00+00:00");
        assert_eq!(
            format_timestamp(Timestamp::new(1_700_000_000, 120_000_000).unwrap()),
            "2023-11-14T22:13:20.120000000+00:00"
        );
    }
}
