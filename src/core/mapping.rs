//! Translates a source payload into a `GoogleEvent`, driven entirely
//! by a `Profile` (K5) — this is the generalized replacement for the
//! old hardcoded `build_google_event_from_task`. Pure: takes already-
//! parsed JSON and a profile, produces a value; no I/O.

use chrono::{DateTime, Duration, NaiveDate};
use serde_json::Value;

use crate::core::calendar::{
    EventDateTime, ExtendedProperties, GoogleEvent, ReminderOverride, Reminders,
};
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
    let (start, end) = if mapping.all_day {
        all_day_window(&start_raw, mapping, origin)?
    } else {
        timed_window(payload, &start_raw, mapping, origin)?
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

    let location = match &mapping.location_field {
        Some(field) => get_field(payload, field).and_then(value_to_string),
        None => None,
    };

    Ok(GoogleEvent {
        id: None,
        summary,
        description,
        location,
        color_id,
        start,
        end,
        transparency: mapping
            .busy
            .map(|busy| if busy { "opaque" } else { "transparent" }.to_string()),
        status: resolve_status(payload, profile),
        reminders: mapping.reminders.as_ref().map(|rule| Reminders {
            // Silence and overrides both mean "not the calendar's
            // default"; the difference is whether the list is empty.
            use_default: false,
            overrides: rule
                .popup_minutes_before
                .iter()
                .map(|m| ReminderOverride {
                    method: "popup".to_string(),
                    minutes: *m,
                })
                .chain(rule.email_minutes_before.iter().map(|m| ReminderOverride {
                    method: "email".to_string(),
                    minutes: *m,
                }))
                .collect(),
        }),
        extended_properties,
    })
}

/// K14. A timed event: a moment plus a length in minutes.
fn timed_window(
    payload: &Value,
    start_raw: &str,
    mapping: &crate::core::profile::FieldMapping,
    origin: &str,
) -> Result<(EventDateTime, EventDateTime), AlmanacError> {
    let start_dt = DateTime::parse_from_rfc3339(start_raw).map_err(|e| {
        AlmanacError::ProfileValidation {
            message: format!(
                "{origin}: payload field \"{}\" (\"{start_raw}\") is not a valid RFC3339 timestamp: {e}",
                mapping.start_field
            ),
            remedy: "check the source sends an RFC3339 timestamp (e.g. 2026-08-28T09:00:00+00:00)"
                .to_string(),
        }
    })?;
    // K18. The end comes from the payload when the profile names a
    // field for it, because a source reporting a *period* knows when it
    // ends and a fixed duration would be a guess dressed up as a fact.
    let end_dt = match &mapping.end_field {
        Some(field) => {
            let end_raw = required_field(payload, field, origin)?;
            let end = DateTime::parse_from_rfc3339(&end_raw).map_err(|e| {
                AlmanacError::ProfileValidation {
                    message: format!(
                        "{origin}: payload field \"{field}\" (\"{end_raw}\") is not a valid RFC3339 timestamp: {e}"
                    ),
                    remedy: "check the source sends an RFC3339 timestamp for the event's end"
                        .to_string(),
                }
            })?;
            // An end at or before the start is a zero- or
            // negative-length event. Google's own behaviour there is
            // unhelpful and the result is invisible on a calendar, so
            // say what is wrong instead of writing it.
            if end <= start_dt {
                return Err(AlmanacError::ProfileValidation {
                    message: format!(
                        "{origin}: the payload's end (\"{end_raw}\") is not after its start (\"{start_raw}\")"
                    ),
                    remedy: "check the source's start and end fields are the right way round — an \
                             event ending before it starts does not appear on a calendar at all"
                        .to_string(),
                });
            }
            end
        }
        // Validated at parse time for a timed profile, so the fallback
        // is unreachable rather than a silent default.
        None => start_dt + Duration::minutes(mapping.duration_minutes.unwrap_or(60)),
    };

    Ok((
        EventDateTime::timed(start_dt.to_rfc3339(), mapping.timezone.clone()),
        EventDateTime::timed(end_dt.to_rfc3339(), mapping.timezone.clone()),
    ))
}

