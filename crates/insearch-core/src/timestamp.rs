//! Parse a leading timestamp out of a log line/block into epoch **seconds**.
//!
//! Recognises the same families the block splitter uses to segment logs
//! (ISO-8601, syslog, US-date, epoch). Zone-less timestamps are interpreted in
//! the **local** zone, so they line up with the entry-time filter bounds the UI
//! computes from local dates. Used only by the entry-time filter — parsing runs
//! on *matching* units, not every line.

use chrono::{Datelike, Local, NaiveDateTime, TimeZone};

/// Compiled patterns for extracting a leading timestamp. Cheap to share.
pub struct TimestampParser {
    iso: regex::Regex,
    syslog: regex::Regex,
    us: regex::Regex,
    epoch: regex::Regex,
    /// Year assumed for year-less (syslog) timestamps.
    year: i32,
}

impl Default for TimestampParser {
    fn default() -> Self {
        TimestampParser {
            // ISO-8601, optionally bracketed; only the date + HH:MM:SS matter.
            iso: regex::Regex::new(r"^\[?(\d{4}-\d{2}-\d{2})[ T](\d{2}:\d{2}:\d{2})").unwrap(),
            // syslog: "Aug 20 08:00:01" (no year).
            syslog: regex::Regex::new(r"^([A-Z][a-z]{2})\s+(\d{1,2})\s+(\d{2}:\d{2}:\d{2})")
                .unwrap(),
            // US: "08/20/2026 08:00:01".
            us: regex::Regex::new(r"^(\d{2})/(\d{2})/(\d{4})\s+(\d{2}:\d{2}:\d{2})").unwrap(),
            // epoch seconds (10 digits) or millis (13), optionally bracketed.
            epoch: regex::Regex::new(r"^\[?(\d{10,13})\b").unwrap(),
            year: Local::now().year(),
        }
    }
}

impl TimestampParser {
    /// Epoch **seconds** for the timestamp at the start of `text`, or `None` if
    /// the text doesn't begin with a recognised timestamp.
    pub fn parse_leading(&self, text: &str) -> Option<i64> {
        let head = text.trim_start();

        if let Some(c) = self.iso.captures(head) {
            let s = format!("{} {}", &c[1], &c[2]);
            return NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S")
                .ok()
                .map(local_epoch);
        }
        if let Some(c) = self.us.captures(head) {
            // MM/DD/YYYY -> YYYY-MM-DD.
            let s = format!("{}-{}-{} {}", &c[3], &c[1], &c[2], &c[4]);
            return NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S")
                .ok()
                .map(local_epoch);
        }
        if let Some(c) = self.syslog.captures(head) {
            // "Mon D HH:MM:SS" — no year; assume the parser's year.
            let day: u32 = c[2].parse().ok()?;
            let s = format!("{} {:02} {} {}", &c[1], day, &c[3], self.year);
            return NaiveDateTime::parse_from_str(&s, "%b %d %H:%M:%S %Y")
                .ok()
                .map(local_epoch);
        }
        if let Some(c) = self.epoch.captures(head) {
            let digits = &c[1];
            let n: i64 = digits.parse().ok()?;
            // 13 digits (or >= 1e12) is milliseconds; 10 is seconds.
            return Some(if digits.len() >= 13 || n >= 1_000_000_000_000 {
                n / 1000
            } else {
                n
            });
        }
        None
    }
}

/// Interpret a zone-less datetime in the local zone and return epoch seconds
/// (falling back to UTC across a DST gap/fold, which is close enough here).
fn local_epoch(n: NaiveDateTime) -> i64 {
    Local
        .from_local_datetime(&n)
        .single()
        .map(|dt| dt.timestamp())
        .unwrap_or_else(|| n.and_utc().timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Local epoch for a naive `YYYY-MM-DD HH:MM:SS`, for zone-independent asserts.
    fn le(s: &str) -> i64 {
        local_epoch(NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap())
    }

    #[test]
    fn parses_iso_bracketed_and_plain() {
        let p = TimestampParser::default();
        assert_eq!(
            p.parse_leading("2026-08-22 10:30:00 msg"),
            Some(le("2026-08-22 10:30:00"))
        );
        assert_eq!(
            p.parse_leading("[2026-08-22T10:30:00] msg"),
            Some(le("2026-08-22 10:30:00"))
        );
        // trailing fractional/zone after seconds is ignored (prefix match).
        assert_eq!(
            p.parse_leading("2026-08-22T10:30:00.123Z x"),
            Some(le("2026-08-22 10:30:00"))
        );
    }

    #[test]
    fn parses_us_date() {
        let p = TimestampParser::default();
        assert_eq!(
            p.parse_leading("08/22/2026 10:30:00 x"),
            Some(le("2026-08-22 10:30:00"))
        );
    }

    #[test]
    fn parses_epoch_seconds_and_millis() {
        let p = TimestampParser::default();
        assert_eq!(p.parse_leading("1755856200 evt"), Some(1_755_856_200));
        assert_eq!(p.parse_leading("1755856200123 evt"), Some(1_755_856_200));
        assert_eq!(p.parse_leading("[1755856200] evt"), Some(1_755_856_200));
    }

    #[test]
    fn syslog_parses_to_something_and_undated_is_none() {
        let p = TimestampParser::default();
        assert!(p
            .parse_leading("Aug 22 10:30:00 host daemon: msg")
            .is_some());
        assert_eq!(p.parse_leading("    indented continuation line"), None);
        assert_eq!(p.parse_leading("no timestamp here"), None);
    }
}
