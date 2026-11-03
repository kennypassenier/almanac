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

fn entry_with(id: &str, payload: serde_json::Value) -> Entry {
    Entry {
        id: id.to_string(),
        source_id: "home-assistant".to_string(),
        received_at: "2026-08-28T09:00:00+00:00".to_string(),
        payload,
        idempotency_key: None,
    }
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
fn a_dead_entry_with_a_later_done_marker_is_no_longer_listed() {
    // Retry and dismiss both end a set-aside entry with a `Done`.
    let records = vec![
        entry("a"),
        entry("b"),
        dead_marker("a", "unmappable"),
        dead_marker("b", "unmappable"),
        done("a"),
    ];
    let ids: Vec<_> = dead(&records).iter().map(|d| d.0.id.as_str()).collect();
    assert_eq!(ids, vec!["b"]);
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

#[test]
fn an_entry_key_prefers_the_external_id_over_the_idempotency_key() {
    let mut e = entry_with("a", json!({"title": "t", "external_id": "task-7"}));
    e.idempotency_key = Some("k-1".to_string());
    assert_eq!(entry_key(&e).as_deref(), Some("home-assistant:task-7"));
    let mut bare = entry_with("b", json!({"title": "t"}));
    assert_eq!(entry_key(&bare), None);
    bare.idempotency_key = Some("k-1".to_string());
    assert_eq!(entry_key(&bare).as_deref(), Some("home-assistant:idem:k-1"));
}

const DAY: i64 = 86_400;

fn unix(rfc3339: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .unwrap()
        .timestamp()
}

#[test]
fn days_left_rounds_up_and_never_goes_negative() {
    let at = "2026-10-01T00:00:00+00:00";
    let t = unix(at);
    assert_eq!(days_left(at, t), 30);
    assert_eq!(days_left(at, t + DAY + 1), 29);
    assert_eq!(days_left(at, t + 30 * DAY + 5), 0);
    assert_eq!(days_left("garbage", t), 30);
}