/// K14. A day marker.
///
/// Accepts either a plain `YYYY-MM-DD` or a full timestamp, because a
/// source that already sends timestamps should not have to change to be
/// usable as an all-day source — a bin-day sensor reporting
/// `2026-09-01T00:00:00Z` means the first of September.
fn all_day_window(
    start_raw: &str,
    mapping: &crate::core::profile::FieldMapping,
    origin: &str,
) -> Result<(EventDateTime, EventDateTime), AlmanacError> {
    let start_date = NaiveDate::parse_from_str(start_raw, "%Y-%m-%d")
        .or_else(|_| DateTime::parse_from_rfc3339(start_raw).map(|dt| dt.date_naive()))
        .map_err(|e| AlmanacError::ProfileValidation {
            message: format!(
                "{origin}: payload field \"{}\" (\"{start_raw}\") is neither a date nor an RFC3339 timestamp: {e}",
                mapping.start_field
            ),
            remedy: "for an all-day profile the source may send either 2026-09-01 or a full \
                     timestamp on that day"
                .to_string(),
        })?;

    let days = mapping.duration_days.unwrap_or(1);
    // Google's end date is EXCLUSIVE: a one-day event on the 1st ends
    // on the 2nd. Treating it as inclusive silently produces an event
    // of zero length that shows up nowhere.
    let end_date = start_date + Duration::days(days);

    Ok((
        EventDateTime::all_day(start_date.to_string()),
        EventDateTime::all_day(end_date.to_string()),
    ))
}

