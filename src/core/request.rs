//! What a source actually sends: Almanac's own event, not a shape
//! Almanac has to learn.
//!
//! Until 2.0.0 a profile was a translation table. It named which
//! payload field meant the title, which meant the start, and carried
//! the per-event choices — all-day, colour, free/busy, reminders — as
//! settings fixed for every event that source would ever send. That was
//! built for webhooks nobody here controls: Grafana sends
//! `commonLabels.alertname` and will not change for us.
//!
//! Kenny's decision, 2026-09-03: *"voor aanpassingen hadden we
//! HTTPSwitchboard! dus doe het volgens mijn model!"* A source speaks
//! Almanac's language, the profile only says which calendar it writes
//! to, and anything that speaks a different shape is translated by the
//! tool built for translating shapes.
//!
//! What it buys, beyond the smaller profile: a source can decide these
//! things **per event**. One post can be an all-day marker, the next a
//! timed block; one red, the next not. That was impossible before —
//! the profile decided once, for everything.
//!
//! Measured before it was chosen: of the three profiles that existed,
//! only `home-assistant` had ever delivered an event. The whole
//! journal history was home-assistant (5), the since-deleted
//! energy-prices (4) and job-tracker (2). Removing the translation
//! layer broke nothing that had ever run.

use serde::Deserialize;

use crate::core::calendar::{
    EventDateTime, ExtendedProperties, GoogleEvent, ReminderOverride, Reminders,
};
use crate::core::error::AlmanacError;
use crate::core::profile::Profile;

/// The three statuses Google accepts, refused here rather than at
/// Google — a rejected status arrives as a 400 with no field name.
const STATUSES: [&str; 3] = ["confirmed", "tentative", "cancelled"];

/// Google's own limits on reminder overrides. Checked here so a source
/// gets a message naming the field instead of a Google 400.
const MAX_REMINDERS: usize = 5;
const MAX_REMINDER_MINUTES: i64 = 4 * 7 * 24 * 60;

/// Reminders as a source asks for them.
#[derive(Debug, Deserialize, Clone, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct ReminderRequest {
    /// Minutes before the start for a popup. Empty means none.
    #[serde(default)]
    pub popup_minutes_before: Vec<i64>,
    #[serde(default)]
    pub email_minutes_before: Vec<i64>,
    /// `true` says "no reminders" out loud, overriding whatever the
    /// calendar defaults to. Omitting the whole block inherits that
    /// default instead, which is a third and different outcome.
    #[serde(default)]
    pub silent: bool,
}

/// One event, as a source sends it.
///
/// `deny_unknown_fields` is deliberate. A source that misspells
/// `all_day` as `allDay` would otherwise get a timed event and no
/// indication why — and this is the shape Almanac defines, so a field
/// it does not know is a mistake worth naming rather than data worth
/// keeping.
#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EventRequest {
    /// The event title. Required: an untitled calendar entry tells
    /// nobody anything.
    pub title: String,
    /// RFC3339 for a timed event, or `YYYY-MM-DD` when `all_day`.
    pub start: String,
    /// This source's own id for the thing the event is about.
    ///
    /// It becomes the marker stored on the Google event, which is what
    /// makes resending update instead of duplicate and what the delete
    /// endpoint looks up. Without it Almanac cannot find, correct or
    /// remove its own work — so a call that omits it must supply an
    /// `Idempotency-Key` header instead (M7), and the ingest layer
    /// refuses a call carrying neither.
    #[serde(default)]
    pub external_id: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    /// The end, RFC3339. Takes precedence over `duration_minutes`.
    #[serde(default)]
    pub end: Option<String>,
    /// How long, when the source knows a length rather than an end.
    /// Falls back to the profile's default.
    #[serde(default)]
    pub duration_minutes: Option<i64>,
    /// A day marker rather than a block on the clock.
    #[serde(default)]
    pub all_day: bool,
    /// How many days an all-day event covers; one when absent.
    #[serde(default)]
    pub duration_days: Option<i64>,
    /// `false` shows the event without consuming availability — what an
    /// infra incident should do, so nobody sees Kenny as busy because a
    /// server beeped.
    #[serde(default)]
    pub busy: Option<bool>,
    /// A Google colour, by name (`"tomato"`) or id (`"11"`).
    #[serde(default)]
    pub color: Option<String>,
    /// `confirmed`, `tentative` or `cancelled`.
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub reminders: Option<ReminderRequest>,
    /// The zone `start` and `end` should be read in. The profile's
    /// timezone when absent.
    #[serde(default)]
    pub timezone: Option<String>,
}

impl EventRequest {
    /// Parses a payload into a request, naming the field that is wrong.
    ///
    /// `origin` names where the payload came from, for error messages
    /// (standing rule 11).
    pub fn parse(payload: &serde_json::Value, origin: &str) -> Result<Self, AlmanacError> {
        serde_json::from_value::<Self>(payload.clone()).map_err(|e| {
            AlmanacError::ProfileValidation {
                message: format!("{origin}: {e}"),
                remedy: "an event carries title, start and external_id, and optionally \
                         description, location, end or duration_minutes, all_day, duration_days, \
                         busy, color, status, reminders and timezone — see docs/USER_GUIDE.md"
                    .to_string(),
            }
        })
    }
}

