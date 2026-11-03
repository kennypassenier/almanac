//! The schema step that creates the journal table and moves the old
//! `journal.jsonl` into it, once, in the same transaction (Kenny, form
//! journal-store, 2026-11-03).
//!
//! Order, so nothing is lost whatever fails: copy the file to
//! `journal.jsonl.pre-db` (the backup), insert what is still owed, then
//! rename the file to `journal.jsonl.migrated`. If the transaction does
//! not commit, the schema step runs again at the next start and reads the
//! renamed file; the backup stays either way.

use std::path::{Path, PathBuf};

use chassis::storage::Migration;
use chassis::storage::rusqlite::{Transaction, params};

use crate::core::journal::{self, Record};

use super::rows;

/// The schema step for the app database's migration list.
pub fn migration(legacy_file: PathBuf) -> Migration {
    Migration::code(move |tx| {
        tx.execute_batch(rows::SCHEMA)?;
        import(tx, &legacy_file)
    })
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Imports the old file when there is one (or the renamed copy of an
/// earlier attempt that did not commit).
fn import(tx: &Transaction, legacy: &Path) -> Result<(), chassis::Error> {
    let migrated = with_suffix(legacy, ".migrated");
    let source = if legacy.exists() {
        legacy.to_path_buf()
    } else if migrated.exists() {
        migrated.clone()
    } else {
        return Ok(());
    };
    let text = std::fs::read_to_string(&source).map_err(|e| io_error(&source, e))?;
    if source == legacy {
        let backup = with_suffix(legacy, ".pre-db");
        std::fs::copy(legacy, &backup).map_err(|e| io_error(&backup, e))?;
    }
    let records = parse(&text, &source)?;
    let (owed, set_aside) = insert_all(tx, &records)?;
    if source == legacy {
        std::fs::rename(legacy, &migrated).map_err(|e| io_error(&migrated, e))?;
    }
    tracing::info!(
        pending = owed, set_aside, file = %legacy.display(),
        "moved the journal file into the database; the old file is kept as .pre-db and .migrated"
    );
    Ok(())
}

/// Inserts the pending entries and the set-aside ones; returns both counts.
fn insert_all(tx: &Transaction, records: &[Record]) -> Result<(usize, usize), chassis::Error> {
    let pending = journal::pending(records);
    for entry in &pending {
        rows::insert(tx, entry)?;
    }
    let dead = journal::dead(records);
    for (entry, reason, at) in &dead {
        rows::insert(tx, entry)?;
        tx.execute(
            "UPDATE journal SET state = 'dead', reason = ?2, set_aside_at = ?3,
                set_aside_unix = ?4 WHERE id = ?1",
            params![
                entry.id,
                reason,
                at,
                crate::core::history::epoch_of(at).unwrap_or(0)
            ],
        )?;
    }
    Ok((pending.len(), dead.len()))
}

/// Every intact record; a torn final line (a crash mid-append) is skipped,
/// any other unreadable line refuses the import so nothing is half moved.
fn parse(text: &str, source: &Path) -> Result<Vec<Record>, chassis::Error> {
    let ends_cleanly = text.is_empty() || text.ends_with('\n');
    let lines: Vec<&str> = text.lines().collect();
    let mut records = Vec::with_capacity(lines.len());
    for (i, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Record>(line) {
            Ok(record) => records.push(record),
            Err(_) if !ends_cleanly && i + 1 == lines.len() => {}
            Err(e) => {
                return Err(chassis::Error::internal(
                    format!("{} line {} is unreadable: {e}", source.display(), i + 1),
                    "move the file aside to inspect it before restarting; the database was \
                     left unchanged",
                ));
            }
        }
    }
    Ok(records)
}

fn io_error(path: &Path, e: std::io::Error) -> chassis::Error {
    chassis::Error::internal(
        format!("the old journal {} could not be moved: {e}", path.display()),
        "check free disk space and permissions on the state folder; nothing was changed",
    )
}
