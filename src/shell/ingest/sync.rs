//! `POST /v1/ingest/{source_id}/sync`: the synchronous endpoint (K8).

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use chassis::Caller;
use serde_json::{Value, json};

use super::auth::authenticate;
use super::request::accept_payload;
use super::{AppState, Reply, error};
use crate::core::error::AlmanacError;
use crate::core::journal::Entry;
use crate::shell::delivery::{Delivered, deliver};

/// `POST /v1/ingest/{source_id}/sync` — accept, journal, deliver, and
/// answer with the Google event id (K8).
pub(super) async fn ingest_sync(
    State(state): State<Arc<AppState>>,
    Path(source_id): Path<String>,
    caller: Caller,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> Reply {
    let profile = match authenticate(&state, &source_id, &caller).await {
        Ok(profile) => profile,
        Err(reply) => return reply,
    };
    let entry = match accept_payload(&state, &profile, &headers, payload).await {
        Ok(entry) => entry,
        Err(reply) => return reply,
    };

    // Counted and recorded exactly as the worker does, so a source that
    // only ever posts here still shows up on /metrics and the debug
    // surface. Without this, JobTracker's deliveries left
    // `almanac_events_delivered_total` at 0 while its events sat on the
    // calendar (homelab ask-6).
    let result = deliver(&entry, &profile, &state.client, &state.locks).await;
    crate::shell::worker::record_route(&state, &entry, &result).await;

    match result {
        Ok(delivered) => delivered_reply(&state, &entry, delivered).await,
        Err(e) => failed_reply(&state, &entry, &e),
    }
}

/// The 200 answer, after marking the journal entry done.
async fn delivered_reply(state: &AppState, entry: &Entry, delivered: Delivered) -> Reply {
    state.metrics.delivered(1);
    if let Err(e) = state.journal.mark_done(&entry.id).await {
        // The event IS on the calendar; only the bookkeeping failed.
        // Replay would redeliver it, which upsert makes harmless, so
        // this is a warning rather than an error to the caller.
        tracing::warn!(
            entry_id = %entry.id, error = %e,
            "delivered but failed to mark the journal entry done; replay will converge"
        );
    }
    (
        StatusCode::OK,
        Json(json!({
            "status": "delivered",
            "event_id": delivered.event_id,
            "created": delivered.created,
        })),
    )
}

/// The 502 answer. The entry stays pending in the journal, so the worker
/// retries it later: the caller's payload is not lost even though this
/// response reports the failure.
fn failed_reply(state: &AppState, entry: &Entry, e: &AlmanacError) -> Reply {
    state.metrics.failed(1);
    tracing::error!(
        source_id = %entry.source_id, error = %e,
        "synchronous delivery failed; left pending for retry"
    );
    error(StatusCode::BAD_GATEWAY, e.message(), e.remedy())
}
