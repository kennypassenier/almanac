//! Tests of the synchronous endpoint (K8).

use super::*;

#[tokio::test]
async fn the_synchronous_endpoint_delivers_and_returns_the_event_id() {
    // K8: a Claude session posts and wants the Google event id
    // back. Nothing tested this endpoint at all — not the happy
    // path, not the response shape.
    let calendar = crate::shell::testing::CalendarStub::start().await;
    let state = Arc::new(state_with_calendar("home-assistant", "tok", &calendar).await);

    let (status, Json(body)) = ingest_sync(
        State(Arc::clone(&state)),
        Path("home-assistant".to_string()),
        Caller::Admin,
        headers_with_token("tok"),
        Json(sync_payload()),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "delivered");
    assert!(
        body["event_id"].as_str().is_some_and(|id| !id.is_empty()),
        "the caller needs a real event id back, got {body}"
    );
    assert_eq!(body["created"], true);

    assert!(
        state.journal.pending().unwrap().is_empty(),
        "a delivered entry must be marked done"
    );

    // homelab ask-6: this path delivered without counting it, so a
    // source that only posts here read as accepted 2, delivered 0.
    let counts = state.metrics.snapshot();
    assert_eq!(counts.accepted, 1);
    assert_eq!(
        counts.delivered, 1,
        "a synchronous delivery must be counted"
    );
    assert_eq!(counts.failed, 0);
    assert_eq!(
        state.routes.lock().await.len(),
        1,
        "and must show on the debug surface like a worker delivery"
    );
}

#[tokio::test]
async fn the_synchronous_endpoint_rejects_a_wrong_token() {
    // The auth guard on this route was covered by nothing.
    let calendar = crate::shell::testing::CalendarStub::start().await;
    let state = Arc::new(state_with_calendar("home-assistant", "tok", &calendar).await);

    let (status, _) = ingest_sync(
        State(Arc::clone(&state)),
        Path("home-assistant".to_string()),
        client("someone-else"),
        headers_with_token("wrong-token"),
        Json(sync_payload()),
    )
    .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(
        state.journal.pending().unwrap().is_empty(),
        "an unauthenticated request must journal nothing"
    );
    assert_eq!(
        calendar.state.request_count().await,
        0,
        "and must never reach Google"
    );
}

#[tokio::test]
async fn a_failed_synchronous_delivery_reports_502_but_keeps_the_payload() {
    // The promise in the handler's own comment: the caller is told
    // it failed, and the entry stays pending so the worker retries
    // it. Losing the payload here would make the synchronous
    // endpoint strictly worse than the asynchronous one.
    let calendar = crate::shell::testing::CalendarStub::start().await;
    calendar.reject_next(99);
    let state = Arc::new(state_with_calendar("home-assistant", "tok", &calendar).await);

    let (status, _) = ingest_sync(
        State(Arc::clone(&state)),
        Path("home-assistant".to_string()),
        Caller::Admin,
        headers_with_token("tok"),
        Json(sync_payload()),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(
        state.journal.pending().unwrap().len(),
        1,
        "the payload must survive for the worker to retry"
    );
    let counts = state.metrics.snapshot();
    assert_eq!(counts.delivered, 0);
    assert_eq!(
        counts.failed, 1,
        "a failed attempt is counted like the worker's"
    );
}
