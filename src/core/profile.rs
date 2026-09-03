//! Mapping profiles (K5): a declarative, per-source description of how
//! to turn a source's JSON payload into a `GoogleEvent`, replacing the
//! hardcoded Vikunja-specific Rust the old codebase had. Parsing and
//! validating a profile from a TOML string is pure — reading the file
//! off disk is `shell::profiles`' job (AR13).

use std::collections::HashMap;

use serde::Deserialize;

use crate::core::error::AlmanacError;

/// Only schema version this build understands (AR15). A profile with
/// any other value fails validation with a message naming both the
/// version found and what's supported, rather than silently
/// misinterpreting an old or newer shape.
const SUPPORTED_SCHEMA_VERSION: u32 = 1;

/// Google's own limits, checked at startup so a profile that breaks
/// them fails once and visibly rather than on every event forever.
const MAX_REMINDER_OVERRIDES: usize = 5;
const MAX_REMINDER_MINUTES: u32 = 40_320;

/// K17. Maps a payload value onto a Google event status, in exactly
/// the same shape as `ColorRule` below — someone who has written one
/// has written the other.
#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct StatusRule {
    pub field: String,
    pub default: String,
    #[serde(default)]
    pub values: HashMap<String, String>,
}

/// K16. Either deliberate silence or a set of overrides — never both,
/// which is checked at parse time rather than left to Google.
#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct ReminderRule {
    /// No reminders at all, overriding whatever the calendar defaults
    /// to. Not the same as omitting the block, which inherits it.
    #[serde(default)]
    pub silent: bool,
    #[serde(default)]
    pub popup_minutes_before: Vec<u32>,
    #[serde(default)]
    pub email_minutes_before: Vec<u32>,
}

/// A conditional colour rule (generalizes VIK-4/VIK-6): look up
/// `field`'s value in `values`, falling back to `default` when the
/// value is absent from the payload or not in the table.
#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct ColorRule {
    pub field: String,
    pub default: String,
    #[serde(default)]
    pub values: HashMap<String, String>,
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct FieldMapping {
    pub title_field: String,
    /// K15. Optional; when absent the event carries no location — which
    /// is what every profile did before this existed, because the field
    /// was hardcoded empty and unreachable.
    #[serde(default)]
    pub location_field: Option<String>,
    /// K14. A day marker rather than a timed block. When true,
    /// `duration_minutes` must be absent and `duration_days` decides
    /// the length.
    #[serde(default)]
    pub all_day: bool,
    /// K14. How many days an all-day event covers; defaults to one.
    #[serde(default)]
    pub duration_days: Option<i64>,
    /// K18. The payload field holding the event's end, for sources that
    /// report a period rather than a moment — a cheap-power window, a
    /// wash cycle, a week away. Mutually exclusive with the two
    /// durations above.
    #[serde(default)]
    pub end_field: Option<String>,
    /// K17. `false` makes the event show on the calendar without
    /// consuming availability. Absent leaves Google's default (busy).
    #[serde(default)]
    pub busy: Option<bool>,
    /// K17. Maps a payload field onto Google's event status, in the
    /// same shape as `color_by`.
    #[serde(default)]
    pub status_by: Option<StatusRule>,
    /// K16. Absent inherits the calendar's own default.
    #[serde(default)]
    pub reminders: Option<ReminderRule>,
    #[serde(default)]
    pub description_field: Option<String>,
    /// Used to build the AR15 upsert key
    /// (`almanac_source_id = "<source_id>:<value>"`). Optional: a
    /// source without a natural per-payload id (K8's ad-hoc calls)
    /// supplies an idempotency key out-of-band instead (M7, wired at
    /// the ingest layer in L3).
    #[serde(default)]
    pub external_id_field: Option<String>,
    /// An RFC3339 timestamp field in the source payload. Required to
    /// be present and parseable at mapping time — no silent fallback
    /// to a placeholder time (standing rule 12).
    pub start_field: String,
    /// Optional since K14: an all-day profile has no minutes to give.
    /// Required for a timed profile, which is checked at parse time.
    #[serde(default)]
    pub duration_minutes: Option<i64>,
    #[serde(default = "default_timezone")]
    pub timezone: String,
    #[serde(default)]
    pub color_by: Option<ColorRule>,
}

