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

/// Wrapper around the `items` array returned by the Calendar v3 events
/// list endpoint. Google omits the `items` key entirely (not an empty
/// array) when nothing matches, hence `Option`.
#[derive(Debug, Deserialize)]
pub struct EventListResponse {
    pub items: Option<Vec<GoogleEvent>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_optional_fields_are_omitted_from_the_serialized_json() {
        let event = GoogleEvent {
            id: None,
            summary: "Wasmachine klaar".to_string(),
            description: None,
            location: None,
            color_id: None,
            start: EventDateTime::timed("2026-08-28T09:00:00+00:00".to_string(), "UTC".to_string()),
            end: EventDateTime::timed("2026-08-28T10:00:00+00:00".to_string(), "UTC".to_string()),
            transparency: None,
            status: None,
            reminders: None,
            extended_properties: None,
        };

        let json = serde_json::to_value(&event).unwrap();
        let obj = json.as_object().unwrap();
        assert!(!obj.contains_key("id"));
        assert!(!obj.contains_key("description"));
        assert!(!obj.contains_key("location"));
        assert!(!obj.contains_key("colorId"));
        assert!(!obj.contains_key("extendedProperties"));
        assert_eq!(obj.get("summary").unwrap(), "Wasmachine klaar");
    }

    #[test]
    fn extended_properties_round_trip_through_json() {
        let mut private = HashMap::new();
        private.insert(
            "almanac_source_id".to_string(),
            "home-assistant:switch.wasmachine".to_string(),
        );

        let event = GoogleEvent {
            id: Some("evt123".to_string()),
            summary: "test".to_string(),
            description: None,
            location: None,
            color_id: Some("2".to_string()),
            start: EventDateTime::timed("2026-08-28T09:00:00+00:00".to_string(), "UTC".to_string()),
            end: EventDateTime::timed("2026-08-28T10:00:00+00:00".to_string(), "UTC".to_string()),
            transparency: None,
            status: None,
            reminders: None,
            extended_properties: Some(ExtendedProperties { private }),
        };

        let json = serde_json::to_string(&event).unwrap();
        let round_tripped: GoogleEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(round_tripped, event);
    }

    #[test]
    fn event_list_response_treats_a_missing_items_key_as_empty() {
        let parsed: EventListResponse = serde_json::from_str("{}").unwrap();
        assert!(parsed.items.is_none());
    }

    #[test]
    fn a_timed_boundary_serializes_the_way_google_expects_it() {
        // This is not decoration. `untagged` on the enum makes the
        // outer `rename_all` apply to variant *names*, so without a
        // rename on the variant itself the fields go out as
        // `date_time` and `time_zone`. Google rejects that, and the
        // symptom is every delivery failing at once with nothing in the
        // code looking wrong. It happened on the first build of K14.
        let json = serde_json::to_value(EventDateTime::timed(
            "2026-09-01T09:00:00+00:00",
            "Europe/Brussels",
        ))
        .unwrap();
        assert_eq!(json["dateTime"], "2026-09-01T09:00:00+00:00");
        assert_eq!(json["timeZone"], "Europe/Brussels");
        assert!(
            json.get("date").is_none(),
            "a timed boundary must not carry a date"
        );
    }

    #[test]
    fn an_all_day_boundary_carries_a_date_and_nothing_else() {
        // Google accepts exactly one of the two. Sending both is a
        // rejected request; sending neither is a rejected request.
        let json = serde_json::to_value(EventDateTime::all_day("2026-09-01")).unwrap();
        assert_eq!(json["date"], "2026-09-01");
        assert!(json.get("dateTime").is_none());
        assert!(json.get("timeZone").is_none());
        assert_eq!(json.as_object().unwrap().len(), 1);
    }

    #[test]
    fn both_boundary_shapes_survive_a_round_trip_through_google_json() {
        // Reading events back is how K2 finds the one to update, so a
        // boundary that serializes correctly but cannot be parsed back
        // would break the upsert rather than the create.
        for original in [
            EventDateTime::timed("2026-09-01T09:00:00+00:00", "UTC"),
            EventDateTime::all_day("2026-09-01"),
        ] {
            let json = serde_json::to_string(&original).unwrap();
            let back: EventDateTime = serde_json::from_str(&json).unwrap();
            assert_eq!(back, original, "round trip changed {json}");
        }
    }

    #[test]
    fn reminders_serialize_as_google_names_them() {
        let json = serde_json::to_value(Reminders {
            use_default: false,
            overrides: vec![ReminderOverride {
                method: "popup".to_string(),
                minutes: 30,
            }],
        })
        .unwrap();
        assert_eq!(json["useDefault"], false);
        assert_eq!(json["overrides"][0]["method"], "popup");
        assert_eq!(json["overrides"][0]["minutes"], 30);
    }

    #[test]
    fn deliberate_silence_omits_the_override_list_rather_than_sending_an_empty_one() {
        // `useDefault: false` with no overrides is how Google is told
        // "no reminders at all", and it is a different instruction from
        // omitting the block, which inherits the calendar's default.
        let json = serde_json::to_value(Reminders {
            use_default: false,
            overrides: vec![],
        })
        .unwrap();
        assert_eq!(json["useDefault"], false);
        assert!(json.get("overrides").is_none());
    }

    #[test]
    fn the_optional_event_fields_are_omitted_when_unset_rather_than_sent_as_null() {
        // A null where Google expects a string is a rejected request,
        // and every profile written before K16/K17 leaves these unset.
        let event = GoogleEvent {
            id: None,
            summary: "s".to_string(),
            description: None,
            location: None,
            color_id: None,
            start: EventDateTime::all_day("2026-09-01"),
            end: EventDateTime::all_day("2026-09-02"),
            transparency: None,
            status: None,
            reminders: None,
            extended_properties: None,
        };
        let json = serde_json::to_value(&event).unwrap();
        for absent in ["transparency", "status", "reminders", "location", "colorId"] {
            assert!(
                json.get(absent).is_none(),
                "{absent} should be omitted, not null"
            );
        }
    }
}
