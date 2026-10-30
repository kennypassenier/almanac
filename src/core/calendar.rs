//! The pure Google Calendar event data model — what should be written,
//! not how. `shell::calendar_client` is the only place that turns a
//! `GoogleEvent` into an actual HTTP call. L2's mapping-profile engine
//! is what will *produce* these values from arbitrary source payloads;
//! for L1 they are constructed directly by callers proving the CRUD
//! round-trip.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Date/time boundary for a calendar event (start or end).
///
/// Google accepts exactly one of `date` and `dateTime`, never both and
/// never neither: `date` marks the whole day, `dateTime` a moment. That
/// is why this is an enum rather than two optional fields — a struct
/// with both optional would make "both set" and "neither set"
/// representable, and both are requests Google refuses.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(untagged)]
pub enum EventDateTime {
    /// A moment: an RFC3339 timestamp with the zone it should be read
    /// in, e.g. `"2026-05-19T09:00:00+00:00"` in `"Europe/Brussels"`.
    ///
    /// `rename_all` sits on the variant, not on the enum: on an
    /// untagged enum the outer attribute renames variants, which is
    /// meaningless here, and the fields would go out as `date_time`.
    /// Google rejects that, and the only symptom would have been every
    /// delivery failing at once.
    #[serde(rename_all = "camelCase")]
    Timed {
        date_time: String,
        time_zone: String,
    },
    /// A whole day: `"2026-09-01"`, with no zone — an all-day event is
    /// the same day everywhere, which is the point of it.
    ///
    /// Google's end date is **exclusive**: a one-day event on the 1st
    /// ends on the 2nd. Getting that wrong shortens every all-day event
    /// to nothing, so it has its own test.
    AllDay { date: String },
}

impl EventDateTime {
    pub fn timed(date_time: impl Into<String>, time_zone: impl Into<String>) -> Self {
        Self::Timed {
            date_time: date_time.into(),
            time_zone: time_zone.into(),
        }
    }

    pub fn all_day(date: impl Into<String>) -> Self {
        Self::AllDay { date: date.into() }
    }

    pub fn is_all_day(&self) -> bool {
        matches!(self, Self::AllDay { .. })
    }

    /// The timestamp, for a timed boundary; `None` for an all-day one.
    pub fn date_time(&self) -> Option<&str> {
        match self {
            Self::Timed { date_time, .. } => Some(date_time),
            Self::AllDay { .. } => None,
        }
    }

    /// The date, for an all-day boundary; `None` for a timed one.
    pub fn date(&self) -> Option<&str> {
        match self {
            Self::AllDay { date } => Some(date),
            Self::Timed { .. } => None,
        }
    }

    pub fn time_zone(&self) -> Option<&str> {
        match self {
            Self::Timed { time_zone, .. } => Some(time_zone),
            Self::AllDay { .. } => None,
        }
    }
}

/// One reminder Google should raise for an event (K16).
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct ReminderOverride {
    /// `"popup"` or `"email"` — the only two Google accepts.
    pub method: String,
    /// How long before the start, in minutes.
    pub minutes: u32,
}

/// The reminder block on an event (K16).
///
/// Absent from an event entirely means "whatever the calendar's own
/// default is". Present with `use_default: false` and no overrides
/// means deliberate silence, which is a different and useful thing —
/// an infra calendar should not buzz a phone at 3am for something
/// already alerted on elsewhere.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Reminders {
    pub use_default: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overrides: Vec<ReminderOverride>,
}

/// Application-private metadata attached to a calendar event. Only
/// `private` is used — visible solely to the creating application,
/// which is exactly what K2's upsert lookup and AR15's `source_id`
/// tagging need.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct ExtendedProperties {
    pub private: HashMap<String, String>,
}

/// A Google Calendar Event resource — the subset of fields this
/// project uses. Mirrors
/// <https://developers.google.com/calendar/api/v3/reference/events>.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GoogleEvent {
    /// Assigned by Google; `None` when constructing a new event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,

    pub summary: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,

    /// Google Calendar colour ID (`"1"`-`"11"`); the calendar's
    /// default colour is used when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_id: Option<String>,

    pub start: EventDateTime,
    pub end: EventDateTime,

    /// Whether this event makes the owner look busy (K17).
    /// `"opaque"` (busy) is Google's default; `"transparent"` means
    /// the event shows on the calendar without consuming availability.
    /// Absent leaves Google's default alone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transparency: Option<String>,

    /// `"confirmed"`, `"tentative"` or `"cancelled"` (K17). Absent
    /// leaves Google's default, which is confirmed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,

    /// Reminder overrides (K16); absent inherits the calendar default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reminders: Option<Reminders>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub extended_properties: Option<ExtendedProperties>,
}

/// One page of a Calendar v3 list call (events or the calendar list).
/// Google omits the `items` key entirely (not an empty array) when
/// nothing matches, hence `Option`, and sends `nextPageToken` only
/// while more pages follow.
#[derive(Debug, Deserialize)]
pub struct ListPage<T> {
    pub items: Option<Vec<T>>,
    #[serde(rename = "nextPageToken")]
    pub next_page_token: Option<String>,
}

/// One page of the events list endpoint.
pub type EventListResponse = ListPage<GoogleEvent>;

#[cfg(test)]
mod tests;
