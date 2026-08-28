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

fn profile(source_id: &str, token: &str) -> Profile {
    let toml = format!(
        r#"
schema_version = 1
source_id = "{source_id}"
target_calendar_id = "primary"
token_hash = "{}"

[mapping]
title_field = "title"
external_id_field = "entity_id"
start_field = "start"
duration_minutes = 60
"#,
        hash_token(token)
    );
    Profile::parse(&toml, "test.toml").unwrap()
}

fn state(journal_path: std::path::PathBuf) -> Arc<AppState> {
    let mut profiles = HashMap::new();
    profiles.insert(
        "home-assistant".to_string(),
        profile("home-assistant", HA_TOKEN),
    );
    profiles.insert(
        "uptime-kuma".to_string(),
        profile("uptime-kuma", KUMA_TOKEN),
    );

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
    ))
}

fn ha_payload() -> &'static str {
    r#"{"entity_id":"switch.wasmachine","title":"Wasmachine klaar","start":"2026-08-28T09:00:00+00:00"}"#
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
    let state = state(dir.join("journal.jsonl"));
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
    let state = state(dir.join("journal.jsonl"));
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
    let state = state(dir.join("journal.jsonl"));
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
    let state = state(dir.join("journal.jsonl"));
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
    let state = state(dir.join("journal.jsonl"));

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
    let state = state(dir.join("journal.jsonl"));
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
    let state = state(dir.join("journal.jsonl"));

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
