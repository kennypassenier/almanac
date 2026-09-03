//! Turning machine timestamps into something a person reads at a glance.
//!
//! The dashboard showed `2026-09-03T01:47:02.351384747+00:00` in the
//! "Token issued" column — every digit true, and nobody reads it. Kenny
//! asked for it in a shape a human takes in, which is the whole reason
//! this exists.
//!
//! It never *loses* information: an unparseable value comes back
//! unchanged rather than becoming "unknown". A timestamp that cannot be
//! read is still evidence; blanking it would be worse than ugly.

use chrono::{DateTime, Datelike, Utc};

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// `2026-09-03T01:47:02.351384747+00:00` → `3 Sep 2026, 01:47`.
///
/// Rendered in the reader's own zone rather than the one the timestamp
/// carries: "when was this issued" is a question about their day, and a
/// UTC clock reading two hours off is the kind of small wrongness that
/// costs someone a minute of doubt.
pub fn timestamp(raw: &str, zone: chrono_tz::Tz) -> String {
    let Ok(parsed) = DateTime::parse_from_rfc3339(raw) else {
        return raw.to_string();
    };
    let local = parsed.with_timezone(&zone);
    format!(
        "{} {} {}, {:02}:{:02}",
        local.day(),
        MONTHS[(local.month() as usize).saturating_sub(1).min(11)],
        local.year(),
        local.hour_minute().0,
        local.hour_minute().1
    )
}

/// How long ago, in the coarsest unit that is still true: `just now`,
/// `12 minutes ago`, `3 hours ago`, `5 days ago`.
///
/// Coarse on purpose. The exact second is in the full timestamp beside
/// it; what this answers is "recent or not", which is the question
/// somebody actually has when they look at a token list.
pub fn how_long_ago(raw: &str, now: DateTime<Utc>) -> Option<String> {
    let parsed = DateTime::parse_from_rfc3339(raw).ok()?;
    let seconds = (now - parsed.with_timezone(&Utc)).num_seconds();
    if seconds < 0 {
        // A clock skew or a hand-edited file. Saying "in 3 minutes"
        // about something already issued would read as a bug.
        return None;
    }
    Some(match seconds {
        s if s < 90 => "just now".to_string(),
        s if s < 3600 => plural(s / 60, "minute"),
        s if s < 86_400 => plural(s / 3600, "hour"),
        s => plural(s / 86_400, "day"),
    })
}

fn plural(count: i64, unit: &str) -> String {
    if count == 1 {
        format!("1 {unit} ago")
    } else {
        format!("{count} {unit}s ago")
    }
}

/// `local.hour()` and `local.minute()` in one call, so the format
/// string above reads as one thought.
trait HourMinute {
    fn hour_minute(&self) -> (u32, u32);
}

impl<Tz: chrono::TimeZone> HourMinute for DateTime<Tz> {
    fn hour_minute(&self) -> (u32, u32) {
        use chrono::Timelike;
        (self.hour(), self.minute())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BRUSSELS: chrono_tz::Tz = chrono_tz::Europe::Brussels;

    #[test]
    fn a_machine_timestamp_becomes_something_a_person_reads() {
        assert_eq!(
            timestamp("2026-09-03T01:47:02.351384747+00:00", BRUSSELS),
            "3 Sep 2026, 03:47",
            "and in the reader's own zone: 01:47 UTC is 03:47 in Brussels"
        );
    }

    #[test]
    fn a_value_that_cannot_be_parsed_is_shown_unchanged() {
        // Never blank it: a timestamp nobody can read is still
        // evidence, and "unknown" would throw it away.
        assert_eq!(
            timestamp("some time last tuesday", BRUSSELS),
            "some time last tuesday"
        );
        assert_eq!(timestamp("", BRUSSELS), "");
    }

    #[test]
    fn the_coarsest_unit_that_is_still_true() {
        let now: DateTime<Utc> = "2026-09-03T12:00:00+00:00".parse().unwrap();
        let ago = |raw: &str| how_long_ago(raw, now).unwrap();

        assert_eq!(ago("2026-09-03T11:59:30+00:00"), "just now");
        assert_eq!(ago("2026-09-03T11:45:00+00:00"), "15 minutes ago");
        assert_eq!(ago("2026-09-03T11:00:00+00:00"), "1 hour ago");
        assert_eq!(ago("2026-09-03T09:00:00+00:00"), "3 hours ago");
        assert_eq!(ago("2026-09-01T12:00:00+00:00"), "2 days ago");
    }

    #[test]
    fn a_timestamp_in_the_future_says_nothing_rather_than_nonsense() {
        // Clock skew, or a hand-edited store. "In 3 minutes" about
        // something already issued reads as a bug in the dashboard.
        let now: DateTime<Utc> = "2026-09-03T12:00:00+00:00".parse().unwrap();
        assert_eq!(how_long_ago("2026-09-03T12:05:00+00:00", now), None);
    }

    #[test]
    fn something_unparseable_has_no_age_rather_than_a_wrong_one() {
        let now: DateTime<Utc> = "2026-09-03T12:00:00+00:00".parse().unwrap();
        assert_eq!(how_long_ago("not a timestamp", now), None);
    }
}
