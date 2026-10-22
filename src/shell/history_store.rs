//! The database side of the per-day delivery history (`core::history`).
//!
//! The history is the `deliveries` table of the kit's app database
//! (`<state_dir>/almanac.db`, chassis-rs 3.11.0): one row per final outcome
//! of a journal entry. The kit's Storage page lists it as "Delivery log"
//! (size, what is stored, clean up) and prunes the rows older than
//! [`history::MAX_DAYS`] days on its own. Before 4.4.0 this was `history.json`
//! beside the journal; [`import_from`] moves that file in once.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use chassis::storage::rusqlite::{Transaction, params};
use chassis::storage::{
    AppDbSpec, DataKind, Filter, ImportContext, ImportReport, ImportStore, Migration, TimeFormat,
};

use crate::core::error::AlmanacError;
use crate::core::history::{
    self, Book, DayCount, DeliveryRecord, DeliveryResult, MAX_DELIVERIES_PER_DAY, Row,
};
use crate::shell::delivery::Delivered;
use crate::shell::ingest::AppState;

/// The old file, beside the journal in the state directory.
const OLD_FILE: &str = "history.json";
/// The store's name in the import report.
const STORE: &str = "delivery history";

const INSERT: &str = "INSERT OR REPLACE INTO deliveries
    (id, delivered_at, day, source, calendar, title, result, reason, upsert_key)
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)";

const SELECT_ROW: &str = "SELECT id, delivered_at, day, source, calendar, title, result, reason, upsert_key FROM deliveries";

/// The bound parameters of a [`Row`], in the order of [`INSERT`].
macro_rules! row_params {
    ($r:expr) => {
        params![
            $r.id,
            $r.delivered_at,
            $r.day,
            $r.source,
            $r.calendar,
            $r.title,
            $r.result.as_str(),
            $r.reason,
            $r.upsert_key,
        ]
    };
}

/// The schema, append only. Counters per day are counted from the rows
/// (`day` is indexed), so a clean-up on the Storage page keeps the heatmap
/// true and there is no second table to drift.
pub fn migrations() -> Vec<Migration> {
    vec![Migration::sql(
        "CREATE TABLE deliveries (
            id TEXT PRIMARY KEY,
            delivered_at INTEGER NOT NULL,
            day TEXT NOT NULL,
            source TEXT NOT NULL,
            calendar TEXT NOT NULL,
            title TEXT NOT NULL,
            result TEXT NOT NULL CHECK (result IN ('created', 'updated', 'failed')),
            reason TEXT,
            upsert_key TEXT
        ) STRICT;
        CREATE INDEX deliveries_day ON deliveries (day);
        CREATE INDEX deliveries_upsert_key ON deliveries (upsert_key, delivered_at);
        CREATE INDEX deliveries_delivered_at ON deliveries (delivered_at);",
    )]
}

/// The app database spec: the schema and the one-time import.
pub fn app_db_spec() -> AppDbSpec {
    AppDbSpec::new(migrations())
        .with_import(|tx, ctx: &ImportContext| import_from(tx, &ctx.state_dir))
}

/// The Storage page's "Delivery log". Rows older than [`history::MAX_DAYS`] days
/// are finished by their own definition and pruned daily by the kit;
/// anything else goes through the page's preview and typed confirmation.
pub fn kind() -> DataKind {
    DataKind::new("deliveries", "Delivery log")
        .description(
            "Every event Almanac finished with, delivered or set aside as failed: \
             what the History page shows. Rows older than 400 days are removed on their own.",
        )
        .unit("deliveries")
        .table("deliveries")
        .time_column("delivered_at", TimeFormat::UnixSeconds)
        .filter(Filter::choice(
            "result",
            "Result",
            "result",
            ["created", "updated", "failed"],
        ))
        .filter(Filter::text("source", "Source", "source"))
        .finished_by_definition(&format!(
            "delivered_at < :now - {}",
            history::retention_secs()
        ))
}

