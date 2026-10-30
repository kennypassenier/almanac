use super::*;
use serde_json::json;

fn temp_journal(name: &str) -> (Journal, PathBuf) {
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

fn entry(id: &str) -> Entry {
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
