//! Translates a source payload into a `GoogleEvent`, driven entirely
//! by a `Profile` (K5) — this is the generalized replacement for the
//! old hardcoded `build_google_event_from_task`. Pure: takes already-
//! parsed JSON and a profile, produces a value; no I/O.

use chrono::{DateTime, Duration};
use serde_json::Value;

use crate::core::calendar::{EventDateTime, ExtendedProperties, GoogleEvent};
use crate::core::error::AlmanacError;
use crate::core::profile::Profile;

/// Reads a (possibly one-level-nested, dot-separated) field out of a
/// JSON payload, e.g. `"attributes.friendly_name"`.
fn get_field<'a>(payload: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = payload;
    for segment in path.split('.') {
        current = current.get(segment)?;
    }
    Some(current)
}

/// Coerces a JSON value to a string the way a human would expect: a
/// JSON string as-is, a number or bool via its natural display form.
/// Objects/arrays/null have no sensible string form and return `None`.
fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Null | Value::Object(_) | Value::Array(_) => None,
    }
}

fn required_field(payload: &Value, path: &str, origin: &str) -> Result<String, AlmanacError> {
    get_field(payload, path)
        .and_then(value_to_string)
        .ok_or_else(|| AlmanacError::ProfileValidation {
            message: format!("{origin}: payload is missing required field \"{path}\""),
            remedy: format!(
                "check the source is actually sending \"{path}\", or fix the profile's field mapping"
            ),
        })
}

/// Resolves the event's colour, if the profile configures a
/// conditional rule — generalizes VIK-4 (priority→colour) and VIK-6
/// (conditional override): look up the configured field's value in
/// the rule's table, falling back to `default` if the field is
/// missing or its value isn't in the table.
fn resolve_color(payload: &Value, profile: &Profile) -> Option<String> {
    let rule = profile.mapping.color_by.as_ref()?;
    let key = get_field(payload, &rule.field).and_then(value_to_string);
    match key.and_then(|k| rule.values.get(&k).cloned()) {
        Some(color) => Some(color),
        None => Some(rule.default.clone()),
    }
}

