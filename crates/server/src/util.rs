//! Small shared helpers.

use std::time::Duration;

/// Compact, human-readable duration: `9s`, `1m 15s`, `2h 05m`, `3d 04h`.
#[must_use]
pub fn format_duration(duration: Duration) -> String {
    let secs = duration.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3_600 {
        format!("{}m {:02}s", secs / 60, secs % 60)
    } else if secs < 86_400 {
        format!("{}h {:02}m", secs / 3_600, (secs % 3_600) / 60)
    } else {
        format!("{}d {:02}h", secs / 86_400, (secs % 86_400) / 3_600)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_durations() {
        assert_eq!(format_duration(Duration::from_secs(9)), "9s");
        assert_eq!(format_duration(Duration::from_secs(75)), "1m 15s");
        assert_eq!(format_duration(Duration::from_secs(3_900)), "1h 05m");
        assert_eq!(format_duration(Duration::from_secs(90_000)), "1d 01h");
    }
}
