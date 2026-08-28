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
        start: EventDateTime {
            date_time: "2026-08-28T09:00:00+00:00".to_string(),
            time_zone: "UTC".to_string(),
        },
        end: EventDateTime {
            date_time: "2026-08-28T10:00:00+00:00".to_string(),
            time_zone: "UTC".to_string(),
        },
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
