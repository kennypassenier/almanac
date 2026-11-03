//! Acting on set-aside entries (retry, dismiss) and closing the older
//! versions of an event once a newer one is delivered.

use chassis::storage::rusqlite::{OptionalExtension, Transaction, params};

use crate::core::error::AlmanacError;
use crate::core::journal::{self, Entry};

use super::{Journal, db_error, rows};

/// Why a retry or a dismiss did not happen.
#[derive(Debug)]
pub enum RequeueError {
    /// These ids are not set aside (never were, or already handled).
    /// Nothing was written.
    NotSetAside(Vec<String>),
    /// A newer version of these events arrived after they were set aside;
    /// retrying would put the older one back over it. Nothing was written.
    Superseded(Vec<String>),
    /// The journal could not be read or written.
    Journal(AlmanacError),
}

impl From<AlmanacError> for RequeueError {
    fn from(e: AlmanacError) -> Self {
        RequeueError::Journal(e)
    }
}

/// Whether the refusal inside a transaction or the database failed.
enum Outcome {
    Done(usize),
    Refused(RequeueError),
}

impl Journal {
    /// Delivers set-aside entries again: each becomes a new pending entry
    /// (fresh id, same source, payload and key) and the old row goes, in
    /// one transaction. All or nothing: one id that is not set aside, or
    /// that a newer version followed, refuses the whole call.
    pub async fn retry_dead(&self, ids: &[String], now: &str) -> Result<usize, RequeueError> {
        let _turn = self.requeue_lock.lock().await;
        let (ids, now) = (unique(ids), now.to_string());
        let outcome = self
            .db()?
            .write(move |tx| {
                let entries = match set_aside(tx, &ids)? {
                    Ok(entries) => entries,
                    Err(refused) => return Ok(Outcome::Refused(refused)),
                };
                let superseded = superseded(tx, &entries)?;
                if !superseded.is_empty() {
                    return Ok(Outcome::Refused(RequeueError::Superseded(superseded)));
                }
                for entry in &entries {
                    let fresh = journal::requeued(entry, uuid::Uuid::new_v4().to_string(), &now);
                    rows::insert(tx, &fresh)?;
                    tx.execute("DELETE FROM journal WHERE id = ?1", params![entry.id])?;
                }
                Ok(Outcome::Done(entries.len()))
            })
            .await
            .map_err(db_error)?;
        finish(outcome)
    }

    /// Marks set-aside entries as handled: their rows go. Same
    /// all-or-nothing check as `retry_dead`.
    pub async fn dismiss_dead(&self, ids: &[String]) -> Result<usize, RequeueError> {
        let _turn = self.requeue_lock.lock().await;
        let ids = unique(ids);
        let outcome = self
            .db()?
            .write(move |tx| {
                let entries = match set_aside(tx, &ids)? {
                    Ok(entries) => entries,
                    Err(refused) => return Ok(Outcome::Refused(refused)),
                };
                for entry in &entries {
                    tx.execute("DELETE FROM journal WHERE id = ?1", params![entry.id])?;
                }
                Ok(Outcome::Done(entries.len()))
            })
            .await
            .map_err(db_error)?;
        finish(outcome)
    }

    /// Before `entry` is marked done: removes every pending entry accepted
    /// before it under the same upsert key, so an older version cannot be
    /// sent after it (Kenny, form newer-wins). Returns how many.
    pub async fn supersede_older(&self, entry: &Entry) -> Result<usize, AlmanacError> {
        let Some(key) = Journal::key_of(entry) else {
            return Ok(0);
        };
        let id = entry.id.clone();
        self.db()?
            .write(move |tx| {
                Ok(tx.execute(
                    "DELETE FROM journal WHERE upsert_key = ?1 AND state = 'pending' AND id != ?2
                        AND seq < (SELECT seq FROM journal WHERE id = ?2)",
                    params![key, id],
                )?)
            })
            .await
            .map_err(db_error)
    }
}

fn finish(outcome: Outcome) -> Result<usize, RequeueError> {
    match outcome {
        Outcome::Done(n) => Ok(n),
        Outcome::Refused(refused) => Err(refused),
    }
}

/// The ids once each, in the order given.
fn unique(ids: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    ids.iter()
        .filter(|id| seen.insert(id.as_str()))
        .cloned()
        .collect()
}

/// The set-aside entries for `ids`, or the refusal naming those that are not.
fn set_aside(
    tx: &Transaction,
    ids: &[String],
) -> Result<Result<Vec<Entry>, RequeueError>, chassis::Error> {
    let (mut found, mut refused) = (Vec::new(), Vec::new());
    for id in ids {
        let entry = tx
            .query_row(
                "SELECT id, source_id, received_at, payload, idempotency_key FROM journal
                    WHERE id = ?1 AND state = 'dead'",
                params![id],
                rows::entry_of,
            )
            .optional()?;
        match entry {
            Some(entry) => found.push(entry),
            None => refused.push(id.clone()),
        }
    }
    Ok(if refused.is_empty() {
        Ok(found)
    } else {
        Err(RequeueError::NotSetAside(refused))
    })
}

/// The entries a newer version of the same event followed: a later row
/// under the same key, or a delivery of that key after it was received.
fn superseded(tx: &Transaction, entries: &[Entry]) -> Result<Vec<String>, chassis::Error> {
    let mut out = Vec::new();
    for entry in entries {
        let Some(key) = journal::entry_key(entry) else {
            continue;
        };
        let received = crate::core::history::epoch_of(&entry.received_at).unwrap_or(0);
        let newer: i64 = tx.query_row(
            "SELECT (SELECT COUNT(*) FROM journal WHERE upsert_key = ?1
                        AND seq > (SELECT seq FROM journal WHERE id = ?2))
                  + (SELECT COUNT(*) FROM deliveries WHERE upsert_key = ?1 AND delivered_at > ?3)",
            params![key, entry.id, received],
            |r| r.get(0),
        )?;
        if newer > 0 {
            out.push(entry.id.clone());
        }
    }
    Ok(out)
}
