//! The profile the dashboard writes for a new source.

use super::{FALLBACK_DURATION_MINUTES, SUPPORTED_SCHEMA_VERSION};

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
