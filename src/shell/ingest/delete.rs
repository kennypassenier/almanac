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

    let key = format!("{}:{external_id}", profile.source_id);
    let calendar = profile.target_calendar_id.clone();

    // Serialize on the same key the delivery path uses, so a delete
    // cannot interleave with an upsert of the same event and leave a
    // recreated copy behind.
    let lock = state.locks.for_key(&key).await;
    let _guard = lock.lock().await;

    let event_id = match find_event_id(&state, &profile.source_id, &calendar, &key).await {
        Ok(id) => id,
        Err(reply) => return reply,
    };

    match state.client.delete_event(&calendar, &event_id).await {
        Ok(()) => {
            tracing::info!(
                source_id = %profile.source_id, %event_id, %key,
                "deleted an event on its source's request"
            );
            (
                StatusCode::OK,
                Json(json!({"status": "deleted", "event_id": event_id})),
            )
        }
        Err(e) => {
            tracing::warn!(source_id = %profile.source_id, error = %e, "delete failed");
            error(StatusCode::BAD_GATEWAY, &e.to_string(), e.remedy())
        }
    }
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
            return Err(error(StatusCode::BAD_GATEWAY, &e.to_string(), e.remedy()));
        }
    };

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
