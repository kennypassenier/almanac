//! The ingest surface as real HTTP (K6, K7, M7): status codes,
//! per-source authentication, and the guarantee that a 202 is only
//! ever returned after the payload is durably journalled.
//!
//! Drives the router directly rather than binding a port — the
//! request/response path is the real one, but nothing here needs
//! Google, so these run in CI on every push. The parts that genuinely
//! need Google live in tests/power_loss_drill.rs and
//! tests/calendar_e2e.rs.

use std::collections::HashMap;
use std::sync::Arc;

use almanac::core::profile::Profile;
use almanac::core::token::hash_token;
use almanac::shell::auth::TokenManager;
use almanac::shell::calendar_client::GoogleCalendarClient;
use almanac::shell::ingest::AppState;
use almanac::shell::journal::{DEFAULT_MAX_BYTES, Journal};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

const HA_TOKEN: &str = "home-assistant-token";
const KUMA_TOKEN: &str = "uptime-kuma-token";
const ADMIN_TOKEN: &str = "bootstrap-admin-token";

fn store_at(dir: &std::path::Path) -> almanac::shell::token_store::TokenStore {
    almanac::shell::token_store::TokenStore::with_key(dir.join("tokens.json"), [5u8; 32])
}

fn scratch_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "almanac-http-{}-{}-{name}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn profile(source_id: &str) -> Profile {
    let toml = format!(
        r#"
schema_version = 2
source_id = "{source_id}"
target_calendar_id = "primary"

"#
    );
    Profile::parse(&toml, "test.toml").unwrap()
}

async fn state(dir: &std::path::Path) -> Arc<AppState> {
    let journal_path = dir.join("journal.jsonl");
    let mut profiles = HashMap::new();
    profiles.insert("home-assistant".to_string(), profile("home-assistant"));
    profiles.insert("uptime-kuma".to_string(), profile("uptime-kuma"));

    // Tokens live in the encrypted store, not the profile — the single
    // authentication path since the AR17 amendment.
    let store = store_at(dir);
    store
        .issue("home-assistant", HA_TOKEN, "now")
        .await
        .unwrap();
    store.issue("uptime-kuma", KUMA_TOKEN, "now").await.unwrap();

    // Points at an unreachable host: these tests exercise the ingest
    // surface only, and the asynchronous path never calls Google.
    let http = reqwest::Client::new();
    let tokens = TokenManager::new(
        http.clone(),
        almanac::core::auth::ServiceAccountCredentials {
            client_email: "unused".to_string(),
            private_key: "unused".to_string(),
            token_url: "https://example.invalid/token".to_string(),
        },
    );

    Arc::new(AppState::new(
        profiles,
        Journal::new(journal_path, DEFAULT_MAX_BYTES),
        GoogleCalendarClient::new(http, tokens),
        Some(hash_token(ADMIN_TOKEN)),
        store,
    ))
}

fn ha_payload() -> &'static str {
    r#"{"external_id":"switch.wasmachine","title":"Wasmachine klaar","start":"2026-08-28T09:00:00+00:00"}"#
}

fn post(uri: &str, token: Option<&str>, body: &str) -> Request<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    builder.body(Body::from(body.to_string())).unwrap()
}

