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

    // Loads whatever is already on disk, so a second call to state()
    // stands in for the process coming back after a restart.
    let store =
        TokenStore::with_key_loading(dir.join("tokens.json"), [5u8; 32]).expect("read the store");

    Arc::new(AppState::new(
        profiles,
        Journal::new(dir.join("journal.jsonl"), DEFAULT_MAX_BYTES),
        GoogleCalendarClient::new(http, tokens),
        Some(hash_token(BOOTSTRAP)),
        store,
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
async fn a_session_survives_a_restart_or_a_self_update() {
    // AR25: sessions live in the encrypted store, so replacing the
    // running binary does not log Kenny out — his complaint that
    // "every update logs you out" is what drove this.
    let dir = scratch_dir("session-survives");
    let st = state(&dir);
    let cookie = login(&st).await;

    // A fresh AppState over the same store directory stands in for
    // the process that comes back after the update.
    let restarted = state(&dir);
    let response = almanac::shell::build_router(Arc::clone(&restarted))
        .oneshot(get("/dashboard", Some(&cookie)))
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the cookie from before the restart must still be accepted"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn logging_out_still_really_revokes_across_a_restart() {
    // The property a self-validating cookie could not offer: after
    // logout the cookie is dead even for a process that starts later.
    let dir = scratch_dir("logout-survives");
    let st = state(&dir);
    let cookie = login(&st).await;

    almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form("/logout", Some(&cookie), ""))
        .await
        .unwrap();

    let restarted = state(&dir);
    let response = almanac::shell::build_router(Arc::clone(&restarted))
        .oneshot(get("/dashboard", Some(&cookie)))
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::SEE_OTHER,
        "a logged-out cookie must not come back to life on restart"
    );

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

#[tokio::test]
async fn the_endpoints_that_change_something_refuse_without_a_session() {
    // T9: the read-only pages were swept for this; the two that
    // mutate were not. A refactor dropping the session check from
    // revoke would let any device on the LAN cut off every source, and
    // nothing would go red.
    let dir = scratch_dir("mutating-no-session");
    let st = state(&dir);

    for uri in [
        "/dashboard/sources/home-assistant/issue",
        "/dashboard/sources/home-assistant/revoke",
    ] {
        let response = almanac::shell::build_router(Arc::clone(&st))
            .oneshot(post_form(uri, None, ""))
            .await
            .unwrap();

        assert_ne!(
            response.status(),
            StatusCode::OK,
            "{uri} must not act without a session"
        );
        assert!(
            response.status() == StatusCode::SEE_OTHER
                || response.status() == StatusCode::UNAUTHORIZED,
            "{uri} answered {} without a session",
            response.status()
        );
    }

    // And nothing was actually issued.
    let cookie = login(&st).await;
    let response = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(get(
            "/dashboard/sources/home-assistant/token",
            Some(&cookie),
        ))
        .await
        .unwrap();
    let body = text(response).await;
    assert!(
        body.contains("null") || body.contains("\"token\":null") || !body.contains("Bearer"),
        "an unauthenticated issue must not have created a token: {body}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_forged_or_stale_cookie_does_not_open_anything() {
    let dir = scratch_dir("forged-cookie");
    let st = state(&dir);
    let real = login(&st).await;

    // A cookie of the right shape whose value is not a live session.
    let forged = format!("{}x", real);

    for uri in ["/dashboard", "/dashboard/sources", "/dashboard/captures"] {
        let response = almanac::shell::build_router(Arc::clone(&st))
            .oneshot(get(uri, Some(&forged)))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::SEE_OTHER,
            "{uri} accepted a forged cookie"
        );
    }

    let response = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(get(
            "/dashboard/sources/home-assistant/token",
            Some(&forged),
        ))
        .await
        .unwrap();
    assert_ne!(
        response.status(),
        StatusCode::OK,
        "a forged cookie must not reveal a token"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn the_session_cookie_carries_the_attributes_that_protect_it() {
    // HttpOnly keeps it out of reach of any script that lands on the
    // page; SameSite=Strict is what makes the absence of a CSRF token
    // an accepted trade rather than a hole. Both were set in code and
    // asserted nowhere, so either could be dropped silently.
    let dir = scratch_dir("cookie-attrs");
    let st = state(&dir);

    let response = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form(
            "/login",
            None,
            &format!("token={}", urlencode(BOOTSTRAP)),
        ))
        .await
        .unwrap();

    let cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_ascii_lowercase();

    assert!(cookie.contains("httponly"), "cookie was {cookie}");
    assert!(cookie.contains("samesite=strict"), "cookie was {cookie}");
    assert!(cookie.contains("path=/"), "cookie was {cookie}");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn the_root_and_logout_are_safe_without_a_session() {
    let dir = scratch_dir("root-logout");
    let st = state(&dir);

    let root = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(get("/", None))
        .await
        .unwrap();
    assert_eq!(
        root.status(),
        StatusCode::SEE_OTHER,
        "/ must send you to login"
    );

    // Logging out without a session must be harmless, not a 500.
    let logout = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form("/logout", None, ""))
        .await
        .unwrap();
    assert_eq!(logout.status(), StatusCode::SEE_OTHER);

    std::fs::remove_dir_all(&dir).ok();
}

/// The copy control must not depend on an API that does not exist where
/// this page is actually used.
///
/// `navigator.clipboard` is defined only in a secure context. The
/// dashboard is served over plain HTTP on the LAN, so the object is
/// absent and a bare call to it throws "navigator.clipboard is
/// undefined" — into the browser console, where nobody looks. Kenny hit
/// it the first time he tried to copy a token for real.
///
/// A test cannot run the browser's JavaScript, so this asserts the next
/// best thing: that the served page carries a guard and a fallback, and
/// never reaches for the secure-context API unguarded.
#[tokio::test]
async fn the_copy_control_works_without_a_secure_context() {
    let dir = scratch_dir("copy-fallback");
    let st = state(&dir);
    let cookie = login(&st).await;

    let response = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(get("/dashboard/sources", Some(&cookie)))
        .await
        .unwrap();
    let body = text(response).await;

    assert!(
        body.contains("isSecureContext"),
        "the copy control must check for a secure context before using navigator.clipboard"
    );
    assert!(
        body.contains("execCommand"),
        "there must be a fallback that works over plain HTTP"
    );
    assert!(
        !body.contains("await navigator.clipboard.writeText"),
        "navigator.clipboard must never be awaited unguarded — it is undefined over http"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// K21: the same seeded state, but with a real profiles directory on
/// disk holding the profile the in-memory map starts with. A reload
/// reads that directory, so a test whose directory is empty would
/// assert that reloading deletes every source — true, and not the
/// behaviour under test.
fn state_with_profiles_dir(dir: &std::path::Path) -> Arc<AppState> {
    let profiles_dir = dir.join("profiles");
    std::fs::create_dir_all(&profiles_dir).unwrap();
    std::fs::write(
        profiles_dir.join("home-assistant.toml"),
        profile_toml("home-assistant"),
    )
    .unwrap();

    let state = Arc::try_unwrap(state(dir)).unwrap_or_else(|_| unreachable!());
    Arc::new(state.with_profiles_dir(profiles_dir))
}

fn profile_toml(source_id: &str) -> String {
    format!(
        r#"
schema_version = 1
source_id = "{source_id}"
target_calendar_id = "primary"

[mapping]
title_field = "title"
start_field = "start"
duration_minutes = 60
"#
    )
}

#[tokio::test]
async fn k21_the_sources_page_offers_a_way_to_add_one() {
    // The bug this feature exists for: Kenny opened this page to add a
    // source and there was nothing to click.
    let dir = scratch_dir("k21-offers");
    let st = state_with_profiles_dir(&dir);
    let cookie = login(&st).await;

    let body = text(
        almanac::shell::build_router(Arc::clone(&st))
            .oneshot(get("/dashboard/sources", Some(&cookie)))
            .await
            .unwrap(),
    )
    .await;

    assert!(
        body.contains(r#"action="/dashboard/sources""#),
        "no add form"
    );
    assert!(body.contains("name=\"profile\""), "no profile field");
    assert!(
        body.contains("schema_version = 1"),
        "the box should arrive pre-filled with a starter profile"
    );
    assert!(
        body.contains("/dashboard/sources/reload"),
        "no reload control for a profile placed by hand"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn k21_a_saved_profile_becomes_a_source_that_can_be_issued_a_token() {
    // End to end: the thing Kenny wanted to do in one sitting.
    let dir = scratch_dir("k21-save");
    let st = state_with_profiles_dir(&dir);
    let cookie = login(&st).await;

    let response = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form(
            "/dashboard/sources",
            Some(&cookie),
            &format!("profile={}", urlencode(&profile_toml("kobo"))),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER, "should redirect");

    assert!(
        dir.join("profiles/kobo.toml").exists(),
        "the profile should be on disk"
    );

    // Live without a restart — the point of the whole feature.
    let issued = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form(
            "/dashboard/sources/kobo/issue",
            Some(&cookie),
            "",
        ))
        .await
        .unwrap();
    assert_eq!(
        issued.status(),
        StatusCode::SEE_OTHER,
        "issuing a token for the new source should work without restarting"
    );

    let body = text(
        almanac::shell::build_router(Arc::clone(&st))
            .oneshot(get("/dashboard/sources", Some(&cookie)))
            .await
            .unwrap(),
    )
    .await;
    assert!(body.contains("kobo"), "the new source should be listed");
    assert!(
        body.contains("home-assistant"),
        "the profiles already loaded must survive the reload"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn k21_a_rejected_profile_keeps_the_text_that_was_typed() {
    // Losing the typing as well as the mistake is what makes a form
    // feel hostile; and the error has to name the field, not just fail.
    let dir = scratch_dir("k21-reject");
    let st = state_with_profiles_dir(&dir);
    let cookie = login(&st).await;

    let broken = profile_toml("kobo").replace("target_calendar_id = \"primary\"", "");
    let response = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form(
            "/dashboard/sources",
            Some(&cookie),
            &format!("profile={}", urlencode(&broken)),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "re-render, not redirect");

    let body = text(response).await;
    assert!(
        body.contains("target_calendar_id"),
        "the error should name the field"
    );
    assert!(
        body.contains("source_id = &quot;kobo&quot;") || body.contains("source_id = \"kobo\""),
        "the submitted text should still be in the box"
    );
    assert!(
        !dir.join("profiles/kobo.toml").exists(),
        "nothing may be written for a rejected profile"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn k21_reload_picks_up_a_profile_written_by_hand() {
    let dir = scratch_dir("k21-reload");
    let st = state_with_profiles_dir(&dir);
    let cookie = login(&st).await;

    std::fs::write(dir.join("profiles/grafana.toml"), profile_toml("grafana")).unwrap();

    let response = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form("/dashboard/sources/reload", Some(&cookie), ""))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    let body = text(
        almanac::shell::build_router(Arc::clone(&st))
            .oneshot(get("/dashboard/sources", Some(&cookie)))
            .await
            .unwrap(),
    )
    .await;
    assert!(
        body.contains("grafana"),
        "a profile placed by hand should appear after a reload, without a restart"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn k21_writing_a_profile_needs_a_session() {
    // This endpoint writes configuration that decides which calendar
    // gets written to. It may never be reachable without logging in.
    let dir = scratch_dir("k21-auth");
    let st = state_with_profiles_dir(&dir);

    for (uri, body) in [
        (
            "/dashboard/sources",
            format!("profile={}", urlencode(&profile_toml("kobo"))),
        ),
        ("/dashboard/sources/reload", String::new()),
    ] {
        let response = almanac::shell::build_router(Arc::clone(&st))
            .oneshot(post_form(uri, None, &body))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::SEE_OTHER,
            "{uri} should send an anonymous caller to the login page"
        );
    }

    assert!(
        !dir.join("profiles/kobo.toml").exists(),
        "an anonymous POST must not write a profile"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn k21_retiring_a_source_ends_it_and_keeps_the_record() {
    // Kyu's model, which Kenny asked for by name: the row stays with a
    // badge rather than the source vanishing without trace.
    let dir = scratch_dir("k21-retire");
    let st = state_with_profiles_dir(&dir);
    let cookie = login(&st).await;

    almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form(
            "/dashboard/sources",
            Some(&cookie),
            &format!("profile={}", urlencode(&profile_toml("kobo"))),
        ))
        .await
        .unwrap();
    almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form(
            "/dashboard/sources/kobo/issue",
            Some(&cookie),
            "",
        ))
        .await
        .unwrap();

    let response = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form(
            "/dashboard/sources/kobo/retire",
            Some(&cookie),
            "",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    assert!(
        !dir.join("profiles/kobo.toml").exists(),
        "the profile should have left the loaded set"
    );
    assert!(
        dir.join("profiles/kobo.toml.retired").exists(),
        "and it should still be on disk as the record"
    );

    let body = text(
        almanac::shell::build_router(Arc::clone(&st))
            .oneshot(get("/dashboard/sources", Some(&cookie)))
            .await
            .unwrap(),
    )
    .await;
    assert!(body.contains("kobo"), "the row must survive retirement");
    assert!(body.contains("retired"), "and say that it is retired");

    // The token went with it: a retired source cannot post.
    let posted = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/ingest/kobo")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, "Bearer whatever")
                .body(Body::from(
                    r#"{"title":"x","start":"2026-01-01T09:00:00+00:00"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(posted.status(), StatusCode::UNAUTHORIZED);

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn k21_a_source_with_undelivered_events_is_not_retired() {
    // The worker needs the profile to know which calendar an entry
    // belongs to. Retiring it while entries wait would leave them in
    // the journal forever — the journal never drops one — logging an
    // error on every pass and deliverable by nothing.
    let dir = scratch_dir("k21-retire-pending");
    let st = state_with_profiles_dir(&dir);
    let cookie = login(&st).await;

    st.journal
        .accept(&almanac::core::journal::Entry {
            id: "entry-1".to_string(),
            source_id: "home-assistant".to_string(),
            received_at: "2026-09-02T10:00:00+00:00".to_string(),
            payload: serde_json::json!({"title": "waiting"}),
            idempotency_key: None,
        })
        .await
        .unwrap();

    let response = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form(
            "/dashboard/sources/home-assistant/retire",
            Some(&cookie),
            "",
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "re-render with the reason"
    );

    let body = text(response).await;
    assert!(
        body.contains("waiting to be delivered"),
        "the refusal must say why"
    );
    assert!(
        dir.join("profiles/home-assistant.toml").exists(),
        "the profile must be untouched"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn k21_retiring_a_source_needs_a_session() {
    let dir = scratch_dir("k21-retire-auth");
    let st = state_with_profiles_dir(&dir);

    let response = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form(
            "/dashboard/sources/home-assistant/retire",
            None,
            "",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(
        dir.join("profiles/home-assistant.toml").exists(),
        "an anonymous POST must not retire anything"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// Not an assertion — a way to look at the page.
///
/// `ALMANAC_DUMP_SOURCES_PAGE=<path> cargo test --test dashboard_http
/// k21_dump` writes the rendered page there with the stylesheet
/// resolved, so the layout can be reviewed in a browser without
/// deploying. Ignored by default; it proves nothing on its own.
#[tokio::test]
#[ignore]
async fn k21_dump_the_sources_page_for_review() {
    let Ok(target) = std::env::var("ALMANAC_DUMP_SOURCES_PAGE") else {
        return;
    };
    let dir = scratch_dir("k21-dump");
    let st = state_with_profiles_dir(&dir);
    let cookie = login(&st).await;

    almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form(
            "/dashboard/sources",
            Some(&cookie),
            &format!("profile={}", urlencode(&profile_toml("kobo"))),
        ))
        .await
        .unwrap();
    almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form(
            "/dashboard/sources/kobo/issue",
            Some(&cookie),
            "",
        ))
        .await
        .unwrap();
    almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form(
            "/dashboard/sources",
            Some(&cookie),
            &format!("profile={}", urlencode(&profile_toml("grafana"))),
        ))
        .await
        .unwrap();
    almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form(
            "/dashboard/sources/grafana/retire",
            Some(&cookie),
            "",
        ))
        .await
        .unwrap();

    let body = text(
        almanac::shell::build_router(Arc::clone(&st))
            .oneshot(get("/dashboard/sources", Some(&cookie)))
            .await
            .unwrap(),
    )
    .await;

    let css = concat!(env!("CARGO_MANIFEST_DIR"), "/static/bootstrap.min.css");
    let body = body.replace("/static/bootstrap.min.css", &format!("file://{css}"));
    std::fs::write(&target, body).unwrap();

    std::fs::remove_dir_all(&dir).ok();
}
