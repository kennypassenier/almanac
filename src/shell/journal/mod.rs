//! The durable ingest journal (AR16), a table in the app database since
//! 2026-11-03 (Kenny, form journal-store: one row per event). Every
//! accepted payload is a committed row *before* the source is told the
//! request succeeded: the kit's database writes with WAL and
//! `synchronous=FULL`, so a crash or power cut between "accepted" and
//! "delivered to Google" loses nothing, and replay-on-start re-delivers.
//! Upsert (K2/AR15) plus idempotency keys (M7) make that converge.
//!
//! A row is `pending` until delivered, then deleted (the delivery history
//! keeps the record), or `dead` when set aside, until it is retried,
//! dismissed or 30 days old. The old `journal.jsonl` is imported once by
//! the schema step in [`migrate`].

use std::path::PathBuf;
use std::sync::OnceLock;

use chassis::storage::AppDb;
use chassis::storage::rusqlite::params;
use tokio::sync::Mutex;

use crate::core::error::AlmanacError;
use crate::core::journal::{Entry, entry_key};

mod expire;
pub mod migrate;
mod requeue;
mod rows;

pub use requeue::RequeueError;

pub struct Journal {
    /// The 2.x/6.x file, read once by the import and then renamed.
    legacy_file: PathBuf,
    /// The app database, attached when the kit hands it over.
    db: OnceLock<AppDb>,
    /// Serializes retry and dismiss, which read the set-aside list and
    /// then write: two clicks on the same event must not retry it twice.
    requeue_lock: Mutex<()>,
}

/// A database error as Almanac's own, keeping the kit's remedy.
pub(crate) fn db_error(e: chassis::Error) -> AlmanacError {
    AlmanacError::Config {
        message: format!("the journal could not be read or written: {}", e.message),
        remedy: e.remedy.to_string(),
    }
}

impl Journal {
    pub fn new(legacy_file: PathBuf) -> Self {
        Self {
            legacy_file,
            db: OnceLock::new(),
            requeue_lock: Mutex::new(()),
        }
    }

    /// A journal on its own app database `dir/almanac.db` (the full
    /// schema), importing `dir/journal.jsonl` once if it is there. For the
    /// in-process harnesses and drills, which run without the kit.
    pub fn open_in(dir: &std::path::Path) -> Result<Self, AlmanacError> {
        let legacy = dir.join("journal.jsonl");
        let journal = Self::new(legacy.clone());
        let db = AppDb::open(
            &dir.join("almanac.db"),
            crate::shell::history_store::app_db_spec(legacy),
            Vec::new(),
        )
        .map_err(db_error)?;
        journal.attach(db);
        Ok(journal)
    }

    /// Hands the journal the app database (once; a second call is ignored).
    pub fn attach(&self, db: AppDb) {
        let _ = self.db.set(db);
    }

    /// Where the old journal file lived; the import reads it once.
    pub fn legacy_file(&self) -> &std::path::Path {
        &self.legacy_file
    }

    fn db(&self) -> Result<&AppDb, AlmanacError> {
        self.db.get().ok_or_else(|| AlmanacError::Config {
            message: "the journal has no database yet".to_string(),
            remedy: "it is attached when the service is mounted on the kit; this is a bug \
                     if it shows after start"
                .to_string(),
        })
    }

    /// Durably records an accepted payload. Must complete before the
    /// source is acknowledged.
    pub async fn accept(&self, entry: &Entry) -> Result<(), AlmanacError> {
        let entry = entry.clone();
        self.db()?
            .write(move |tx| rows::insert(tx, &entry))
            .await
            .map_err(db_error)
    }

    /// The entry was delivered: its row goes.
    pub async fn mark_done(&self, id: &str) -> Result<(), AlmanacError> {
        let id = id.to_string();
        self.db()?
            .write(move |tx| {
                tx.execute("DELETE FROM journal WHERE id = ?1", params![id])?;
                Ok(())
            })
            .await
            .map_err(db_error)
    }

    /// Sets an entry aside as permanently undeliverable (T1), keeping its
    /// payload: the source was told 202, and the reason is what you need
    /// to fix the profile.
    pub async fn mark_dead(&self, id: &str, reason: &str, at: &str) -> Result<(), AlmanacError> {
        let (id, reason, at) = (id.to_string(), reason.to_string(), at.to_string());
        let unix = crate::core::history::epoch_of(&at).unwrap_or(0);
        self.db()?
            .write(move |tx| {
                tx.execute(
                    "UPDATE journal SET state = 'dead', reason = ?2, set_aside_at = ?3,
                        set_aside_unix = ?4 WHERE id = ?1",
                    params![id, reason, at, unix],
                )?;
                Ok(())
            })
            .await
            .map_err(db_error)
    }

    /// Entries that were set aside, with the reason and when, oldest first.
    pub fn dead(&self) -> Result<Vec<(Entry, String, String)>, AlmanacError> {
        self.db()?.read_blocking(rows::dead).map_err(db_error)
    }

    /// The entries accepted but not yet delivered, in acceptance order.
    /// Called on startup (replay) and by the worker loop.
    pub fn pending(&self) -> Result<Vec<Entry>, AlmanacError> {
        self.db()?.read_blocking(rows::pending).map_err(db_error)
    }

    /// The upsert key an entry is stored under (see [`entry_key`]).
    pub(crate) fn key_of(entry: &Entry) -> Option<String> {
        entry_key(entry)
    }
}

#[cfg(test)]
mod supersede_tests;
#[cfg(test)]
pub(crate) mod testing;
#[cfg(test)]
mod tests;
