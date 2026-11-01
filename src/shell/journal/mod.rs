//! The durable ingest journal's disk side (AR16). Every accepted
//! payload is appended and fsynced *before* the source is told the
//! request succeeded, so a crash or power cut between "accepted" and
//! "delivered to Google" loses nothing: replay-on-start re-delivers,
//! and upsert (K2/AR15) plus idempotency keys (M7) make that
//! redelivery converge instead of duplicating.
//!
//! Append-only JSON lines, never mutated in place — a torn write can
//! only ever damage the final line, which replay skips (see
//! `read_records`). Compaction rewrites the file atomically (temp +
//! rename, standing rule 12), never truncates in place.

use std::path::{Path, PathBuf};

use tokio::sync::Mutex;

use crate::core::error::AlmanacError;
use crate::core::journal::{Entry, Record};

/// Refuse to append beyond this and say so loudly rather than filling
/// the disk silently (standing rule 12: no silent caps). Compaction
/// keeps a healthy journal far below it; hitting this means delivery
/// has been failing for a long time and needs a human.
pub const DEFAULT_MAX_BYTES: u64 = 64 * 1024 * 1024;

pub struct Journal {
    path: PathBuf,
    max_bytes: u64,
    /// Serializes writers so two appends can never interleave a
    /// partial line into the file.
    write_lock: Mutex<()>,
    /// Serializes retry and dismiss, which read the set-aside list and
    /// then append: two clicks on the same event must not retry it twice.
    requeue_lock: Mutex<()>,
}

impl Journal {
    pub fn new(path: PathBuf, max_bytes: u64) -> Self {
        Self {
            path,
            max_bytes,
            write_lock: Mutex::new(()),
            requeue_lock: Mutex::new(()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn max_bytes(&self) -> u64 {
        self.max_bytes
    }

    /// Durably records an accepted payload. Must complete before the
    /// source is acknowledged.
    pub async fn accept(&self, entry: &Entry) -> Result<(), AlmanacError> {
        self.append(&Record::Entry(entry.clone())).await
    }

    /// Durably records that an entry has been delivered.
    pub async fn mark_done(&self, id: &str) -> Result<(), AlmanacError> {
        self.append(&Record::Done { id: id.to_string() }).await
    }

    /// Sets an entry aside as permanently undeliverable (T1).
    ///
    /// A marker, like `mark_done` — the payload itself stays in the
    /// log, because the source was told 202 and because the reason is
    /// what you need in order to fix the profile.
    pub async fn mark_dead(&self, id: &str, reason: &str, at: &str) -> Result<(), AlmanacError> {
        self.append(&Record::Dead {
            id: id.to_string(),
            reason: reason.to_string(),
            at: at.to_string(),
        })
        .await
    }

    /// Entries that were set aside, with the reason each was.
    pub fn dead(&self) -> Result<Vec<(crate::core::journal::Entry, String, String)>, AlmanacError> {
        let records = self.read_records()?;
        Ok(crate::core::journal::dead(&records)
            .into_iter()
            .map(|(entry, reason, at)| (entry.clone(), reason.to_string(), at.to_string()))
            .collect())
    }

    /// The entries accepted but not yet delivered, in acceptance
    /// order. Called on startup (replay) and by the worker loop.
    pub fn pending(&self) -> Result<Vec<Entry>, AlmanacError> {
        let records = self.read_records()?;
        Ok(crate::core::journal::pending(&records)
            .into_iter()
            .cloned()
            .collect())
    }
}

mod append;
mod compact;
mod read;
mod requeue;

pub use requeue::RequeueError;

#[cfg(test)]
mod tests;