/// Inserts or replaces one delivery (a retried entry replaces its earlier
/// record), then keeps the day within [`MAX_DELIVERIES_PER_DAY`] rows.
pub fn insert(tx: &Transaction, row: &Row) -> Result<(), chassis::Error> {
    tx.execute(INSERT, row_params!(row))?;
    tx.execute(
        "DELETE FROM deliveries WHERE day = ?1 AND rowid NOT IN
            (SELECT rowid FROM deliveries WHERE day = ?1 ORDER BY rowid DESC LIMIT ?2)",
        params![row.day, MAX_DELIVERIES_PER_DAY as i64],
    )?;
    Ok(())
}

/// The kit's one-time import hook body: reads `<dir>/history.json`, writes
/// every row in the kit's transaction and reports what the file held and
/// what was written (the kit rolls everything back when they differ).
///
/// * No file: nothing to do, counts 0/0.
/// * An unparseable file: logged and reported as 0/0 and moved, so the kit
///   renames it `.migrated`; its bytes stay for a human, like the old
///   `history.json.bad`.
/// * An existing row of the same entry (a delivery recorded after an
///   earlier failed import) is the newer truth and is kept.
pub fn import_from(tx: &Transaction, dir: &Path) -> Result<ImportReport, chassis::Error> {
    let path = dir.join(OLD_FILE);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ImportReport::from_stores(Vec::new()));
        }
        Err(e) => {
            return Err(chassis::Error::internal(
                format!("cannot read {}: {e}", path.display()),
                "check that the file is readable by the almanac user; the import is retried at the next start",
            ));
        }
    };
    let book: Book = match serde_json::from_slice(&bytes) {
        Ok(book) => book,
        Err(e) => {
            tracing::error!(
                error = %e,
                file = %path.display(),
                "the old delivery history is unreadable; keeping it aside and starting a new one"
            );
            return Ok(ImportReport::from_stores(vec![
                ImportStore::new(STORE, 0, 0).moved(vec![path]),
            ]));
        }
    };
    let rows = history::rows_from_book(&book)
        .map_err(|why| chassis::Error::internal(why, "fix or move history.json, then restart"))?;
    let expected = rows.len() as u64;
    let mut imported = 0u64;
    {
        let mut stmt = tx.prepare(&INSERT.replace("OR REPLACE", "OR IGNORE"))?;
        for row in &rows {
            stmt.execute(row_params!(row))?;
            imported += 1;
        }
    }
    Ok(ImportReport::from_stores(vec![
        ImportStore::new(STORE, expected, imported).moved(vec![path]),
    ]))
}

/// A plain sentence for a failure, never the raw error.
fn plain_reason(e: &AlmanacError) -> &'static str {
    match e {
        AlmanacError::GoogleApi { .. } | AlmanacError::Auth { .. } => {
            "Google Calendar would not take this event."
        }
        _ => "The event could not be built from what the source sent.",
    }
}

/// The hook `worker::record_route` calls after every delivery attempt.
/// Only final outcomes are kept: a delivery, or a failure that will not
/// fix itself. A transient failure stays pending and is retried, so it is
/// not an outcome yet. Never fails the delivery: a history problem is logged.
pub async fn note(
    state: &AppState,
    entry: &crate::core::journal::Entry,
    result: &Result<Delivered, AlmanacError>,
) {
    let (outcome, reason, upsert_key) = match result {
        Ok(d) if d.created => (DeliveryResult::Created, None, d.upsert_key.clone()),
        Ok(d) => (DeliveryResult::Updated, None, d.upsert_key.clone()),
        Err(e) if e.is_transient() => return,
        Err(e) => (
            DeliveryResult::Failed,
            Some(plain_reason(e).to_string()),
            None,
        ),
    };
    let profile = state.profiles().get(&entry.source_id).cloned();
    let title = profile
        .as_ref()
        .and_then(|p| crate::core::mapping::map_payload(&entry.payload, p, "history").ok())
        .map(|event| event.summary)
        .unwrap_or_else(|| "Untitled event".to_string());
    let rec = DeliveryRecord {
        at: (state.now)(),
        entry_id: entry.id.clone(),
        source: entry.source_id.clone(),
        calendar_id: profile.map(|p| p.target_calendar_id).unwrap_or_default(),
        title,
        result: outcome,
        reason,
        upsert_key,
    };
    let Some(row) = Row::from_record(&rec) else {
        tracing::warn!(entry = %rec.entry_id, "the delivery has an unreadable timestamp; not recorded in the history");
        return;
    };
    // The write waits for a backup pause by itself (the kit's ticket).
    let written = match state.app_db() {
        Ok(db) => db.write(move |tx| insert(tx, &row)).await,
        Err(e) => Err(e),
    };
    if let Err(e) = written {
        tracing::warn!(error = %e, remedy = %e.remedy, "could not record the delivery history");
    }
}

