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

use tokio::sync::Mutex;

use crate::core::journal::Entry;
use crate::core::metrics::Metrics;
use crate::core::observability::{CaptureRecord, RingBuffer, RouteRecord};
use crate::core::profile::Profile;
use crate::core::token::parse_bearer;
use crate::shell::calendar_client::GoogleCalendarClient;
use crate::shell::delivery::{KeyLocks, deliver};
use crate::shell::journal::Journal;
use crate::shell::token_store::TokenStore;

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
    /// Unix seconds, for the capture surface's expiry arithmetic (M11).
    /// Separate from `now` so retention never has to parse a timestamp
    /// back out of a formatted string.
    pub now_unix: Box<dyn Fn() -> u64 + Send + Sync>,
    /// SHA-256 of the bootstrap token that guards the admin surface
    /// (AR17 as amended). `None` when unset, in which case the admin
    /// surface refuses everything rather than opening up.
    pub bootstrap_token_hash: Option<String>,
    /// SHA-256 of the capture-only token (S2). `None` when unset, in
    /// which case posting a capture needs the bootstrap token — the
    /// old behaviour, kept so an existing deployment does not break.
    pub capture_token_hash: Option<String>,
    /// Recent delivery routes, for the K11 debug surface.
    pub routes: Mutex<RingBuffer<RouteRecord>>,
    /// Verbatim captured requests, for the M11 capture surface.
    pub captures: Mutex<RingBuffer<CaptureRecord>>,
    /// Encrypted per-source tokens (M12/AR17) — the single source of
    /// truth for who may post, replacing the profile's `token_hash`.
    pub tokens: TokenStore,
    /// M13 counters. Shared with the token manager, which is built
    /// before this state exists, so it is an `Arc` rather than owned.
    pub metrics: Arc<Metrics>,
}

/// How many recent routes and captures to keep. Enough to debug what
/// just happened; small enough that neither can grow into a memory
/// problem on a long-running process.
pub const HISTORY_CAPACITY: usize = 100;

