//! Tests of how one pass behaves while Google is unreachable.

use super::*;
use crate::core::journal::Entry;

/// A state with two pending entries for two different calendars, so the
/// stub's request log shows which entries a pass actually tried.
async fn state_with_two_entries(
    calendar: &crate::shell::testing::CalendarStub,
) -> (Arc<AppState>, std::path::PathBuf) {
    let dir = scratch("outage");
    let tokens = crate::shell::testing::TokenStub::start(3600).await;
    let mut profiles = HashMap::new();
    for (source, target) in [("first", "cal-first"), ("second", "cal-second")] {
        let toml = format!(
            "schema_version = 2\nsource_id = \"{source}\"\ntarget_calendar_id = \"{target}\"\n"
        );
        profiles.insert(
            source.to_string(),
            crate::core::profile::Profile::parse(&toml, "t.toml").unwrap(),
        );
    }
    let state = Arc::new(AppState::new_for_test(
        profiles,
        Journal::new(dir.join("journal.jsonl"), DEFAULT_MAX_BYTES),
        GoogleCalendarClient::with_base_url(
            reqwest::Client::new(),
            crate::shell::auth::TokenManager::new(
                reqwest::Client::new(),
                crate::shell::testing::stub_credentials(&tokens.url),
            ),
            &calendar.base_url,
        ),
    ));
    for source in ["first", "second"] {
        let entry = Entry {
            id: source.to_string(),
            source_id: source.to_string(),
            received_at: "2026-08-29T09:00:00+00:00".to_string(),
            payload: serde_json::json!({"title": "t", "start": "2026-08-29T09:00:00+00:00"}),
            idempotency_key: None,
        };
        state.journal.accept(&entry).await.unwrap();
    }
    (state, dir)
}

#[tokio::test(start_paused = true)]
async fn a_pass_stops_after_the_first_transient_failure() {
    // During an outage every entry would spend its own minute of
    // in-call retry; the first transient failure already says Google
    // is unreachable, so the rest waits for the next (backed-off) pass.
    let calendar = crate::shell::testing::CalendarStub::start().await;
    calendar.fail_next(9_999);
    let (state, dir) = state_with_two_entries(&calendar).await;

    let outcome = drain_once_detailed(&state).await;

    assert_eq!(outcome.delivered, 0);
    assert_eq!(outcome.failed, 1, "only the first entry was attempted");
    let requests = calendar.state.requests.lock().await.clone();
    assert!(
        !requests.is_empty() && requests.iter().all(|(_, c)| c != "cal-second"),
        "the second entry must not have reached Google: {requests:?}"
    );
    assert_eq!(
        state.journal.pending().unwrap().len(),
        2,
        "both stay pending"
    );

    std::fs::remove_dir_all(&dir).ok();
}
