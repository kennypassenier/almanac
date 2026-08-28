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

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::{Json, Router};
use serde_json::{Value, json};

use crate::core::journal::Entry;
use crate::core::profile::Profile;
use crate::core::token::{parse_bearer, verify_token};
use crate::shell::calendar_client::GoogleCalendarClient;
use crate::shell::delivery::{KeyLocks, deliver};
use crate::shell::journal::Journal;

/// Header a source may send to make a redelivery converge instead of
/// duplicating, when it has no natural per-payload id (M7).
const IDEMPOTENCY_HEADER: &str = "idempotency-key";

pub struct AppState {
    pub profiles: HashMap<String, Profile>,
    pub journal: Journal,
    pub client: GoogleCalendarClient,
    pub locks: KeyLocks,
    /// Supplies the acceptance timestamp; injected rather than read
    /// ambiently so tests can pin it.
    pub now: Box<dyn Fn() -> String + Send + Sync>,
}

type Reply = (StatusCode, Json<Value>);

fn error(status: StatusCode, message: &str, remedy: &str) -> Reply {
    (
        status,
        Json(json!({"status": "error", "message": message, "remedy": remedy})),
    )
}

/// Resolves the profile for `source_id` and checks the request's
/// bearer token against it.
///
/// An unknown source and a wrong token both answer 401 with the same
/// body: distinguishing them would tell an unauthenticated caller
/// which source ids exist.
fn authenticate<'a>(
    state: &'a AppState,
    source_id: &str,
    headers: &HeaderMap,
) -> Result<&'a Profile, Reply> {
    let unauthorized = || {
        error(
            StatusCode::UNAUTHORIZED,
            "unknown source or invalid token",
            "check the Authorization: Bearer header matches the token issued for this source",
        )
    };

    let Some(profile) = state.profiles.get(source_id) else {
        return Err(unauthorized());
    };

    let presented = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_bearer);

    match presented {
        Some(token) if verify_token(token, &profile.token_hash) => Ok(profile),
        _ => Err(unauthorized()),
    }
}

