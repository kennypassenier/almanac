//! When an event starts and ends: a timed window or whole days.

use crate::core::calendar::EventDateTime;
use crate::core::error::AlmanacError;
use crate::core::profile::Profile;

use super::EventRequest;

/// The start and end of a timed event.
///
/// Precedence: an explicit `end` beats `duration_minutes`, which beats
/// the profile's default length. An end at or before the start is
/// refused — Google accepts it and the result appears on no calendar,
/// which is the worst kind of accepted.
pub(super) fn timed_window(
    request: &EventRequest,
    profile: &Profile,
    timezone: &str,
    origin: &str,
) -> Result<(EventDateTime, EventDateTime), AlmanacError> {
    use chrono::DateTime;

    let start = DateTime::parse_from_rfc3339(&request.start).map_err(|e| {
        AlmanacError::ProfileValidation {
            message: format!(
                "{origin}: \"start\" ({:?}) is not a valid RFC3339 timestamp: {e}",
                request.start
            ),
            remedy: "send an RFC3339 timestamp, e.g. 2026-09-03T21:00:00+02:00 — or set \
                     all_day = true and send a date"
                .to_string(),
        }
    })?;

    let end = match &request.end {
        Some(raw) => {
            let end =
                DateTime::parse_from_rfc3339(raw).map_err(|e| AlmanacError::ProfileValidation {
                    message: format!(
                        "{origin}: \"end\" ({raw:?}) is not a valid RFC3339 timestamp: {e}"
                    ),
                    remedy: "send an RFC3339 timestamp, or use duration_minutes instead"
                        .to_string(),
                })?;
            if end <= start {
                return Err(AlmanacError::ProfileValidation {
                    message: format!(
                        "{origin}: \"end\" ({raw:?}) is at or before \"start\" ({:?})",
                        request.start
                    ),
                    remedy: "send an end after the start — Google accepts an inverted event and \
                             then shows it on no calendar at all"
                        .to_string(),
                });
            }
            end
        }
        None => {
            let minutes = request
                .duration_minutes
                .or(profile.default_duration_minutes)
                .unwrap_or(crate::core::profile::FALLBACK_DURATION_MINUTES);
            if minutes <= 0 {
                return Err(AlmanacError::ProfileValidation {
                    message: format!("{origin}: \"duration_minutes\" is {minutes}"),
                    remedy: "send a duration greater than zero, or an explicit end".to_string(),
                });
            }
            start + chrono::Duration::minutes(minutes)
        }
    };

    Ok((
        EventDateTime::timed(start.to_rfc3339(), timezone),
        EventDateTime::timed(end.to_rfc3339(), timezone),
    ))
}

/// The start and end of an all-day event.
///
/// Accepts either a plain date or a timestamp, so a source that already
/// sends RFC3339 does not have to change to become an all-day source.
/// Google's end date is **exclusive**: a one-day event on the 1st ends
/// on the 2nd, and getting that wrong shortens every all-day event to
/// nothing.
pub(super) fn all_day_window(
    request: &EventRequest,
    origin: &str,
) -> Result<(EventDateTime, EventDateTime), AlmanacError> {
    use chrono::{DateTime, NaiveDate};

    let start = NaiveDate::parse_from_str(&request.start, "%Y-%m-%d")
        .or_else(|_| DateTime::parse_from_rfc3339(&request.start).map(|dt| dt.date_naive()))
        .map_err(|e| AlmanacError::ProfileValidation {
            message: format!(
                "{origin}: \"start\" ({:?}) is neither a date nor a timestamp: {e}",
                request.start
            ),
            remedy: "for an all-day event send 2026-09-01, or a full timestamp on that day"
                .to_string(),
        })?;

    let days = request.duration_days.unwrap_or(1);
    if days <= 0 {
        return Err(AlmanacError::ProfileValidation {
            message: format!("{origin}: \"duration_days\" is {days}"),
            remedy: "an all-day event covers at least one day".to_string(),
        });
    }

    let end = start + chrono::Duration::days(days);
    Ok((
        EventDateTime::all_day(start.format("%Y-%m-%d").to_string()),
        EventDateTime::all_day(end.format("%Y-%m-%d").to_string()),
    ))
}
