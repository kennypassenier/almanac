//! The durable ingest journal's record format (AR16). Writing these
//! records to disk is `shell::journal`'s job; this module defines what
//! a record *is* and how a replay reconstructs "what still needs
//! delivering" from an append-only log.
//!
//! The log holds three kinds of line: an `Entry` (a payload accepted
//! from a source but not yet delivered to Google), a `Done` marker
//! naming an entry that has been delivered, and a `Dead` marker naming
//! one that never can be. Nothing is ever mutated in place — a crash
//! can therefore only ever lose the tail of the file, never corrupt an
//! earlier record.

use serde::{Deserialize, Serialize};

/// One accepted-but-not-yet-delivered payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    /// Unique per accepted request; the `Done` marker refers to it.
    pub id: String,
    /// Which profile (and therefore which calendar and mapping) this
    /// payload belongs to — the AR15 immutable identity, not a
    /// filename.
    pub source_id: String,
    /// RFC3339 timestamp of acceptance, for the K11 debug surface.
    pub received_at: String,
    /// The source's payload, verbatim as accepted.
    pub payload: serde_json::Value,
    /// M7: supplied by sources without a natural external id, so a
    /// redelivery of the same logical event converges instead of
    /// duplicating.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Record {
    Entry(Entry),
    Done {
        id: String,
    },
    /// T1: an entry that failed permanently, repeatedly, and will
    /// never succeed — a mapping error, a calendar the service account
    /// cannot write to, a timestamp Google refuses.
    ///
    /// Set aside rather than deleted, deliberately. The source was
    /// told 202 ("durably accepted"), so throwing the payload away
    /// would make that a lie; and the reason is what you need to fix
    /// the profile. Left pending instead, it would be retried forever,
    /// hold the worker in its slowest backoff, and eventually raise a
    /// backlog alert about a queue of one dead payload.
    Dead {
        id: String,
        reason: String,
        at: String,
    },
}

/// Entries set aside as undeliverable, most recent last.
///
/// An entry whose id has a `Done` marker is no longer listed: that is how
/// a set-aside entry is retried (a new entry stands in for it) or
/// dismissed, without a new record kind an older binary could not read.
/// Compaction drops such an entry's payload, because it reads this list.
pub fn dead(records: &[Record]) -> Vec<(&Entry, &str, &str)> {
    let reasons: std::collections::HashMap<&str, (&str, &str)> = records
        .iter()
        .filter_map(|r| match r {
            Record::Dead { id, reason, at } => Some((id.as_str(), (reason.as_str(), at.as_str()))),
            _ => None,
        })
        .collect();
    let finished: std::collections::HashSet<&str> = records
        .iter()
        .filter_map(|r| match r {
            Record::Done { id } => Some(id.as_str()),
            _ => None,
        })
        .collect();

    records
        .iter()
        .filter_map(|r| match r {
            Record::Entry(e) if !finished.contains(e.id.as_str()) => reasons
                .get(e.id.as_str())
                .map(|(reason, at)| (e, *reason, *at)),
            _ => None,
        })
        .collect()
}

/// The entry that stands in for a set-aside one when it is retried: a
/// fresh id, received now, with the same source, payload and
/// idempotency key, so Google converges on one event.
pub fn requeued(old: &Entry, new_id: String, now: &str) -> Entry {
    Entry {
        id: new_id,
        received_at: now.to_string(),
        ..old.clone()
    }
}

/// Reconstructs the still-undelivered entries from a replayed log, in
/// acceptance order.
///
/// A `Done` marker may legitimately appear for an entry this replay
/// never saw (after a compaction that dropped the entry but kept a
/// later marker); such markers are simply inert. The reverse — an
/// entry with no marker — is exactly what must be redelivered.
pub fn pending(records: &[Record]) -> Vec<&Entry> {
    let done: std::collections::HashSet<&str> = records
        .iter()
        .filter_map(|r| match r {
            Record::Done { id } => Some(id.as_str()),
            // A dead entry is not pending either: it is kept for
            // reading, not for retrying (T1).
            Record::Dead { id, .. } => Some(id.as_str()),
            Record::Entry(_) => None,
        })
        .collect();

    records
        .iter()
        .filter_map(|r| match r {
            Record::Entry(e) if !done.contains(e.id.as_str()) => Some(e),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests;
