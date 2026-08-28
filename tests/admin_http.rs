//! The operator surface as real HTTP: health (M1), debug status
//! (K11), raw capture (M11) and dry-run (M9). None of these need
//! Google, so they run in CI on every push.

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
use http_body_util::BodyExt;
use tower::ServiceExt;

const ADMIN_TOKEN: &str = "bootstrap-admin-token";

fn store_at(dir: &std::path::Path) -> almanac::shell::token_store::TokenStore {
    almanac::shell::token_store::TokenStore::with_key(dir.join("tokens.json"), [5u8; 32])
}

fn scratch_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "almanac-admin-{}-{}-{name}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn state(dir: &std::path::Path, admin: Option<&str>) -> Arc<AppState> {
    let journal_path = dir.join("journal.jsonl");
    let toml = r#"
schema_version = 1
source_id = "home-assistant"
target_calendar_id = "primary"

[mapping]
title_field = "title"
external_id_field = "entity_id"
start_field = "start"
duration_minutes = 60
"#;
    let mut profiles = HashMap::new();
    profiles.insert(
        "home-assistant".to_string(),
        Profile::parse(toml, "test.toml").unwrap(),
    );

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
        admin.map(hash_token),
        store_at(dir),
    ))
}

/// State with a capture-only token as well (S2).
fn state_with_capture_token(
    dir: &std::path::Path,
    admin: Option<&str>,
    capture: &str,
) -> Arc<AppState> {
    let state = state(dir, admin);
    let state = Arc::try_unwrap(state).ok().expect("sole owner");
    Arc::new(state.with_capture_token(Some(hash_token(capture))))
}

