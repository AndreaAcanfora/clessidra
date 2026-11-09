//! Reading and writing moments the way people say them: `10m`, `2h ago`, `14:30`,
//! `2026-11-10 14:30`, RFC 3339.

use anyhow::{Result, bail};
use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};

/// Parse a moment relative to `now` (ms since the epoch) into ms since the epoch.
pub fn parse_moment(input: &str, now: i64) -> Result<i64> {
    let s = input.trim().to_lowercase();
    let s = s.strip_suffix(" ago").unwrap_or(&s).trim();
    if s == "now" {
        return Ok(now);
    }
    if let Some(ms) = parse_duration(s) {
        return Ok(now - ms);
    }
    if let Ok(t) = DateTime::parse_from_rfc3339(input.trim()) {
        return Ok(t.timestamp_millis());
    }
    for fmt in [
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
    ] {
        if let Ok(t) = NaiveDateTime::parse_from_str(input.trim(), fmt) {
            return local(t);
        }
    }
    if let Ok(d) = NaiveDate::parse_from_str(input.trim(), "%Y-%m-%d") {
        return local(d.and_time(NaiveTime::MIN));
    }
    for fmt in ["%H:%M:%S", "%H:%M"] {
        if let Ok(t) = NaiveTime::parse_from_str(input.trim(), fmt) {
            let today = DateTime::from_timestamp_millis(now)
                .unwrap_or_default()
                .with_timezone(&Local)
                .date_naive();
            let mut at = local(today.and_time(t))?;
            if at > now {
                at -= 86_400_000; // "14:30" late at night means this afternoon, not tomorrow
            }
            return Ok(at);
        }
    }
    bail!("cannot read \"{input}\" as a moment; try 10m, 2h ago, 14:30 or 2026-11-10 14:30")
}

/// `90s`, `10m`, `2h`, `3d`, `1w`, also `1h30m`. Milliseconds.
pub fn parse_duration(s: &str) -> Option<i64> {
    let s = s.replace(' ', "");
    if s.is_empty() {
        return None;
    }
    let mut total = 0i64;
    let mut num = String::new();
    let mut unit = String::new();
    let mut chars = s.chars().peekable();
    while chars.peek().is_some() {
        num.clear();
        unit.clear();
        while let Some(c) = chars.peek().copied().filter(|c| c.is_ascii_digit()) {
            num.push(c);
            chars.next();
        }
        while let Some(c) = chars.peek().copied().filter(|c| c.is_ascii_alphabetic()) {
            unit.push(c);
            chars.next();
        }
        let n: i64 = num.parse().ok()?;
        let ms = match unit.as_str() {
            "ms" => 1,
            "s" | "sec" | "secs" | "second" | "seconds" => 1_000,
            "m" | "min" | "mins" | "minute" | "minutes" => 60_000,
            "h" | "hr" | "hrs" | "hour" | "hours" => 3_600_000,
            "d" | "day" | "days" => 86_400_000,
            "w" | "week" | "weeks" => 604_800_000,
            _ => return None,
        };
        total += n * ms;
    }
    Some(total)
}

fn local(t: NaiveDateTime) -> Result<i64> {
    match Local.from_local_datetime(&t).earliest() {
        Some(t) => Ok(t.timestamp_millis()),
        None => bail!("{t} does not exist in the local time zone"),
    }
}

/// `just now`, `42s ago`, `5m ago`, `3h ago`, `2d ago`.
pub fn ago(ts: i64, now: i64) -> String {
    let d = (now - ts).max(0) / 1000;
    match d {
        0..=4 => "just now".into(),
        5..=59 => format!("{d}s ago"),
        60..=3599 => format!("{}m ago", d / 60),
        3600..=86_399 => format!("{}h ago", d / 3600),
        _ => format!("{}d ago", d / 86_400),
    }
}

/// Local wall-clock time, `2026-11-10 14:30:05`.
pub fn local_time(ts: i64) -> String {
    DateTime::from_timestamp_millis(ts)
        .map(|t| t.with_timezone(&Local).format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(parse_duration("10m"), Some(600_000));
        assert_eq!(parse_duration("1h30m"), Some(5_400_000));
        assert_eq!(parse_duration("2 hours"), Some(7_200_000));
        assert_eq!(parse_duration("x"), None);
        assert_eq!(parse_duration("10"), None);
    }

    #[test]
    fn moments() {
        let now = 1_800_000_000_000;
        assert_eq!(parse_moment("10m ago", now).unwrap(), now - 600_000);
        assert_eq!(parse_moment("now", now).unwrap(), now);
        assert_eq!(parse_moment("2026-11-10T10:00:00Z", now).unwrap(), 1_794_304_800_000);
        assert!(parse_moment("14:30", now).unwrap() <= now);
        assert!(parse_moment("yesterday-ish", now).is_err());
    }

    #[test]
    fn ago_reads_well() {
        assert_eq!(ago(0, 3_000), "just now");
        assert_eq!(ago(0, 300_000), "5m ago");
    }
}
