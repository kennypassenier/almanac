use std::io::Write;

use super::*;
use serde_json::json;

pub(super) fn temp_journal(name: &str) -> (Journal, PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "almanac-journal-{}-{}-{name}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("journal.jsonl");
    (Journal::new(path.clone(), DEFAULT_MAX_BYTES), dir)
}

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
async fn a_journal_that_does_not_exist_yet_has_nothing_pending() {
    let (journal, dir) = temp_journal("missing");
    assert!(journal.pending().unwrap().is_empty());
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn replay_survives_a_torn_final_line() {
    // Exactly what a power cut mid-append leaves behind.
    let (journal, dir) = temp_journal("torn");
    journal.accept(&entry("a")).await.unwrap();

    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(journal.path())
        .unwrap();
    file.write_all(b"{\"kind\":\"entry\",\"id\":\"b\",\"sou")
        .unwrap();
    drop(file);

    let pending = journal.pending().unwrap();
    assert_eq!(pending.len(), 1, "the intact entry must survive");
    assert_eq!(pending[0].id, "a");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_damaged_line_that_is_not_the_tail_is_reported_not_silently_skipped() {
    let (journal, dir) = temp_journal("damaged");
    journal.accept(&entry("a")).await.unwrap();

    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(journal.path())
        .unwrap();
    file.write_all(b"garbage that is not json\n").unwrap();
    file.write_all(b"{\"kind\":\"done\",\"id\":\"a\"}\n")
        .unwrap();
    drop(file);

    let err = journal.pending().unwrap_err();
    assert!(err.to_string().contains("unparseable"));
    assert!(err.remedy().contains("move it aside"));

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn compaction_keeps_pending_entries_and_drops_delivered_ones() {
    let (journal, dir) = temp_journal("compact");

    journal.accept(&entry("a")).await.unwrap();
    journal.accept(&entry("b")).await.unwrap();
    journal.mark_done("a").await.unwrap();

    let before = std::fs::metadata(journal.path()).unwrap().len();
    let kept = journal.compact().await.unwrap();
    let after = std::fs::metadata(journal.path()).unwrap().len();

    assert_eq!(kept, 1);
    assert!(after < before, "compaction should shrink the file");

    let pending = journal.pending().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, "b");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn the_size_cap_refuses_loudly_instead_of_filling_the_disk() {
    let (_, dir) = temp_journal("cap");
    let journal = Journal::new(dir.join("journal.jsonl"), 200);

    journal.accept(&entry("a")).await.unwrap();
    let err = loop {
        match journal.accept(&entry("filler")).await {
            Ok(()) => continue,
            Err(e) => break e,
        }
    };

    assert!(err.to_string().contains("cap"));
    assert!(err.remedy().contains("failing"));

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn entries_replay_in_acceptance_order() {
    let (journal, dir) = temp_journal("order");
    for id in ["a", "b", "c"] {
        journal.accept(&entry(id)).await.unwrap();
    }
    let pending = journal.pending().unwrap();
    let ids: Vec<_> = pending.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, vec!["a", "b", "c"]);
    std::fs::remove_dir_all(&dir).ok();
}

/// A journal holding entry `id` set aside, with a reason.
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
    assert_eq!(pending[0].source_id, "home-assistant");
    assert_eq!(pending[0].payload, json!({"title": "t"}));
    assert_eq!(pending[0].idempotency_key.as_deref(), Some("key-a"));
    assert_eq!(pending[0].received_at, "2026-11-01T10:00:00+00:00");
    let dead = journal.dead().unwrap();
    assert_eq!(dead.len(), 1, "only b is still set aside");
    assert_eq!(dead[0].0.id, "b");
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn dismissing_removes_it_from_the_list_and_compaction_drops_its_payload() {
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
    assert_eq!(journal.dead().unwrap().len(), 1);
    // The day after the fixture set them aside, well inside 30 days.
    journal.compact_at(1_788_080_400).await.unwrap();
    let text = std::fs::read_to_string(journal.path()).unwrap();
    assert!(!text.contains("\"id\":\"a\""), "the payload went: {text}");
    assert!(text.contains("\"id\":\"b\""));
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn an_id_that_is_not_set_aside_refuses_the_whole_call_and_writes_nothing() {
    let (journal, dir) = journal_with_dead("refuse", &["a"]).await;
    journal.accept(&entry("waiting")).await.unwrap();
    let before = std::fs::read_to_string(journal.path()).unwrap();

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

    assert_eq!(std::fs::read_to_string(journal.path()).unwrap(), before);
    // A second retry of an id that was already retried is refused too.
    journal.retry_dead(&["a".to_string()], "t").await.unwrap();
    assert!(matches!(
        journal.retry_dead(&["a".to_string()], "t").await,
        Err(RequeueError::NotSetAside(_))
    ));
    std::fs::remove_dir_all(&dir).ok();
}
