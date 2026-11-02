//! Newer versions win (Kenny, form newer-wins, 2026-11-02).

use serde_json::json;

use super::tests::temp_journal;
use super::{Entry, RequeueError};

fn version(id: &str, title: &str) -> Entry {
    Entry {
        id: id.to_string(),
        source_id: "home-assistant".to_string(),
        received_at: "2026-11-02T03:00:00+00:00".to_string(),
        payload: json!({"title": title, "external_id": "bin-day"}),
        idempotency_key: None,
    }
}

#[tokio::test]
async fn delivering_a_version_closes_the_older_one_still_waiting() {
    // v1 failed for a moment, v2 went through; v1 must not be sent after it.
    let (journal, dir) = temp_journal("supersede");
    journal
        .accept(&version("v1", "Afvalophaling 08:00"))
        .await
        .unwrap();
    journal
        .accept(&version("v2", "Afvalophaling 07:30"))
        .await
        .unwrap();
    journal.mark_done("v2").await.unwrap();

    assert_eq!(
        journal.supersede_older(&version("v2", "")).await.unwrap(),
        1
    );
    assert!(journal.pending().unwrap().is_empty());
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_set_aside_version_with_a_newer_one_after_it_is_not_retried() {
    let (journal, dir) = temp_journal("superseded-retry");
    journal.accept(&version("v1", "old")).await.unwrap();
    journal
        .mark_dead("v1", "refused", "2026-11-02T03:01:00+00:00")
        .await
        .unwrap();
    journal.accept(&version("v2", "new")).await.unwrap();

    let refused = journal
        .retry_dead(&["v1".to_string()], "2026-11-02T03:02:00+00:00")
        .await;
    assert!(matches!(refused, Err(RequeueError::Superseded(ids)) if ids == ["v1"]));
    std::fs::remove_dir_all(&dir).ok();
}
