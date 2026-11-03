//! Tests of the worker, split by concern; the fixtures they share live
//! here.

mod drain;
mod outage;
mod set_aside;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;

use super::*;
use crate::shell::calendar_client::GoogleCalendarClient;
use crate::shell::notify::{Notifier, ops};
use crate::shell::state::AppState;

/// A unique scratch directory.
///
/// The counter matters: naming these by process id and nanoseconds
/// alone lets two tests running on different threads land on the
/// same directory and share a journal, which shows up as a rare,
/// confusing failure in one of them and nowhere else.
fn scratch(name: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "almanac-{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A minimal state for driving the self-update loop, which only
/// reads the capture buffer.
pub(crate) async fn state_for_update_loop() -> (Arc<AppState>, std::path::PathBuf) {
    let (state, dir) = state_with_empty_journal();
    (state, dir)
}

fn state_with_empty_journal() -> (Arc<AppState>, std::path::PathBuf) {
    let dir = scratch("worker-test");

    let http = reqwest::Client::new();
    let tokens = crate::shell::auth::TokenManager::new(
        http.clone(),
        crate::core::auth::ServiceAccountCredentials {
            client_email: "unused".to_string(),
            private_key: "unused".to_string(),
            token_url: "https://example.invalid/token".to_string(),
        },
    );

    let state = Arc::new(AppState::new_for_test(
        HashMap::new(),
        crate::shell::journal::testing::journal_in(&dir),
        GoogleCalendarClient::new(http, tokens),
    ));

    (state, dir)
}

/// A state whose deliveries always fail permanently.
async fn state_rejecting_everything() -> (Arc<AppState>, std::path::PathBuf) {
    let dir = scratch("deadletter");

    let calendar = crate::shell::testing::CalendarStub::start().await;
    calendar.reject_next(9_999);
    let tokens = crate::shell::testing::TokenStub::start(3600).await;

    let toml = r#"
schema_version = 2
source_id = "home-assistant"
target_calendar_id = "household"

"#;
    let mut profiles = HashMap::new();
    profiles.insert(
        "home-assistant".to_string(),
        crate::core::profile::Profile::parse(toml, "t.toml").unwrap(),
    );

    let state = Arc::new(AppState::new_for_test(
        profiles,
        crate::shell::journal::testing::journal_in(&dir),
        GoogleCalendarClient::with_base_url(
            reqwest::Client::new(),
            crate::shell::auth::TokenManager::new(
                reqwest::Client::new(),
                crate::shell::testing::stub_credentials(&tokens.url),
            ),
            &calendar.base_url,
        ),
    ));

    state
        .journal
        .accept(&crate::core::journal::Entry {
            id: "doomed".to_string(),
            source_id: "home-assistant".to_string(),
            received_at: "2026-08-29T09:00:00+00:00".to_string(),
            payload: serde_json::json!({
                "title": "t",
                "start": "2026-08-29T09:00:00+00:00"
            }),
            idempotency_key: None,
        })
        .await
        .unwrap();

    (state, dir)
}
