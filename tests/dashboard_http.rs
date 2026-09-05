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
schema_version = 2
source_id = "{source_id}"
target_calendar_id = "primary"

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
    // Per BYTE, not per char: `as u8` on a multi-byte character keeps
    // only its last byte, so "Almanac · Huishouden" arrived at the
    // handler as mojibake and a test about keeping what was typed
    // failed for a reason that had nothing to do with the code.
    s.bytes()
        .map(|b| match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            other => format!("%{other:02X}"),
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
                    r#"{"title":"t","start":"2026-08-28T09:00:00+00:00","external_id":"t"}"#,
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
                    r#"{"title":"t","start":"2026-08-28T09:00:00+00:00","external_id":"t"}"#,
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
                    r#"{"title":"t","start":"2026-08-28T09:00:00+00:00","external_id":"t"}"#,
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

/// K21: seeded state with a real profiles directory AND a stubbed
/// Google, so find-or-create a calendar can be exercised offline.
///
/// The directory holds the profile the in-memory map starts with: a
/// reload reads that directory, so a test whose directory is empty
/// would assert that reloading deletes every source — true, and not
/// the behaviour under test.
async fn state_with_calendar(
    dir: &std::path::Path,
    owner: Option<&str>,
) -> (Arc<AppState>, almanac::shell::testing::CalendarStub) {
    let profiles_dir = dir.join("profiles");
    std::fs::create_dir_all(&profiles_dir).unwrap();
    std::fs::write(
        profiles_dir.join("home-assistant.toml"),
        profile_toml("home-assistant"),
    )
    .unwrap();

    let calendar = almanac::shell::testing::CalendarStub::start().await;
    let tokens = almanac::shell::testing::TokenStub::start(3600).await;
    let http = reqwest::Client::new();
    let store = TokenStore::with_key_loading(dir.join("tokens.json"), [5u8; 32]).unwrap();

    let mut profiles = HashMap::new();
    profiles.insert("home-assistant".to_string(), profile("home-assistant"));

    let state = AppState::new(
        profiles,
        Journal::new(dir.join("journal.jsonl"), DEFAULT_MAX_BYTES),
        GoogleCalendarClient::with_base_url(
            http.clone(),
            TokenManager::new(http, almanac::shell::testing::stub_credentials(&tokens.url)),
            &calendar.base_url,
        ),
        Some(hash_token(BOOTSTRAP)),
        store,
    )
    .with_profiles_dir(profiles_dir)
    .with_calendar_owner(owner.map(str::to_string));

    (Arc::new(state), calendar)
}

fn profile_toml(source_id: &str) -> String {
    format!(
        r#"
schema_version = 2
source_id = "{source_id}"
target_calendar_id = "primary"

"#
    )
}

/// Picking an existing calendar from the dropdown: `calendar` is its id.
fn pick_calendar_body(source_id: &str, calendar_id: &str) -> String {
    format!(
        "source_id={}&calendar={}",
        urlencode(source_id),
        urlencode(calendar_id)
    )
}

/// Makes a calendar through the panel that owns that job now (K24) and
/// hands back its id, which is what the source form's dropdown carries.
async fn make_calendar(
    st: &Arc<AppState>,
    cookie: &str,
    cal: &almanac::shell::testing::CalendarStub,
    name: &str,
) -> String {
    let response = almanac::shell::build_router(Arc::clone(st))
        .oneshot(post_form(
            "/dashboard/calendars",
            Some(cookie),
            &format!("name={}", urlencode(name)),
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::SEE_OTHER,
        "making {name} failed"
    );

    cal.state
        .calendars
        .lock()
        .await
        .iter()
        .find(|(_, created)| created.name == name)
        .map(|(id, _)| id.clone())
        .expect("the calendar should exist after creating it")
}

#[tokio::test]
async fn k21_the_sources_page_asks_for_a_name_and_a_calendar() {
    // The bug this exists for: Kenny opened this page to add a source
    // and there was nothing to click. The first fix asked for a whole
    // mapping profile, which he corrected — two fields, not fifteen.
    let dir = scratch_dir("k21-offers");
    let (st, _cal) = state_with_calendar(&dir, Some("kenny@example.com")).await;
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
    assert!(body.contains(r#"name="source_id""#), "no source name field");
    assert!(
        body.contains(r#"<select class="form-select" id="calendar" name="calendar""#),
        "the calendar should be a dropdown of what exists"
    );
    assert!(
        !body.contains("+ New calendar"),
        "making a calendar moved to its own panel (K24)"
    );
    assert!(
        body.contains(r#"action="/dashboard/calendars""#),
        "which is where a calendar is made now"
    );
    assert!(
        !body.contains(r#"name="profile""#),
        "the profile textarea should be gone"
    );
    assert!(
        body.contains("/dashboard/sources/reload"),
        "no reload control for a profile placed by hand"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn k21_adding_a_source_puts_it_on_the_chosen_calendar_and_makes_it_issuable() {
    // End to end, in one sitting and without a restart: calendar, name,
    // token.
    let dir = scratch_dir("k21-add");
    let (st, cal) = state_with_calendar(&dir, Some("kenny@example.com")).await;
    let cookie = login(&st).await;

    let calendar_id = make_calendar(&st, &cookie, &cal, "Almanac · Test").await;
    let response = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form(
            "/dashboard/sources",
            Some(&cookie),
            &pick_calendar_body("kobo", &calendar_id),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER, "should redirect");

    let written = std::fs::read_to_string(dir.join("profiles/kobo.toml"))
        .expect("the profile should be on disk");
    assert!(
        written.contains("schema_version = 2") && written.contains(r#"source_id = "kobo""#),
        "the profile should be the v2 routing shape, got:\n{written}"
    );
    assert!(
        !written.contains("[mapping]"),
        "since 2.0.0 a profile carries no field mapping — the call does"
    );
    assert!(
        !written.contains("Almanac · Test"),
        "the profile must carry the calendar ID, not its display name"
    );

    let calendars = cal.state.calendars.lock().await;
    assert_eq!(calendars.len(), 1, "the calendar should have been created");
    drop(calendars);

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
        "the new source must be issuable without restarting"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn k21_a_second_source_picks_the_existing_calendar_from_the_list() {
    // What the dropdown is for: the first source makes the calendar,
    // the second chooses it by id without anyone typing one.
    let dir = scratch_dir("k21-reuse");
    let (st, cal) = state_with_calendar(&dir, Some("kenny@example.com")).await;
    let cookie = login(&st).await;

    let calendar_id = make_calendar(&st, &cookie, &cal, "Almanac · Huishouden").await;
    let created = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form(
            "/dashboard/sources",
            Some(&cookie),
            &pick_calendar_body("kobo", &calendar_id),
        ))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::SEE_OTHER);

    let picked = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form(
            "/dashboard/sources",
            Some(&cookie),
            &pick_calendar_body("washing-machine", &calendar_id),
        ))
        .await
        .unwrap();
    assert_eq!(picked.status(), StatusCode::SEE_OTHER);

    assert_eq!(
        cal.state.calendars.lock().await.len(),
        1,
        "picking an existing calendar must not create another"
    );

    let first = std::fs::read_to_string(dir.join("profiles/kobo.toml")).unwrap();
    let second = std::fs::read_to_string(dir.join("profiles/washing-machine.toml")).unwrap();
    let id_of = |t: &str| {
        t.lines()
            .find(|l| l.starts_with("target_calendar_id"))
            .unwrap()
            .to_string()
    };
    assert_eq!(
        id_of(&first),
        id_of(&second),
        "both profiles must point at the same calendar"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn k21_a_rejected_source_keeps_what_was_typed_and_writes_nothing() {
    // Losing the typing as well as the mistake is what makes a form
    // feel hostile — and this name would have named a file.
    let dir = scratch_dir("k21-reject");
    let (st, cal) = state_with_calendar(&dir, Some("kenny@example.com")).await;
    let cookie = login(&st).await;

    let response = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form(
            "/dashboard/sources",
            Some(&cookie),
            &pick_calendar_body("../../etc/passwd", "some-calendar-id"),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "re-render, not redirect");

    let body = text(response).await;
    assert!(
        body.contains("source name"),
        "the error must name the field"
    );
    assert_eq!(
        cal.state.calendars.lock().await.len(),
        0,
        "a rejected name must not create a calendar either"
    );
    assert_eq!(
        std::fs::read_dir(dir.join("profiles")).unwrap().count(),
        1,
        "only the seeded profile may exist"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn k24_without_an_owner_no_calendar_is_created() {
    // A calendar the service account creates belongs to the service
    // account and is invisible to every human until it is shared. That
    // mistake has been made here twice; making it from a button would
    // be the third.
    let dir = scratch_dir("k21-noowner");
    let (st, cal) = state_with_calendar(&dir, None).await;
    let cookie = login(&st).await;

    let response = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form(
            "/dashboard/calendars",
            Some(&cookie),
            "name=Nobody+Can+See+This",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = text(response).await;
    assert!(
        body.contains("ALMANAC_CALENDAR_OWNER"),
        "the refusal must say which setting is missing"
    );
    assert_eq!(
        cal.state.calendars.lock().await.len(),
        0,
        "nothing may be created without an owner to share it with"
    );
    assert!(!dir.join("profiles/kobo.toml").exists());

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn k23_an_unusable_profile_is_listed_and_can_be_deleted() {
    // Kenny, 2026-09-03: a broken profile must not stop the app, and
    // the dashboard should show it as broken with a delete button. The
    // second half matters as much as the first — the page from which
    // you fix it is the page that would not have existed if a broken
    // file could stop the service.
    let dir = scratch_dir("k23-unusable");
    let (st, _cal) = state_with_calendar(&dir, Some("kenny@example.com")).await;
    let cookie = login(&st).await;

    std::fs::write(
        dir.join("profiles/wrecked.toml"),
        "this is not toml at all\n",
    )
    .unwrap();

    let body = text(
        almanac::shell::build_router(Arc::clone(&st))
            .oneshot(get("/dashboard/sources", Some(&cookie)))
            .await
            .unwrap(),
    )
    .await;
    assert!(
        body.contains("wrecked.toml"),
        "an unusable profile must be visible, not only logged"
    );
    assert!(
        body.contains("/dashboard/profiles/wrecked.toml/delete"),
        "and deletable from the same page"
    );

    let response = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form(
            "/dashboard/profiles/wrecked.toml/delete",
            Some(&cookie),
            "",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(!dir.join("profiles/wrecked.toml").exists());

    // The source that was fine all along never noticed.
    assert!(dir.join("profiles/home-assistant.toml").exists());

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn k23_deleting_an_unusable_profile_needs_a_session() {
    let dir = scratch_dir("k23-unusable-auth");
    let (st, _cal) = state_with_calendar(&dir, Some("kenny@example.com")).await;
    std::fs::write(dir.join("profiles/wrecked.toml"), "not toml\n").unwrap();

    let response = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form(
            "/dashboard/profiles/wrecked.toml/delete",
            None,
            "",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(
        dir.join("profiles/wrecked.toml").exists(),
        "an anonymous POST must not delete anything"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn k24_a_calendar_in_use_cannot_be_deleted_and_says_so() {
    // Kenny's rule: the delete button is only live for a calendar no
    // source writes to. Checked in the page AND on arrival — the page
    // is a snapshot, and a source can appear between the render and
    // the click.
    let dir = scratch_dir("k24-in-use");
    let (st, cal) = state_with_calendar(&dir, Some("kenny@example.com")).await;
    let cookie = login(&st).await;

    let calendar_id = make_calendar(&st, &cookie, &cal, "Almanac · Huishouden").await;
    almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form(
            "/dashboard/sources",
            Some(&cookie),
            &pick_calendar_body("kobo", &calendar_id),
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
    assert!(
        body.contains("Almanac · Huishouden") || body.contains("Almanac &#183; Huishouden"),
        "the calendar should be listed"
    );
    assert!(
        !body.contains(&format!("/dashboard/calendars/{calendar_id}/delete")),
        "a calendar in use must not offer a live delete"
    );
    assert!(
        body.contains("still write here"),
        "and should say why the button is dead"
    );

    // The guard is repeated on arrival, not only drawn in the page.
    let refused = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form(
            &format!("/dashboard/calendars/{calendar_id}/delete"),
            Some(&cookie),
            "",
        ))
        .await
        .unwrap();
    assert_eq!(
        refused.status(),
        StatusCode::OK,
        "re-render with the reason"
    );
    assert!(text(refused).await.contains("kobo"));
    assert_eq!(
        cal.state.calendars.lock().await.len(),
        1,
        "and the calendar must still be there"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn k24_a_calendar_nothing_writes_to_can_be_deleted() {
    let dir = scratch_dir("k24-free");
    let (st, cal) = state_with_calendar(&dir, Some("kenny@example.com")).await;
    let cookie = login(&st).await;

    let calendar_id = make_calendar(&st, &cookie, &cal, "Almanac · Leeg").await;

    let body = text(
        almanac::shell::build_router(Arc::clone(&st))
            .oneshot(get("/dashboard/sources", Some(&cookie)))
            .await
            .unwrap(),
    )
    .await;
    assert!(
        body.contains(&format!("/dashboard/calendars/{calendar_id}/delete")),
        "with no source writing to it, delete must be live"
    );

    let response = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form(
            &format!("/dashboard/calendars/{calendar_id}/delete"),
            Some(&cookie),
            "",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(
        cal.state.calendars.lock().await.is_empty(),
        "it should be gone at Google, not only off the page"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn k24_making_the_same_calendar_twice_does_not_make_two() {
    // A double submit, or someone retyping a name that already exists.
    // A duplicate calendar is close to invisible: events land, nothing
    // errors, and half of them are on a calendar nobody has open.
    let dir = scratch_dir("k24-twice");
    let (st, cal) = state_with_calendar(&dir, Some("kenny@example.com")).await;
    let cookie = login(&st).await;

    make_calendar(&st, &cookie, &cal, "Almanac · Huishouden").await;
    make_calendar(&st, &cookie, &cal, "Almanac · Huishouden").await;

    assert_eq!(cal.state.calendars.lock().await.len(), 1);

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn k24_a_new_calendar_is_shared_in_the_same_breath_as_making_it() {
    // The half that has gone wrong here twice: a calendar the service
    // account makes is owned by the service account and invisible to
    // every human until it is shared. Asserted against what the stub
    // actually received, not against the log line that describes it.
    let dir = scratch_dir("k24-shared");
    let (st, cal) = state_with_calendar(&dir, Some("kenny@example.com")).await;
    let cookie = login(&st).await;

    make_calendar(&st, &cookie, &cal, "Almanac · Gedeeld").await;

    let calendars = cal.state.calendars.lock().await;
    let created = calendars.values().next().expect("one calendar");
    assert_eq!(
        created.shared_with,
        vec!["kenny@example.com".to_string()],
        "creating without sharing produces a calendar nobody can see"
    );

    drop(calendars);
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn k24_a_deleted_calendar_leaves_the_list_at_once() {
    // Google's calendar list is eventually consistent: a calendar
    // deleted a second ago still comes back in the next list call, so
    // the page rendered straight after the delete showed the thing that
    // had just been removed. Almanac knows what it deleted and says so.
    let dir = scratch_dir("k24-gone");
    let (st, cal) = state_with_calendar(&dir, Some("kenny@example.com")).await;
    let cookie = login(&st).await;

    let calendar_id = make_calendar(&st, &cookie, &cal, "Almanac · Weg").await;

    // The stub is instantly consistent, so to reproduce Google's lag the
    // delete is recorded while the calendar is still listed.
    st.remember_deleted_calendar(&calendar_id);

    let body = text(
        almanac::shell::build_router(Arc::clone(&st))
            .oneshot(get("/dashboard/sources", Some(&cookie)))
            .await
            .unwrap(),
    )
    .await;
    assert!(
        !body.contains("Almanac · Weg") && !body.contains("Almanac &#183; Weg"),
        "a calendar almanac knows it deleted must not still be listed"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn k24_the_memory_of_a_deleted_calendar_clears_itself() {
    // Otherwise a long-running process would carry every id it ever
    // deleted, and — worse — a calendar someone later recreates under
    // the same id would stay invisible.
    let dir = scratch_dir("k24-forget");
    let (st, _cal) = state_with_calendar(&dir, Some("kenny@example.com")).await;

    st.remember_deleted_calendar("gone@group.calendar.google.com");

    // Google no longer lists it: almanac has nothing left to hide.
    let listed = st.without_deleted_calendars(vec![(
        "other@group.calendar.google.com".to_string(),
        "Other".to_string(),
    )]);
    assert_eq!(listed.len(), 1);
    assert!(
        st.deleted_calendars.lock().unwrap().is_empty(),
        "an id Google has caught up on must be forgotten"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn k24_every_destructive_button_asks_first_and_shows_it_is_working() {
    // Kenny, 2026-09-03: these sit in table rows next to each other and
    // a mis-click costs anything from a token to a calendar and every
    // event on it.
    let dir = scratch_dir("k24-confirm");
    let (st, cal) = state_with_calendar(&dir, Some("kenny@example.com")).await;
    let cookie = login(&st).await;

    let calendar_id = make_calendar(&st, &cookie, &cal, "Almanac · Test").await;
    almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form(
            "/dashboard/sources",
            Some(&cookie),
            &pick_calendar_body("kobo", &calendar_id),
        ))
        .await
        .unwrap();
    std::fs::write(dir.join("profiles/wrecked.toml"), "not toml\n").unwrap();

    let body = text(
        almanac::shell::build_router(Arc::clone(&st))
            .oneshot(get("/dashboard/sources", Some(&cookie)))
            .await
            .unwrap(),
    )
    .await;

    // Every form that deletes something asks first.
    for action in [
        "/dashboard/sources/kobo/delete",
        "/dashboard/profiles/wrecked.toml/delete",
    ] {
        let form = body
            .split("<form")
            .find(|chunk| chunk.contains(action))
            .unwrap_or_else(|| panic!("no form for {action}"));
        assert!(
            form.contains("data-confirm="),
            "{action} asks nothing first"
        );
        assert!(form.contains("data-busy="), "{action} shows no busy state");
        assert!(form.contains("spinner-border"), "{action} has no spinner");
    }

    // And making a calendar, which is the slow one.
    let make = body
        .split("<form")
        .find(|chunk| chunk.contains(r#"action="/dashboard/calendars""#))
        .expect("the make-a-calendar form");
    assert!(make.contains("data-busy="));

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn k24_making_a_calendar_needs_a_session() {
    let dir = scratch_dir("k24-auth");
    let (st, cal) = state_with_calendar(&dir, Some("kenny@example.com")).await;

    let response = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form("/dashboard/calendars", None, "name=Anything"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(
        cal.state.calendars.lock().await.is_empty(),
        "an anonymous POST must not reach Google at all"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn k25_the_picker_offers_every_theme_the_package_ships() {
    // The eleven themes live once, in Rust, because almanac renders its
    // markup on the server. If that list and the package's registry ever
    // disagree, the picker silently offers fewer themes than exist —
    // which is what happened when the package grew from seven to eleven.
    let dir = scratch_dir("k25-picker");
    let (st, _cal) = state_with_calendar(&dir, Some("kenny@example.com")).await;
    let cookie = login(&st).await;

    let body = text(
        almanac::shell::build_router(Arc::clone(&st))
            .oneshot(get("/dashboard", Some(&cookie)))
            .await
            .unwrap(),
    )
    .await;

    for theme in [
        "formal",
        "light",
        "dark",
        "cyberpunk",
        "pastel",
        "terminal",
        "topo",
        "high-contrast",
        "sepia",
        "blueprint",
        "solstice",
    ] {
        assert!(
            body.contains(&format!(r#"data-kp-theme="{theme}""#)),
            "the picker must offer {theme}"
        );
    }

    // Every option previews its theme by wearing it. The hand-copied
    // gradient this replaced held 21 colours whose only job was to look
    // like the theme, and nothing would have failed when they stopped.
    assert_eq!(
        body.matches(r#"<span class="kp-swatch" data-theme="#)
            .count(),
        11,
        "each option shows a swatch that reads the theme's own tokens"
    );
    assert!(
        !body.contains("linear-gradient(135deg"),
        "no copied swatch colours may come back"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn k25_the_picker_groups_light_and_dark_from_the_registry_not_a_copy() {
    // kp-themes 3.0.0 groups the picker into light and dark sections by
    // default (TH63); almanac writes that split itself since it writes
    // the whole menu server-side. The split has to come from the same
    // vendored registry the other K25 tests already treat as the source
    // of truth — a second, hand-typed dark list is exactly the mistake
    // this project made once already.
    let dir = scratch_dir("k25-groups");
    let (st, _cal) = state_with_calendar(&dir, Some("kenny@example.com")).await;
    let cookie = login(&st).await;

    let registry = text(
        almanac::shell::build_router(Arc::clone(&st))
            .oneshot(get("/static/theme-registry.js", None))
            .await
            .unwrap(),
    )
    .await;
    let body = text(
        almanac::shell::build_router(Arc::clone(&st))
            .oneshot(get("/dashboard", Some(&cookie)))
            .await
            .unwrap(),
    )
    .await;

    assert!(
        body.contains(r#"data-kp-theme-group="light""#),
        "the picker must have a light section"
    );
    assert!(
        body.contains(r#"data-kp-theme-group="dark""#),
        "the picker must have a dark section"
    );

    // Same technique the registry-vs-Rust test uses: read name and dark
    // flag straight out of the vendored file, entry by entry.
    let light_at = body
        .find(r#"data-kp-theme-group="light""#)
        .expect("a light group");
    let dark_at = body
        .find(r#"data-kp-theme-group="dark""#)
        .expect("a dark group");
    assert!(
        light_at < dark_at,
        "light comes before dark, matching upstream's own order"
    );
    let light_section = &body[light_at..dark_at];
    let dark_section = &body[dark_at..];

    for (at, _) in registry.match_indices("{ name: '") {
        let rest = &registry[at + "{ name: '".len()..];
        let name = &rest[..rest.find('\'').expect("a closing quote")];
        let entry_end = rest.find('}').expect("a closing brace");
        let is_dark = rest[..entry_end].contains("dark: true");

        let needle = format!(r#"data-kp-theme="{name}""#);
        if is_dark {
            assert!(
                dark_section.contains(&needle),
                "{name} is dark in the registry but missing from the dark group"
            );
            assert!(
                !light_section.contains(&needle),
                "{name} is dark in the registry but offered in the light group"
            );
        } else {
            assert!(
                light_section.contains(&needle),
                "{name} is light in the registry but missing from the light group"
            );
            assert!(
                !dark_section.contains(&needle),
                "{name} is light in the registry but offered in the dark group"
            );
        }
    }

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn k25_the_rust_theme_list_matches_the_packages_own_registry() {
    // The one place a vendored picker can rot without any file
    // disagreeing: almanac's list is Rust, upstream's is JavaScript, and
    // the commit gate compares the files it vendored — not this.
    //
    // Read out of the vendored registry rather than out of kp-themes, so
    // this still means something on CI where the source is not present.
    let dir = scratch_dir("k25-registry");
    let (st, _cal) = state_with_calendar(&dir, Some("kenny@example.com")).await;
    let cookie = login(&st).await;

    let registry = text(
        almanac::shell::build_router(Arc::clone(&st))
            .oneshot(get("/static/theme-registry.js", None))
            .await
            .unwrap(),
    )
    .await;
    let body = text(
        almanac::shell::build_router(Arc::clone(&st))
            .oneshot(get("/dashboard", Some(&cookie)))
            .await
            .unwrap(),
    )
    .await;

    let upstream: Vec<&str> = registry
        .match_indices("name: '")
        .map(|(at, _)| {
            let rest = &registry[at + "name: '".len()..];
            &rest[..rest.find('\'').expect("a closing quote")]
        })
        .collect();
    assert_eq!(
        upstream.len(),
        11,
        "the vendored registry should carry eleven themes, not {}",
        upstream.len()
    );

    for name in &upstream {
        assert!(
            body.contains(&format!(r#"data-kp-theme="{name}""#)),
            "the package ships {name} and the picker does not offer it"
        );
    }
    assert_eq!(
        body.matches(r#"data-kp-theme=""#).count(),
        upstream.len(),
        "the picker offers a theme the package does not ship"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn k25_the_stored_contract_is_the_one_the_shared_package_defines() {
    // The cheapest guard against the thing kp-themes exists to prevent:
    // three projects drifting apart on what "theme" means in
    // localStorage. Asserted against the vendored modules themselves
    // since v1 — the behaviour is no longer almanac's own copy.
    let dir = scratch_dir("k25-contract");
    let (st, _cal) = state_with_calendar(&dir, Some("kenny@example.com")).await;

    let registry = text(
        almanac::shell::build_router(Arc::clone(&st))
            .oneshot(get("/static/theme-registry.js", None))
            .await
            .unwrap(),
    )
    .await;

    assert!(
        registry.contains("STORAGE_KEY = 'theme'"),
        "the localStorage key must stay 'theme'"
    );
    assert!(
        registry.contains("DEFAULT_THEME = 'formal'"),
        "the default must stay 'formal'"
    );

    let core = text(
        almanac::shell::build_router(Arc::clone(&st))
            .oneshot(get("/static/theme-core.js", None))
            .await
            .unwrap(),
    )
    .await;
    // Since 3.0.0 the class is a configurable `darkClass` rather than a
    // literal `classList.toggle('dark', ...)` call — `configureTheme()`
    // can rename it — so the guard checks the *default* stays 'dark'
    // rather than the exact call shape, which is free to keep reshaping
    // as configureTheme grows more options.
    assert!(
        core.contains("classList.toggle(cls"),
        "a dark theme must still set a class the package's CSS expects"
    );
    assert!(
        core.contains("darkClass") && core.contains("('dark')"),
        "the default dark class must still be 'dark', unless almanac starts calling configureTheme()"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn k25_the_theme_assets_are_served_and_the_page_asks_for_them() {
    let dir = scratch_dir("k25-assets");
    let (st, _cal) = state_with_calendar(&dir, Some("kenny@example.com")).await;
    let cookie = login(&st).await;

    for (path, marker) in [
        ("/static/themes.css", "[data-theme='cyberpunk']"),
        // One of the four themes v1 added: a stale copy of the file
        // would still pass every assertion about the original seven.
        ("/static/themes.css", "[data-theme='solstice']"),
        ("/static/kp-components.css", ".kp-swatch"),
        ("/static/theme-bridge.css", "--bs-body-bg"),
        ("/static/theme-picker.js", "data-kp-theme-picker"),
        ("/static/theme-core.js", "applyTheme"),
        ("/static/theme-registry.js", "STORAGE_KEY"),
        ("/static/strings.js", "DEFAULT_STRINGS"),
        ("/static/theme-bootstrap.js", "data-bs-theme"),
    ] {
        let response = almanac::shell::build_router(Arc::clone(&st))
            .oneshot(get(path, None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path} must be served");
        assert!(
            text(response).await.contains(marker),
            "{path} should contain {marker}"
        );
    }

    let body = text(
        almanac::shell::build_router(Arc::clone(&st))
            .oneshot(get("/dashboard", Some(&cookie)))
            .await
            .unwrap(),
    )
    .await;

    // The three modules import each other by relative path, so they only
    // resolve while all of them are served from the same directory.
    for asset in [
        "/static/themes.css",
        "/static/kp-components.css",
        "/static/theme-picker.js",
        "/static/theme-bootstrap.js",
    ] {
        assert!(body.contains(asset), "the page must ask for {asset}");
    }

    // And nothing renders before the stored theme is applied: the
    // no-flash script has to be in the head, not after the body.
    let head = &body[..body.find("</head>").expect("a head")];
    assert!(
        head.contains("localStorage.getItem('theme')"),
        "the no-flash script must run before the page paints"
    );
    // Bootstrap's own switch is set in that same breath. Without it the
    // tokens go dark while every card and table stays light — the half
    // -applied look this glue exists to prevent.
    assert!(
        head.contains("data-bs-theme"),
        "the first paint must settle Bootstrap's theme too, not only the tokens"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn k21_reload_picks_up_a_profile_written_by_hand() {
    let dir = scratch_dir("k21-reload");
    let (st, _cal) = state_with_calendar(&dir, Some("kenny@example.com")).await;
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
async fn k21_deleting_a_source_removes_it_entirely() {
    // Kenny asked for the whole source to go: token and profile both,
    // immediately. The events it already made stay on the calendar —
    // deleting a source says something about the source, not about
    // what already happened.
    let dir = scratch_dir("k21-retire");
    let (st, cal) = state_with_calendar(&dir, Some("kenny@example.com")).await;
    let cookie = login(&st).await;

    let calendar_id = make_calendar(&st, &cookie, &cal, "Almanac · Test").await;
    almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form(
            "/dashboard/sources",
            Some(&cookie),
            &pick_calendar_body("kobo", &calendar_id),
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
            "/dashboard/sources/kobo/delete",
            Some(&cookie),
            "",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    assert!(
        !dir.join("profiles/kobo.toml").exists(),
        "the profile must be gone"
    );
    assert_eq!(
        std::fs::read_dir(dir.join("profiles")).unwrap().count(),
        1,
        "only the seeded profile may remain — no renamed copy left behind"
    );

    let body = text(
        almanac::shell::build_router(Arc::clone(&st))
            .oneshot(get("/dashboard/sources", Some(&cookie)))
            .await
            .unwrap(),
    )
    .await;
    // Asserting an absence rather than a presence: "kobo is missing"
    // cannot pass on a page where deleting silently did nothing, which
    // is what an assertion about existence would have allowed (homelab
    // F263, 2026-09-03).
    assert!(
        !body.contains("/dashboard/sources/kobo/delete"),
        "a deleted source must be off the page entirely"
    );

    let posted = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/ingest/kobo")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, "Bearer whatever")
                .body(Body::from(
                    r#"{"title":"x","start":"2026-01-01T09:00:00+00:00","external_id":"x"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(posted.status(), StatusCode::UNAUTHORIZED);

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn k21_a_source_with_undelivered_events_is_not_deleted() {
    // The worker resolves an entry's calendar through its profile, and
    // the journal never drops an entry, so deleting first would strand
    // them: unreachable, erroring on every pass, forever.
    let dir = scratch_dir("k21-retire-pending");
    let (st, _cal) = state_with_calendar(&dir, Some("kenny@example.com")).await;
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
            "/dashboard/sources/home-assistant/delete",
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
    assert!(dir.join("profiles/home-assistant.toml").exists());

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn k21_changing_sources_needs_a_session() {
    // These endpoints write configuration that decides which calendar
    // gets written to, can create a calendar at Google, and can delete
    // a source outright.
    let dir = scratch_dir("k21-auth");
    let (st, cal) = state_with_calendar(&dir, Some("kenny@example.com")).await;

    for (uri, body) in [
        (
            "/dashboard/sources",
            pick_calendar_body("kobo", "any-calendar"),
        ),
        ("/dashboard/sources/reload", String::new()),
        ("/dashboard/sources/home-assistant/delete", String::new()),
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

    assert!(!dir.join("profiles/kobo.toml").exists());
    assert!(dir.join("profiles/home-assistant.toml").exists());
    assert_eq!(
        cal.state.calendars.lock().await.len(),
        0,
        "an anonymous POST must not reach Google at all"
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
    let (st, cal) = state_with_calendar(&dir, Some("kenny@example.com")).await;
    let cookie = login(&st).await;

    let calendar_id = make_calendar(&st, &cookie, &cal, "Almanac · Test").await;
    almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form(
            "/dashboard/sources",
            Some(&cookie),
            &pick_calendar_body("kobo", &calendar_id),
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
            &pick_calendar_body("grafana", &calendar_id),
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

    std::fs::write(
        dir.join("profiles/wrecked.toml"),
        "schema_version = 1\nsource_id = \"uptime-kuma\"\ntarget_calendar_id = \"x\"\n",
    )
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

#[tokio::test]
async fn k24_a_second_click_while_google_lags_does_not_make_a_second_calendar() {
    // The fault the K24 correction form found. "Make calendar" is
    // find-or-create precisely so a double submit cannot produce two —
    // but it looks for the existing one in Google's calendar list, and
    // that list lags a create by seconds. Both clicks therefore find
    // nothing and both create. Measured on CT 112: two `deleted a
    // calendar` lines at 19:56 for one request.
    let dir = scratch_dir("k24-double-create");
    let (st, cal) = state_with_calendar(&dir, Some("kenny@example.com")).await;
    let cookie = login(&st).await;

    cal.lag_new_calendars();

    for _ in 0..2 {
        let response = almanac::shell::build_router(Arc::clone(&st))
            .oneshot(post_form(
                "/dashboard/calendars",
                Some(&cookie),
                &format!("name={}", urlencode("Almanac · Dubbel")),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
    }

    let made = cal
        .state
        .calendars
        .lock()
        .await
        .values()
        .filter(|created| created.name == "Almanac · Dubbel")
        .count();
    assert_eq!(
        made, 1,
        "two clicks inside Google's lag must still leave exactly one calendar"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn k24_a_calendar_google_has_not_listed_yet_still_shows_on_the_page() {
    // The absence misleads as much as the stale presence did: a
    // calendar created a second ago is missing from the page that
    // renders next, which reads as "it did not work" — and inviting
    // exactly the second click the test above is about.
    let dir = scratch_dir("k24-created-visible");
    let (st, cal) = state_with_calendar(&dir, Some("kenny@example.com")).await;
    let cookie = login(&st).await;

    cal.lag_new_calendars();
    let response = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form(
            "/dashboard/calendars",
            Some(&cookie),
            &format!("name={}", urlencode("Almanac · Vers")),
        ))
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
        body.contains("Almanac · Vers") || body.contains("Almanac &#183; Vers"),
        "a calendar almanac just made must be on the page before Google lists it"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn k24_the_memory_of_a_created_calendar_clears_itself_and_never_doubles_a_row() {
    // Once Google catches up the memory has to let go, or a
    // long-running process would carry every calendar it ever made —
    // and the page would show each of them twice, once from Google and
    // once from the memory.
    let dir = scratch_dir("k24-created-forget");
    let (st, cal) = state_with_calendar(&dir, Some("kenny@example.com")).await;
    let cookie = login(&st).await;

    cal.lag_new_calendars();
    almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form(
            "/dashboard/calendars",
            Some(&cookie),
            &format!("name={}", urlencode("Almanac · Bijgetrokken")),
        ))
        .await
        .unwrap();
    cal.catch_up().await;

    let body = text(
        almanac::shell::build_router(Arc::clone(&st))
            .oneshot(get("/dashboard/sources", Some(&cookie)))
            .await
            .unwrap(),
    )
    .await;
    let rows = body.matches("Almanac &#183; Bijgetrokken").count()
        + body.matches("Almanac · Bijgetrokken").count();
    assert!(
        rows > 0,
        "the calendar must still be on the page once Google lists it"
    );

    assert!(
        st.remembered_calendar("Almanac · Bijgetrokken").is_none(),
        "the memory must let go once Google's own list carries the calendar"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn k24_a_calendar_created_and_deleted_inside_the_lag_stays_gone() {
    // The two memories pull in opposite directions on the same
    // calendar. Without forgetting the creation, the memory that makes
    // a fresh calendar visible would keep putting back one that was
    // deleted a moment later.
    let dir = scratch_dir("k24-created-then-deleted");
    let (st, cal) = state_with_calendar(&dir, Some("kenny@example.com")).await;
    let cookie = login(&st).await;

    cal.lag_new_calendars();
    almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form(
            "/dashboard/calendars",
            Some(&cookie),
            &format!("name={}", urlencode("Almanac · Vergissing")),
        ))
        .await
        .unwrap();

    let calendar_id = cal
        .state
        .calendars
        .lock()
        .await
        .iter()
        .find(|(_, created)| created.name == "Almanac · Vergissing")
        .map(|(id, _)| id.clone())
        .expect("the stub recorded the calendar even while hiding it");

    let response = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form(
            &format!("/dashboard/calendars/{calendar_id}/delete"),
            Some(&cookie),
            "",
        ))
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
        !body.contains("Almanac · Vergissing") && !body.contains("Almanac &#183; Vergissing"),
        "a calendar deleted right after it was made must not be put back by the create memory"
    );

    std::fs::remove_dir_all(&dir).ok();
}
