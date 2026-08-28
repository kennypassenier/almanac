//! The management dashboard as real HTTP (M12): login, every page
//! rendered with seeded state, token issue/revoke/reveal, and the
//! plaintext-scan that proves no token reaches a page except behind
//! the deliberate reveal control.

use std::collections::HashMap;
use std::sync::Arc;

use almanac::core::profile::Profile;
use almanac::core::token::hash_token;
use almanac::shell::auth::TokenManager;
use almanac::shell::calendar_client::GoogleCalendarClient;
use almanac::shell::ingest::AppState;
use almanac::shell::journal::{DEFAULT_MAX_BYTES, Journal};
use almanac::shell::token_store::TokenStore;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use tower::ServiceExt;

const BOOTSTRAP: &str = "bootstrap-token-value";

fn scratch_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "almanac-dash-{}-{}-{name}",
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

fn state(dir: &std::path::Path) -> Arc<AppState> {
    let mut profiles = HashMap::new();
    profiles.insert("home-assistant".to_string(), profile("home-assistant"));

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
        Journal::new(dir.join("journal.jsonl"), DEFAULT_MAX_BYTES),
        GoogleCalendarClient::new(http, tokens),
        Some(hash_token(BOOTSTRAP)),
        TokenStore::with_key(dir.join("tokens.json"), [5u8; 32]),
    ))
}

fn get(uri: &str, cookie: Option<&str>) -> Request<Body> {
    let mut b = Request::builder().method("GET").uri(uri);
    if let Some(c) = cookie {
        b = b.header(header::COOKIE, c);
    }
    b.body(Body::empty()).unwrap()
}

fn post_form(uri: &str, cookie: Option<&str>, body: &str) -> Request<Body> {
    let mut b = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(c) = cookie {
        b = b.header(header::COOKIE, c);
    }
    b.body(Body::from(body.to_string())).unwrap()
}

async fn text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).to_string()
}

/// Logs in and returns the session cookie to reuse.
async fn login(st: &Arc<AppState>) -> String {
    let response = almanac::shell::build_router(Arc::clone(st))
        .oneshot(post_form(
            "/login",
            None,
            &format!("token={}", urlencode(BOOTSTRAP)),
        ))
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::SEE_OTHER,
        "login should redirect"
    );
    let cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .expect("a session cookie")
        .to_str()
        .unwrap();
    cookie.split(';').next().unwrap().to_string()
}

fn urlencode(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            other => format!("%{:02X}", other as u8),
        })
        .collect()
}

