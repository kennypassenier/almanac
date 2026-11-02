use super::*;
use crate::core::calendar::GoogleEvent;
use crate::shell::testing::{CalendarStub, TokenStub, stub_credentials};

/// A client wired to both stubs.
async fn stubbed(calendar: &CalendarStub) -> GoogleCalendarClient {
    let tokens = TokenStub::start(3600).await;
    GoogleCalendarClient::with_base_url(
        Client::new(),
        TokenManager::new(Client::new(), stub_credentials(&tokens.url)),
        &calendar.base_url,
    )
}

pub(super) fn event(title: &str) -> GoogleEvent {
    use crate::core::calendar::EventDateTime;
    GoogleEvent {
        id: None,
        summary: title.to_string(),
        description: None,
        location: None,
        color_id: None,
        start: EventDateTime::timed(
            "2026-08-28T09:00:00+00:00".to_string(),
            "Europe/Brussels".to_string(),
        ),
        end: EventDateTime::timed(
            "2026-08-28T10:00:00+00:00".to_string(),
            "Europe/Brussels".to_string(),
        ),
        transparency: None,
        status: None,
        reminders: None,
        extended_properties: None,
    }
}

#[tokio::test]
async fn a_transient_failure_is_retried_and_the_event_still_lands() {
    // M3's acceptance criterion, written when the feature was
    // frozen and never built until now: "simulate a 429/503
    // followed by success; the event still lands."
    let calendar = CalendarStub::start().await;
    calendar.fail_next(2);
    let client = stubbed(&calendar).await;

    let created = client
        .create_event("household", &event("dentist"))
        .await
        .expect("two 503s in a row must not lose the event");

    assert!(!created.id.unwrap_or_default().is_empty());
    assert_eq!(
        calendar.state.request_count().await,
        3,
        "two failures and the successful third attempt"
    );
}

#[tokio::test]
async fn a_permanent_failure_is_not_retried() {
    // The other half of the classification: hammering a 403 that
    // says "this service account cannot access this calendar"
    // would burn the retry window on something that will never
    // succeed, and delay every other pending entry behind it.
    let calendar = CalendarStub::start().await;
    calendar.reject_next(99);
    let client = stubbed(&calendar).await;

    let error = client
        .create_event("household", &event("dentist"))
        .await
        .unwrap_err();

    assert!(!error.is_transient());
    assert_eq!(
        calendar.state.request_count().await,
        1,
        "a permanent rejection must be attempted exactly once"
    );
}

#[tokio::test]
async fn a_transient_failure_on_the_lookup_is_retried_too() {
    // The upsert path reads before it writes; a 503 on the read
    // must not surface as a failed delivery.
    let calendar = CalendarStub::start().await;
    calendar.fail_next(1);
    let client = stubbed(&calendar).await;

    let found = client
        .find_event_by_property("household", "almanac_source_id", "home-assistant:42")
        .await
        .expect("a transient failure on the lookup must be ridden out");

    assert!(found.is_none());
    assert_eq!(calendar.state.request_count().await, 2);
}

#[tokio::test]
async fn creating_a_calendar_shares_it_in_the_same_breath() {
    // A calendar this service account creates is invisible to every
    // other human until it is shared, and that is easy not to
    // notice: the almanac-test calendar sat that way for months
    // while the live tests wrote into it, and the first two real
    // calendars were created the same way. The two steps are one
    // method so they cannot drift apart again.
    let calendar = CalendarStub::start().await;
    let client = stubbed(&calendar).await;

    let id = client
        .create_calendar("Almanac · Huishouden", "mendax1@example.com")
        .await
        .unwrap();

    assert!(!id.is_empty());
    assert_eq!(
        calendar.shared_with("Almanac · Huishouden").await,
        vec!["mendax1@example.com".to_string()],
        "a calendar created without being shared is a calendar nobody can see"
    );
}

#[tokio::test]
async fn sharing_an_existing_calendar_works_on_its_own_too() {
    // For the calendars that already exist and were created before
    // the two steps were joined.
    let calendar = CalendarStub::start().await;
    let client = stubbed(&calendar).await;

    let id = client
        .create_calendar("Almanac · Infra", "first@example.com")
        .await
        .unwrap();
    client
        .share_calendar(&id, "second@example.com")
        .await
        .unwrap();

    assert_eq!(
        calendar.shared_with("Almanac · Infra").await,
        vec![
            "first@example.com".to_string(),
            "second@example.com".to_string()
        ]
    );
}

#[tokio::test]
async fn an_event_list_longer_than_one_page_comes_back_whole() {
    // Google pages a list past its page size; reading only the first
    // page lost the rest without a word.
    let calendar = CalendarStub::start().await;
    for n in 0..5 {
        let mut seeded = serde_json::to_value(event(&format!("event {n}"))).unwrap();
        seeded["extendedProperties"] = serde_json::json!({"private": {"source_id": "s"}});
        calendar.seed("household", seeded).await;
    }
    calendar.page_by(2);
    let client = stubbed(&calendar).await;

    let events = client
        .list_events_by_property("household", "source_id", "s")
        .await
        .expect("a paged list must still be read");

    assert_eq!(events.len(), 5, "three pages of 2, 2 and 1");
}

#[tokio::test]
async fn a_calendar_list_longer_than_one_page_comes_back_whole() {
    let calendar = CalendarStub::start().await;
    let client = stubbed(&calendar).await;
    for name in ["Alpha", "Beta", "Gamma"] {
        client
            .create_calendar(name, "kenny@example.com")
            .await
            .unwrap();
    }
    calendar.page_by(2);

    let calendars = client
        .list_calendars()
        .await
        .expect("a paged list must still be read");

    assert_eq!(calendars.len(), 3, "two pages of 2 and 1");
}

#[test]
fn an_id_stays_one_path_segment() {
    // Security review 2026-11-02: an id was formatted raw into the URL, so
    // `#` ended the path (Google's holiday calendars carry one) and `/..`
    // reached another API path.
    assert_eq!(
        super::segment("en.be#holiday@group.v.calendar.google.com"),
        "en.be%23holiday@group.v.calendar.google.com"
    );
    assert_eq!(super::segment("x/../acl?y"), "x%2F..%2Facl%3Fy");
    assert_eq!(
        super::segment("abc123_def@group.calendar.google.com"),
        "abc123_def@group.calendar.google.com"
    );
}