/// Turns a source's event into the one Almanac will write.
///
/// Everything that used to be decided once per profile is decided here,
/// per event, from what the source said — falling back to the profile
/// only for the timezone and the default length.
pub fn to_google_event(
    request: &EventRequest,
    profile: &Profile,
    origin: &str,
) -> Result<GoogleEvent, AlmanacError> {
    let invalid = |message: String, remedy: String| AlmanacError::ProfileValidation {
        message: format!("{origin}: {message}"),
        remedy,
    };

    if request.title.trim().is_empty() {
        return Err(invalid(
            "\"title\" is empty".to_string(),
            "send a title — an untitled calendar entry tells nobody anything".to_string(),
        ));
    }
    if request
        .external_id
        .as_deref()
        .is_some_and(|id| id.trim().is_empty())
    {
        return Err(invalid(
            "\"external_id\" is empty".to_string(),
            "send a stable id for this thing, or leave the field out and send an \
             Idempotency-Key header instead"
                .to_string(),
        ));
    }

    let timezone = request
        .timezone
        .as_deref()
        .unwrap_or(&profile.timezone)
        .to_string();
    crate::core::profile::validate_timezone(&timezone, origin)?;

    let (start, end) = if request.all_day {
        all_day_window(request, origin)?
    } else {
        timed_window(request, profile, &timezone, origin)?
    };

    let color_id = match &request.color {
        Some(requested) => Some(
            crate::core::colors::resolve(requested)
                .ok_or_else(|| {
                    invalid(
                        format!("\"color\": {requested:?} is not a Google colour"),
                        format!(
                            "use one of: {}, or a colour id 1-11",
                            crate::core::colors::names()
                        ),
                    )
                })?
                .to_string(),
        ),
        None => None,
    };

    if let Some(status) = &request.status
        && !STATUSES.contains(&status.as_str())
    {
        return Err(invalid(
            format!("\"status\": {status:?} is not a Google event status"),
            format!("use one of: {}", STATUSES.join(", ")),
        ));
    }

    let reminders = match &request.reminders {
        Some(asked) => Some(to_reminders(asked, origin)?),
        None => None,
    };

    // AR15's pinned upsert-key format: "<source_id>:<external-id>".
    // Absent when the source sent no external_id — the delivery layer
    // then fills the marker from the Idempotency-Key header (M7), and
    // ingest has already refused a call that offered neither.
    let extended_properties = request.external_id.as_deref().map(|external_id| {
        let mut private = std::collections::HashMap::new();
        private.insert(
            "almanac_source_id".to_string(),
            format!("{}:{external_id}", profile.source_id),
        );
        ExtendedProperties { private }
    });

    Ok(GoogleEvent {
        id: None,
        summary: request.title.clone(),
        description: request.description.clone(),
        location: request.location.clone(),
        start,
        end,
        color_id,
        transparency: match request.busy {
            Some(false) => Some("transparent".to_string()),
            _ => None,
        },
        status: request.status.clone(),
        reminders,
        extended_properties,
    })
}

fn to_reminders(asked: &ReminderRequest, origin: &str) -> Result<Reminders, AlmanacError> {
    let count = asked.popup_minutes_before.len() + asked.email_minutes_before.len();
    if asked.silent && count > 0 {
        return Err(AlmanacError::ProfileValidation {
            message: format!("{origin}: \"reminders\" is silent and also lists reminders"),
            remedy: "either set silent = true, or give the minutes — not both".to_string(),
        });
    }
    if count > MAX_REMINDERS {
        return Err(AlmanacError::ProfileValidation {
            message: format!(
                "{origin}: \"reminders\" asks for {count}; Google allows {MAX_REMINDERS}"
            ),
            remedy: format!("keep at most {MAX_REMINDERS} reminders across popup and email"),
        });
    }
    for minutes in asked
        .popup_minutes_before
        .iter()
        .chain(&asked.email_minutes_before)
    {
        if *minutes < 0 || *minutes > MAX_REMINDER_MINUTES {
            return Err(AlmanacError::ProfileValidation {
                message: format!("{origin}: \"reminders\" asks for {minutes} minutes before"),
                remedy: format!(
                    "Google allows 0 to {MAX_REMINDER_MINUTES} minutes (four weeks) before the start"
                ),
            });
        }
    }
    // Silence and overrides both mean "not the calendar's default";
    // the difference is whether the list is empty. Omitting the block
    // entirely is the third outcome and never reaches here.
    Ok(Reminders {
        use_default: false,
        overrides: asked
            .popup_minutes_before
            .iter()
            .map(|m| ReminderOverride {
                method: "popup".to_string(),
                minutes: *m as u32,
            })
            .chain(asked.email_minutes_before.iter().map(|m| ReminderOverride {
                method: "email".to_string(),
                minutes: *m as u32,
            }))
            .collect(),
    })
}

/// The start and end of a timed event.
///
/// Precedence: an explicit `end` beats `duration_minutes`, which beats
/// the profile's default length. An end at or before the start is
/// refused — Google accepts it and the result appears on no calendar,
/// which is the worst kind of accepted.
fn timed_window(
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
fn all_day_window(
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
