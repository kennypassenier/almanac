//! Tests of the ingest endpoints, split by concern; the fixtures they
//! share live here.

mod auth;
mod delete;
mod entry;
mod sync;

use std::collections::HashMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use chassis::Caller;
use serde_json::{Value, json};

use super::auth::authenticate;
use super::delete::delete_event;
use super::request::build_entry;
use super::sync::ingest_sync;
use super::{AppState, IDEMPOTENCY_HEADER};
use crate::core::profile::Profile;
use crate::shell::calendar_client::GoogleCalendarClient;
use crate::shell::journal::Journal;

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
async fn state_with(source_id: &str, _token: &str) -> AppState {
    let dir = scratch_dir();

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

fn client(name: &str) -> Caller {
    Caller::Client {
        id: format!("source-{name}"),
        name: name.to_string(),
    }
}

/// State whose calendar client points at a stub, so the
/// synchronous path can actually deliver.
async fn state_with_calendar(
    source_id: &str,
    _token: &str,
    calendar: &crate::shell::testing::CalendarStub,
) -> AppState {
    let dir = scratch_dir();

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
    )
}

fn sync_payload() -> Value {
    serde_json::json!({
        "title": "meeting",
        "start": "2026-08-28T09:00:00+00:00",
        "external_id": "claude-session-1"
    })
}
