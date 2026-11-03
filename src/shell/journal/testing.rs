//! A journal on a fresh app database in a temp folder, for unit tests.

use std::path::PathBuf;

use super::Journal;

/// A journal attached to its own `almanac.db` in a new temp folder, with
/// the full schema; returns the folder so the test can remove it.
pub(crate) fn temp_journal(name: &str) -> (Journal, PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "almanac-journal-{}-{}-{name}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    (journal_in(&dir), dir)
}

/// A journal on `dir/almanac.db`, importing `dir/journal.jsonl` if present.
pub(crate) fn journal_in(dir: &std::path::Path) -> Journal {
    Journal::open_in(dir).expect("the test database opens")
}
