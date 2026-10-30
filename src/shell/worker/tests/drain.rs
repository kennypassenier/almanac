//! Tests of draining the journal and the shutdown behaviour.

use super::*;

#[tokio::test]
async fn the_worker_returns_promptly_when_shutdown_is_signalled() {
    // M2: without this the process would hang on the worker handle
    // after the HTTP server stopped, and systemd would eventually
    // SIGKILL it — losing exactly the graceful drain the shutdown
    // path exists to perform.
    let (state, dir) = state_with_empty_journal();
    let (tx, rx) = watch::channel(false);

    let worker = tokio::spawn(run(state, rx, crate::shell::notify::Notifier::disabled()));
    tx.send(true).unwrap();

    let result = tokio::time::timeout(Duration::from_secs(5), worker).await;
    assert!(
        result.is_ok(),
        "the worker must return on the shutdown signal, not wait out its poll interval"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn draining_an_empty_journal_delivers_nothing_and_does_not_error() {
    let (state, dir) = state_with_empty_journal();
    assert_eq!(drain_once(&state).await, 0);
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_drain_reports_failures_so_the_loop_can_back_off() {
    // AR26: without a failure count the loop would keep hammering
    // every five seconds through a multi-hour Google outage.
    let (state, dir) = state_with_empty_journal();
    state
        .journal
        .accept(&crate::core::journal::Entry {
            id: "orphan".to_string(),
            source_id: "profile-that-no-longer-exists".to_string(),
            received_at: "2026-08-28T09:00:00+00:00".to_string(),
            payload: serde_json::json!({"title": "t"}),
            idempotency_key: None,
        })
        .await
        .unwrap();

    let outcome = drain_once_detailed(&state).await;
    assert_eq!(outcome.delivered, 0);
    assert_eq!(
        outcome.failed, 1,
        "the loop must be able to see the failure"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn an_empty_journal_reports_neither_delivery_nor_failure() {
    // An idle hub must not look like an outage, or it would back
    // off to half-hour polls for no reason.
    let (state, dir) = state_with_empty_journal();
    let outcome = drain_once_detailed(&state).await;
    assert_eq!(outcome.delivered, 0);
    assert_eq!(outcome.failed, 0);
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn an_entry_whose_profile_is_gone_is_left_pending_rather_than_dropped() {
    // Losing a payload because someone renamed a profile would be
    // exactly the silent data loss the journal exists to prevent.
    let (state, dir) = state_with_empty_journal();
    state
        .journal
        .accept(&crate::core::journal::Entry {
            id: "orphan".to_string(),
            source_id: "profile-that-no-longer-exists".to_string(),
            received_at: "2026-08-28T09:00:00+00:00".to_string(),
            payload: serde_json::json!({"title": "t"}),
            idempotency_key: None,
        })
        .await
        .unwrap();

    assert_eq!(drain_once(&state).await, 0, "nothing can be delivered");
    assert_eq!(
        state.journal.pending().unwrap().len(),
        1,
        "but the payload must still be there"
    );

    std::fs::remove_dir_all(&dir).ok();
}