/// Translates `payload` into a `GoogleEvent` per `profile`'s mapping.
/// `origin` names the profile for error messages (standing rule 11).
pub fn map_payload(
    payload: &Value,
    profile: &Profile,
    origin: &str,
) -> Result<GoogleEvent, AlmanacError> {
    let mapping = &profile.mapping;

    let summary = required_field(payload, &mapping.title_field, origin)?;

    let description = match &mapping.description_field {
        Some(field) => get_field(payload, field).and_then(value_to_string),
        None => None,
    };

    let start_raw = required_field(payload, &mapping.start_field, origin)?;
    let start_dt = DateTime::parse_from_rfc3339(&start_raw).map_err(|e| AlmanacError::ProfileValidation {
        message: format!(
            "{origin}: payload field \"{}\" (\"{start_raw}\") is not a valid RFC3339 timestamp: {e}",
            mapping.start_field
        ),
        remedy: "check the source sends an RFC3339 timestamp (e.g. 2026-08-28T09:00:00+00:00)".to_string(),
    })?;
    let end_dt = start_dt + Duration::minutes(mapping.duration_minutes);

    let start = EventDateTime {
        date_time: start_dt.to_rfc3339(),
        time_zone: mapping.timezone.clone(),
    };
    let end = EventDateTime {
        date_time: end_dt.to_rfc3339(),
        time_zone: mapping.timezone.clone(),
    };

    let color_id = resolve_color(payload, profile);

    let extended_properties = match &mapping.external_id_field {
        Some(field) => {
            let external_id = required_field(payload, field, origin)?;
            let mut private = std::collections::HashMap::new();
            // AR15's pinned upsert-key format: "<source_id>:<external-id>".
            private.insert(
                "almanac_source_id".to_string(),
                format!("{}:{external_id}", profile.source_id),
            );
            Some(ExtendedProperties { private })
        }
        // No natural external id — M7's idempotency-key mechanism
        // (wired at the ingest layer in L3) supplies one out-of-band
        // instead of this function inventing one.
        None => None,
    };

    Ok(GoogleEvent {
        id: None,
        summary,
        description,
        location: None,
        color_id,
        start,
        end,
        extended_properties,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::profile::Profile;
    use serde_json::json;

    fn profile_with_mapping(extra_toml: &str) -> Profile {
        let toml = format!(
            r#"
schema_version = 1
source_id = "home-assistant"
target_calendar_id = "primary"
token_hash = "deadbeef"

[mapping]
title_field = "title"
description_field = "description"
external_id_field = "id"
start_field = "start"
duration_minutes = 60
{extra_toml}
"#
        );
        Profile::parse(&toml, "test.toml").unwrap()
    }

    #[test]
    fn maps_a_flat_payload_end_to_end() {
        let profile = profile_with_mapping("");
        let payload = json!({
            "id": "switch.wasmachine",
            "title": "Wasmachine klaar",
            "description": "cyclus voltooid",
            "start": "2026-08-28T09:00:00+00:00",
        });

        let event = map_payload(&payload, &profile, "test.toml").unwrap();

        assert_eq!(event.summary, "Wasmachine klaar");
        assert_eq!(event.description.as_deref(), Some("cyclus voltooid"));
        assert_eq!(event.start.date_time, "2026-08-28T09:00:00+00:00");
        assert_eq!(event.end.date_time, "2026-08-28T10:00:00+00:00");
        assert_eq!(event.start.time_zone, "UTC");
        assert_eq!(
            event
                .extended_properties
                .unwrap()
                .private
                .get("almanac_source_id"),
            Some(&"home-assistant:switch.wasmachine".to_string())
        );
    }

    #[test]
    fn a_missing_required_field_names_itself_and_the_profile() {
        let profile = profile_with_mapping("");
        let payload = json!({"description": "no title or start here"});
        let err = map_payload(&payload, &profile, "ha.toml").unwrap_err();
        assert!(err.to_string().contains("title"));
        assert!(err.to_string().contains("ha.toml"));
    }

    #[test]
    fn an_unparseable_start_timestamp_is_a_clear_error_not_a_silent_fallback() {
        let profile = profile_with_mapping("");
        let payload = json!({"id": "x", "title": "t", "start": "not a timestamp"});
        let err = map_payload(&payload, &profile, "ha.toml").unwrap_err();
        assert!(err.to_string().contains("not a valid RFC3339"));
    }

    #[test]
    fn numeric_and_boolean_fields_coerce_to_strings() {
        let profile = profile_with_mapping("");
        let payload =
            json!({"id": 42, "title": "numeric id", "start": "2026-08-28T09:00:00+00:00"});
        let event = map_payload(&payload, &profile, "test.toml").unwrap();
        assert_eq!(
            event
                .extended_properties
                .unwrap()
                .private
                .get("almanac_source_id"),
            Some(&"home-assistant:42".to_string())
        );
    }

    #[test]
    fn nested_fields_are_reachable_via_dotted_paths() {
        let toml = r#"
schema_version = 1
source_id = "uptime-kuma"
target_calendar_id = "infra"
token_hash = "deadbeef"

[mapping]
title_field = "monitor.name"
start_field = "time"
duration_minutes = 15
"#;
        let profile = Profile::parse(toml, "kuma.toml").unwrap();
        let payload = json!({
            "monitor": {"name": "Jellyfin down"},
            "time": "2026-08-28T02:14:00+00:00",
        });
        let event = map_payload(&payload, &profile, "kuma.toml").unwrap();
        assert_eq!(event.summary, "Jellyfin down");
    }

    #[test]
    fn color_by_picks_the_matching_value_and_falls_back_to_default() {
        let profile = profile_with_mapping(
            "\n[mapping.color_by]\nfield = \"priority\"\ndefault = \"peacock\"\n[mapping.color_by.values]\n\"5\" = \"tomato\"\n",
        );

        let urgent =
            json!({"id": "1", "title": "t", "start": "2026-08-28T09:00:00+00:00", "priority": "5"});
        let event = map_payload(&urgent, &profile, "t.toml").unwrap();
        assert_eq!(event.color_id.as_deref(), Some("tomato"));

        let unranked = json!({"id": "2", "title": "t", "start": "2026-08-28T09:00:00+00:00"});
        let event = map_payload(&unranked, &profile, "t.toml").unwrap();
        assert_eq!(event.color_id.as_deref(), Some("peacock"));
    }

    #[test]
    fn no_external_id_field_means_no_extended_properties_from_mapping_alone() {
        let toml = r#"
schema_version = 1
source_id = "claude-session"
target_calendar_id = "primary"
token_hash = "deadbeef"

[mapping]
title_field = "title"
start_field = "start"
duration_minutes = 30
"#;
        let profile = Profile::parse(toml, "claude.toml").unwrap();
        let payload = json!({"title": "ad-hoc event", "start": "2026-08-28T09:00:00+00:00"});
        let event = map_payload(&payload, &profile, "claude.toml").unwrap();
        assert_eq!(event.extended_properties, None);
    }
}
