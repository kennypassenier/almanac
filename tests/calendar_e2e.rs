//! L1 exit-criterion E2E test (standing rule 9: real dependencies, not
//! mocks) — a full create→read→update→delete round-trip against a
//! REAL Google Calendar.
//!
//! Requires `ALMANAC_TEST_CALENDAR_ID` plus the usual `CLIENT_EMAIL` /
//! `PRIVATE_KEY` / `TOKEN_URI` service-account credentials (normally
//! supplied by `latch run --`) in the process environment. Marked
//! `#[ignore]` so a plain `cargo test` — and CI, until the scratch
//! calendar and its credentials are wired in as secrets — stays green
//! without them; run explicitly with:
//!   ALMANAC_TEST_CALENDAR_ID=... latch run -- cargo test --test calendar_e2e -- --ignored

use std::collections::HashMap;

use almanac::core::calendar::{EventDateTime, ExtendedProperties, GoogleEvent};
use almanac::shell::auth::{TokenManager, load_credentials};
use almanac::shell::calendar_client::GoogleCalendarClient;

fn test_event(marker: &str) -> GoogleEvent {
    let mut private = HashMap::new();
    private.insert("almanac_source_id".to_string(), marker.to_string());

    GoogleEvent {
        id: None,
        summary: format!("almanac L1 E2E test — {marker}"),
        description: Some("created by tests/calendar_e2e.rs; safe to delete".to_string()),
        location: None,
        color_id: None,
        start: EventDateTime::timed("2026-08-28T09:00:00+00:00".to_string(), "UTC".to_string()),
        end: EventDateTime::timed("2026-08-28T10:00:00+00:00".to_string(), "UTC".to_string()),
        transparency: None,
        status: None,
        reminders: None,
        extended_properties: Some(ExtendedProperties { private }),
    }
}

#[tokio::test]
#[ignore = "requires ALMANAC_TEST_CALENDAR_ID and Google service-account credentials via latch run"]
async fn create_read_update_delete_round_trip_against_a_real_calendar() {
    let calendar_id = std::env::var("ALMANAC_TEST_CALENDAR_ID")
        .expect("set ALMANAC_TEST_CALENDAR_ID to the almanac-test calendar's id");
    let credentials = load_credentials().expect("service-account credentials via latch run");

    let http = reqwest::Client::new();
    let tokens = TokenManager::new(http.clone(), credentials);
    let client = GoogleCalendarClient::new(http, tokens);

    let marker = format!("e2e-{}", chrono::Utc::now().timestamp());
    let created = client
        .create_event(&calendar_id, &test_event(&marker))
        .await
        .expect("create_event");
    let event_id = created.id.clone().expect("Google assigns an id on create");

    let fetched = client
        .get_event(&calendar_id, &event_id)
        .await
        .expect("get_event");
    assert_eq!(fetched.summary, created.summary);

    let mut updated_event = fetched.clone();
    updated_event.summary = format!("almanac L1 E2E test — {marker} — updated");
    let updated = client
        .update_event(&calendar_id, &event_id, &updated_event)
        .await
        .expect("update_event");
    assert!(updated.summary.ends_with("updated"));

    let found = client
        .find_event_by_property(&calendar_id, "almanac_source_id", &marker)
        .await
        .expect("find_event_by_property")
        .expect("the event just created and updated should be found by its marker");
    assert_eq!(found.id.as_deref(), Some(event_id.as_str()));

    client
        .delete_event(&calendar_id, &event_id)
        .await
        .expect("delete_event");

    let after_delete = client
        .find_event_by_property(&calendar_id, "almanac_source_id", &marker)
        .await
        .expect("find_event_by_property after delete");
    assert!(
        after_delete.is_none(),
        "deleted event should no longer be findable"
    );
}

/// K14/K16/K17 against the real thing.
///
/// The unit tests pin what Almanac *believes* Google's JSON contract
/// is. That belief was already wrong once during this feature — an
/// untagged enum quietly dropped the camelCase renaming and would have
/// sent `date_time`, which Google rejects — and a serialization test
/// written from the same belief cannot catch that class of mistake.
/// Only Google can.
#[tokio::test]
#[ignore = "requires ALMANAC_TEST_CALENDAR_ID and Google service-account credentials via latch run"]
async fn an_all_day_event_with_reminders_is_accepted_by_google() {
    let calendar_id = std::env::var("ALMANAC_TEST_CALENDAR_ID")
        .expect("set ALMANAC_TEST_CALENDAR_ID to the almanac-test calendar's id");
    let credentials = load_credentials().expect("service-account credentials via latch run");

    let http = reqwest::Client::new();
    let tokens = TokenManager::new(http.clone(), credentials);
    let client = GoogleCalendarClient::new(http, tokens);

    let mut private = HashMap::new();
    private.insert(
        "almanac_source_id".to_string(),
        "e2e:all-day-fields".to_string(),
    );

    let event = GoogleEvent {
        id: None,
        summary: "almanac K14 E2E — all-day with reminders".to_string(),
        description: Some("created by tests/calendar_e2e.rs; safe to delete".to_string()),
        location: Some("Kerkstraat 1".to_string()),
        color_id: None,
        start: EventDateTime::all_day("2026-09-01"),
        end: EventDateTime::all_day("2026-09-02"),
        transparency: Some("transparent".to_string()),
        status: Some("confirmed".to_string()),
        reminders: Some(almanac::core::calendar::Reminders {
            use_default: false,
            overrides: vec![almanac::core::calendar::ReminderOverride {
                method: "popup".to_string(),
                minutes: 30,
            }],
        }),
        extended_properties: Some(ExtendedProperties { private }),
    };

    let created = client
        .create_event(&calendar_id, &event)
        .await
        .expect("Google accepted the all-day event");
    let id = created.id.clone().expect("Google returned an id");

    // Read it back: an event Google accepted but stored as something
    // else would still pass a create-only assertion.
    assert!(
        created.start.is_all_day(),
        "Google stored it as a timed event: {:?}",
        created.start
    );
    assert_eq!(created.start.date(), Some("2026-09-01"));
    assert_eq!(created.end.date(), Some("2026-09-02"));
    assert_eq!(created.location.as_deref(), Some("Kerkstraat 1"));
    assert_eq!(created.transparency.as_deref(), Some("transparent"));

    client
        .delete_event(&calendar_id, &id)
        .await
        .expect("cleaning up the test event");
}