/// K17. The same shape as `resolve_color`, deliberately.
fn resolve_status(payload: &Value, profile: &Profile) -> Option<String> {
    let rule = profile.mapping.status_by.as_ref()?;
    let observed = get_field(payload, &rule.field).and_then(value_to_string);
    Some(
        observed
            .and_then(|v| rule.values.get(&v).cloned())
            .unwrap_or_else(|| rule.default.clone()),
    )
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
        assert_eq!(
            event.start.date_time().unwrap(),
            "2026-08-28T09:00:00+00:00"
        );
        assert_eq!(event.end.date_time().unwrap(), "2026-08-28T10:00:00+00:00");
        assert_eq!(event.start.time_zone().unwrap(), "UTC");
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

    fn profile_with(extra: &str) -> Profile {
        let toml = format!(
            r#"
schema_version = 1
source_id = "house"
target_calendar_id = "cal"

[mapping]
title_field = "title"
start_field = "start"
timezone = "Europe/Brussels"
{extra}
"#
        );
        Profile::parse(&toml, "test.toml").unwrap()
    }

    #[test]
    fn an_all_day_profile_produces_a_day_marker_and_never_a_timestamp() {
        // K14. The whole point: "bin day" belongs at the top of the
        // day, not as a 60-minute block at whatever time the sensor
        // happened to fire.
        let profile = profile_with("all_day = true");
        let payload = json!({"title": "Vuilnis buitenzetten", "start": "2026-09-01"});
        let event = map_payload(&payload, &profile, "test.toml").unwrap();

        assert_eq!(event.start.date(), Some("2026-09-01"));
        assert_eq!(event.start.date_time(), None);
        assert!(event.start.is_all_day() && event.end.is_all_day());
    }

    #[test]
    fn a_one_day_event_ends_the_next_day_because_googles_end_is_exclusive() {
        // Off by one here does not look like an off-by-one: an event
        // that starts and ends on the same date has zero length and
        // simply does not appear.
        let profile = profile_with("all_day = true");
        let payload = json!({"title": "x", "start": "2026-09-01"});
        let event = map_payload(&payload, &profile, "test.toml").unwrap();
        assert_eq!(event.end.date(), Some("2026-09-02"));
    }

    #[test]
    fn several_days_are_counted_from_the_start() {
        let profile = profile_with("all_day = true\nduration_days = 3");
        let payload = json!({"title": "weg", "start": "2026-09-01"});
        let event = map_payload(&payload, &profile, "test.toml").unwrap();
        assert_eq!(event.start.date(), Some("2026-09-01"));
        assert_eq!(event.end.date(), Some("2026-09-04"));
    }

    #[test]
    fn an_all_day_profile_also_accepts_a_source_that_only_speaks_timestamps() {
        // A sensor reporting 2026-09-01T00:00:00Z means the first of
        // September. Refusing that would force every existing source to
        // change before it could feed a day marker.
        let profile = profile_with("all_day = true");
        let payload = json!({"title": "x", "start": "2026-09-01T06:30:00+02:00"});
        let event = map_payload(&payload, &profile, "test.toml").unwrap();
        assert_eq!(event.start.date(), Some("2026-09-01"));
    }

    #[test]
    fn a_timed_profile_is_unchanged_by_all_of_this() {
        // Every profile written before K14 must keep working exactly
        // as it did.
        let profile = profile_with("duration_minutes = 30");
        let payload = json!({"title": "x", "start": "2026-09-01T09:00:00+00:00"});
        let event = map_payload(&payload, &profile, "test.toml").unwrap();
        assert_eq!(event.start.date_time(), Some("2026-09-01T09:00:00+00:00"));
        assert_eq!(event.end.date_time(), Some("2026-09-01T09:30:00+00:00"));
        assert_eq!(event.start.time_zone(), Some("Europe/Brussels"));
        assert!(!event.start.is_all_day());
    }

    #[test]
    fn location_reaches_the_event_now_that_a_profile_can_name_it() {
        // K15. Before this the field existed on the event, was
        // serialized, and was hardcoded empty — a half-built field that
        // looked finished.
        let profile = profile_with("duration_minutes = 60\nlocation_field = \"where\"");
        let payload =
            json!({"title": "x", "start": "2026-09-01T09:00:00+00:00", "where": "Kerkstraat 1"});
        let event = map_payload(&payload, &profile, "test.toml").unwrap();
        assert_eq!(event.location.as_deref(), Some("Kerkstraat 1"));
    }

    #[test]
    fn a_profile_that_names_no_location_still_sends_none() {
        let profile = profile_with("duration_minutes = 60");
        let payload =
            json!({"title": "x", "start": "2026-09-01T09:00:00+00:00", "where": "ignored"});
        assert_eq!(
            map_payload(&payload, &profile, "test.toml")
                .unwrap()
                .location,
            None
        );
    }

    #[test]
    fn an_infra_profile_can_say_it_does_not_make_you_busy() {
        // K17, and the one recommendation that met real data: Grafana
        // and Uptime Kuma both send incidents, and an incident should
        // not tell everyone you were unavailable that evening.
        let profile = profile_with("duration_minutes = 60\nbusy = false");
        let payload = json!({"title": "x", "start": "2026-09-01T09:00:00+00:00"});
        let event = map_payload(&payload, &profile, "test.toml").unwrap();
        assert_eq!(event.transparency.as_deref(), Some("transparent"));
    }

    #[test]
    fn saying_nothing_about_busy_leaves_googles_default_alone() {
        let profile = profile_with("duration_minutes = 60");
        let payload = json!({"title": "x", "start": "2026-09-01T09:00:00+00:00"});
        assert_eq!(
            map_payload(&payload, &profile, "test.toml")
                .unwrap()
                .transparency,
            None
        );
    }

    #[test]
    fn status_is_looked_up_the_same_way_a_colour_is() {
        let profile = profile_with(
            "duration_minutes = 60\n[mapping.status_by]\nfield = \"state\"\ndefault = \"confirmed\"\nvalues = { resolved = \"cancelled\" }",
        );
        let firing = json!({"title": "x", "start": "2026-09-01T09:00:00+00:00", "state": "firing"});
        let resolved =
            json!({"title": "x", "start": "2026-09-01T09:00:00+00:00", "state": "resolved"});
        assert_eq!(
            map_payload(&firing, &profile, "t")
                .unwrap()
                .status
                .as_deref(),
            Some("confirmed")
        );
        assert_eq!(
            map_payload(&resolved, &profile, "t")
                .unwrap()
                .status
                .as_deref(),
            Some("cancelled")
        );
    }

    #[test]
    fn reminders_asked_for_are_produced_and_silence_is_a_different_thing() {
        // K16. Three distinct outcomes, and the difference between the
        // last two is the whole reason the block exists: silence means
        // "override the calendar's default with nothing", absence means
        // "use the calendar's default".
        let asked = profile_with(
            "duration_minutes = 60\n[mapping.reminders]\npopup_minutes_before = [30]\nemail_minutes_before = [1440]",
        );
        let silent = profile_with("duration_minutes = 60\n[mapping.reminders]\nsilent = true");
        let quiet = profile_with("duration_minutes = 60");
        let payload = json!({"title": "x", "start": "2026-09-01T09:00:00+00:00"});

        let r = map_payload(&payload, &asked, "t")
            .unwrap()
            .reminders
            .unwrap();
        assert!(!r.use_default);
        assert_eq!(r.overrides.len(), 2);
        assert!(
            r.overrides
                .iter()
                .any(|o| o.method == "popup" && o.minutes == 30)
        );
        assert!(
            r.overrides
                .iter()
                .any(|o| o.method == "email" && o.minutes == 1440)
        );

        let s = map_payload(&payload, &silent, "t")
            .unwrap()
            .reminders
            .unwrap();
        assert!(!s.use_default);
        assert!(s.overrides.is_empty());

        assert_eq!(map_payload(&payload, &quiet, "t").unwrap().reminders, None);
    }

    #[test]
    fn the_end_can_come_from_the_payload_when_the_profile_names_it() {
        // K18, and the case that forced it: a cheap-power window is
        // 480 minutes today and might be 45 tomorrow. A fixed duration
        // would put an hour-long block on the calendar for an
        // eight-hour window — a calendar showing something other than
        // what it says.
        let profile = profile_with("end_field = \"until\"");
        let payload = json!({
            "title": "Goedkope stroom",
            "start": "2026-09-01T08:45:00+02:00",
            "until": "2026-09-01T16:45:00+02:00"
        });
        let event = map_payload(&payload, &profile, "test.toml").unwrap();
        assert_eq!(event.start.date_time(), Some("2026-09-01T08:45:00+02:00"));
        assert_eq!(event.end.date_time(), Some("2026-09-01T16:45:00+02:00"));
    }

    #[test]
    fn an_end_before_the_start_is_refused_rather_than_written() {
        // Google accepts this and the result is invisible: an event of
        // negative length appears on no calendar. Saying so beats
        // writing something that silently is not there.
        let profile = profile_with("end_field = \"until\"");
        let payload = json!({
            "title": "x",
            "start": "2026-09-01T16:45:00+02:00",
            "until": "2026-09-01T08:45:00+02:00"
        });
        let err = map_payload(&payload, &profile, "test.toml").unwrap_err();
        assert!(err.to_string().contains("not after its start"), "{err}");
        assert!(err.remedy().contains("right way round"), "{}", err.remedy());
    }

    #[test]
    fn an_end_equal_to_the_start_is_refused_too() {
        let profile = profile_with("end_field = \"until\"");
        let payload = json!({
            "title": "x",
            "start": "2026-09-01T08:45:00+02:00",
            "until": "2026-09-01T08:45:00+02:00"
        });
        assert!(map_payload(&payload, &profile, "test.toml").is_err());
    }

    #[test]
    fn a_missing_end_names_the_field_and_the_profile() {
        let profile = profile_with("end_field = \"until\"");
        let payload = json!({"title": "x", "start": "2026-09-01T08:45:00+02:00"});
        let err = map_payload(&payload, &profile, "test.toml").unwrap_err();
        assert!(err.to_string().contains("until"), "{err}");
    }

    #[test]
    fn a_profile_using_a_fixed_duration_is_untouched_by_k18() {
        let profile = profile_with("duration_minutes = 30");
        let payload = json!({
            "title": "x",
            "start": "2026-09-01T09:00:00+00:00",
            "until": "ignored because no end_field names it"
        });
        let event = map_payload(&payload, &profile, "test.toml").unwrap();
        assert_eq!(event.end.date_time(), Some("2026-09-01T09:30:00+00:00"));
    }
}
