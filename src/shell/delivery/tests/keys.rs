//! The upsert key a delivery deduplicates against and the lock per key.

use super::*;

#[test]
fn the_mappings_own_property_is_the_upsert_key_when_present() {
    let event = event_with_property(Some("home-assistant:switch.wasmachine"));
    assert_eq!(
        upsert_key(&entry(None), &event),
        Some("home-assistant:switch.wasmachine".to_string())
    );
}

#[test]
fn an_idempotency_key_is_used_when_the_mapping_has_no_external_id() {
    let event = event_with_property(None);
    assert_eq!(
        upsert_key(&entry(Some("abc123")), &event),
        Some("home-assistant:idem:abc123".to_string())
    );
}

#[test]
fn the_mappings_property_wins_over_an_idempotency_key() {
    // A source that has both a real external id and sends a key
    // should dedupe on the durable identity, not the per-request
    // one, or a retry with a fresh key would duplicate.
    let event = event_with_property(Some("home-assistant:switch.wasmachine"));
    assert_eq!(
        upsert_key(&entry(Some("abc123")), &event),
        Some("home-assistant:switch.wasmachine".to_string())
    );
}

#[test]
fn neither_source_of_identity_means_no_key_and_an_unconditional_create() {
    let event = event_with_property(None);
    assert_eq!(upsert_key(&entry(None), &event), None);
}

#[tokio::test]
async fn a_delivery_reports_the_key_it_deduplicated_against() {
    // Found by using Almanac once as a source really would, in
    // Phase 9. The debug surface's `upsert_key` was hardcoded to
    // None, so every routing decision claimed there had been no
    // upsert key — including the redelivery that had just converged
    // on the same Google event and therefore obviously had one.
    //
    // That is a lie in exactly the place someone looks when they
    // are chasing a duplicate: the debugging guide sends them to
    // the routing decisions, they read "no upsert key", and they go
    // and add an `external_id_field` to a profile that already has
    // one.
    let calendar = crate::shell::testing::CalendarStub::start().await;
    let client = stubbed_client(&calendar).await;
    let locks = KeyLocks::new();

    // No external_id in the payload: this is M7's path, where the
    // header supplies the key instead.
    let mut entry = payload_entry("home-assistant", "bin day");
    entry.payload = json!({"title": "bin day", "start": "2026-08-28T09:00:00+00:00"});
    entry.idempotency_key = Some("bin-day-2026-09-01".to_string());

    let delivered = deliver(
        &entry,
        &profile_for("home-assistant", "household@group.calendar.google.com"),
        &client,
        &locks,
    )
    .await
    .unwrap();

    assert_eq!(
        delivered.upsert_key.as_deref(),
        Some("home-assistant:idem:bin-day-2026-09-01"),
        "a delivery that deduplicated must say what it deduplicated against"
    );

    // And a delivery with nothing to deduplicate against still
    // reports honestly, rather than inventing a key. Since 2.0.0
    // the ingest layer refuses such a call outright, so this can
    // only arise from an entry journalled by an older build and
    // replayed after the upgrade — which is exactly why the
    // delivery layer must still handle it rather than assume.
    let mut anonymous_entry = payload_entry("uptime-kuma", "jellyfin down");
    anonymous_entry.payload =
        json!({"title": "jellyfin down", "start": "2026-08-28T09:00:00+00:00"});
    let anonymous = deliver(
        &anonymous_entry,
        &profile_for("uptime-kuma", "infra@group.calendar.google.com"),
        &client,
        &locks,
    )
    .await
    .unwrap();
    assert_eq!(anonymous.upsert_key, None);
}

#[tokio::test]
async fn the_same_key_hands_back_the_same_lock_and_different_keys_do_not() {
    let locks = KeyLocks::new();
    let a1 = locks.for_key("k1").await;
    let a2 = locks.for_key("k1").await;
    let b = locks.for_key("k2").await;

    assert!(Arc::ptr_eq(&a1, &a2), "same key must share one lock");
    assert!(
        !Arc::ptr_eq(&a1, &b),
        "different keys must not share a lock"
    );
}

#[tokio::test]
async fn a_header_keyed_event_from_before_the_split_is_updated_not_duplicated() {
    // Kenny, form key-namespace (2026-11-02): header keys moved to
    // `<source>:idem:<key>`. An event written before carries
    // `<source>:<key>`; the next delivery must find it, not add a copy.
    let calendar = crate::shell::testing::CalendarStub::start().await;
    calendar
        .seed(
            "household@group.calendar.google.com",
            json!({
                "id": "old-1",
                "summary": "bin day",
                "start": {"dateTime": "2026-08-28T09:00:00+00:00", "timeZone": "UTC"},
                "end": {"dateTime": "2026-08-28T10:00:00+00:00", "timeZone": "UTC"},
                "extendedProperties": {"private": {"almanac_source_id": "home-assistant:bin-day"}}
            }),
        )
        .await;
    let client = stubbed_client(&calendar).await;
    let mut entry = payload_entry("home-assistant", "bin day");
    entry.payload = json!({"title": "bin day", "start": "2026-08-28T09:00:00+00:00"});
    entry.idempotency_key = Some("bin-day".to_string());

    let delivered = deliver(
        &entry,
        &profile_for("home-assistant", "household@group.calendar.google.com"),
        &client,
        &KeyLocks::new(),
    )
    .await
    .unwrap();

    assert!(!delivered.created, "the old event is updated");
    assert_eq!(delivered.event_id, "old-1");
    assert_eq!(
        delivered.upsert_key.as_deref(),
        Some("home-assistant:idem:bin-day")
    );
}
