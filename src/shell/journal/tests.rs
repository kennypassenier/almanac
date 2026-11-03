use super::testing::{journal_in, temp_journal};
use super::*;
use serde_json::json;

pub(super) fn entry(id: &str) -> Entry {
    Entry {
        id: id.to_string(),
        source_id: "home-assistant".to_string(),
        received_at: "2026-08-28T09:00:00+00:00".to_string(),
        payload: json!({"title": "t"}),
        idempotency_key: None,
    }
}

#[tokio::test]
async fn an_accepted_entry_is_pending_until_marked_done() {
    let (journal, dir) = temp_journal("accept");
    journal.accept(&entry("a")).await.unwrap();
    assert_eq!(journal.pending().unwrap().len(), 1);
    journal.mark_done("a").await.unwrap();
    assert!(journal.pending().unwrap().is_empty());
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn entries_replay_in_acceptance_order() {
    let (journal, dir) = temp_journal("order");
    for id in ["a", "b", "c"] {
        journal.accept(&entry(id)).await.unwrap();
    }
    let ids: Vec<String> = journal
        .pending()
        .unwrap()
        .into_iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(ids, ["a", "b", "c"]);
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn the_old_journal_file_is_imported_once_with_a_backup() {
    // Kenny, form journal-store (2026-11-03): journal.jsonl moves into the
    // database; what is owed comes along, delivered entries do not, and the
    // file is kept twice (.pre-db and .migrated). A torn final line is the
    // usual crash damage and is skipped.
    let dir = std::env::temp_dir().join(format!("almanac-journal-import-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let line = |r: serde_json::Value| format!("{r}\n");
    let text = [
        line(json!({"kind": "entry", "id": "a", "source_id": "ha", "received_at": "2026-11-01T09:00:00Z", "payload": {"title": "owed"}})),
        line(json!({"kind": "entry", "id": "b", "source_id": "ha", "received_at": "2026-11-01T09:01:00Z", "payload": {"title": "sent"}})),
        line(json!({"kind": "done", "id": "b"})),
        line(json!({"kind": "entry", "id": "c", "source_id": "ha", "received_at": "2026-11-01T09:02:00Z", "payload": {"title": "aside"}})),
        line(json!({"kind": "dead", "id": "c", "reason": "refused", "at": "2026-11-01T10:00:00Z"})),
    ]
    .concat()
        + r#"{"kind":"entry","id":"torn"#;
    std::fs::write(dir.join("journal.jsonl"), &text).unwrap();

    let journal = journal_in(&dir);

    let pending: Vec<String> = journal
        .pending()
        .unwrap()
        .into_iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(pending, ["a"]);
    let dead = journal.dead().unwrap();
    assert_eq!(
        (dead[0].0.id.as_str(), dead[0].1.as_str()),
        ("c", "refused")
    );
    assert!(!dir.join("journal.jsonl").exists());
    assert_eq!(
        std::fs::read_to_string(dir.join("journal.jsonl.pre-db")).unwrap(),
        text
    );
    assert!(dir.join("journal.jsonl.migrated").exists());
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_damaged_line_in_the_middle_refuses_the_import_and_moves_nothing() {
    let dir = std::env::temp_dir().join(format!("almanac-journal-damaged-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("journal.jsonl"),
        "not json\n{\"kind\":\"done\",\"id\":\"x\"}\n",
    )
    .unwrap();
    let legacy = dir.join("journal.jsonl");
    let opened = chassis::storage::AppDb::open(
        &dir.join("almanac.db"),
        crate::shell::history_store::app_db_spec(legacy.clone()),
        Vec::new(),
    );
    assert!(
        opened.is_err(),
        "a database that half imported a journal must not open"
    );
    assert!(legacy.exists(), "the old file stays where it was");
    std::fs::remove_dir_all(&dir).ok();
}

/// A journal holding entries `ids` set aside, with a reason.
async fn journal_with_dead(name: &str, ids: &[&str]) -> (Journal, PathBuf) {
    let (journal, dir) = temp_journal(name);
    for id in ids {
        let mut e = entry(id);
        e.idempotency_key = Some(format!("key-{id}"));
        journal.accept(&e).await.unwrap();
        journal
            .mark_dead(id, "title is missing", "2026-08-29T09:00:00+00:00")
            .await
            .unwrap();
    }
    (journal, dir)
}

#[tokio::test]
async fn retrying_a_set_aside_entry_queues_a_new_one_and_retires_the_old() {
    let (journal, dir) = journal_with_dead("retry", &["a", "b"]).await;
    let n = journal
        .retry_dead(&["a".to_string()], "2026-11-01T10:00:00+00:00")
        .await
        .unwrap();
    assert_eq!(n, 1);
    let pending = journal.pending().unwrap();
    assert_eq!(pending.len(), 1);
    assert_ne!(pending[0].id, "a", "the retry has a fresh id");
    assert_eq!(pending[0].payload, json!({"title": "t"}));
    assert_eq!(pending[0].idempotency_key.as_deref(), Some("key-a"));
    assert_eq!(pending[0].received_at, "2026-11-01T10:00:00+00:00");
    let dead = journal.dead().unwrap();
    assert_eq!(dead.len(), 1, "only b is still set aside");
    assert_eq!(dead[0].0.id, "b");
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn dismissing_removes_it_from_the_list() {
    let (journal, dir) = journal_with_dead("dismiss", &["a", "b"]).await;
    let n = journal
        .dismiss_dead(&["a".to_string(), "a".to_string()])
        .await
        .unwrap();
    assert_eq!(n, 1, "a repeated id counts once");
    assert!(
        journal.pending().unwrap().is_empty(),
        "dismiss queues nothing"
    );
    let dead = journal.dead().unwrap();
    assert_eq!(dead.len(), 1);
    assert_eq!(dead[0].0.id, "b");
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn an_id_that_is_not_set_aside_refuses_the_whole_call_and_writes_nothing() {
    let (journal, dir) = journal_with_dead("refuse", &["a"]).await;
    journal.accept(&entry("waiting")).await.unwrap();
    for ids in [vec!["a", "waiting"], vec!["a", "nope"]] {
        let ids: Vec<String> = ids.into_iter().map(String::from).collect();
        match journal.retry_dead(&ids, "t").await {
            Err(RequeueError::NotSetAside(refused)) => assert_eq!(refused, vec![ids[1].clone()]),
            other => panic!("expected a refusal, got {other:?}"),
        }
        assert!(matches!(
            journal.dismiss_dead(&ids).await,
            Err(RequeueError::NotSetAside(_))
        ));
    }
    assert_eq!(journal.dead().unwrap().len(), 1, "a is still set aside");
    assert_eq!(journal.pending().unwrap().len(), 1, "nothing was queued");
    journal.retry_dead(&["a".to_string()], "t").await.unwrap();
    assert!(matches!(
        journal.retry_dead(&["a".to_string()], "t").await,
        Err(RequeueError::NotSetAside(_))
    ));
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn set_aside_entries_go_after_thirty_days() {
    let (journal, dir) = journal_with_dead("expire", &["a"]).await;
    let set_aside = crate::core::history::epoch_of("2026-08-29T09:00:00+00:00").unwrap();
    assert_eq!(
        journal.drop_expired(set_aside + 29 * 86_400).await.unwrap(),
        0
    );
    assert_eq!(
        journal.drop_expired(set_aside + 31 * 86_400).await.unwrap(),
        1
    );
    assert!(journal.dead().unwrap().is_empty());
    std::fs::remove_dir_all(&dir).ok();
}
