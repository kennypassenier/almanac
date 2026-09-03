//! A mapping profile: which calendar a source writes to, and nothing
//! more than it has to be.
//!
//! **This is the 2.0.0 shape.** Until then a profile was a translation
//! table — it named which payload field meant the title, which meant
//! the start, and carried every per-event choice as a setting fixed for
//! that source forever. It was built for webhooks nobody here controls.
//!
//! Kenny's decision, 2026-09-03: *"voor aanpassingen hadden we
//! HTTPSwitchboard! dus doe het volgens mijn model!"* A source speaks
//! Almanac's own event shape (`core::request`), the profile says where
//! its events land, and anything speaking a different shape is
//! translated by the tool built for translating shapes.
//!
//! What survives here is what genuinely belongs to the source rather
//! than to the event: its identity (AR15), its calendar (K3), and two
//! defaults a source may leave out of every call.

use std::collections::HashMap;

use serde::Deserialize;

use crate::core::error::AlmanacError;

/// The profile format this build reads. Bumped to 2 by the 2.0.0
/// change above: a v1 profile describes field mappings that no longer
/// exist, and reading it as if it were v2 would silently ignore
/// everything it says.
pub const SUPPORTED_SCHEMA_VERSION: u32 = 2;

/// How long a timed event lasts when neither the call nor the profile
/// says. An hour is the length of the thing a person means by "an
/// appointment"; a source that means otherwise says so.
pub const FALLBACK_DURATION_MINUTES: i64 = 60;

#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub schema_version: u32,
    /// Immutable identity of this source (AR15) — deliberately
    /// separate from the profile's filename or any display label, so
    /// renaming either never silently orphans already-created events.
    pub source_id: String,
    /// Which calendar this source's events land on (K3).
    pub target_calendar_id: String,
    /// The zone timestamps are read in when a call does not say.
    #[serde(default = "default_timezone")]
    pub timezone: String,
    /// How long this source's events last when a call gives neither an
    /// end nor a duration. Absent falls back to
    /// [`FALLBACK_DURATION_MINUTES`].
    #[serde(default)]
    pub default_duration_minutes: Option<i64>,
}

fn default_timezone() -> String {
    "Europe/Brussels".to_string()
}

impl Profile {
    /// Parses and validates a profile from its TOML source. `origin`
    /// (typically the file path) is used only to make error messages
    /// name where the problem is.
    pub fn parse(toml_str: &str, origin: &str) -> Result<Profile, AlmanacError> {
        // A v1 profile parses as nothing here — `deny_unknown_fields`
        // sees `[mapping]` and refuses — but the message would be about
        // an unknown key rather than about a format that changed, so
        // the version is read first and answered on its own terms.
        if let Ok(peek) = toml_str.parse::<toml::Table>()
            && let Some(version) = peek.get("schema_version").and_then(|v| v.as_integer())
            && version != i64::from(SUPPORTED_SCHEMA_VERSION)
        {
            return Err(AlmanacError::ProfileValidation {
                message: format!(
                    "{origin}: schema_version {version} is not supported (this build reads {SUPPORTED_SCHEMA_VERSION})"
                ),
                remedy: if version == 1 {
                    "this is a v1 profile: it names payload fields and per-event settings, which \
                     almanac 2.0 takes from the call itself. Reduce it to schema_version, \
                     source_id, target_calendar_id and optionally timezone and \
                     default_duration_minutes; have the source send almanac's event shape, or put \
                     HTTPSwitchboard in front of it. See docs/USER_GUIDE.md."
                        .to_string()
                } else {
                    format!("set schema_version = {SUPPORTED_SCHEMA_VERSION} in {origin}")
                },
            });
        }

        let profile: Profile =
            toml::from_str(toml_str).map_err(|e| AlmanacError::ProfileValidation {
                message: format!("{origin}: failed to parse as TOML: {e}"),
                remedy: format!("fix the syntax error in {origin}"),
            })?;

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
                    "use only letters, digits, '.', '-' and '_' in the source_id in {origin}, and \
                     do not start it with a dot — it is both a URL segment and the name of this \
                     profile's file on disk"
                ),
            });
        }

        if profile.target_calendar_id.trim().is_empty() {
            return Err(AlmanacError::ProfileValidation {
                message: format!("{origin}: target_calendar_id is empty"),
                remedy: format!(
                    "set target_calendar_id in {origin} to the calendar this source's events \
                     should land on"
                ),
            });
        }

        validate_timezone(&profile.timezone, origin)?;

        if let Some(minutes) = profile.default_duration_minutes
            && minutes <= 0
        {
            return Err(AlmanacError::ProfileValidation {
                message: format!("{origin}: default_duration_minutes is {minutes}"),
                remedy: "a default length must be greater than zero, or absent".to_string(),
            });
        }

        Ok(profile)
    }
}

/// Checks an IANA timezone name at the moment it is offered (M4).
///
/// A typo here used to surface as an event Google put in the wrong
/// place days later, which is why it is refused when the profile loads
/// — and now also when a call names one.
pub fn validate_timezone(timezone: &str, origin: &str) -> Result<(), AlmanacError> {
    if timezone.parse::<chrono_tz::Tz>().is_err() {
        return Err(AlmanacError::ProfileValidation {
            message: format!("{origin}: {timezone:?} is not an IANA timezone name"),
            remedy: "use a name like Europe/Brussels or UTC".to_string(),
        });
    }
    Ok(())
}

/// The profile the dashboard writes when someone adds a source with
/// nothing but a name and a calendar (K21).
///
/// Since 2.0.0 this is the whole file. There is no field mapping to
/// fill in and no per-event setting to guess at: the source says what
/// each event is when it sends it.
pub fn default_profile_toml(source_id: &str, calendar_id: &str) -> String {
    format!(
        r#"# Written from the dashboard.
#
# A profile says where a source's events land. What each event IS —
# its title, start, length, colour, whether it is all-day — comes from
# the source in the call itself. See docs/USER_GUIDE.md.

schema_version = {SUPPORTED_SCHEMA_VERSION}
source_id = "{source_id}"
target_calendar_id = "{calendar_id}"
timezone = "Europe/Brussels"
default_duration_minutes = {FALLBACK_DURATION_MINUTES}
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
/// two that do would make the upsert key ambiguous, so it stops
/// startup rather than being resolved by read order.
pub fn validate_unique_source_ids(profiles: &[Profile]) -> Result<(), AlmanacError> {
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for (i, p) in profiles.iter().enumerate() {
        if let Some(&first) = seen.get(p.source_id.as_str()) {
            return Err(AlmanacError::ProfileValidation {
                message: format!(
                    "profiles at index {first} and {i} both use source_id \"{}\"",
                    p.source_id
                ),
                remedy: "give each profile a unique source_id — it is the identity used for \
                         upsert lookups (AR15) and must never be shared"
                    .to_string(),
            });
        }
        seen.insert(p.source_id.as_str(), i);
    }
    Ok(())
}
