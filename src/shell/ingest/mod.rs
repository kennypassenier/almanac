//! The inbound HTTP surface (K6, K7, K8, M7). One ingest endpoint per
//! source, addressed by the profile's immutable `source_id` (AR15):
//!
//!   POST /v1/ingest/{source_id}        → 202, durably journalled (K7)
//!   POST /v1/ingest/{source_id}/sync   → 200 + the event id (K8)
//!
//! Both authenticate with that source's own bearer token (K6): the
//! presented token is hashed and compared, in constant time, against
//! the `token_hash` in its profile. A source only ever holds a token
//! for itself, so one can be revoked without touching the others.
//!
//! Both journal the payload and fsync it *before* answering, so an
//! accepted request survives a crash or power cut (AR16). The
//! asynchronous form returns as soon as that is durable; the
//! synchronous form additionally waits for delivery, because its
//! caller wants the Google event id back.

mod accept;
mod auth;
mod delete;
mod request;
mod sync;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::http::StatusCode;
use serde_json::{Value, json};

// `AppState` and its capacity moved to `crate::shell::state`; the old
// path stays valid so no caller has to change.
pub use crate::shell::state::{AppState, HISTORY_CAPACITY};
pub(crate) use request::requires_an_id;

/// Header a source may send to make a redelivery converge instead of
/// duplicating, when it has no natural per-payload id (M7).
const IDEMPOTENCY_HEADER: &str = "idempotency-key";

/// What every handler answers: a status and a JSON body.
type Reply = (StatusCode, Json<Value>);

/// The one error body shape of the ingest API: what went wrong and what
/// the caller can do about it.
fn error(status: StatusCode, message: &str, remedy: &str) -> Reply {
    (
        status,
        Json(json!({"status": "error", "message": message, "remedy": remedy})),
    )
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/v1/ingest/{source_id}",
            axum::routing::post(accept::ingest),
        )
        .route(
            "/v1/ingest/{source_id}/sync",
            axum::routing::post(sync::ingest_sync),
        )
        .route(
            "/v1/ingest/{source_id}/events/{external_id}",
            axum::routing::delete(delete::delete_event),
        )
}