#[tokio::test]
async fn the_login_page_renders_without_a_session() {
    let dir = scratch_dir("loginpage");
    let response = almanac::shell::build_router(state(&dir))
        .oneshot(get("/login", None))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = text(response).await;
    assert!(body.contains("Bootstrap token"));
    assert!(
        body.contains("/static/bootstrap.min.css"),
        "the CSS is served locally, not from a CDN"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_wrong_bootstrap_token_does_not_start_a_session() {
    let dir = scratch_dir("badlogin");
    let response = almanac::shell::build_router(state(&dir))
        .oneshot(post_form("/login", None, "token=wrong"))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK, "stays on the login page");
    assert!(response.headers().get(header::SET_COOKIE).is_none());
    assert!(text(response).await.contains("not correct"));

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn the_dashboard_redirects_to_login_without_a_session() {
    let dir = scratch_dir("redirect");
    for path in ["/dashboard", "/dashboard/sources", "/dashboard/captures"] {
        let response = almanac::shell::build_router(state(&dir))
            .oneshot(get(path, None))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::SEE_OTHER,
            "{path} must not render to a stranger"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn every_page_renders_with_seeded_state() {
    let dir = scratch_dir("pages");
    let st = state(&dir);
    st.tokens
        .issue("home-assistant", "issued-token-value", "2026-08-28")
        .await
        .unwrap();
    let cookie = login(&st).await;

    // `lists_profile` is false for the captures page on purpose: it
    // shows what arrived from outside, which has nothing to do with
    // which profiles are configured.
    for (path, marker, lists_profile) in [
        ("/dashboard", "Loaded profiles", true),
        ("/dashboard/sources", "Token issued", true),
        ("/dashboard/captures", "Captured requests", false),
    ] {
        let response = almanac::shell::build_router(Arc::clone(&st))
            .oneshot(get(path, Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        let body = text(response).await;
        assert!(body.contains(marker), "{path} should contain {marker:?}");
        if lists_profile {
            assert!(
                body.contains("home-assistant"),
                "{path} should list the profile"
            );
        }
    }

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn no_page_ever_contains_a_token_in_the_clear() {
    // M12's mandatory plaintext scan. The token is fetched by the
    // reveal control on demand; it must never sit in page source.
    let dir = scratch_dir("plaintext");
    let st = state(&dir);
    st.tokens
        .issue("home-assistant", "PLAINTEXT-MARKER-7c1", "2026-08-28")
        .await
        .unwrap();
    let cookie = login(&st).await;

    for path in [
        "/dashboard",
        "/dashboard/sources",
        "/dashboard/captures",
        "/login",
    ] {
        let response = almanac::shell::build_router(Arc::clone(&st))
            .oneshot(get(path, Some(&cookie)))
            .await
            .unwrap();
        let body = text(response).await;
        assert!(
            !body.contains("PLAINTEXT-MARKER-7c1"),
            "{path} leaked the token into its HTML"
        );
        assert!(
            !body.contains(BOOTSTRAP),
            "{path} leaked the bootstrap token into its HTML"
        );
    }

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn issuing_a_token_from_the_dashboard_produces_a_working_one() {
    // The dashboard's whole reason for existing: a token minted here
    // must actually authenticate an ingest request.
    let dir = scratch_dir("issue");
    let st = state(&dir);
    let cookie = login(&st).await;

    let response = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form(
            "/dashboard/sources/home-assistant/issue",
            Some(&cookie),
            "",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    let token = st
        .tokens
        .reveal("home-assistant")
        .await
        .unwrap()
        .expect("a token was issued");

    let ingest = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/ingest/home-assistant")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::from(
                    r#"{"title":"t","start":"2026-08-28T09:00:00+00:00"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        ingest.status(),
        StatusCode::ACCEPTED,
        "a dashboard-issued token must work against the ingest endpoint"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_revoked_token_stops_working_on_the_very_next_request() {
    let dir = scratch_dir("revoke");
    let st = state(&dir);
    st.tokens
        .issue("home-assistant", "tok", "now")
        .await
        .unwrap();
    let cookie = login(&st).await;

    let ok = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/ingest/home-assistant")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, "Bearer tok")
                .body(Body::from(
                    r#"{"title":"t","start":"2026-08-28T09:00:00+00:00"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::ACCEPTED);

    almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form(
            "/dashboard/sources/home-assistant/revoke",
            Some(&cookie),
            "",
        ))
        .await
        .unwrap();

    let after = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/ingest/home-assistant")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, "Bearer tok")
                .body(Body::from(
                    r#"{"title":"t","start":"2026-08-28T09:00:00+00:00"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        after.status(),
        StatusCode::UNAUTHORIZED,
        "revocation must take effect immediately, not on the next restart"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn the_reveal_endpoint_needs_a_session() {
    let dir = scratch_dir("reveal-auth");
    let st = state(&dir);
    st.tokens
        .issue("home-assistant", "tok", "now")
        .await
        .unwrap();

    let response = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(get("/dashboard/sources/home-assistant/token", None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn the_reveal_endpoint_returns_the_real_token_to_a_logged_in_operator() {
    let dir = scratch_dir("reveal");
    let st = state(&dir);
    st.tokens
        .issue("home-assistant", "tok-abc", "now")
        .await
        .unwrap();
    let cookie = login(&st).await;

    let response = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(get(
            "/dashboard/sources/home-assistant/token",
            Some(&cookie),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(text(response).await.contains("tok-abc"));

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn logging_out_ends_the_session() {
    let dir = scratch_dir("logout");
    let st = state(&dir);
    let cookie = login(&st).await;

    almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form("/logout", Some(&cookie), ""))
        .await
        .unwrap();

    let after = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(get("/dashboard", Some(&cookie)))
        .await
        .unwrap();
    assert_eq!(
        after.status(),
        StatusCode::SEE_OTHER,
        "the old cookie must stop working"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_captured_script_tag_renders_inert() {
    // A capture comes from whatever system was pointed at the endpoint.
    // Rendering it unescaped would make the debugging tool a way to run
    // script in the operator's own browser.
    let dir = scratch_dir("xss");
    let st = state(&dir);
    let cookie = login(&st).await;

    almanac::shell::build_router(Arc::clone(&st))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/debug/capture/evil")
                .header(header::AUTHORIZATION, format!("Bearer {BOOTSTRAP}"))
                .body(Body::from("<script>alert(1)</script>"))
                .unwrap(),
        )
        .await
        .unwrap();

    let response = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(get("/dashboard/captures", Some(&cookie)))
        .await
        .unwrap();
    let body = text(response).await;

    assert!(body.contains("&lt;script&gt;"), "it must be shown, escaped");
    assert!(
        !body.contains("<script>alert(1)</script>"),
        "and must not be live markup"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn the_vendored_stylesheet_is_served_locally() {
    let dir = scratch_dir("css");
    let response = almanac::shell::build_router(state(&dir))
        .oneshot(get("/static/bootstrap.min.css", None))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = text(response).await;
    assert!(
        body.contains("Bootstrap"),
        "the real stylesheet, not a stub"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn health_still_answers_without_a_session() {
    // The dashboard's arrival must not have closed the door monitoring
    // comes through.
    let dir = scratch_dir("health");
    let response = almanac::shell::build_router(state(&dir))
        .oneshot(get("/healthz", None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    std::fs::remove_dir_all(&dir).ok();
}
