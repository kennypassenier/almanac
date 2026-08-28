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
pub fn dead(records: &[Record]) -> Vec<(&Entry, &str, &str)> {
    let reasons: std::collections::HashMap<&str, (&str, &str)> = records
        .iter()
        .filter_map(|r| match r {
            Record::Dead { id, reason, at } => Some((id.as_str(), (reason.as_str(), at.as_str()))),
            _ => None,
        })
        .collect();

    records
        .iter()
        .filter_map(|r| match r {
            Record::Entry(e) => reasons
                .get(e.id.as_str())
                .map(|(reason, at)| (e, *reason, *at)),
            _ => None,
        })
        .collect()
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
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(id: &str) -> Record {
        Record::Entry(Entry {
            id: id.to_string(),
            source_id: "home-assistant".to_string(),
            received_at: "2026-08-28T09:00:00+00:00".to_string(),
            payload: json!({"title": "t"}),
            idempotency_key: None,
        })
    }

    fn done(id: &str) -> Record {
        Record::Done { id: id.to_string() }
    }

    #[test]
    fn an_entry_with_no_done_marker_is_pending() {
        let records = vec![entry("a")];
        assert_eq!(pending(&records).len(), 1);
    }

    #[test]
    fn an_entry_followed_by_its_marker_is_not_pending() {
        let records = vec![entry("a"), done("a")];
        assert!(pending(&records).is_empty());
    }

    #[test]
    fn only_the_undelivered_entries_come_back_and_in_order() {
        let records = vec![entry("a"), entry("b"), done("a"), entry("c")];
        let pending = pending(&records);
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].id, "b");
        assert_eq!(pending[1].id, "c");
    }

    #[test]
    fn a_marker_ordered_before_its_entry_still_counts() {
        // Ordering within the file is acceptance order, but a replay
        // must not depend on a marker appearing after its entry — a
        // compaction could reorder them. Treat the set of markers as a
        // whole, not as a running state machine.
        let records = vec![done("a"), entry("a")];
        assert!(pending(&records).is_empty());
    }

    #[test]
    fn an_orphan_marker_for_an_unknown_entry_is_inert() {
        let records = vec![done("gone-in-a-compaction"), entry("a")];
        let pending = pending(&records);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "a");
    }

    #[test]
    fn an_empty_log_has_nothing_pending() {
        assert!(pending(&[]).is_empty());
    }

    #[test]
    fn records_round_trip_through_json_lines() {
        let e = entry("a");
        let line = serde_json::to_string(&e).unwrap();
        assert!(!line.contains('\n'), "a record must serialize to one line");
        let back: Record = serde_json::from_str(&line).unwrap();
        assert_eq!(back, e);

        let d = done("a");
        let line = serde_json::to_string(&d).unwrap();
        let back: Record = serde_json::from_str(&line).unwrap();
        assert_eq!(back, d);
    }

    fn dead_marker(id: &str, reason: &str) -> Record {
        Record::Dead {
            id: id.to_string(),
            reason: reason.to_string(),
            at: "2026-08-29T09:00:00+00:00".to_string(),
        }
    }

    #[test]
    fn an_entry_set_aside_as_dead_is_no_longer_pending() {
        // Otherwise one payload that can never succeed is retried
        // forever, holds the worker at its slowest backoff, and
        // eventually raises a backlog alert about a queue of one.
        let records = vec![entry("a"), entry("b"), dead_marker("a", "unmappable")];
        let pending = pending(&records);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "b");
    }

    #[test]
    fn a_dead_entry_is_still_readable_with_its_reason() {
        // Set aside, not deleted: the source was told 202, and the
        // reason is what you need to fix the profile.
        let records = vec![
            entry("a"),
            dead_marker("a", "mapping.title_field is missing"),
        ];
        let dead = dead(&records);
        assert_eq!(dead.len(), 1);
        assert_eq!(dead[0].0.id, "a");
        assert_eq!(dead[0].1, "mapping.title_field is missing");
        assert_eq!(dead[0].2, "2026-08-29T09:00:00+00:00");
    }

    #[test]
    fn nothing_is_dead_in_an_ordinary_log() {
        assert!(
            dead(&[
                entry("a"),
                Record::Done {
                    id: "a".to_string()
                }
            ])
            .is_empty()
        );
    }

    #[test]
    fn a_dead_marker_for_an_unknown_entry_is_inert() {
        // Same rule as a Done marker after compaction.
        assert!(dead(&[dead_marker("gone", "whatever")]).is_empty());
        assert!(pending(&[dead_marker("gone", "whatever")]).is_empty());
    }

    #[test]
    fn records_including_dead_round_trip_through_json_lines() {
        let record = dead_marker("a", "unmappable");
        let line = serde_json::to_string(&record).unwrap();
        assert_eq!(serde_json::from_str::<Record>(&line).unwrap(), record);
        assert!(line.contains("\"kind\":\"dead\""), "{line}");
    }
}
