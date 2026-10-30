//! The one-time import of the old `history.json` into the `deliveries` table.

use std::path::{Path, PathBuf};

use chassis::storage::rusqlite::Transaction;
use chassis::storage::{ImportReport, ImportStore};

use super::rows::{INSERT, row_params};
use crate::core::history::{self, Book, Row};

/// The old file, beside the journal in the state directory.
pub(super) const OLD_FILE: &str = "history.json";
/// The store's name in the import report.
const STORE: &str = "delivery history";

/// The kit's one-time import hook body: reads `<dir>/history.json`, writes
/// every row in the kit's transaction and reports what the file held and
/// what was written (the kit rolls everything back when they differ).
///
/// Deliberately a function of `(tx, dir)` and not of the kit's
/// `ImportContext`: that context cannot be built in a unit test (queued
/// package gap 28), so [`super::app_db_spec`] unpacks it and the tests call
/// this directly.
///
/// * No file: nothing to do, counts 0/0.
/// * An unparseable file: logged and reported as 0/0 and moved, so the kit
///   renames it `.migrated`; its bytes stay for a human, like the old
///   `history.json.bad`.
/// * An existing row of the same entry (a delivery recorded after an
///   earlier failed import) is the newer truth and is kept.
pub fn import_from(tx: &Transaction, dir: &Path) -> Result<ImportReport, chassis::Error> {
    let path = dir.join(OLD_FILE);
    let Some(bytes) = read_old_file(&path)? else {
        return Ok(ImportReport::from_stores(Vec::new()));
    };
    let book: Book = match serde_json::from_slice(&bytes) {
        Ok(book) => book,
        Err(e) => {
            tracing::error!(
                error = %e,
                file = %path.display(),
                "the old delivery history is unreadable; keeping it aside and starting a new one"
            );
            return Ok(report(0, 0, path));
        }
    };
    let rows = history::rows_from_book(&book)
        .map_err(|why| chassis::Error::internal(why, "fix or move history.json, then restart"))?;
    let imported = insert_ignoring_existing(tx, &rows)?;
    Ok(report(rows.len() as u64, imported, path))
}

/// The file's bytes, or `None` when there is no old file.
fn read_old_file(path: &Path) -> Result<Option<Vec<u8>>, chassis::Error> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(chassis::Error::internal(
            format!("cannot read {}: {e}", path.display()),
            "check that the file is readable by the almanac user; the import is retried at the next start",
        )),
    }
}

/// Writes the rows, keeping any row already there; returns how many
/// statements ran.
fn insert_ignoring_existing(tx: &Transaction, rows: &[Row]) -> Result<u64, chassis::Error> {
    let mut stmt = tx.prepare(&INSERT.replace("OR REPLACE", "OR IGNORE"))?;
    let mut imported = 0u64;
    for row in rows {
        stmt.execute(row_params!(row))?;
        imported += 1;
    }
    Ok(imported)
}

/// The import report: the file is handed back to the kit to be renamed.
fn report(expected: u64, imported: u64, path: PathBuf) -> ImportReport {
    ImportReport::from_stores(vec![
        ImportStore::new(STORE, expected, imported).moved(vec![path]),
    ])
}
