//! In-process HTTP fixtures: the router driven directly with `oneshot`,
//! no port bound and no Google involved. Used by the ingest and admin
//! suites.

use std::collections::HashMap;
use std::sync::Arc;

use almanac::core::auth::ServiceAccountCredentials;
use almanac::core::profile::Profile;
use almanac::shell::auth::TokenManager;
use almanac::shell::calendar_client::GoogleCalendarClient;
use almanac::shell::journal::Journal;
use almanac::shell::state::AppState;
use axum::body::Body;
use axum::http::Request;
use http_body_util::BodyExt;

pub const HA_TOKEN: &str = "home-assistant-token";
pub const KUMA_TOKEN: &str = "uptime-kuma-token";

/// A minimal valid payload for the `home-assistant` source.
pub fn ha_payload() -> &'static str {
    r#"{"external_id":"switch.wasmachine","title":"Wasmachine klaar","start":"2026-08-28T09:00:00+00:00"}"#
}

/// A token endpoint that is never reached: these suites exercise the
/// ingest and operator surfaces only, and the asynchronous path never
/// calls Google.
const UNREACHABLE_TOKEN_URL: &str = "https://example.invalid/token";

/// A fresh scratch directory under the system temp dir, unique per
/// process, instant and test name.
pub fn scratch_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "almanac-inproc-{}-{}-{name}",
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

/// App state with one profile per given source, a journal at `journal_path` and a
/// Google client that points at an unreachable host.
pub fn state_with_journal(journal_path: std::path::PathBuf, sources: &[&str]) -> Arc<AppState> {
    let journal =
        Journal::open_in(journal_path.parent().expect("a folder")).expect("the journal opens");
    state_on(journal, sources)
}

/// App state whose journal has no database to write to, the way a store
/// that refuses every write looks to the ingest path.
pub fn state_without_journal_store(sources: &[&str]) -> Arc<AppState> {
    state_on(
        Journal::new(std::path::PathBuf::from("unused.jsonl")),
        sources,
    )
}

fn state_on(journal: Journal, sources: &[&str]) -> Arc<AppState> {
    let mut profiles = HashMap::new();
    for source in sources {
        profiles.insert(source.to_string(), profile(source));
    }
    let http = reqwest::Client::new();
    let tokens = TokenManager::new(
        http.clone(),
        ServiceAccountCredentials {
            client_email: "unused".to_string(),
            private_key: "unused".to_string(),
            token_url: UNREACHABLE_TOKEN_URL.to_string(),
        },
    );
    Arc::new(AppState::new(
        profiles,
        journal,
        GoogleCalendarClient::new(http, tokens),
    ))
}

/// The ingest suites' state: `home-assistant` and `uptime-kuma`.
pub fn ingest_state(dir: &std::path::Path) -> Arc<AppState> {
    state_with_journal(
        dir.join("journal.jsonl"),
        &["home-assistant", "uptime-kuma"],
    )
}

/// The admin suites' state: `home-assistant` only.
pub fn admin_state(dir: &std::path::Path) -> Arc<AppState> {
    state_with_journal(dir.join("journal.jsonl"), &["home-assistant"])
}

/// A GET with an optional bearer token.
pub fn get(uri: &str, token: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method("GET").uri(uri);
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    builder.body(Body::empty()).unwrap()
}

/// A JSON POST with an optional bearer token.
pub fn post(uri: &str, token: Option<&str>, body: &str) -> Request<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    builder.body(Body::from(body.to_string())).unwrap()
}

pub async fn body_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

pub async fn body_text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}