impl AppState {
    /// Assembles the shared state with real clocks and empty history.
    /// A constructor rather than a struct literal at each call site:
    /// the observability fields are the same everywhere, and every
    /// future field would otherwise have to be added to five places.
    pub fn new(
        profiles: HashMap<String, Profile>,
        journal: Journal,
        client: GoogleCalendarClient,
        bootstrap_token_hash: Option<String>,
        tokens: TokenStore,
    ) -> Self {
        Self {
            profiles,
            journal,
            client,
            tokens,
            locks: KeyLocks::new(),
            now: Box::new(|| chrono::Utc::now().to_rfc3339()),
            now_unix: Box::new(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
            }),
            bootstrap_token_hash,
            capture_token_hash: None,
            routes: Mutex::new(RingBuffer::new(HISTORY_CAPACITY)),
            captures: Mutex::new(RingBuffer::new(HISTORY_CAPACITY)),
            metrics: Arc::new(Metrics::default()),
        }
    }

    /// Shares one set of counters with the token manager, which is
    /// constructed earlier in startup. Without this the two would each
    /// count into their own instance and `almanac_token_refreshes_total`
    /// would sit at zero forever.
    pub fn with_metrics(mut self, metrics: Arc<Metrics>) -> Self {
        self.metrics = metrics;
        self
    }

    /// The capture buffer with everything past its TTL already
    /// dropped.
    ///
    /// Every reader must go through this rather than locking
    /// `captures` directly. Expiry used to be repeated at each call
    /// site, and the self-updater's "are captures still retained?"
    /// check was written without it — so a single capture that nobody
    /// ever looked at again suppressed every update for the life of
    /// the process, because the TTL only ever ran while someone was
    /// reading the page.
    pub async fn captures_after_expiry(
        &self,
    ) -> tokio::sync::MutexGuard<'_, RingBuffer<CaptureRecord>> {
        let mut captures = self.captures.lock().await;
        crate::core::observability::expire_captures(
            &mut captures,
            (self.now_unix)(),
            crate::shell::admin::CAPTURE_TTL_SECS,
        );
        captures
    }

    /// Sets the capture-only token (S2). A builder method rather than
    /// another constructor argument: only `main` supplies it, and
    /// reading the environment from inside the constructor would give
    /// every test an invisible dependency on ambient state.
    pub fn with_capture_token(mut self, hash: Option<String>) -> Self {
        self.capture_token_hash = hash;
        self
    }

    /// Same, but with both clocks pinned — for tests that assert on
    /// timestamps or drive expiry without waiting.
    #[cfg(test)]
    pub fn new_for_test(
        profiles: HashMap<String, Profile>,
        journal: Journal,
        client: GoogleCalendarClient,
        bootstrap_token_hash: Option<String>,
        tokens: TokenStore,
    ) -> Self {
        Self {
            now: Box::new(|| "2026-08-28T09:00:00+00:00".to_string()),
            now_unix: Box::new(|| 1_787_000_000),
            ..Self::new(profiles, journal, client, bootstrap_token_hash, tokens)
        }
    }
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
async fn authenticate<'a>(
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
        Some(token) if state.tokens.verify(source_id, token).await => Ok(profile),
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
    let profile = match authenticate(&state, &source_id, &headers).await {
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

    state.metrics.accepted();
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
    let profile = match authenticate(&state, &source_id, &headers).await {
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

    state.metrics.accepted();

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
async fn delete_event(
    State(state): State<Arc<AppState>>,
    Path((source_id, external_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Reply {
    let profile = match authenticate(&state, &source_id, &headers).await {
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

    let found = match state
        .client
        .find_event_by_property(&calendar, crate::shell::delivery::UPSERT_PROPERTY, &key)
        .await
    {
        Ok(found) => found,
        Err(e) => {
            tracing::warn!(source_id = %profile.source_id, error = %e, "delete lookup failed");
            return error(StatusCode::BAD_GATEWAY, &e.to_string(), e.remedy());
        }
    };

    let Some(event) = found else {
        // Deliberately distinct from success: a caller retrying a
        // delete needs to be able to tell "already gone" from "just
        // removed", and silently answering 200 would hide a wrong
        // external id forever.
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "status": "not_found",
                "message": format!("no event on {calendar} carries {key}"),
                "remedy": "check the external id; it must be the one this source used when the \
                           event was created"
            })),
        );
    };

    let Some(event_id) = event.id.clone() else {
        return error(
            StatusCode::BAD_GATEWAY,
            "the Calendar API returned a matching event with no id",
            "this is unexpected; nothing was deleted",
        );
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

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/ingest/{source_id}", axum::routing::post(ingest))
        .route(
            "/v1/ingest/{source_id}/sync",
            axum::routing::post(ingest_sync),
        )
        .route(
            "/v1/ingest/{source_id}/events/{external_id}",
            axum::routing::delete(delete_event),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::token_store::TokenStore;

    fn profile(source_id: &str) -> Profile {
        let toml = format!(
            r#"
schema_version = 1
source_id = "{source_id}"
target_calendar_id = "primary"

[mapping]
title_field = "title"
start_field = "start"
duration_minutes = 60
"#
        );
        Profile::parse(&toml, "test.toml").unwrap()
    }

    fn scratch_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "almanac-ingest-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// State holding one source whose token is already issued in the
    /// encrypted store — the only place ingest auth consults since the
    /// AR17 amendment.
    async fn state_with(source_id: &str, token: &str) -> AppState {
        let dir = scratch_dir();
        let store = TokenStore::with_key(dir.join("tokens.json"), [5u8; 32]);
        store.issue(source_id, token, "now").await.unwrap();

        let mut profiles = HashMap::new();
        profiles.insert(source_id.to_string(), profile(source_id));

        AppState::new_for_test(
            profiles,
            Journal::new(
                dir.join("journal.jsonl"),
                crate::shell::journal::DEFAULT_MAX_BYTES,
            ),
            GoogleCalendarClient::new(
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
            None,
            store,
        )
    }

    fn headers_with_token(token: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        headers
    }

    #[tokio::test]
    async fn the_right_token_authenticates() {
        let state = state_with("home-assistant", "correct-token").await;
        let result = authenticate(
            &state,
            "home-assistant",
            &headers_with_token("correct-token"),
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn a_wrong_token_is_rejected_with_401() {
        let state = state_with("home-assistant", "correct-token").await;
        let err = authenticate(&state, "home-assistant", &headers_with_token("wrong-token"))
            .await
            .unwrap_err();
        assert_eq!(err.0, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn a_missing_authorization_header_is_rejected() {
        let state = state_with("home-assistant", "correct-token").await;
        let err = authenticate(&state, "home-assistant", &HeaderMap::new())
            .await
            .unwrap_err();
        assert_eq!(err.0, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn one_sources_token_does_not_open_another_source() {
        // K6's actual promise: revoking or leaking one source's token
        // must not affect the others.
        let state = state_with("home-assistant", "ha-token").await;
        state
            .tokens
            .issue("uptime-kuma", "kuma-token", "now")
            .await
            .unwrap();

        let mut profiles = state.profiles.clone();
        profiles.insert("uptime-kuma".to_string(), profile("uptime-kuma"));
        let state = AppState { profiles, ..state };

        assert!(
            authenticate(&state, "uptime-kuma", &headers_with_token("kuma-token"))
                .await
                .is_ok()
        );
        assert_eq!(
            authenticate(&state, "uptime-kuma", &headers_with_token("ha-token"))
                .await
                .unwrap_err()
                .0,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn an_unknown_source_is_indistinguishable_from_a_bad_token() {
        // Answering 404 for an unknown source would let an
        // unauthenticated caller enumerate which sources exist.
        let state = state_with("home-assistant", "correct-token").await;
        let unknown = authenticate(
            &state,
            "does-not-exist",
            &headers_with_token("correct-token"),
        )
        .await
        .unwrap_err();
        let bad_token = authenticate(&state, "home-assistant", &headers_with_token("wrong"))
            .await
            .unwrap_err();
        assert_eq!(unknown.0, bad_token.0);
        assert_eq!(format!("{:?}", unknown.1.0), format!("{:?}", bad_token.1.0));
    }

    #[tokio::test]
    async fn a_revoked_token_stops_working_immediately() {
        // M12's sharpest promise: revocation takes effect on the next
        // request, not on the next restart.
        let state = state_with("home-assistant", "correct-token").await;
        assert!(
            authenticate(
                &state,
                "home-assistant",
                &headers_with_token("correct-token")
            )
            .await
            .is_ok()
        );

        state.tokens.revoke("home-assistant").await.unwrap();

        assert_eq!(
            authenticate(
                &state,
                "home-assistant",
                &headers_with_token("correct-token")
            )
            .await
            .unwrap_err()
            .0,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn a_source_with_no_issued_token_cannot_post_at_all() {
        // A profile existing is not permission; the token store is the
        // only authority since the AR17 amendment.
        let state = state_with("home-assistant", "correct-token").await;
        let mut profiles = state.profiles.clone();
        profiles.insert("uptime-kuma".to_string(), profile("uptime-kuma"));
        let state = AppState { profiles, ..state };

        assert_eq!(
            authenticate(&state, "uptime-kuma", &headers_with_token("anything"))
                .await
                .unwrap_err()
                .0,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn an_idempotency_key_header_lands_on_the_entry() {
        let state = state_with("home-assistant", "t").await;
        let mut headers = headers_with_token("t");
        headers.insert(IDEMPOTENCY_HEADER, "abc123".parse().unwrap());
        let entry = build_entry(&state, "home-assistant", &headers, json!({}));
        assert_eq!(entry.idempotency_key.as_deref(), Some("abc123"));
    }

    #[tokio::test]
    async fn an_absent_or_blank_idempotency_key_is_none_not_an_empty_string() {
        let state = state_with("home-assistant", "t").await;
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

    #[tokio::test]
    async fn each_accepted_payload_gets_its_own_entry_id() {
        let state = state_with("home-assistant", "t").await;
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

    #[tokio::test]
    async fn a_capture_that_aged_out_no_longer_counts_as_retained() {
        // AR25 suppresses self-update while captures are retained. The
        // suppression check used to read the buffer without expiring
        // it, and expiry only ran while somebody had a capture page
        // open — so one capture that Kenny looked at once and forgot
        // stopped every update for the life of the process. Months of
        // releases, including security fixes, silently never installed.
        let state = state_with("home-assistant", "tok").await;

        // The pinned clock is 1_787_000_000 and the TTL is an hour, so
        // this record is two hours old.
        state.captures.lock().await.push(CaptureRecord {
            at: "2026-08-28T07:00:00+00:00".to_string(),
            at_unix: 1_787_000_000 - 7_200,
            label: "unknown-webhook".to_string(),
            method: "POST".to_string(),
            headers: Vec::new(),
            body: "{}".to_string(),
            truncated_from_bytes: None,
        });

        assert!(
            state.captures_after_expiry().await.is_empty(),
            "an expired capture must not keep suppressing self-update"
        );
    }

    #[tokio::test]
    async fn a_fresh_capture_does_still_count_as_retained() {
        // The other half: expiry must not throw away a capture Kenny
        // is actually looking at, or a restart would discard exactly
        // the requests he is reverse-engineering.
        let state = state_with("home-assistant", "tok").await;

        state.captures.lock().await.push(CaptureRecord {
            at: "2026-08-28T08:55:00+00:00".to_string(),
            at_unix: 1_787_000_000 - 300,
            label: "unknown-webhook".to_string(),
            method: "POST".to_string(),
            headers: Vec::new(),
            body: "{}".to_string(),
            truncated_from_bytes: None,
        });

        assert_eq!(state.captures_after_expiry().await.len(), 1);
    }

    /// State whose calendar client points at a stub, so the
    /// synchronous path can actually deliver.
    async fn state_with_calendar(
        source_id: &str,
        token: &str,
        calendar: &crate::shell::testing::CalendarStub,
    ) -> AppState {
        let dir = scratch_dir();
        let store = TokenStore::with_key(dir.join("tokens.json"), [5u8; 32]);
        store.issue(source_id, token, "now").await.unwrap();

        let mut profiles = HashMap::new();
        profiles.insert(source_id.to_string(), profile(source_id));

        let tokens = crate::shell::testing::TokenStub::start(3600).await;
        AppState::new_for_test(
            profiles,
            Journal::new(
                dir.join("journal.jsonl"),
                crate::shell::journal::DEFAULT_MAX_BYTES,
            ),
            GoogleCalendarClient::with_base_url(
                reqwest::Client::new(),
                crate::shell::auth::TokenManager::new(
                    reqwest::Client::new(),
                    crate::shell::testing::stub_credentials(&tokens.url),
                ),
                &calendar.base_url,
            ),
            None,
            store,
        )
    }

    fn bearer(token: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        headers
    }

    fn sync_payload() -> Value {
        serde_json::json!({
            "title": "meeting",
            "start": "2026-08-28T09:00:00+00:00",
            "entity_id": "claude-session-1"
        })
    }

    #[tokio::test]
    async fn the_synchronous_endpoint_delivers_and_returns_the_event_id() {
        // K8: a Claude session posts and wants the Google event id
        // back. Nothing tested this endpoint at all — not the happy
        // path, not the response shape.
        let calendar = crate::shell::testing::CalendarStub::start().await;
        let state = Arc::new(state_with_calendar("home-assistant", "tok", &calendar).await);

        let (status, Json(body)) = ingest_sync(
            State(Arc::clone(&state)),
            Path("home-assistant".to_string()),
            bearer("tok"),
            Json(sync_payload()),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "delivered");
        assert!(
            body["event_id"].as_str().is_some_and(|id| !id.is_empty()),
            "the caller needs a real event id back, got {body}"
        );
        assert_eq!(body["created"], true);

        assert!(
            state.journal.pending().unwrap().is_empty(),
            "a delivered entry must be marked done"
        );
    }

    #[tokio::test]
    async fn the_synchronous_endpoint_rejects_a_wrong_token() {
        // The auth guard on this route was covered by nothing.
        let calendar = crate::shell::testing::CalendarStub::start().await;
        let state = Arc::new(state_with_calendar("home-assistant", "tok", &calendar).await);

        let (status, _) = ingest_sync(
            State(Arc::clone(&state)),
            Path("home-assistant".to_string()),
            bearer("wrong-token"),
            Json(sync_payload()),
        )
        .await;

        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(
            state.journal.pending().unwrap().is_empty(),
            "an unauthenticated request must journal nothing"
        );
        assert_eq!(
            calendar.state.request_count().await,
            0,
            "and must never reach Google"
        );
    }

    #[tokio::test]
    async fn a_failed_synchronous_delivery_reports_502_but_keeps_the_payload() {
        // The promise in the handler's own comment: the caller is told
        // it failed, and the entry stays pending so the worker retries
        // it. Losing the payload here would make the synchronous
        // endpoint strictly worse than the asynchronous one.
        let calendar = crate::shell::testing::CalendarStub::start().await;
        calendar.reject_next(99);
        let state = Arc::new(state_with_calendar("home-assistant", "tok", &calendar).await);

        let (status, _) = ingest_sync(
            State(Arc::clone(&state)),
            Path("home-assistant".to_string()),
            bearer("tok"),
            Json(sync_payload()),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert_eq!(
            state.journal.pending().unwrap().len(),
            1,
            "the payload must survive for the worker to retry"
        );
    }

    #[tokio::test]
    async fn a_source_can_delete_the_event_it_created() {
        // K8's criterion says create, update *and* delete. The verb was
        // never built; Kenny asked for it when the gap was reported.
        let calendar = crate::shell::testing::CalendarStub::start().await;
        calendar
            .seed(
                "primary",
                serde_json::json!({
                    "id": "google-event-1",
                    "summary": "meeting",
                    "start": {"dateTime": "2026-08-29T09:00:00+00:00", "timeZone": "UTC"},
                    "end": {"dateTime": "2026-08-29T10:00:00+00:00", "timeZone": "UTC"},
                    "extendedProperties": {
                        "private": {"almanac_source_id": "home-assistant:task-7"}
                    }
                }),
            )
            .await;
        let state = Arc::new(state_with_calendar("home-assistant", "tok", &calendar).await);

        let (status, Json(body)) = delete_event(
            State(Arc::clone(&state)),
            Path(("home-assistant".to_string(), "task-7".to_string())),
            bearer("tok"),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "deleted");
        assert_eq!(body["event_id"], "google-event-1");
        assert!(
            calendar
                .state
                .requests
                .lock()
                .await
                .iter()
                .any(|(method, _)| method == "DELETE"),
            "it must actually have asked Google to delete it"
        );
    }

    #[tokio::test]
    async fn deleting_something_that_is_not_there_says_so_rather_than_pretending() {
        // A caller retrying a delete needs to tell "already gone" from
        // "just removed", and answering 200 for a wrong external id
        // would hide the mistake forever.
        let calendar = crate::shell::testing::CalendarStub::start().await;
        let state = Arc::new(state_with_calendar("home-assistant", "tok", &calendar).await);

        let (status, Json(body)) = delete_event(
            State(Arc::clone(&state)),
            Path(("home-assistant".to_string(), "never-existed".to_string())),
            bearer("tok"),
        )
        .await;

        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["status"], "not_found");
        assert!(
            body["remedy"].as_str().unwrap().contains("external id"),
            "and point at the likely cause"
        );
    }

    #[tokio::test]
    async fn one_source_cannot_delete_another_sources_event() {
        // The upsert key is prefixed with the source id, so even a
        // correct guess of someone else's external id addresses a key
        // this source can never name.
        let calendar = crate::shell::testing::CalendarStub::start().await;
        calendar
            .seed(
                "primary",
                serde_json::json!({
                    "id": "google-event-1",
                    "summary": "someone else's event",
                    "start": {"dateTime": "2026-08-29T09:00:00+00:00", "timeZone": "UTC"},
                    "end": {"dateTime": "2026-08-29T10:00:00+00:00", "timeZone": "UTC"},
                    "extendedProperties": {
                        "private": {"almanac_source_id": "uptime-kuma:task-7"}
                    }
                }),
            )
            .await;
        let state = Arc::new(state_with_calendar("home-assistant", "tok", &calendar).await);

        let (status, _) = delete_event(
            State(Arc::clone(&state)),
            Path(("home-assistant".to_string(), "task-7".to_string())),
            bearer("tok"),
        )
        .await;

        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "another source's event must be invisible, not deletable"
        );
        assert!(
            !calendar
                .state
                .requests
                .lock()
                .await
                .iter()
                .any(|(method, _)| method == "DELETE"),
            "and nothing may be deleted"
        );
    }

    #[tokio::test]
    async fn deleting_needs_this_sources_own_token() {
        let calendar = crate::shell::testing::CalendarStub::start().await;
        let state = Arc::new(state_with_calendar("home-assistant", "tok", &calendar).await);

        let (status, _) = delete_event(
            State(Arc::clone(&state)),
            Path(("home-assistant".to_string(), "task-7".to_string())),
            bearer("wrong-token"),
        )
        .await;

        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(
            calendar.state.request_count().await,
            0,
            "an unauthenticated delete must never reach Google"
        );
    }

    #[tokio::test]
    async fn a_delete_that_google_refuses_is_reported_rather_than_claimed() {
        let calendar = crate::shell::testing::CalendarStub::start().await;
        calendar.reject_next(99);
        let state = Arc::new(state_with_calendar("home-assistant", "tok", &calendar).await);

        let (status, _) = delete_event(
            State(Arc::clone(&state)),
            Path(("home-assistant".to_string(), "task-7".to_string())),
            bearer("tok"),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_GATEWAY);
    }
}
