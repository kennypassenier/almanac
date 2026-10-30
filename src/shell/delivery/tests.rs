use super::*;
use crate::core::calendar::{EventDateTime, ExtendedProperties, GoogleEvent};
use serde_json::json;

fn event_with_property(value: Option<&str>) -> GoogleEvent {
    let extended_properties = value.map(|v| {
        let mut private = HashMap::new();
        private.insert(UPSERT_PROPERTY.to_string(), v.to_string());
        ExtendedProperties { private }
    });
    GoogleEvent {
        id: None,
        summary: "t".to_string(),
        description: None,
        location: None,
        color_id: None,
        start: EventDateTime::timed("2026-08-28T09:00:00+00:00".to_string(), "UTC".to_string()),
        end: EventDateTime::timed("2026-08-28T10:00:00+00:00".to_string(), "UTC".to_string()),
        transparency: None,
        status: None,
        reminders: None,
        extended_properties,
    }
}

fn entry(idempotency_key: Option<&str>) -> Entry {
    Entry {
        id: "j1".to_string(),
        source_id: "home-assistant".to_string(),
        received_at: "2026-08-28T09:00:00+00:00".to_string(),
        payload: json!({}),
        idempotency_key: idempotency_key.map(|k| k.to_string()),
    }
}

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
        Some("home-assistant:abc123".to_string())
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
        Some("home-assistant:bin-day-2026-09-01"),
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

/// A profile that maps a trivial payload onto `calendar`.
fn profile_for(source_id: &str, calendar: &str) -> Profile {
    let toml = format!(
        r#"
schema_version = 2
source_id = "{source_id}"
target_calendar_id = "{calendar}"

"#
    );
    Profile::parse(&toml, "test.toml").unwrap()
}

fn payload_entry(source_id: &str, title: &str) -> Entry {
    Entry {
        id: format!("j-{title}"),
        source_id: source_id.to_string(),
        received_at: "2026-08-28T09:00:00+00:00".to_string(),
        payload: json!({
            "title": title,
            "start": "2026-08-28T09:00:00+00:00",
            "external_id": title
        }),
        idempotency_key: None,
    }
}

async fn stubbed_client(calendar: &crate::shell::testing::CalendarStub) -> GoogleCalendarClient {
    let tokens = crate::shell::testing::TokenStub::start(3600).await;
    GoogleCalendarClient::with_base_url(
        reqwest::Client::new(),
        crate::shell::auth::TokenManager::new(
            reqwest::Client::new(),
            crate::shell::testing::stub_credentials(&tokens.url),
        ),
        &calendar.base_url,
    )
}

#[tokio::test]
async fn each_source_lands_on_its_own_calendar() {
    // K3's acceptance criterion, and the point of the whole
    // project: two profiles writing to two different calendars,
    // with nothing crossing over. It had no test at any level —
    // the calendar id is read inside `deliver`, which until now
    // could only be reached by talking to the real Google.
    let calendar = crate::shell::testing::CalendarStub::start().await;
    let client = stubbed_client(&calendar).await;
    let locks = KeyLocks::new();

    deliver(
        &payload_entry("home-assistant", "bin day"),
        &profile_for("home-assistant", "household@group.calendar.google.com"),
        &client,
        &locks,
    )
    .await
    .unwrap();

    deliver(
        &payload_entry("uptime-kuma", "jellyfin down"),
        &profile_for("uptime-kuma", "infra@group.calendar.google.com"),
        &client,
        &locks,
    )
    .await
    .unwrap();

    assert_eq!(
        calendar.state.calendars_written().await,
        vec![
            "household@group.calendar.google.com".to_string(),
            "infra@group.calendar.google.com".to_string()
        ],
        "an infra alert on the household calendar is exactly what K3 exists to prevent"
    );

    let created = calendar.state.created.lock().await;
    let household: Vec<&str> = created
        .iter()
        .filter(|(cal, _)| cal.starts_with("household"))
        .map(|(_, body)| body["summary"].as_str().unwrap())
        .collect();
    assert_eq!(household, vec!["bin day"], "no cross-contamination");
}

#[tokio::test]
async fn the_lookup_before_an_upsert_uses_the_profiles_own_calendar() {
    // Reading from the wrong calendar would make every delivery
    // look new, so a redelivery would duplicate rather than update
    // — K2 broken by a K3 bug.
    let calendar = crate::shell::testing::CalendarStub::start().await;
    let client = stubbed_client(&calendar).await;
    let locks = KeyLocks::new();

    deliver(
        &payload_entry("uptime-kuma", "jellyfin down"),
        &profile_for("uptime-kuma", "infra@group.calendar.google.com"),
        &client,
        &locks,
    )
    .await
    .unwrap();

    let requests = calendar.state.requests.lock().await;
    assert!(
        requests
            .iter()
            .all(|(_, cal)| cal == "infra@group.calendar.google.com"),
        "every request for this delivery must name the profile's calendar, got {requests:?}"
    );
}
