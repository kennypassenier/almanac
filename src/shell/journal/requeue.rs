//! Acting on set-aside entries: retry them or dismiss them.
//!
//! Both are built from the records the journal already has, so an older
//! binary still reads the file. Retry appends a NEW entry (fresh id, same
//! source, payload and idempotency key) and then a `Done` for the old id;
//! dismiss appends only the `Done`. `core::journal::dead` stops listing an
//! entry with a `Done`, and the next compaction drops its payload.

use std::collections::HashSet;

use crate::core::error::AlmanacError;
use crate::core::journal::{self, Entry, Record};

use super::Journal;

/// Why a retry or a dismiss did not happen.
#[derive(Debug)]
pub enum RequeueError {
    /// These ids are not set aside (never were, or already handled).
    /// Nothing was written.
    NotSetAside(Vec<String>),
    /// The journal could not be read or appended to.
    Journal(AlmanacError),
}

impl From<AlmanacError> for RequeueError {
    fn from(e: AlmanacError) -> Self {
        RequeueError::Journal(e)
    }
}

impl Journal {
    /// Delivers set-aside entries again. For each id: a new entry first,
    /// then the old id's `Done`, so a crash between the two leaves a
    /// duplicate the idempotency key converges, never a lost event.
    /// All-or-nothing on the check: one id that is not set aside refuses
    /// the whole call before anything is written. Returns how many.
    pub async fn retry_dead(&self, ids: &[String], now: &str) -> Result<usize, RequeueError> {
        let _turn = self.requeue_lock.lock().await;
        let entries = self.set_aside_entries(ids)?;
        for entry in &entries {
            let fresh = journal::requeued(entry, uuid::Uuid::new_v4().to_string(), now);
            self.append(&Record::Entry(fresh)).await?;
            self.mark_done(&entry.id).await?;
        }
        Ok(entries.len())
    }

    /// Marks set-aside entries as handled: they leave the set-aside list
    /// and their payload goes at the next compaction. Same all-or-nothing
    /// check as `retry_dead`. Returns how many.
    pub async fn dismiss_dead(&self, ids: &[String]) -> Result<usize, RequeueError> {
        let _turn = self.requeue_lock.lock().await;
        let entries = self.set_aside_entries(ids)?;
        for entry in &entries {
            self.mark_done(&entry.id).await?;
        }
        Ok(entries.len())
    }

    /// The set-aside entries for `ids` (each once, in the order given), or
    /// the ids that are not set aside.
    fn set_aside_entries(&self, ids: &[String]) -> Result<Vec<Entry>, RequeueError> {
        let records = self.read_records()?;
        let dead = journal::dead(&records);
        let mut seen = HashSet::new();
        let mut found = Vec::new();
        let mut refused = Vec::new();
        for id in ids.iter().filter(|id| seen.insert(id.as_str())) {
            match dead.iter().find(|(entry, _, _)| entry.id == *id) {
                Some((entry, _, _)) => found.push((*entry).clone()),
                None => refused.push(id.clone()),
            }
        }
        if refused.is_empty() {
            Ok(found)
        } else {
            Err(RequeueError::NotSetAside(refused))
        }
    }
}
