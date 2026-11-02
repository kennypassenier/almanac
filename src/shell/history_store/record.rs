//! Recording a delivery outcome after the worker's attempt.

use super::rows::insert;
use crate::core::error::AlmanacError;
use crate::core::history::{DeliveryRecord, DeliveryResult, Row};
use crate::shell::delivery::Delivered;
use crate::shell::state::AppState;

/// The title when the mapping cannot be run again to find the real one.
const UNTITLED: &str = "Untitled event";
/// The origin label the mapping reports in its errors when re-run for a title.
const TITLE_ORIGIN: &str = "history";

/// A plain sentence for a failure, never the raw error.
fn plain_reason(e: &AlmanacError) -> &'static str {
    match e {
        AlmanacError::GoogleApi { .. } | AlmanacError::Auth { .. } => {
            "Google Calendar would not take this event."
        }
        _ => "The event could not be built from what the source sent.",
    }
}

/// The hook `worker::record_route` calls after every delivery attempt.
/// Only final outcomes are kept: a delivery, or a failure that will not
/// fix itself. A transient failure stays pending and is retried, so it is
/// not an outcome yet. Never fails the delivery: a history problem is logged.
pub async fn note(
    state: &AppState,
    entry: &crate::core::journal::Entry,
    result: &Result<Delivered, AlmanacError>,
) {
    let (outcome, reason, upsert_key) = match result {
        Ok(d) if d.created => (DeliveryResult::Created, None, d.upsert_key.clone()),
        Ok(d) => (DeliveryResult::Updated, None, d.upsert_key.clone()),
        Err(e) if e.is_transient() => return,
        Err(e) => (
            DeliveryResult::Failed,
            Some(plain_reason(e).to_string()),
            None,
        ),
    };
    let profile = state.profiles().get(&entry.source_id).cloned();
    let title = title_of(entry, profile.as_ref());
    let rec = DeliveryRecord {
        at: (state.now)(),
        entry_id: entry.id.clone(),
        source: entry.source_id.clone(),
        calendar_id: profile.map(|p| p.target_calendar_id).unwrap_or_default(),
        title,
        result: outcome,
        reason,
        upsert_key,
    };
    let Some(row) = Row::from_record(&rec) else {
        tracing::warn!(entry = %rec.entry_id, "the delivery has an unreadable timestamp; not recorded in the history");
        return;
    };
    // The write waits for a backup pause by itself (the kit's ticket).
    let written = match state.app_db() {
        Ok(db) => db.write(move |tx| insert(tx, &row)).await,
        Err(e) => Err(e),
    };
    if let Err(e) = written {
        // Counted so /metrics and the Status page show it, not only the log.
        state.metrics.history_write_failed((state.now_unix)());
        tracing::warn!(error = %e, remedy = %e.remedy, "could not record the delivery history");
    }
}

/// The event title the entry maps to, or [`UNTITLED`] when the profile is
/// gone or the payload no longer maps (the reason is logged at debug: a
/// failed delivery has already reported it).
fn title_of(
    entry: &crate::core::journal::Entry,
    profile: Option<&crate::core::profile::Profile>,
) -> String {
    let Some(profile) = profile else {
        return UNTITLED.to_string();
    };
    match crate::core::mapping::map_payload(&entry.payload, profile, TITLE_ORIGIN) {
        Ok(event) => event.summary,
        Err(e) => {
            tracing::debug!(entry = %entry.id, error = %e, "no title for the history; using the placeholder");
            UNTITLED.to_string()
        }
    }
}
