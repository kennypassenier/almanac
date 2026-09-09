//! The in-process kit harness: Almanac assembled exactly as the binary
//! assembles it — `almanac::shell::kit::mount` on a real `chassis::App` —
//! started on a free port with the kit's door in front, Google and its
//! token endpoint stubbed. Tests about the dashboard, the door and the
//! debug surfaces go through here.
//!
//! Wraps `chassis::testing::TestApp` (chassis-rs 1.8.0) since [meta]: the
//! kit now ships the harness this file used to hand-write — starting the
//! app, logging in, issuing a client, sending JSON, shutting down cleanly.
//! What stays Almanac's own: building `AppState` (the journal, the Google
//! stub, the profiles directory), seeding profile files and a 3.x token
//! store on disk before the app ever starts, and the calendar owner. The
//! public shape of `KitHub` is unchanged — every test file that calls it
//! still does.
#![allow(dead_code)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use almanac::core::profile::Profile;
use almanac::shell::auth::TokenManager;
use almanac::shell::calendar_client::GoogleCalendarClient;
use almanac::shell::ingest::AppState;
use almanac::shell::journal::{DEFAULT_MAX_BYTES, Journal};
use almanac::shell::kit::{import_source_tokens, mount};
use almanac::shell::testing::{CalendarStub, TokenStub, stub_credentials};
use axum::Router;
use chassis::AppSpec;
use chassis::testing::TestApp;
use reqwest::Method;

// No fixed TOKEN constant (unlike KEY below): `TestApp` generates its own
// `ALMANAC_TOKEN` per spawn and remembers it for `login()` — overriding
// the value it hands the app without also updating what it remembers for
// itself leaves the two disagreeing, so a spawned hub's actual login
// token is read back with `KitHub::token()` instead of assumed fixed.
pub const KEY: &str = "abababababababababababababababababababababababababababababababab";

pub fn profile_toml(source_id: &str) -> String {
    format!(
        r#"
schema_version = 2
source_id = "{source_id}"
target_calendar_id = "primary"
"#
    )
}

pub struct KitHub {
    pub addr: SocketAddr,
    pub state: Arc<AppState>,
    pub calendar: CalendarStub,
    pub dir: tempfile::TempDir,
    app: TestApp,
    _tokens: TokenStub,
}

impl KitHub {
    pub fn url(&self, path: &str) -> String {
        self.app.url(path)
    }

    /// This hub's actual `ALMANAC_TOKEN` — generated fresh per spawn by
    /// the kit's test harness, never the fixed `KEY`.
    pub fn token(&self) -> &str {
        self.app.token()
    }

    fn http() -> reqwest::Client {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("a client")
    }

    /// A browser with the admin session.
    pub async fn get(&self, path: &str) -> reqwest::Response {
        self.app
            .request(Method::GET, path)
            .send()
            .await
            .expect("a response")
    }

    pub async fn get_anon(&self, path: &str) -> reqwest::Response {
        Self::http()
            .get(self.url(path))
            .send()
            .await
            .expect("a response")
    }

    pub async fn page(&self, path: &str) -> String {
        let response = self.get(path).await;
        assert_eq!(response.status(), 200, "{path} must render");
        response.text().await.expect("a body")
    }

    /// One of the dashboard's own forms, posted by the admin's browser.
    pub async fn form(&self, path: &str, body: &str) -> reqwest::Response {
        self.app
            .request(Method::POST, path)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(body.to_string())
            .send()
            .await
            .expect("a response")
    }

    /// A script with a bearer token.
    pub fn bearer(&self, method: Method, path: &str, token: &str) -> reqwest::RequestBuilder {
        self.app.bearer(method, path, token)
    }

    pub async fn post_json(
        &self,
        path: &str,
        token: Option<&str>,
        body: &str,
    ) -> reqwest::Response {
        let mut request = Self::http()
            .post(self.url(path))
            .header("content-type", "application/json")
            .body(body.to_string());
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        request.send().await.expect("a response")
    }

