//! `DELETE /v1/ingest/{source_id}/events/{external_id}` (K8).

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chassis::Caller;
use serde_json::json;

use super::auth::authenticate;
use super::{AppState, Reply, error};
use crate::shell::delivery::UPSERT_PROPERTY;

/// `DELETE /v1/ingest/{source_id}/events/{external_id}` (K8) — removes
/// the event a source previously created.
///
/// The caller addresses the event by the id *it* used, not by Google's:
/// a Claude session that created an event with `external_id = "task-7"`
/// deletes it with the same name, and never has to have kept the
/// Google event id. That works because the upsert key is pinned to
/// `<source_id>:<external-id>` (AR15) and stored on the event, which
/// is the same lookup a redelivery uses to update instead of
/// duplicating.
///
/// Synchronous and not journalled, unlike ingest. There is no payload
/// to lose here: if Google is unreachable the caller is told so and
/// retries, whereas an accepted-but-undelivered *deletion* would be a
/// promise Almanac cannot keep — the event would stay on the calendar
/// while the caller believed it was gone.
///
/// A source can only delete under its own prefix, so one source can
/// never remove another's events even if it guesses the external id.
pub(super) async fn delete_event(
    State(state): State<Arc<AppState>>,
    Path((source_id, external_id)): Path<(String, String)>,
    caller: Caller,
) -> Reply {
    let profile = match authenticate(&state, &source_id, &caller).await {
        Ok(profile) => profile,
        Err(reply) => return reply,
    };

    let calendar = profile.target_calendar_id.clone();
    // The id a source used is either its `external_id` or the
    // Idempotency-Key it sent; the two live in separate namespaces since
    // 2026-11-02, so both are tried.
    let keys = [
        format!("{}:{external_id}", profile.source_id),
        crate::core::journal::idempotency_upsert_key(&profile.source_id, &external_id),
    ];
    let mut cancelled = 0;
    let mut not_found = None;
    for key in &keys {
        match delete_under_key(&state, &profile.source_id, &calendar, key).await {
            Ok((n, Some(event_id))) => {
                return deleted(json!(event_id), cancelled + n);
            }
            Ok((n, None)) => cancelled += n,
            Err(reply) if reply.0 == StatusCode::NOT_FOUND => not_found = Some(reply),
            Err(reply) => return reply,
        }
    }
    match not_found {
        Some(reply) if cancelled == 0 => reply,
        _ => deleted(serde_json::Value::Null, cancelled),
    }
}

fn deleted(event_id: serde_json::Value, cancelled: usize) -> Reply {
    (
        StatusCode::OK,
        Json(json!({"status": "deleted", "event_id": event_id, "pending_cancelled": cancelled})),
    )
}

/// Under `key`'s lock (the one delivery takes, so a delete cannot
/// interleave with an upsert and leave a recreated copy): cancels the
/// entries still waiting for it, then deletes its event. Answers how many
/// were cancelled and the deleted event's id; 404 when there is neither.
async fn delete_under_key(
    state: &AppState,
    source_id: &str,
    calendar: &str,
    key: &str,
) -> Result<(usize, Option<String>), Reply> {
    let lock = state.locks.for_key(key).await;
    let _guard = lock.lock().await;
    let cancelled = cancel_pending(state, key).await?;
    let event_id = match find_event_id(state, source_id, calendar, key).await {
        Ok(id) => id,
        Err((StatusCode::NOT_FOUND, _)) if cancelled > 0 => return Ok((cancelled, None)),
        Err(reply) => return Err(reply),
    };
    match state.client.delete_event(calendar, &event_id).await {
        Ok(()) => {
            tracing::info!(%source_id, %event_id, %key, "deleted an event on its source's request");
            Ok((cancelled, Some(event_id)))
        }
        Err(e) => {
            tracing::warn!(%source_id, error = %e, "delete failed");
            Err(error(StatusCode::BAD_GATEWAY, e.message(), e.remedy()))
        }
    }
}

/// Marks every journal entry still waiting under `key` as done, and says
/// how many there were (bug review 2026-11-02: a delete of an event that
/// was still pending answered 404, and the worker created it afterwards).
async fn cancel_pending(state: &AppState, key: &str) -> Result<usize, Reply> {
    let fail = |e: crate::core::error::AlmanacError| {
        error(StatusCode::INTERNAL_SERVER_ERROR, e.message(), e.remedy())
    };
    let waiting: Vec<String> = state
        .journal
        .pending()
        .map_err(fail)?
        .into_iter()
        .filter(|entry| crate::core::journal::entry_key(entry).as_deref() == Some(key))
        .map(|entry| entry.id)
        .collect();
    for id in &waiting {
        state.journal.mark_done(id).await.map_err(fail)?;
        state.locks.cancel(id);
    }
    if !waiting.is_empty() {
        tracing::info!(%key, cancelled = waiting.len(), "a delete cancelled events still waiting in the journal");
    }
    Ok(waiting.len())
}

/// Looks the event up by its upsert key and returns Google's id for it,
/// or the reply that explains why there is nothing to delete.
async fn find_event_id(
    state: &AppState,
    source_id: &str,
    calendar: &str,
    key: &str,
) -> Result<String, Reply> {
    let found = match state
        .client
        .find_event_by_property(calendar, UPSERT_PROPERTY, key)
        .await
    {
        Ok(found) => found,
        Err(e) => {
            tracing::warn!(source_id = %source_id, error = %e, "delete lookup failed");
            return Err(error(StatusCode::BAD_GATEWAY, e.message(), e.remedy()));
        }
    };

    // The lookup includes cancelled events (for the upsert); one that is
    // already cancelled is gone as far as the source is concerned.
    let found = found.filter(|event| event.status.as_deref() != Some("cancelled"));
    let Some(event) = found else {
        // Deliberately distinct from success: a caller retrying a
        // delete needs to be able to tell "already gone" from "just
        // removed", and silently answering 200 would hide a wrong
        // external id forever.
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({
                "status": "not_found",
                "message": format!("no event on {calendar} carries {key}"),
                "remedy": "check the external id; it must be the one this source used when the \
                           event was created"
            })),
        ));
    };

    event.id.ok_or_else(|| {
        error(
            StatusCode::BAD_GATEWAY,
            "the Calendar API returned a matching event with no id",
            "this is unexpected; nothing was deleted",
        )
    })
}
