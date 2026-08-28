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
    pub duration_minutes: i64,
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

        if profile.mapping.duration_minutes <= 0 {
            return Err(AlmanacError::ProfileValidation {
                message: format!(
                    "{origin}: mapping.duration_minutes must be positive, got {}",
                    profile.mapping.duration_minutes
                ),
                remedy: format!(
                    "set mapping.duration_minutes in {origin} to a positive number of minutes"
                ),
            });
        }

        Ok(profile)
    }
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
        assert_eq!(profile.mapping.duration_minutes, 60);
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
}
