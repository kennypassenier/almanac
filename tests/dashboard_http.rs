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

/// Choosing "+ New calendar…" and naming it.
fn new_calendar_body(source_id: &str, name: &str) -> String {
    format!(
        "source_id={}&calendar=__new__&new_calendar={}",
        urlencode(source_id),
        urlencode(name)
    )
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
        body.contains("+ New calendar"),
        "and offer making a new one"
    );
    assert!(
        body.contains(r#"name="new_calendar""#),
        "with a box for its name"
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
async fn k21_adding_a_source_creates_its_calendar_and_makes_it_issuable() {
    // End to end, in one sitting and without a restart: name, calendar,
    // token.
    let dir = scratch_dir("k21-add");
    let (st, cal) = state_with_calendar(&dir, Some("kenny@example.com")).await;
    let cookie = login(&st).await;

    let response = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form(
            "/dashboard/sources",
            Some(&cookie),
            &new_calendar_body("kobo", "Almanac · Test"),
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

    let created = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form(
            "/dashboard/sources",
            Some(&cookie),
            &new_calendar_body("kobo", "Almanac · Huishouden"),
        ))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::SEE_OTHER);

    // The id the dropdown would now carry for that calendar.
    let calendar_id = cal
        .state
        .calendars
        .lock()
        .await
        .keys()
        .next()
        .expect("the first source created one")
        .clone();

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
            &new_calendar_body("../../etc/passwd", "Almanac · Test"),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "re-render, not redirect");

    let body = text(response).await;
    assert!(
        body.contains("source name"),
        "the error must name the field"
    );
    assert!(
        body.contains("Almanac · Test") || body.contains("Almanac &#183; Test"),
        "the calendar that was typed should still be in the form"
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
async fn k21_without_an_owner_an_unknown_calendar_is_refused_rather_than_created() {
    // A calendar the service account creates belongs to the service
    // account and is invisible to every human until it is shared. That
    // mistake has been made here twice; making it from a button would
    // be the third.
    let dir = scratch_dir("k21-noowner");
    let (st, cal) = state_with_calendar(&dir, None).await;
    let cookie = login(&st).await;

    let response = almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form(
            "/dashboard/sources",
            Some(&cookie),
            &new_calendar_body("kobo", "Nobody Can See This"),
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
    let (st, _cal) = state_with_calendar(&dir, Some("kenny@example.com")).await;
    let cookie = login(&st).await;

    almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form(
            "/dashboard/sources",
            Some(&cookie),
            &new_calendar_body("kobo", "Almanac · Test"),
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
        ("/dashboard/sources", new_calendar_body("kobo", "Anything")),
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
    let (st, _cal) = state_with_calendar(&dir, Some("kenny@example.com")).await;
    let cookie = login(&st).await;

    almanac::shell::build_router(Arc::clone(&st))
        .oneshot(post_form(
            "/dashboard/sources",
            Some(&cookie),
            &new_calendar_body("kobo", "Almanac · Test"),
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
            &new_calendar_body("grafana", "Almanac · Infra"),
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
