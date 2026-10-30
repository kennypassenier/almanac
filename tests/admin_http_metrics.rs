//! The Prometheus scrape target (M13) and the journal-health reporting, in-process.
//! None of these need Google.

mod common;

use std::sync::Arc;

use axum::http::StatusCode;
use common::inproc::{admin_state, body_text, get, post, scratch_dir};
use tower::ServiceExt;

#[tokio::test]
async fn metrics_answer_without_a_token_so_monitoring_cannot_fail_closed() {
    // M12's rule, and the practical reason for it: a scraper that
    // cannot authenticate reports the service as down. An admin token
    // *is* configured here, so this proves the endpoint is deliberately
    // open rather than merely open by accident.
    let dir = scratch_dir("metrics-open");
    let app = almanac::shell::build_router_with_probes(admin_state(&dir));

    let response = app.oneshot(get("/metrics", None)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(
        content_type.starts_with("text/plain"),
        "the exposition format must not be served as JSON: {content_type}"
    );

    let body = body_text(response).await;
    assert!(body.contains("almanac_events_accepted_total"));
    assert!(body.contains("almanac_journal_pending 0\n"));

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_scrape_never_carries_a_token_a_calendar_id_or_payload_content() {
    // The M13 acceptance criterion, asserted against a state that
    // holds all three: a source token, a calendar id, and a payload
    // with a household detail in it. A metrics database keeps what it
    // scrapes for years and renders it on a dashboard, so a leak here
    // is not a leak that can be taken back.
    let dir = scratch_dir("metrics-no-secrets");
    let state = admin_state(&dir);
    // 4.0.0: the tokens are the kit's and never touch this state; the
    // bearer below is decoration for the in-process router.
    let token = "a-source-token-that-must-never-be-scraped";

    // Accept a real payload so the counters are non-zero and the
    // journal has something in it.
    let app = almanac::shell::build_router_with_probes(Arc::clone(&state));
    let accepted = app
        .oneshot(post(
            "/v1/ingest/home-assistant",
            Some(token),
            r#"{"title":"Sarah's dentist appointment","external_id":"binary_sensor.hallway",
                "start":"2026-08-29T10:00:00Z"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::ACCEPTED);

    let app = almanac::shell::build_router_with_probes(Arc::clone(&state));
    let body = body_text(app.oneshot(get("/metrics", None)).await.unwrap()).await;

    assert!(
        body.contains("almanac_events_accepted_total 1\n"),
        "the acceptance was not counted, so this test would pass vacuously:\n{body}"
    );
    assert!(body.contains("almanac_journal_pending 1\n"));

    for secret in [
        token,
        "primary",
        "Sarah's dentist appointment",
        "binary_sensor.hallway",
        "home-assistant",
    ] {
        assert!(
            !body.contains(secret),
            "a scrape carried {secret:?}:\n{body}"
        );
    }

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn an_unreadable_journal_is_reported_as_unreadable_not_as_empty() {
    // A directory where the journal file should be: reading it fails.
    // The dangerous rendering is "0 pending" — a flat, green backlog
    // graph on a hub that has quietly stopped being able to tell.
    let dir = scratch_dir("metrics-broken-journal");
    let state = admin_state(&dir);
    std::fs::create_dir_all(dir.join("journal.jsonl")).unwrap();

    let app = almanac::shell::build_router_with_probes(state);
    let body = body_text(app.oneshot(get("/metrics", None)).await.unwrap()).await;

    assert!(body.contains("almanac_journal_readable 0\n"));
    assert!(
        !body.contains("almanac_journal_pending 0"),
        "an unreadable journal must not look like an empty one:\n{body}"
    );

    std::fs::remove_dir_all(&dir).ok();
}