    /// Issue a client token on the kit's Sources page (as the admin) and
    /// return the token, the way a person would with Reveal.
    pub async fn issue_client(&self, name: &str) -> String {
        // 4.0.2: the issue form carries the calendar; a source with a
        // profile on disk keeps it, a new one gets this test calendar.
        self.app
            .issue_client(name, &[("calendar", "cal-test")])
            .await
            .token
    }

    pub async fn shutdown(mut self) {
        self.app.shutdown().await;
    }
}

pub async fn body_json(response: reqwest::Response) -> serde_json::Value {
    let text = response.text().await.expect("a body");
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("expected JSON, got {text:?}: {e}"))
}

pub fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

/// A hub with one profile (`home-assistant` on `primary`), no owner.
pub async fn spawn_kit() -> KitHub {
    spawn_kit_with(&["home-assistant"], None).await
}

/// A hub with the given profiles written to disk and an optional calendar
/// owner (K24 needs one to create calendars).
pub async fn spawn_kit_with(sources: &[&str], owner: Option<&str>) -> KitHub {
    let dir = tempfile::tempdir().expect("a temp dir");
    let profiles_dir = dir.path().join("profiles");
    std::fs::create_dir_all(&profiles_dir).unwrap();
    for source in sources {
        std::fs::write(
            profiles_dir.join(format!("{source}.toml")),
            profile_toml(source),
        )
        .unwrap();
    }
    spawn_kit_in(dir, owner).await
}

pub async fn spawn_kit_in(dir: tempfile::TempDir, owner: Option<&str>) -> KitHub {
    let profiles_dir = dir.path().join("profiles");
    std::fs::create_dir_all(&profiles_dir).unwrap();
    let calendar = CalendarStub::start().await;
    let tokens = TokenStub::start(3600).await;
    let http = reqwest::Client::new();
    let profiles: HashMap<String, Profile> = almanac::shell::profiles::load_map(&profiles_dir);
    let state = Arc::new(
        AppState::new(
            profiles,
            Journal::new(dir.path().join("journal.jsonl"), DEFAULT_MAX_BYTES),
            GoogleCalendarClient::with_base_url(
                http.clone(),
                TokenManager::new(http, stub_credentials(&tokens.url)),
                &calendar.base_url,
            ),
        )
        .with_profiles_dir(profiles_dir)
        .with_calendar_owner(owner.map(str::to_string)),
    );
    // The 3.x token store, when a test seeded one, is imported like the
    // binary does it on the first start of 4.0.0. `dir.path()` is the
    // state directory this function is about to hand the kit via
    // ALMANAC_STATE_DIR below, so there is nothing to read back out of
    // the App for it.
    import_source_tokens(dir.path(), &dir.path().join("tokens.json"), KEY)
        .await
        .expect("the import runs");

    let spec = AppSpec {
        name: "almanac",
        version: env!("CARGO_PKG_VERSION"),
        repository: Some("kennypassenier/almanac"),
        ..Default::default()
    };
    // ALMANAC_TOKEN is deliberately not overridden here: the kit's harness
    // generates its own and remembers it for login() (see KitHub::token).
    // ALMANAC_SECRET_KEY has no such caching, so KEY — fixed, because a
    // 3.x token store may have been sealed with it before this call — is
    // safe to override.
    let state_dir = dir.path().display().to_string();
    let for_mount = Arc::clone(&state);
    let mut app = TestApp::start_with_env(
        spec,
        Router::new(),
        &[
            ("ALMANAC_STATE_DIR", state_dir.as_str()),
            ("ALMANAC_SECRET_KEY", KEY),
        ],
        move |app| mount(app, for_mount),
    )
    .await;
    app.login().await;
    let addr = app.addr();

    KitHub {
        addr,
        state,
        calendar,
        dir,
        app,
        _tokens: tokens,
    }
}
