//! The pure Google Calendar event data model — what should be written,
//! not how. `shell::calendar_client` is the only place that turns a
//! `GoogleEvent` into an actual HTTP call. L2's mapping-profile engine
//! is what will *produce* these values from arbitrary source payloads;
//! for L1 they are constructed directly by callers proving the CRUD
//! round-trip.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Date/time boundary for a calendar event (start or end).
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EventDateTime {
    /// RFC3339 timestamp, e.g. `"2026-05-19T09:00:00+00:00"`.
    pub date_time: String,

    /// IANA time zone name, e.g. `"UTC"` or `"Europe/Brussels"`.
    pub time_zone: String,
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
            start: EventDateTime {
                date_time: "2026-08-28T09:00:00+00:00".to_string(),
                time_zone: "UTC".to_string(),
            },
            end: EventDateTime {
                date_time: "2026-08-28T10:00:00+00:00".to_string(),
                time_zone: "UTC".to_string(),
            },
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
            start: EventDateTime {
                date_time: "2026-08-28T09:00:00+00:00".to_string(),
                time_zone: "UTC".to_string(),
            },
            end: EventDateTime {
                date_time: "2026-08-28T10:00:00+00:00".to_string(),
                time_zone: "UTC".to_string(),
            },
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
}
