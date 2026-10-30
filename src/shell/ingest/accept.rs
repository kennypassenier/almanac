//! `POST /v1/ingest/{source_id}`: the asynchronous endpoint (K7).

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use chassis::Caller;
use serde_json::{Value, json};

use super::auth::authenticate;
use super::request::accept_payload;
use super::{AppState, Reply};

/// `POST /v1/ingest/{source_id}` — accept, journal, answer 202 (K7).
pub(super) async fn ingest(
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

    tracing::info!(
        source_id = %profile.source_id, entry_id = %entry.id,
        "payload accepted and journalled"
    );

    (
        StatusCode::ACCEPTED,
        Json(json!({"status": "accepted", "entry_id": entry.id})),
    )
}