#[tokio::test]
async fn a_valid_home_assistant_payload_is_accepted_and_journalled() {
    let dir = scratch_dir("accept");
    let state = state(&dir).await;
    let app = almanac::shell::build_router(Arc::clone(&state));

    let response = app
        .oneshot(post(
            "/v1/ingest/home-assistant",
            Some(HA_TOKEN),
            ha_payload(),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // The 202 must mean "durably stored", not "we'll get to it".
    let pending = state.journal.pending().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].source_id, "home-assistant");
    assert_eq!(pending[0].payload["title"], "Wasmachine klaar");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_request_with_no_token_is_rejected_and_journals_nothing() {
    let dir = scratch_dir("no-token");
    let state = state(&dir).await;
    let app = almanac::shell::build_router(Arc::clone(&state));

    let response = app
        .oneshot(post("/v1/ingest/home-assistant", None, ha_payload()))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(
        state.journal.pending().unwrap().is_empty(),
        "an unauthenticated payload must not reach the journal"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_request_with_a_wrong_token_is_rejected() {
    let dir = scratch_dir("wrong-token");
    let state = state(&dir).await;
    let app = almanac::shell::build_router(Arc::clone(&state));

    let response = app
        .oneshot(post(
            "/v1/ingest/home-assistant",
            Some("not-the-right-token"),
            ha_payload(),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(state.journal.pending().unwrap().is_empty());

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn one_sources_token_cannot_post_as_another_source() {
    // K6's actual promise: tokens are per source, so a leaked or
    // revoked one is contained to that source.
    let dir = scratch_dir("cross-source");
    let state = state(&dir).await;
    let app = almanac::shell::build_router(Arc::clone(&state));

    let response = app
        .oneshot(post("/v1/ingest/uptime-kuma", Some(HA_TOKEN), ha_payload()))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(state.journal.pending().unwrap().is_empty());

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn an_unknown_source_answers_the_same_401_as_a_bad_token() {
    // Answering 404 here would let an unauthenticated caller
    // enumerate which sources exist.
    let dir = scratch_dir("unknown-source");
    let state = state(&dir).await;

    let unknown = almanac::shell::build_router(Arc::clone(&state))
        .oneshot(post(
            "/v1/ingest/no-such-source",
            Some(HA_TOKEN),
            ha_payload(),
        ))
        .await
        .unwrap();
    let bad_token = almanac::shell::build_router(Arc::clone(&state))
        .oneshot(post(
            "/v1/ingest/home-assistant",
            Some("wrong"),
            ha_payload(),
        ))
        .await
        .unwrap();

    assert_eq!(unknown.status(), bad_token.status());
    assert_eq!(unknown.status(), StatusCode::UNAUTHORIZED);

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn an_idempotency_key_header_is_recorded_on_the_journal_entry() {
    // M7: the header is what lets a source without a natural external
    // id have its retries converge instead of duplicating.
    let dir = scratch_dir("idempotency");
    let state = state(&dir).await;
    let app = almanac::shell::build_router(Arc::clone(&state));

    let request = Request::builder()
        .method("POST")
        .uri("/v1/ingest/home-assistant")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {HA_TOKEN}"))
        .header("idempotency-key", "run-42")
        .body(Body::from(ha_payload().to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    let pending = state.journal.pending().unwrap();
    assert_eq!(pending[0].idempotency_key.as_deref(), Some("run-42"));

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn two_accepted_payloads_both_survive_in_the_journal() {
    let dir = scratch_dir("two");
    let state = state(&dir).await;

    for _ in 0..2 {
        let response = almanac::shell::build_router(Arc::clone(&state))
            .oneshot(post(
                "/v1/ingest/home-assistant",
                Some(HA_TOKEN),
                ha_payload(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);
    }

    assert_eq!(state.journal.pending().unwrap().len(), 2);

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_journal_that_cannot_be_written_answers_500_so_the_sender_retries() {
    // AR16's one silent-data-loss path. The rule is: if the write
    // fails, say so, because Home Assistant's retry script only tries
    // again on a failure. A regression to "log it and answer 202
    // anyway" means a full disk quietly eats events while every sender
    // believes it succeeded — and nothing on the dashboard would show
    // it either.
    let dir = scratch_dir("journal-readonly");
    let readonly = dir.join("readonly");
    std::fs::create_dir_all(&readonly).unwrap();

    // A state whose journal points inside a directory nothing may
    // write to, but whose token store is perfectly normal — otherwise
    // the request would be rejected before it ever reached the write.
    let store = store_at(&dir);
    store
        .issue("home-assistant", HA_TOKEN, "now")
        .await
        .unwrap();

    let mut profiles = HashMap::new();
    profiles.insert("home-assistant".to_string(), profile("home-assistant"));

    let http = reqwest::Client::new();
    let state = Arc::new(AppState::new(
        profiles,
        Journal::new(readonly.join("journal.jsonl"), DEFAULT_MAX_BYTES),
        GoogleCalendarClient::new(
            http.clone(),
            TokenManager::new(
                http,
                almanac::core::auth::ServiceAccountCredentials {
                    client_email: "unused".to_string(),
                    private_key: "unused".to_string(),
                    token_url: "https://example.invalid/token".to_string(),
                },
            ),
        ),
        Some(hash_token(ADMIN_TOKEN)),
        store,
    ));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&readonly, std::fs::Permissions::from_mode(0o500)).unwrap();
    }

    let response = almanac::shell::build_router(Arc::clone(&state))
        .oneshot(post(
            "/v1/ingest/home-assistant",
            Some(HA_TOKEN),
            ha_payload(),
        ))
        .await
        .unwrap();

    // Restore permissions before asserting, so a failure still cleans up.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&readonly, std::fs::Permissions::from_mode(0o700)).ok();
    }
    let status = response.status();
    std::fs::remove_dir_all(&dir).ok();

    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "an unwritable journal must be reported, never answered with 202"
    );
}

#[tokio::test]
async fn a_payload_using_every_option_is_accepted_at_the_http_layer() {
    // K9's criterion said "an E2E test per alert system", and until
    // 2.0.0 this ran the grafana and uptime-kuma fixtures through auth,
    // the router and the journal — the only place those payloads ever
    // met anything but the mapping engine.
    //
    // Those fixtures went with the translation layer they existed to
    // prove. What still matters is the same claim about the shape that
    // replaced them: a call using every per-event option must survive
    // the whole HTTP path, not just the mapper. A content type or a
    // field the ingest layer refuses would otherwise surface only when
    // a real event failed to appear.
    let dir = scratch_dir("every-option");
    let state = state(&dir).await;

    let payload = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/payloads/everything_sample.json"
    ))
    .unwrap();

    let response = almanac::shell::build_router(Arc::clone(&state))
        .oneshot(post("/v1/ingest/uptime-kuma", Some(KUMA_TOKEN), &payload))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::ACCEPTED,
        "a payload using every option must be accepted as it is actually sent"
    );

    assert_eq!(
        state.journal.pending().unwrap().len(),
        1,
        "and it must be durably journalled"
    );

    std::fs::remove_dir_all(&dir).ok();
}