fn get(uri: &str, token: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method("GET").uri(uri);
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    builder.body(Body::empty()).unwrap()
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

async fn body_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn health_answers_without_a_token() {
    // M1: a monitoring stack that fails closed lies to you during an
    // outage, so this must never require credentials.
    let dir = scratch_dir("health");
    let app = almanac::shell::build_router(state(&dir, Some(ADMIN_TOKEN)));

    let response = app.oneshot(get("/healthz", None)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_json(response).await["status"], "ok");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn health_still_answers_when_no_admin_token_is_configured() {
    let dir = scratch_dir("health-noadmin");
    let app = almanac::shell::build_router(state(&dir, None));

    let response = app.oneshot(get("/healthz", None)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn the_debug_status_needs_the_admin_token() {
    let dir = scratch_dir("status-auth");
    let st = state(&dir, Some(ADMIN_TOKEN));

    let no_token = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(get("/v1/debug/status", None))
        .await
        .unwrap();
    assert_eq!(no_token.status(), StatusCode::UNAUTHORIZED);

    let wrong = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(get("/v1/debug/status", Some("nope")))
        .await
        .unwrap();
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn the_debug_status_reports_profiles_and_the_journal() {
    let dir = scratch_dir("status");
    let app = almanac::shell::build_router(state(&dir, Some(ADMIN_TOKEN)));

    let response = app
        .oneshot(get("/v1/debug/status", Some(ADMIN_TOKEN)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = body_json(response).await;
    assert_eq!(body["profiles"][0]["source_id"], "home-assistant");
    assert_eq!(body["profiles"][0]["target_calendar_id"], "primary");
    assert_eq!(body["journal"]["count"], 0);

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn an_unconfigured_admin_surface_refuses_rather_than_opening_up() {
    // Fail-closed: a forgotten ALMANAC_BOOTSTRAP_TOKEN must not leave
    // the debug views readable by anyone on the LAN.
    let dir = scratch_dir("noadmin");
    let app = almanac::shell::build_router(state(&dir, None));

    let response = app
        .oneshot(get("/v1/debug/status", Some("anything")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = body_json(response).await;
    assert!(
        body["remedy"]
            .as_str()
            .unwrap()
            .contains("ALMANAC_BOOTSTRAP_TOKEN"),
        "the error must name the variable that fixes it"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_captured_request_reads_back_verbatim() {
    // M11's whole purpose: learn an undocumented webhook's real shape.
    // If the body came back altered, the profile written from it would
    // be wrong.
    let dir = scratch_dir("capture");
    let st = state(&dir, Some(ADMIN_TOKEN));
    let payload = r#"{"weird":{"nested":[1,2,3]},"unicode":"héllo"}"#;

    let posted = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post(
            "/v1/debug/capture/unknown-app",
            Some(ADMIN_TOKEN),
            payload,
        ))
        .await
        .unwrap();
    assert_eq!(posted.status(), StatusCode::OK);

    let listed = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(get("/v1/debug/capture", Some(ADMIN_TOKEN)))
        .await
        .unwrap();
    let body = body_json(listed).await;

    assert_eq!(body["captures"][0]["label"], "unknown-app");
    assert_eq!(
        body["captures"][0]["body"].as_str().unwrap(),
        payload,
        "the body must come back byte-identical"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_captured_authorization_header_is_redacted() {
    // Capturing an unknown webhook means capturing whatever it sends,
    // including its own credentials. Those must not become readable
    // afterwards just because someone pointed it here.
    let dir = scratch_dir("capture-redact");
    let st = state(&dir, Some(ADMIN_TOKEN));

    let posted = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post("/v1/debug/capture/x", Some(ADMIN_TOKEN), "{}"))
        .await
        .unwrap();
    assert_eq!(posted.status(), StatusCode::OK);

    let listed = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(get("/v1/debug/capture", Some(ADMIN_TOKEN)))
        .await
        .unwrap();
    let raw = serde_json::to_string(&body_json(listed).await).unwrap();

    assert!(
        !raw.contains(ADMIN_TOKEN),
        "no captured header may echo a bearer token back:\n{raw}"
    );
    assert!(
        raw.contains("<redacted>"),
        "and it must say it redacted one"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn capturing_needs_the_admin_token_too() {
    let dir = scratch_dir("capture-auth");
    let app = almanac::shell::build_router(state(&dir, Some(ADMIN_TOKEN)));

    let response = app
        .oneshot(post("/v1/debug/capture/x", None, "{}"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn dry_run_shows_the_event_without_writing_it() {
    // M9: check a profile against a real payload before letting it
    // near a calendar.
    let dir = scratch_dir("dryrun");
    let app = almanac::shell::build_router(state(&dir, Some(ADMIN_TOKEN)));

    let response = app
        .oneshot(post(
            "/v1/debug/dry-run/home-assistant",
            Some(ADMIN_TOKEN),
            r#"{"entity_id":"switch.wasmachine","title":"Wasmachine klaar","start":"2026-08-28T09:00:00+00:00"}"#,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["would_write_to_calendar"], "primary");
    assert_eq!(body["event"]["summary"], "Wasmachine klaar");
    assert_eq!(
        body["event"]["end"]["dateTime"],
        "2026-08-28T10:00:00+00:00"
    );
    assert_eq!(
        body["event"]["extendedProperties"]["private"]["almanac_source_id"],
        "home-assistant:switch.wasmachine"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn dry_run_explains_a_payload_the_profile_cannot_map() {
    let dir = scratch_dir("dryrun-bad");
    let app = almanac::shell::build_router(state(&dir, Some(ADMIN_TOKEN)));

    let response = app
        .oneshot(post(
            "/v1/debug/dry-run/home-assistant",
            Some(ADMIN_TOKEN),
            r#"{"title":"no start field here"}"#,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = body_json(response).await;
    assert!(body["message"].as_str().unwrap().contains("start"));
    assert!(!body["remedy"].as_str().unwrap().is_empty());

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn dry_run_on_an_unknown_source_says_where_to_look() {
    let dir = scratch_dir("dryrun-unknown");
    let app = almanac::shell::build_router(state(&dir, Some(ADMIN_TOKEN)));

    let response = app
        .oneshot(post("/v1/debug/dry-run/nope", Some(ADMIN_TOKEN), "{}"))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(
        body_json(response).await["remedy"]
            .as_str()
            .unwrap()
            .contains("/v1/debug/status")
    );

    std::fs::remove_dir_all(&dir).ok();
}

const CAPTURE_TOKEN: &str = "capture-only-token";

#[tokio::test]
async fn the_capture_token_can_post_a_capture() {
    // S2: the whole point is that a system you are still investigating
    // can be given a credential that does this and nothing else.
    let dir = scratch_dir("capture-token");
    let state = state_with_capture_token(&dir, Some(ADMIN_TOKEN), CAPTURE_TOKEN);

    let response = almanac::shell::build_router(Arc::clone(&state))
        .oneshot(post(
            "/v1/debug/capture/unknown-app",
            Some(CAPTURE_TOKEN),
            r#"{"hello":"world"}"#,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn the_capture_token_cannot_read_captures_back() {
    // If it could, handing it to a third party would expose every
    // other captured payload to that party.
    let dir = scratch_dir("capture-token-read");
    let state = state_with_capture_token(&dir, Some(ADMIN_TOKEN), CAPTURE_TOKEN);

    let response = almanac::shell::build_router(Arc::clone(&state))
        .oneshot(get("/v1/debug/capture", Some(CAPTURE_TOKEN)))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn the_capture_token_opens_nothing_else_on_the_admin_surface() {
    let dir = scratch_dir("capture-token-scope");
    let state = state_with_capture_token(&dir, Some(ADMIN_TOKEN), CAPTURE_TOKEN);

    for uri in ["/v1/debug/status", "/v1/debug/capture"] {
        let response = almanac::shell::build_router(Arc::clone(&state))
            .oneshot(get(uri, Some(CAPTURE_TOKEN)))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "{uri} must not accept the capture token"
        );
    }

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn the_admin_token_still_posts_captures() {
    // The operator's own credential must not stop working just
    // because a narrower one now exists.
    let dir = scratch_dir("capture-admin-still");
    let state = state_with_capture_token(&dir, Some(ADMIN_TOKEN), CAPTURE_TOKEN);

    let response = almanac::shell::build_router(Arc::clone(&state))
        .oneshot(post("/v1/debug/capture/x", Some(ADMIN_TOKEN), "{}"))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_wrong_capture_token_is_still_refused() {
    let dir = scratch_dir("capture-token-wrong");
    let state = state_with_capture_token(&dir, Some(ADMIN_TOKEN), CAPTURE_TOKEN);

    let response = almanac::shell::build_router(Arc::clone(&state))
        .oneshot(post("/v1/debug/capture/x", Some("not-the-token"), "{}"))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn captures_past_the_capacity_drop_the_oldest_through_the_real_endpoints() {
    // T13: the cap was tested as a pure ring buffer, never through the
    // wiring. That is exactly the class of bug that let a forgotten
    // capture disable self-update for months — the function was right,
    // the place it was called from was not.
    let dir = scratch_dir("capture-cap");
    let state = state(&dir, Some(ADMIN_TOKEN));

    for i in 0..105 {
        almanac::shell::build_router(Arc::clone(&state))
            .oneshot(post(
                &format!("/v1/debug/capture/label-{i}"),
                Some(ADMIN_TOKEN),
                &format!(r#"{{"n":{i}}}"#),
            ))
            .await
            .unwrap();
    }

    let response = almanac::shell::build_router(Arc::clone(&state))
        .oneshot(get("/v1/debug/capture", Some(ADMIN_TOKEN)))
        .await
        .unwrap();
    let body = body_json(response).await;
    let captures = body["captures"].as_array().unwrap();

    assert_eq!(
        captures.len(),
        100,
        "the cap must hold through the endpoints"
    );
    assert_eq!(
        captures[0]["label"], "label-104",
        "newest first, and the oldest five are gone"
    );
    assert!(
        !captures.iter().any(|c| c["label"] == "label-0"),
        "the oldest must actually have been dropped"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn every_credential_header_a_webhook_might_send_is_redacted() {
    // Capturing an unknown webhook means capturing whatever it sends,
    // including its own credentials. The only test here asserted the
    // redaction list was lowercase; nothing proved a real credential
    // header actually gets redacted, or that the check is
    // case-insensitive against what a real sender writes.
    let dir = scratch_dir("capture-redact-all");
    let state = state(&dir, Some(ADMIN_TOKEN));

    let request = Request::builder()
        .method("POST")
        .uri("/v1/debug/capture/x")
        .header("authorization", format!("Bearer {ADMIN_TOKEN}"))
        .header("Cookie", "session=super-secret")
        .header("X-Api-Key", "vendor-api-key-value")
        .header("Proxy-Authorization", "Basic abc123")
        .body(Body::from("{}"))
        .unwrap();

    almanac::shell::build_router(Arc::clone(&state))
        .oneshot(request)
        .await
        .unwrap();

    let response = almanac::shell::build_router(Arc::clone(&state))
        .oneshot(get("/v1/debug/capture", Some(ADMIN_TOKEN)))
        .await
        .unwrap();
    let rendered = serde_json::to_string(&body_json(response).await).unwrap();

    for secret in [
        "super-secret",
        "vendor-api-key-value",
        "abc123",
        ADMIN_TOKEN,
    ] {
        assert!(
            !rendered.contains(secret),
            "a credential header was stored verbatim: {secret} in {rendered}"
        );
    }

    std::fs::remove_dir_all(&dir).ok();
}
