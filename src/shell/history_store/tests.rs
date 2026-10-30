use super::*;
use chassis::storage::{AppDb, ImportState, Now};

fn row(id: &str, at: &str, result: DeliveryResult) -> Row {
    Row::from_record(&DeliveryRecord {
        at: at.to_string(),
        entry_id: id.to_string(),
        source: "s".to_string(),
        calendar_id: "c".to_string(),
        title: "t".to_string(),
        result,
        reason: None,
        upsert_key: Some(format!("s:{id}")),
    })
    .unwrap()
}

/// An in-memory app database whose import reads `dir`.
fn open(dir: &Path) -> AppDb {
    let dir = dir.to_path_buf();
    let spec = AppDbSpec::new(migrations()).with_import(move |tx, _| import_from(tx, &dir));
    AppDb::open_in_memory(spec, vec![kind()]).unwrap()
}

fn count(db: &AppDb) -> i64 {
    db.read_blocking(|c| Ok(c.query_row("SELECT COUNT(*) FROM deliveries", [], |r| r.get(0))?))
        .unwrap()
}

fn temp_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("almanac-history-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn old_file(dir: &Path) {
    let book = Book {
        version: history::FORMAT_VERSION,
        days: vec![history::DayRecord {
            date: "2026-10-19".to_string(),
            accepted: 2,
            delivered: 1,
            failed: 1,
            deliveries: vec![
                row("a", "2026-10-19T08:00:00+00:00", DeliveryResult::Created).to_record(),
                row("b", "2026-10-19T09:00:00+00:00", DeliveryResult::Failed).to_record(),
            ],
        }],
    };
    std::fs::write(dir.join(OLD_FILE), serde_json::to_vec(&book).unwrap()).unwrap();
}

#[test]
fn a_retried_entry_replaces_its_earlier_record() {
    let db = open(&temp_dir());
    let first = row("e1", "2026-10-12T10:00:00+00:00", DeliveryResult::Failed);
    let retry = row("e1", "2026-10-13T10:00:00+00:00", DeliveryResult::Created);
    db.write_blocking(|tx| insert(tx, &first)).unwrap();
    db.write_blocking(|tx| insert(tx, &retry)).unwrap();
    assert_eq!(count(&db), 1);
    let day: String = db
        .read_blocking(|c| Ok(c.query_row("SELECT day FROM deliveries", [], |r| r.get(0))?))
        .unwrap();
    assert_eq!(day, "2026-10-13");
}

#[test]
fn a_day_keeps_at_most_the_row_cap_and_drops_the_oldest() {
    let db = open(&temp_dir());
    db.write_blocking(|tx| {
        for i in 0..(MAX_DELIVERIES_PER_DAY + 3) {
            let r = row(
                &format!("e{i}"),
                "2026-10-19T10:00:00+00:00",
                DeliveryResult::Created,
            );
            insert(tx, &r)?;
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(count(&db), MAX_DELIVERIES_PER_DAY as i64);
    let oldest: String = db
        .read_blocking(|c| {
            Ok(c.query_row(
                "SELECT id FROM deliveries ORDER BY rowid LIMIT 1",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(oldest, "e3");
}

#[test]
fn rows_older_than_the_retention_are_finished_by_definition() {
    let db = open(&temp_dir());
    let now = chrono::DateTime::parse_from_rfc3339("2026-10-22T12:00:00+00:00")
        .unwrap()
        .timestamp();
    let old = history::rfc3339_of(now - history::retention_secs() - 60);
    let fresh = history::rfc3339_of(now - 86_400);
    db.write_blocking(|tx| {
        insert(tx, &row("old", &old, DeliveryResult::Created))?;
        insert(tx, &row("fresh", &fresh, DeliveryResult::Created))
    })
    .unwrap();
    db.prune_finished_at(&Now::at(now)).unwrap();
    assert_eq!(count(&db), 1);
}

#[test]
fn the_import_moves_every_row_and_the_old_file() {
    let dir = temp_dir();
    old_file(&dir);
    let db = open(&dir);
    assert!(matches!(db.import_state(), ImportState::Done));
    assert_eq!(count(&db), 2);
    assert!(!dir.join(OLD_FILE).exists(), "the kit renamed the old file");
    assert!(dir.join("history.json.migrated").exists());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_import_reports_what_the_file_held_and_what_was_written() {
    let dir = temp_dir();
    old_file(&dir);
    let db = AppDb::open_in_memory(AppDbSpec::new(migrations()), vec![kind()]).unwrap();
    let report = db.write_blocking(|tx| import_from(tx, &dir)).unwrap();
    assert_eq!(report.stores.len(), 1);
    assert_eq!(
        (report.stores[0].expected, report.stores[0].imported),
        (2, 2)
    );
    assert_eq!(count(&db), 2);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn without_an_old_file_the_import_writes_nothing() {
    let dir = temp_dir();
    let db = open(&dir);
    assert!(matches!(db.import_state(), ImportState::Done));
    assert_eq!(count(&db), 0);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_unreadable_old_file_is_kept_aside_not_imported() {
    let dir = temp_dir();
    std::fs::write(dir.join(OLD_FILE), b"{ not json").unwrap();
    let db = open(&dir);
    assert_eq!(count(&db), 0);
    assert!(dir.join("history.json.migrated").exists());
    std::fs::remove_dir_all(&dir).ok();
}
