//! The entries Almanac set aside, worded for the Deliveries page.

use std::collections::HashMap;

use serde_json::{Value, json};

use crate::core::journal::{self, Entry};
use crate::core::pacing;
use crate::core::profile::Profile;
use crate::shell::worker;

/// The set-aside entries as page items, the latest first. The board's last
/// lane and the list under it are both drawn from this one list.
pub(super) fn items(
    dead: &[(Entry, String, String)],
    profiles: &HashMap<String, Profile>,
    now_unix: i64,
) -> Vec<Value> {
    let mut items: Vec<(u64, Value)> = dead
        .iter()
        .map(|(entry, reason, at)| {
            let at_ms = chrono::DateTime::parse_from_rfc3339(at)
                .map(|t| t.timestamp_millis().max(0) as u64)
                .unwrap_or(0);
            let (cause, remedy) = pacing::plain_set_aside_reason(reason);
            (
                at_ms,
                json!({
                    "id": entry.id,
                    "source": entry.source_id,
                    "title": worker::entry_title(entry, profiles.get(&entry.source_id)),
                    "at": at_ms,
                    "days_left": journal::days_left(at, now_unix),
                    "cause": cause,
                    "short": pacing::short_set_aside_reason(reason),
                    "remedy": remedy,
                }),
            )
        })
        .collect();
    items.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
    items.into_iter().map(|(_, v)| v).collect()
}
