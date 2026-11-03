//! `GET /status/journal`: the Journal section's glance and recent
//! deliveries.

use std::collections::HashSet;
use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use crate::core::error::AlmanacError;
use crate::core::journal::Entry;
use crate::core::status_view::{self, DeliveryView, Outcomes, Trend};
use crate::shell::state::AppState;

/// Whether the journal's database took its last writes: the kit records a
/// write that failed in the last minute.
fn store_state(state: &AppState) -> (&'static str, &'static str) {
    let failing = state
        .app_db()
        .ok()
        .and_then(|db| db.db().ok())
        .and_then(|db| db.recent_write_failure());
    match failing {
        None => ("ok", "Writable"),
        Some(why) => {
            tracing::warn!(%why, "the journal's database refused a write in the last minute");
            (
                "down",
                "A write failed; check free disk space and the data folder's permissions",
            )
        }
    }
}

/// The ids of the entries set aside as dead, or `None` when they could
/// not be read: logged, and the page says so rather than showing none.
fn dead_ids(dead: Result<Vec<(Entry, String, String)>, AlmanacError>) -> Option<HashSet<String>> {
    match dead {
        Ok(d) => Some(d.into_iter().map(|(e, _, _)| e.id).collect()),
        Err(e) => {
            tracing::warn!(error = %e, "could not read the dead entries for the status page");
            None
        }
    }
}

/// The journal health mark: readable and empty, readable with a backlog,
/// or unreadable.
fn journal_mark(pending: &Result<Vec<Entry>, AlmanacError>) -> Value {
    match pending {
        Ok(p) if p.is_empty() => json!({ "state": "ok", "text": "Readable, nothing pending" }),
        Ok(p) => {
            json!({ "state": "warn", "text": format!("Readable, {} waiting to be delivered", p.len()) })
        }
        Err(_) => json!({ "state": "down", "text": "Cannot be read; deliveries are on hold" }),
    }
}

/// The recent-deliveries list, each row with the name of its calendar.
fn deliveries_json(state: &AppState, rows: &[DeliveryView]) -> Vec<Value> {
    rows.iter()
        .map(|r| {
            let calendar = state
                .profiles()
                .get(&r.source_id)
                .map(|p| state.calendar_name(&p.target_calendar_id));
            json!({
                "id": r.id,
                "at": r.at,
                "source": r.source_id,
                "result": r.result.as_str(),
                "calendar": calendar,
                "error": r.error,
            })
        })
        .collect()
}

/// The outcome bar. The ring keeps the latest HISTORY_CAPACITY attempts;
/// once deliveries outnumber what it still holds, the bar says it shows
/// the latest ones (`partial`).
fn outcomes_json(outcomes: &Outcomes, delivered: u64) -> Value {
    json!({
        "created": outcomes.created,
        "updated": outcomes.updated,
        "failed": outcomes.failed,
        "partial": (outcomes.created + outcomes.updated) as u64 != delivered,
    })
}

/// The per-hour trend block.
fn trend_json(trend: &Trend) -> Value {
    json!({
        "start": trend.start_ms,
        "step": trend.step_ms,
        "values": trend.values,
        "lastHour": trend.last_hour,
    })
}

/// `GET /status/journal`: the figures, the two health marks, the outcome
/// bar, the per-hour trend and the deliveries, all from what Almanac holds
/// in memory (counters, the route ring) and the journal file.
pub(super) async fn journal(State(state): State<Arc<AppState>>) -> Response {
    let routes: Vec<_> = state.routes.lock().await.iter().cloned().collect();
    let read = Arc::clone(&state);
    let read = tokio::task::spawn_blocking(move || {
        let pending = read.journal.pending();
        let dead = read.journal.dead();
        let store = store_state(&read);
        (pending, dead, store)
    })
    .await;
    let (pending, dead, store) = match read {
        Ok(read) => read,
        Err(e) => {
            tracing::warn!(error = %e, "the journal read for the status page did not finish");
            return chassis::Error::internal(
                "the journal read did not finish",
                "try again in a moment",
            )
            .into_response();
        }
    };
    let pending_ids: Option<HashSet<String>> = pending
        .as_ref()
        .ok()
        .map(|p| p.iter().map(|e| e.id.clone()).collect());
    let dead_ids = dead_ids(dead);
    let dead_readable = dead_ids.is_some();
    let dead_ids = dead_ids.unwrap_or_default();

    let rows = status_view::fold_deliveries(&routes, pending_ids.as_ref(), &dead_ids);
    let outcomes = status_view::outcomes(&rows);
    let counts = state.metrics.snapshot();
    let trend = status_view::hourly_trend(&routes, chrono::Utc::now());

    Json(json!({
        "at": chrono::Utc::now().to_rfc3339(),
        "waiting": pending.as_ref().ok().map(Vec::len),
        "accepted": counts.accepted,
        "delivered": counts.delivered,
        "gaveUp": counts.dead,
        "deadReadable": dead_readable,
        "journal": journal_mark(&pending),
        "store": { "state": store.0, "text": store.1 },
        "outcomes": outcomes_json(&outcomes, counts.delivered),
        "trend": trend_json(&trend),
        "deliveries": deliveries_json(&state, &rows),
    }))
    .into_response()
}
