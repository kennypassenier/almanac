//! The journal's repairs: a torn tail, a lost newline, a full journal.

use std::io::Write;

use super::Journal;
use super::tests::{entry, temp_journal};

#[tokio::test]
async fn an_append_after_a_torn_tail_keeps_the_journal_readable() {
    // Robustness review 2026-11-02: a crash left `{"kind":"entry",...` with
    // no newline, the next append glued onto it, and the file then held an
    // unparseable middle line that stopped every read and the next start.
    let (journal, dir) = temp_journal("torn-append");
    journal.accept(&entry("a")).await.unwrap();
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(journal.path())
        .unwrap();
    file.write_all(br#"{"kind":"entry","id":"torn","sou"#)
        .unwrap();
    drop(file);

    journal.accept(&entry("b")).await.unwrap();

    let ids: Vec<String> = journal
        .pending()
        .unwrap()
        .into_iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(
        ids,
        ["a", "b"],
        "the torn line is cut and both real entries read back"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_full_journal_still_takes_the_markers_that_free_it() {
    // A journal at its cap refused Done markers too, so delivered entries
    // stayed pending and were sent again on every pass.
    let (journal, dir) = temp_journal("cap-markers");
    journal.accept(&entry("a")).await.unwrap();
    let full = Journal::new(
        journal.path().to_path_buf(),
        std::fs::metadata(journal.path()).unwrap().len(),
    );
    assert!(
        full.accept(&entry("b")).await.is_err(),
        "a new entry is refused at the cap"
    );
    full.mark_done("a").await.unwrap();
    assert!(full.pending().unwrap().is_empty());
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_whole_last_record_that_only_lost_its_newline_is_kept() {
    // Review 2026-11-02: the repair cut any final line without a newline,
    // so a complete Done marker could vanish and its entry be sent again.
    let (journal, dir) = temp_journal("lost-newline");
    journal.accept(&entry("a")).await.unwrap();
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(journal.path())
        .unwrap();
    file.write_all(br#"{"kind":"done","id":"a"}"#).unwrap();
    drop(file);

    journal.accept(&entry("b")).await.unwrap();

    let ids: Vec<String> = journal
        .pending()
        .unwrap()
        .into_iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(ids, ["b"], "the Done marker for a survives the repair");
    std::fs::remove_dir_all(&dir).ok();
}