fn build_entry(state: &AppState, source_id: &str, headers: &HeaderMap, payload: Value) -> Entry {
    let idempotency_key = headers
        .get(IDEMPOTENCY_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    Entry {
        id: uuid::Uuid::new_v4().to_string(),
        source_id: source_id.to_string(),
        received_at: (state.now)(),
        payload,
        idempotency_key,
    }
}

/// `POST /v1/ingest/{source_id}` — accept, journal, answer 202 (K7).
async fn ingest(
    State(state): State<Arc<AppState>>,
    Path(source_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> Reply {
    let profile = match authenticate(&state, &source_id, &headers) {
        Ok(profile) => profile,
        Err(reply) => return reply,
    };
    let source_id = profile.source_id.clone();

    let entry = build_entry(&state, &source_id, &headers, payload);

    if let Err(e) = state.journal.accept(&entry).await {
        tracing::error!(source_id = %source_id, error = %e, "failed to journal an accepted payload");
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &e.to_string(),
            e.remedy(),
        );
    }

    tracing::info!(source_id = %source_id, entry_id = %entry.id, "payload accepted and journalled");

    (
        StatusCode::ACCEPTED,
        Json(json!({"status": "accepted", "entry_id": entry.id})),
    )
}

/// `POST /v1/ingest/{source_id}/sync` — accept, journal, deliver, and
/// answer with the Google event id (K8).
async fn ingest_sync(
    State(state): State<Arc<AppState>>,
    Path(source_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> Reply {
    let profile = match authenticate(&state, &source_id, &headers) {
        Ok(profile) => profile,
        Err(reply) => return reply,
    };
    let profile = profile.clone();

    let entry = build_entry(&state, &profile.source_id, &headers, payload);

    if let Err(e) = state.journal.accept(&entry).await {
        tracing::error!(source_id = %profile.source_id, error = %e, "failed to journal an accepted payload");
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &e.to_string(),
            e.remedy(),
        );
    }

    match deliver(&entry, &profile, &state.client, &state.locks).await {
        Ok(delivered) => {
            if let Err(e) = state.journal.mark_done(&entry.id).await {
                // The event IS on the calendar; only the bookkeeping
                // failed. Replay would redeliver it, which upsert makes
                // harmless, so this is a warning rather than an error
                // to the caller.
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
        Err(e) => {
            // The entry stays pending in the journal, so the worker
            // retries it later — the caller's payload is not lost even
            // though this response reports the failure.
            tracing::error!(source_id = %profile.source_id, error = %e, "synchronous delivery failed; left pending for retry");
            error(StatusCode::BAD_GATEWAY, &e.to_string(), e.remedy())
        }
    }
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/ingest/{source_id}", axum::routing::post(ingest))
        .route(
            "/v1/ingest/{source_id}/sync",
            axum::routing::post(ingest_sync),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::token::hash_token;

    fn profile(source_id: &str, token: &str) -> Profile {
        let toml = format!(
            r#"
schema_version = 1
source_id = "{source_id}"
target_calendar_id = "primary"
token_hash = "{}"

[mapping]
title_field = "title"
start_field = "start"
duration_minutes = 60
"#,
            hash_token(token)
        );
        Profile::parse(&toml, "test.toml").unwrap()
    }

    fn state_with(source_id: &str, token: &str) -> AppState {
        let mut profiles = HashMap::new();
        profiles.insert(source_id.to_string(), profile(source_id, token));
        AppState {
            profiles,
            journal: Journal::new(
                std::env::temp_dir().join("almanac-ingest-test-unused.jsonl"),
                crate::shell::journal::DEFAULT_MAX_BYTES,
            ),
            client: GoogleCalendarClient::new(
                reqwest::Client::new(),
                crate::shell::auth::TokenManager::new(
                    reqwest::Client::new(),
                    crate::core::auth::ServiceAccountCredentials {
                        client_email: "unused".to_string(),
                        private_key: "unused".to_string(),
                        token_url: "https://example.invalid/token".to_string(),
                    },
                ),
            ),
            locks: KeyLocks::new(),
            now: Box::new(|| "2026-08-28T09:00:00+00:00".to_string()),
        }
    }

    fn headers_with_token(token: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        headers
    }

    #[test]
    fn the_right_token_authenticates() {
        let state = state_with("home-assistant", "correct-token");
        let result = authenticate(
            &state,
            "home-assistant",
            &headers_with_token("correct-token"),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn a_wrong_token_is_rejected_with_401() {
        let state = state_with("home-assistant", "correct-token");
        let err =
            authenticate(&state, "home-assistant", &headers_with_token("wrong-token")).unwrap_err();
        assert_eq!(err.0, StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn a_missing_authorization_header_is_rejected() {
        let state = state_with("home-assistant", "correct-token");
        let err = authenticate(&state, "home-assistant", &HeaderMap::new()).unwrap_err();
        assert_eq!(err.0, StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn one_sources_token_does_not_open_another_source() {
        // The whole point of per-source tokens (K6): revoking or
        // leaking one must not affect the others.
        let mut state = state_with("home-assistant", "ha-token");
        state.profiles.insert(
            "uptime-kuma".to_string(),
            profile("uptime-kuma", "kuma-token"),
        );

        assert!(authenticate(&state, "uptime-kuma", &headers_with_token("kuma-token")).is_ok());
        assert_eq!(
            authenticate(&state, "uptime-kuma", &headers_with_token("ha-token"))
                .unwrap_err()
                .0,
            StatusCode::UNAUTHORIZED
        );
    }

    #[test]
    fn an_unknown_source_is_indistinguishable_from_a_bad_token() {
        // Answering 404 for an unknown source would let an
        // unauthenticated caller enumerate which sources exist.
        let state = state_with("home-assistant", "correct-token");
        let unknown = authenticate(
            &state,
            "does-not-exist",
            &headers_with_token("correct-token"),
        )
        .unwrap_err();
        let bad_token =
            authenticate(&state, "home-assistant", &headers_with_token("wrong")).unwrap_err();
        assert_eq!(unknown.0, bad_token.0);
        assert_eq!(format!("{:?}", unknown.1.0), format!("{:?}", bad_token.1.0));
    }

    #[test]
    fn an_idempotency_key_header_lands_on_the_entry() {
        let state = state_with("home-assistant", "t");
        let mut headers = headers_with_token("t");
        headers.insert(IDEMPOTENCY_HEADER, "abc123".parse().unwrap());
        let entry = build_entry(&state, "home-assistant", &headers, json!({}));
        assert_eq!(entry.idempotency_key.as_deref(), Some("abc123"));
    }

    #[test]
    fn an_absent_or_blank_idempotency_key_is_none_not_an_empty_string() {
        let state = state_with("home-assistant", "t");
        let entry = build_entry(
            &state,
            "home-assistant",
            &headers_with_token("t"),
            json!({}),
        );
        assert_eq!(entry.idempotency_key, None);

        let mut headers = headers_with_token("t");
        headers.insert(IDEMPOTENCY_HEADER, "   ".parse().unwrap());
        let entry = build_entry(&state, "home-assistant", &headers, json!({}));
        assert_eq!(entry.idempotency_key, None);
    }

    #[test]
    fn each_accepted_payload_gets_its_own_entry_id() {
        let state = state_with("home-assistant", "t");
        let a = build_entry(
            &state,
            "home-assistant",
            &headers_with_token("t"),
            json!({}),
        );
        let b = build_entry(
            &state,
            "home-assistant",
            &headers_with_token("t"),
            json!({}),
        );
        assert_ne!(a.id, b.id);
    }
}