fn default_timezone() -> String {
    "UTC".to_string()
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct Profile {
    pub schema_version: u32,
    /// Immutable identity of this source (AR15) — deliberately
    /// separate from the profile's filename or any display label, so
    /// renaming either never silently orphans already-created events.
    pub source_id: String,
    pub target_calendar_id: String,
    pub mapping: FieldMapping,
}

impl Profile {
    /// Parses and validates a profile from its TOML source. `origin`
    /// (typically the file path) is used only to make error messages
    /// name where the problem is.
    pub fn parse(toml_str: &str, origin: &str) -> Result<Profile, AlmanacError> {
        let profile: Profile =
            toml::from_str(toml_str).map_err(|e| AlmanacError::ProfileValidation {
                message: format!("{origin}: failed to parse as TOML: {e}"),
                remedy: format!("fix the syntax error in {origin}"),
            })?;

        if profile.schema_version != SUPPORTED_SCHEMA_VERSION {
            return Err(AlmanacError::ProfileValidation {
                message: format!(
                    "{origin}: schema_version {} is not supported",
                    profile.schema_version
                ),
                remedy: format!(
                    "set schema_version = {SUPPORTED_SCHEMA_VERSION} in {origin}, or upgrade almanac if a newer schema is genuinely needed"
                ),
            });
        }

        if profile.source_id.trim().is_empty() {
            return Err(AlmanacError::ProfileValidation {
                message: format!("{origin}: source_id is empty"),
                remedy: format!("set a non-empty, stable source_id in {origin} — see AR15"),
            });
        }

        if !source_id_is_safe(&profile.source_id) {
            return Err(AlmanacError::ProfileValidation {
                message: format!(
                    "{origin}: source_id \"{}\" contains characters that are not allowed",
                    profile.source_id
                ),
                remedy: format!(
                    "use only letters, digits, '.', '-' and '_' in the source_id in {origin}, and do not start it with a dot — it is both a URL segment and the name of this profile's file on disk"
                ),
            });
        }

        if profile.target_calendar_id.trim().is_empty() {
            return Err(AlmanacError::ProfileValidation {
                message: format!("{origin}: target_calendar_id is empty"),
                remedy: format!(
                    "set target_calendar_id in {origin} to the calendar this source's events should land on"
                ),
            });
        }

        if profile.mapping.title_field.trim().is_empty() {
            return Err(AlmanacError::ProfileValidation {
                message: format!("{origin}: mapping.title_field is empty"),
                remedy: format!(
                    "set mapping.title_field in {origin} to the payload field holding the event title"
                ),
            });
        }

        if profile.mapping.start_field.trim().is_empty() {
            return Err(AlmanacError::ProfileValidation {
                message: format!("{origin}: mapping.start_field is empty"),
                remedy: format!(
                    "set mapping.start_field in {origin} to the payload field holding the event's \
                     start timestamp"
                ),
            });
        }

        // A timezone Google rejects is a *permanent* failure on every
        // event this source ever sends, discovered days later as an
        // unexplained Google error — so it is checked here, once, where
        // the fix is obvious. "Europe/Brussel" is the mistake this
        // catches; the real name is "Europe/Brussels".
        if profile.mapping.timezone.parse::<chrono_tz::Tz>().is_err() {
            return Err(AlmanacError::ProfileValidation {
                message: format!(
                    "{origin}: mapping.timezone \"{}\" is not an IANA time zone",
                    profile.mapping.timezone
                ),
                remedy: format!(
                    "use a name from the IANA database in {origin}, e.g. \"Europe/Brussels\" or \
                     \"UTC\" — Google rejects anything else, permanently, on every event from \
                     this source"
                ),
            });
        }

        // K14. Exactly one of the two length settings applies, and
        // which one depends on `all_day`. Silently ignoring the wrong
        // one would let a profile say "all day, 60 minutes" and get
        // something nobody asked for.
        let m = &profile.mapping;

        // K18. Three ways to say how long an event is, and exactly one
        // of them applies. Two at once is two instructions, and quietly
        // honouring whichever the code reads first produces an event
        // nobody asked for, on every payload, forever.
        let set: Vec<&str> = [
            m.duration_minutes.is_some().then_some("duration_minutes"),
            m.duration_days.is_some().then_some("duration_days"),
            m.end_field.is_some().then_some("end_field"),
        ]
        .into_iter()
        .flatten()
        .collect();
        if set.len() > 1 {
            return Err(AlmanacError::ProfileValidation {
                message: format!(
                    "{origin}: {} are set together, but an event has one length",
                    set.join(" and ")
                ),
                remedy: format!(
                    "keep exactly one of duration_minutes, duration_days or end_field in {origin}"
                ),
            });
        }
        if let Some(field) = &m.end_field
            && field.trim().is_empty()
        {
            return Err(AlmanacError::ProfileValidation {
                message: format!("{origin}: mapping.end_field is empty"),
                remedy: format!(
                    "set mapping.end_field in {origin} to the payload field holding the event's \
                     end, or remove it and use duration_minutes"
                ),
            });
        }

        if m.all_day {
            if m.end_field.is_some() {
                return Err(AlmanacError::ProfileValidation {
                    message: format!("{origin}: mapping.end_field is set on an all-day profile"),
                    remedy: format!(
                        "an all-day event is measured in days — use duration_days in {origin}, or \
                         set all_day = false to take the end from the payload"
                    ),
                });
            }
            if m.duration_minutes.is_some() {
                return Err(AlmanacError::ProfileValidation {
                    message: format!(
                        "{origin}: mapping.duration_minutes is set on an all-day profile"
                    ),
                    remedy: format!(
                        "remove mapping.duration_minutes from {origin}, or set all_day = false —                          an all-day event is measured in days, so use duration_days"
                    ),
                });
            }
            if let Some(days) = m.duration_days
                && days <= 0
            {
                return Err(AlmanacError::ProfileValidation {
                    message: format!(
                        "{origin}: mapping.duration_days must be positive, got {days}"
                    ),
                    remedy: format!("set mapping.duration_days in {origin} to at least 1"),
                });
            }
        } else {
            if m.duration_days.is_some() {
                return Err(AlmanacError::ProfileValidation {
                    message: format!("{origin}: mapping.duration_days is set on a timed profile"),
                    remedy: format!(
                        "set all_day = true in {origin} if you meant a day marker, or use                          duration_minutes for a timed event"
                    ),
                });
            }
            match m.duration_minutes {
                None if m.end_field.is_some() => {}
                None => {
                    return Err(AlmanacError::ProfileValidation {
                        message: format!("{origin}: mapping.duration_minutes is missing"),
                        remedy: format!(
                            "set mapping.duration_minutes in {origin} to how long the event should                              last, or set all_day = true for a day marker"
                        ),
                    });
                }
                Some(minutes) if minutes <= 0 => {
                    return Err(AlmanacError::ProfileValidation {
                        message: format!(
                            "{origin}: mapping.duration_minutes must be positive, got {minutes}"
                        ),
                        remedy: format!(
                            "set mapping.duration_minutes in {origin} to a positive number of minutes"
                        ),
                    });
                }
                Some(_) => {}
            }
        }

        // K17. Google accepts three statuses and nothing else; a fourth
        // is a permanent failure on every event this source sends, so
        // it is caught here rather than days later.
        if let Some(rule) = &m.status_by {
            for value in std::iter::once(&rule.default).chain(rule.values.values()) {
                if !matches!(value.as_str(), "confirmed" | "tentative" | "cancelled") {
                    return Err(AlmanacError::ProfileValidation {
                        message: format!(
                            "{origin}: mapping.status_by yields \"{value}\", which is not a Google event status"
                        ),
                        remedy: format!(
                            "use \"confirmed\", \"tentative\" or \"cancelled\" in {origin} —                              Google rejects anything else"
                        ),
                    });
                }
            }
        }

        // K16. Silence and overrides are contradictory instructions.
        if let Some(rule) = &m.reminders {
            let overrides = rule.popup_minutes_before.len() + rule.email_minutes_before.len();
            if rule.silent && overrides > 0 {
                return Err(AlmanacError::ProfileValidation {
                    message: format!(
                        "{origin}: mapping.reminders asks for silence and for reminders at the same time"
                    ),
                    remedy: format!(
                        "in {origin}, either set silent = true or list minutes, not both"
                    ),
                });
            }
            if overrides > MAX_REMINDER_OVERRIDES {
                return Err(AlmanacError::ProfileValidation {
                    message: format!(
                        "{origin}: mapping.reminders has {overrides} reminders; Google allows at most {MAX_REMINDER_OVERRIDES}"
                    ),
                    remedy: format!("keep at most {MAX_REMINDER_OVERRIDES} reminders in {origin}"),
                });
            }
            for minutes in rule
                .popup_minutes_before
                .iter()
                .chain(&rule.email_minutes_before)
            {
                if *minutes > MAX_REMINDER_MINUTES {
                    return Err(AlmanacError::ProfileValidation {
                        message: format!(
                            "{origin}: a reminder of {minutes} minutes is beyond Google's limit of {MAX_REMINDER_MINUTES} (four weeks)"
                        ),
                        remedy: format!("use at most {MAX_REMINDER_MINUTES} minutes in {origin}"),
                    });
                }
            }
        }

        Ok(profile)
    }
}

/// The profile the dashboard writes when someone adds a source with
/// nothing but a name and a calendar (K21, corrected 2026-09-02).
///
/// The first version of that surface asked for the whole TOML. Kenny's
/// correction: *"dat zou enkel een naam van de bron en de naam van de
/// target kalender moeten zijn"*. This is the rest of the file, and it
/// is deliberately the plain shape almanac already documents rather
/// than a new invention — field for field the deployed `home-assistant`
/// profile, which `tests/mapping_regression.rs` already pins.
///
/// The trade this makes, stated because it is the whole design: the
/// three profiles written by hand each match a third-party webhook's
/// shape (`commonLabels.alertname`, `monitor.name`), because Grafana
/// and Uptime Kuma will not change what they send. A source Kenny adds
/// from the dashboard is one he controls, so it is cheaper for the
/// source to speak almanac's shape than for almanac to learn a fourth.
/// Anything that genuinely cannot is still a file, edited by hand and
/// picked up by the reload.
///
/// `external_id_field` is part of the shape, and leaving it out was a
/// mistake this template made for exactly one day.
///
/// The reasoning for omitting it: naming the field makes it REQUIRED in
/// every payload, so a source that does not send it is refused with a
/// 422 on its first post. True, and the wrong trade. Without the field
/// there is no `almanac_source_id` marker on the Google event, so every
/// resend creates a duplicate AND `DELETE /v1/ingest/{source}/events/
/// {id}` can never find it again — almanac cannot clean up what it
/// made. Measured on 2026-09-03 by the JobTracker session against the
/// live service: two identical posts, two events, and a delete that
/// answered 404.
///
/// A loud refusal naming the missing field beats silent duplicates that
/// nothing can remove. So the shape a dashboard-added source speaks
/// includes `external_id`, alongside `title`, `description` and
/// `start`.
pub fn default_profile_toml(source_id: &str, calendar_id: &str) -> String {
    format!(
        r#"# Written from the dashboard. The plain shape: a payload carrying
# "title", "description" and "start".
# Edit this file for anything else — the dashboard's reload picks it up.

schema_version = {SUPPORTED_SCHEMA_VERSION}
source_id = "{source_id}"
target_calendar_id = "{calendar_id}"

[mapping]
title_field = "title"
description_field = "description"
start_field = "start"
external_id_field = "external_id"
duration_minutes = 60
timezone = "Europe/Brussels"

# external_id_field is what makes resending the same thing update its
# event instead of adding a second one, and it is the only handle the
# delete endpoint has. A payload without "external_id" is refused with
# a message naming it — which is the point: silent duplicates that
# nothing can remove are worse than a clear refusal.
"#
    )
}

/// Whether a `source_id` may be used as-is in a URL path and as a file
/// name.
///
/// It was only ever checked for being non-empty, which was enough while
/// every profile arrived as a file someone had placed by hand. K21 lets
/// the dashboard write a profile, and the file it writes is named after
/// this value — so `"../../etc/cron.d/evil"` would have escaped the
/// profiles directory entirely. The rule is deliberately narrower than
/// "no path separators": anything outside this set has no business in a
/// URL segment either, and a stricter rule cannot be widened by
/// accident.
pub fn source_id_is_safe(source_id: &str) -> bool {
    !source_id.is_empty()
        && !source_id.starts_with('.')
        && source_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// Validates that no two profiles share a `source_id` (AR15) — loading
/// two profiles with the same identity would make upsert lookups
/// ambiguous. Takes the whole loaded set because this is inherently a
/// cross-profile check; `shell::profiles::load_all` calls this after
/// reading every file in the profiles directory.
pub fn validate_unique_source_ids(profiles: &[Profile]) -> Result<(), AlmanacError> {
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for (i, p) in profiles.iter().enumerate() {
        if let Some(&first) = seen.get(p.source_id.as_str()) {
            return Err(AlmanacError::ProfileValidation {
                message: format!(
                    "profiles at index {first} and {i} both use source_id \"{}\"",
                    p.source_id
                ),
                remedy: "give each profile a unique source_id — it is the identity used for upsert lookups (AR15) and must never be shared".to_string(),
            });
        }
        seen.insert(p.source_id.as_str(), i);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_profile_toml() -> &'static str {
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
"#
    }

    #[test]
    fn a_well_formed_profile_parses() {
        let profile = Profile::parse(valid_profile_toml(), "test.toml").unwrap();
        assert_eq!(profile.source_id, "home-assistant");
        assert_eq!(profile.mapping.duration_minutes, Some(60));
        assert_eq!(profile.mapping.timezone, "UTC"); // default applied
    }

    #[test]
    fn an_unsupported_schema_version_is_rejected_by_name() {
        let toml = valid_profile_toml().replace("schema_version = 1", "schema_version = 99");
        let err = Profile::parse(&toml, "bad.toml").unwrap_err();
        assert!(err.to_string().contains("99"));
        assert!(err.to_string().contains("bad.toml"));
    }

    #[test]
    fn an_empty_source_id_is_rejected() {
        let toml =
            valid_profile_toml().replace("source_id = \"home-assistant\"", "source_id = \"\"");
        let err = Profile::parse(&toml, "bad.toml").unwrap_err();
        assert!(err.to_string().contains("source_id"));
    }

    #[test]
    fn k21_the_profile_the_dashboard_writes_is_a_profile_almanac_accepts() {
        // The template and the parser are the same rules or they are
        // not: a dashboard that writes something startup then refuses
        // is worse than no dashboard.
        let toml = default_profile_toml("kobo", "abc@group.calendar.google.com");
        let profile = Profile::parse(&toml, "the default template").expect("must parse");

        assert_eq!(profile.source_id, "kobo");
        assert_eq!(profile.target_calendar_id, "abc@group.calendar.google.com");
        assert_eq!(profile.mapping.title_field, "title");
        assert_eq!(profile.mapping.duration_minutes, Some(60));
    }

    #[test]
    fn k21_the_default_profile_carries_an_external_id_field() {
        // Without it there is no marker on the Google event, so every
        // resend duplicates and delete-by-id can never find it again.
        // Measured live on 2026-09-03: two identical posts, two events,
        // and a 404 on the delete. A loud refusal naming a missing
        // field beats silent duplicates nothing can remove.
        let toml = default_profile_toml("kobo", "cal");
        let profile = Profile::parse(&toml, "the default template").unwrap();
        assert_eq!(
            profile.mapping.external_id_field.as_deref(),
            Some("external_id"),
            "a dashboard-added source must be deletable and de-duplicating"
        );
    }

    #[test]
    fn k21_the_default_matches_the_shape_the_regression_fixture_pins() {
        // The template is not a new invention: it is the deployed
        // home-assistant profile minus the parts a new source cannot
        // promise. If that shape ever changes, this fails next to it.
        let toml = default_profile_toml("x", "y");
        for field in [
            r#"title_field = "title""#,
            r#"description_field = "description""#,
            r#"start_field = "start""#,
            r#"external_id_field = "external_id""#,
        ] {
            assert!(toml.contains(field), "{field} missing from the default");
        }
    }

    #[test]
    fn a_source_id_that_would_escape_the_profiles_directory_is_rejected() {
        // K21 writes a profile to <profiles_dir>/<source_id>.toml, so a
        // traversal here is a write anywhere the process can reach.
        for hostile in ["../../etc/cron.d/evil", "..", ".hidden", "a/b", "a b"] {
            let toml = valid_profile_toml().replace(
                "source_id = \"home-assistant\"",
                &format!("source_id = \"{hostile}\""),
            );
            let err = Profile::parse(&toml, "test").unwrap_err();
            assert!(
                err.to_string().contains("source_id"),
                "{hostile} was accepted"
            );
        }
    }

    #[test]
    fn the_source_ids_actually_in_use_stay_valid() {
        // The rule arrived after these three were already deployed;
        // tightening validation must not refuse a running profile.
        for real in ["home-assistant", "uptime-kuma", "grafana"] {
            assert!(source_id_is_safe(real), "{real} must remain valid");
        }
    }
    #[test]
    fn a_zero_duration_is_rejected() {
        let toml = valid_profile_toml().replace("duration_minutes = 60", "duration_minutes = 0");
        let err = Profile::parse(&toml, "bad.toml").unwrap_err();
        assert!(err.to_string().contains("duration_minutes"));
    }

    #[test]
    fn malformed_toml_names_the_file_and_the_parse_error() {
        let err = Profile::parse("this is not [ toml", "broken.toml").unwrap_err();
        assert!(err.to_string().contains("broken.toml"));
    }

    #[test]
    fn external_id_field_is_optional() {
        let toml = valid_profile_toml().replace("external_id_field = \"id\"\n", "");
        let profile = Profile::parse(&toml, "test.toml").unwrap();
        assert_eq!(profile.mapping.external_id_field, None);
    }

    #[test]
    fn color_by_parses_when_present() {
        let toml = format!(
            "{}\n[mapping.color_by]\nfield = \"priority\"\ndefault = \"peacock\"\n[mapping.color_by.values]\n\"1\" = \"sage\"\n\"5\" = \"tomato\"\n",
            valid_profile_toml()
        );
        let profile = Profile::parse(&toml, "test.toml").unwrap();
        let color_by = profile.mapping.color_by.unwrap();
        assert_eq!(color_by.default, "peacock");
        assert_eq!(color_by.values.get("5"), Some(&"tomato".to_string()));
    }

    #[test]
    fn duplicate_source_ids_are_rejected_with_both_indices() {
        let a = Profile::parse(valid_profile_toml(), "a.toml").unwrap();
        let b = Profile::parse(valid_profile_toml(), "b.toml").unwrap();
        let err = validate_unique_source_ids(&[a, b]).unwrap_err();
        assert!(err.to_string().contains("home-assistant"));
    }

    #[test]
    fn distinct_source_ids_pass() {
        let a = Profile::parse(valid_profile_toml(), "a.toml").unwrap();
        let mut b = Profile::parse(valid_profile_toml(), "b.toml").unwrap();
        b.source_id = "uptime-kuma".to_string();
        assert!(validate_unique_source_ids(&[a, b]).is_ok());
    }

    #[test]
    fn a_misspelled_timezone_is_caught_at_startup_not_by_google_days_later() {
        // The real failure this prevents: "Europe/Brussel" is one
        // letter short of a real zone. Almanac used to start happily
        // and then fail permanently on every event from that source,
        // with nothing in the log naming the cause.
        let toml = r#"
schema_version = 1
source_id = "home-assistant"
target_calendar_id = "household"

[mapping]
title_field = "title"
start_field = "start"
duration_minutes = 30
timezone = "Europe/Brussel"
"#;
        let err = Profile::parse(toml, "bad-tz.toml").unwrap_err();
        assert!(err.to_string().contains("not an IANA time zone"));
        assert!(err.to_string().contains("bad-tz.toml"), "name the file");
        assert!(
            err.remedy().contains("Europe/Brussels"),
            "show the real one"
        );
    }

    #[test]
    fn real_time_zones_are_accepted() {
        for tz in [
            "Europe/Brussels",
            "UTC",
            "America/New_York",
            "Australia/Eucla",
        ] {
            let toml = format!(
                r#"
schema_version = 1
source_id = "s"
target_calendar_id = "c"

[mapping]
title_field = "title"
start_field = "start"
duration_minutes = 30
timezone = "{tz}"
"#
            );
            assert!(
                Profile::parse(&toml, "tz.toml").is_ok(),
                "{tz} is a real zone and must be accepted"
            );
        }
    }

    #[test]
    fn an_empty_start_field_is_rejected() {
        // Without it every payload from this source fails to map, and
        // the entry sits in the journal forever.
        let toml = r#"
schema_version = 1
source_id = "s"
target_calendar_id = "c"

[mapping]
title_field = "title"
start_field = ""
duration_minutes = 30
"#;
        let err = Profile::parse(toml, "no-start.toml").unwrap_err();
        assert!(err.to_string().contains("start_field is empty"));
    }

    fn parse_mapping(extra: &str) -> Result<Profile, AlmanacError> {
        Profile::parse(
            &format!(
                r#"
schema_version = 1
source_id = "s"
target_calendar_id = "c"

[mapping]
title_field = "t"
start_field = "start"
timezone = "UTC"
{extra}
"#
            ),
            "test.toml",
        )
    }

    #[test]
    fn an_all_day_profile_that_also_gives_minutes_is_rejected() {
        // "All day, 60 minutes" is two contradictory instructions.
        // Silently honouring one of them produces an event nobody
        // asked for, on every payload, forever.
        let err = parse_mapping("all_day = true\nduration_minutes = 60").unwrap_err();
        assert!(err.to_string().contains("duration_minutes"), "{err}");
        assert!(err.remedy().contains("duration_days"), "{}", err.remedy());
    }

    #[test]
    fn a_timed_profile_that_gives_days_is_rejected_too() {
        let err = parse_mapping("duration_minutes = 60\nduration_days = 2").unwrap_err();
        assert!(err.to_string().contains("duration_days"), "{err}");
    }

    #[test]
    fn a_timed_profile_without_a_duration_says_so_at_startup() {
        // duration_minutes became optional for K14; a timed profile
        // that omits it must still fail loudly rather than defaulting
        // to some length nobody chose.
        let err = parse_mapping("").unwrap_err();
        assert!(
            err.to_string().contains("duration_minutes is missing"),
            "{err}"
        );
        assert!(err.remedy().contains("all_day"), "{}", err.remedy());
    }

    #[test]
    fn an_all_day_profile_needs_nothing_more_than_the_flag() {
        let profile = parse_mapping("all_day = true").unwrap();
        assert!(profile.mapping.all_day);
        assert_eq!(profile.mapping.duration_minutes, None);
        assert_eq!(profile.mapping.duration_days, None);
    }

    #[test]
    fn zero_or_negative_days_are_rejected() {
        assert!(parse_mapping("all_day = true\nduration_days = 0").is_err());
        assert!(parse_mapping("all_day = true\nduration_days = -1").is_err());
    }

    #[test]
    fn a_status_google_does_not_know_is_caught_at_startup() {
        // The same reasoning as the timezone check: an invalid status
        // is a permanent failure on every event this source ever sends,
        // discovered days later as an unexplained Google error.
        let err = parse_mapping(
            "duration_minutes = 60\n[mapping.status_by]\nfield = \"s\"\ndefault = \"resolved\"",
        )
        .unwrap_err();
        assert!(err.to_string().contains("resolved"), "{err}");
        assert!(err.remedy().contains("cancelled"), "{}", err.remedy());
    }

    #[test]
    fn a_bad_status_hiding_in_the_lookup_table_is_caught_too() {
        // Not just the default: every value the table can produce.
        let err = parse_mapping(
            "duration_minutes = 60\n[mapping.status_by]\nfield = \"s\"\ndefault = \"confirmed\"\nvalues = { up = \"ok\" }",
        )
        .unwrap_err();
        assert!(err.to_string().contains("\"ok\""), "{err}");
    }

    #[test]
    fn the_three_statuses_google_accepts_all_parse() {
        for status in ["confirmed", "tentative", "cancelled"] {
            assert!(
                parse_mapping(&format!(
                    "duration_minutes = 60\n[mapping.status_by]\nfield = \"s\"\ndefault = \"{status}\""
                ))
                .is_ok(),
                "{status} should be accepted"
            );
        }
    }

    #[test]
    fn asking_for_silence_and_for_reminders_at_once_is_rejected() {
        let err = parse_mapping(
            "duration_minutes = 60\n[mapping.reminders]\nsilent = true\npopup_minutes_before = [30]",
        )
        .unwrap_err();
        assert!(err.to_string().contains("silence"), "{err}");
    }

    #[test]
    fn more_reminders_than_google_allows_is_caught_here_not_there() {
        let err = parse_mapping(
            "duration_minutes = 60\n[mapping.reminders]\npopup_minutes_before = [1, 2, 3, 4, 5, 6]",
        )
        .unwrap_err();
        assert!(err.to_string().contains("at most 5"), "{err}");
    }

    #[test]
    fn a_reminder_further_out_than_google_allows_is_rejected() {
        // Google's ceiling is four weeks. A profile asking for a
        // reminder a year ahead fails on every event otherwise.
        let err = parse_mapping(
            "duration_minutes = 60\n[mapping.reminders]\npopup_minutes_before = [525600]",
        )
        .unwrap_err();
        assert!(err.to_string().contains("four weeks"), "{err}");
    }

    #[test]
    fn a_reasonable_reminder_block_parses() {
        let profile = parse_mapping(
            "duration_minutes = 60\n[mapping.reminders]\npopup_minutes_before = [30, 1440]",
        )
        .unwrap();
        let rule = profile.mapping.reminders.unwrap();
        assert!(!rule.silent);
        assert_eq!(rule.popup_minutes_before, vec![30, 1440]);
    }

    #[test]
    fn two_ways_of_saying_how_long_are_refused_together() {
        // K18. Each pair, because "the code happens to read this one
        // first" is not a specification.
        for combo in [
            "duration_minutes = 60\nend_field = \"until\"",
            "all_day = true\nduration_days = 2\nend_field = \"until\"",
            "duration_minutes = 60\nduration_days = 2",
        ] {
            let err = parse_mapping(combo).unwrap_err();
            assert!(
                err.to_string().contains("one length")
                    || err.to_string().contains("duration_days")
                    || err.to_string().contains("end_field"),
                "{combo:?} should be refused, got: {err}"
            );
        }
    }

    #[test]
    fn a_timed_profile_may_take_its_end_from_the_payload_instead_of_minutes() {
        let profile = parse_mapping("end_field = \"until\"").unwrap();
        assert_eq!(profile.mapping.end_field.as_deref(), Some("until"));
        assert_eq!(profile.mapping.duration_minutes, None);
    }

    #[test]
    fn an_all_day_profile_cannot_take_its_end_from_the_payload() {
        // An all-day event is measured in days; a timestamp field there
        // is a category error, and refusing it says which of the two
        // the author meant to write.
        let err = parse_mapping("all_day = true\nend_field = \"until\"").unwrap_err();
        assert!(err.to_string().contains("all-day"), "{err}");
        assert!(err.remedy().contains("duration_days"), "{}", err.remedy());
    }

    #[test]
    fn an_empty_end_field_is_rejected_rather_than_looked_up() {
        let err = parse_mapping("end_field = \"\"").unwrap_err();
        assert!(err.to_string().contains("end_field is empty"), "{err}");
    }
}