/// Every day's counters, oldest first: the heatmap reads them in one go.
pub async fn day_counts(state: &AppState) -> Result<Vec<DayCount>, chassis::Error> {
    state
        .app_db()?
        .read(|c| {
            let mut stmt = c.prepare(
                "SELECT day, result, COUNT(*) FROM deliveries GROUP BY day, result ORDER BY day",
            )?;
            let mut by_day: BTreeMap<String, Vec<(DeliveryResult, u32)>> = BTreeMap::new();
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, u32>(2)?,
                ))
            })?;
            for row in rows {
                let (day, word, n) = row?;
                if let Some(result) = DeliveryResult::parse(&word) {
                    by_day.entry(day).or_default().push((result, n));
                }
            }
            Ok(by_day
                .into_iter()
                .map(|(day, groups)| history::count_day(&day, groups))
                .collect())
        })
        .await
}

/// One day's rows in the order they were recorded.
pub async fn day_rows(state: &AppState, date: &str) -> Result<Vec<DeliveryRecord>, chassis::Error> {
    let date = date.to_string();
    state
        .app_db()?
        .read(move |c| {
            let mut stmt = c.prepare(&format!("{SELECT_ROW} WHERE day = ?1 ORDER BY rowid"))?;
            let rows = stmt.query_map([&date], read_row)?;
            Ok(rows
                .filter_map(Result::ok)
                .flatten()
                .map(|r| r.to_record())
                .collect())
        })
        .await
}

/// The newest record per upsert key, for the week agenda's "delivered" line.
pub async fn latest_by_key(
    state: &AppState,
    keys: Vec<String>,
) -> Result<HashMap<String, DeliveryRecord>, chassis::Error> {
    state
        .app_db()?
        .read(move |c| {
            let mut stmt = c.prepare(&format!(
                "{SELECT_ROW} WHERE upsert_key = ?1 ORDER BY delivered_at DESC, rowid DESC LIMIT 1"
            ))?;
            let mut out = HashMap::new();
            for key in keys {
                let newest = stmt
                    .query_map([&key], read_row)?
                    .filter_map(Result::ok)
                    .flatten()
                    .next();
                if let Some(row) = newest {
                    out.insert(key, row.to_record());
                }
            }
            Ok(out)
        })
        .await
}

/// A row of [`SELECT_ROW`]; `None` for a result word this build does not know.
fn read_row(
    r: &chassis::storage::rusqlite::Row,
) -> chassis::storage::rusqlite::Result<Option<Row>> {
    let word: String = r.get(6)?;
    let Some(result) = DeliveryResult::parse(&word) else {
        return Ok(None);
    };
    Ok(Some(Row {
        id: r.get(0)?,
        delivered_at: r.get(1)?,
        day: r.get(2)?,
        source: r.get(3)?,
        calendar: r.get(4)?,
        title: r.get(5)?,
        result,
        reason: r.get(7)?,
        upsert_key: r.get(8)?,
    }))
}

#[cfg(test)]
mod tests {
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
}
