//! Tests of deleting an event by its source (K8).

use super::*;

#[tokio::test]
async fn a_source_can_delete_the_event_it_created() {
    // K8's criterion says create, update *and* delete. The verb was
    // never built; Kenny asked for it when the gap was reported.
    let calendar = crate::shell::testing::CalendarStub::start().await;
    calendar
        .seed(
            "primary",
            serde_json::json!({
                "id": "google-event-1",
                "summary": "meeting",
                "start": {"dateTime": "2026-08-29T09:00:00+00:00", "timeZone": "UTC"},
                "end": {"dateTime": "2026-08-29T10:00:00+00:00", "timeZone": "UTC"},
                "extendedProperties": {
                    "private": {"almanac_source_id": "home-assistant:task-7"}
                }
            }),
        )
        .await;
    let state = Arc::new(state_with_calendar("home-assistant", "tok", &calendar).await);

    let (status, Json(body)) = delete_event(
        State(Arc::clone(&state)),
        Path(("home-assistant".to_string(), "task-7".to_string())),
        Caller::Admin,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "deleted");
    assert_eq!(body["event_id"], "google-event-1");
    assert!(
        calendar
            .state
            .requests
            .lock()
            .await
            .iter()
            .any(|(method, _)| method == "DELETE"),
        "it must actually have asked Google to delete it"
    );
}

#[tokio::test]
async fn deleting_something_that_is_not_there_says_so_rather_than_pretending() {
    // A caller retrying a delete needs to tell "already gone" from
    // "just removed", and answering 200 for a wrong external id
    // would hide the mistake forever.
    let calendar = crate::shell::testing::CalendarStub::start().await;
    let state = Arc::new(state_with_calendar("home-assistant", "tok", &calendar).await);

    let (status, Json(body)) = delete_event(
        State(Arc::clone(&state)),
        Path(("home-assistant".to_string(), "never-existed".to_string())),
        Caller::Admin,
    )
    .await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["status"], "not_found");
    assert!(
        body["remedy"].as_str().unwrap().contains("external id"),
        "and point at the likely cause"
    );
}

#[tokio::test]
async fn one_source_cannot_delete_another_sources_event() {
    // The upsert key is prefixed with the source id, so even a
    // correct guess of someone else's external id addresses a key
    // this source can never name.
    let calendar = crate::shell::testing::CalendarStub::start().await;
    calendar
        .seed(
            "primary",
            serde_json::json!({
                "id": "google-event-1",
                "summary": "someone else's event",
                "start": {"dateTime": "2026-08-29T09:00:00+00:00", "timeZone": "UTC"},
                "end": {"dateTime": "2026-08-29T10:00:00+00:00", "timeZone": "UTC"},
                "extendedProperties": {
                    "private": {"almanac_source_id": "uptime-kuma:task-7"}
                }
            }),
        )
        .await;
    let state = Arc::new(state_with_calendar("home-assistant", "tok", &calendar).await);

    let (status, _) = delete_event(
        State(Arc::clone(&state)),
        Path(("home-assistant".to_string(), "task-7".to_string())),
        Caller::Admin,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "another source's event must be invisible, not deletable"
    );
    assert!(
        !calendar
            .state
            .requests
            .lock()
            .await
            .iter()
            .any(|(method, _)| method == "DELETE"),
        "and nothing may be deleted"
    );
}

#[tokio::test]
async fn deleting_needs_this_sources_own_token() {
    let calendar = crate::shell::testing::CalendarStub::start().await;
    let state = Arc::new(state_with_calendar("home-assistant", "tok", &calendar).await);

    let (status, _) = delete_event(
        State(Arc::clone(&state)),
        Path(("home-assistant".to_string(), "task-7".to_string())),
        client("someone-else"),
    )
    .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        calendar.state.request_count().await,
        0,
        "an unauthenticated delete must never reach Google"
    );
}

#[tokio::test]
async fn a_delete_that_google_refuses_is_reported_rather_than_claimed() {
    let calendar = crate::shell::testing::CalendarStub::start().await;
    calendar.reject_next(99);
    let state = Arc::new(state_with_calendar("home-assistant", "tok", &calendar).await);

    let (status, _) = delete_event(
        State(Arc::clone(&state)),
        Path(("home-assistant".to_string(), "task-7".to_string())),
        Caller::Admin,
    )
    .await;

    assert_eq!(status, StatusCode::BAD_GATEWAY);
}
