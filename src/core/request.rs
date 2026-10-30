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

use crate::core::calendar::GoogleEvent;
use crate::core::error::AlmanacError;
use crate::core::profile::Profile;

mod checks;
mod reminders;
mod window;

use checks::{check_identity, check_status, resolve_color, upsert_marker};
use reminders::to_reminders;
use window::{all_day_window, timed_window};

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
    check_identity(request, origin)?;
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
    let color_id = request
        .color
        .as_deref()
        .map(|requested| resolve_color(requested, origin))
        .transpose()?;
    check_status(request, origin)?;
    let reminders = request
        .reminders
        .as_ref()
        .map(|asked| to_reminders(asked, origin))
        .transpose()?;

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
        extended_properties: upsert_marker(request, profile),
    })
}
