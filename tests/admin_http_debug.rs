//! The operator surface as real HTTP, in-process as the admin: health (M1), debug status (K11) and dry-run (M9).
//! None of these need Google. The door (who may call what) is the kit's since
//! 4.0.0: tests/kit_door.rs.

mod common;

use axum::http::StatusCode;
use common::inproc::{admin_state, body_json, get, post, scratch_dir};
use tower::ServiceExt;

const ADMIN_TOKEN: &str = "bootstrap-admin-token";

#[tokio::test]
async fn health_answers_without_a_token() {
    // M1: a monitoring stack that fails closed lies to you during an
    // outage, so this must never require credentials.
    let dir = scratch_dir("health");
    let app = almanac::shell::build_router_with_probes(admin_state(&dir));

    let response = app.oneshot(get("/healthz", None)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_json(response).await["status"], "ok");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn the_debug_status_reports_profiles_and_the_journal() {
    let dir = scratch_dir("status");
    let app = almanac::shell::build_router_with_probes(admin_state(&dir));

    let response = app
        .oneshot(get("/v1/debug/status", Some(ADMIN_TOKEN)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = body_json(response).await;
    assert_eq!(body["profiles"][0]["source_id"], "home-assistant");
    assert_eq!(body["profiles"][0]["target_calendar_id"], "primary");
    assert_eq!(body["journal"]["count"], 0);

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn dry_run_shows_the_event_without_writing_it() {
    // M9: check a profile against a real payload before letting it
    // near a calendar.
    let dir = scratch_dir("dryrun");
    let app = almanac::shell::build_router_with_probes(admin_state(&dir));

    let response = app
        .oneshot(post(
            "/v1/debug/dry-run/home-assistant",
            Some(ADMIN_TOKEN),
            r#"{"external_id":"switch.wasmachine","title":"Wasmachine klaar","start":"2026-08-28T09:00:00+00:00"}"#,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["would_write_to_calendar"], "primary");
    assert_eq!(body["event"]["summary"], "Wasmachine klaar");
    assert_eq!(
        body["event"]["end"]["dateTime"],
        "2026-08-28T10:00:00+00:00"
    );
    assert_eq!(
        body["event"]["extendedProperties"]["private"]["almanac_source_id"],
        "home-assistant:switch.wasmachine"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn dry_run_explains_a_payload_the_profile_cannot_map() {
    let dir = scratch_dir("dryrun-bad");
    let app = almanac::shell::build_router_with_probes(admin_state(&dir));

    let response = app
        .oneshot(post(
            "/v1/debug/dry-run/home-assistant",
            Some(ADMIN_TOKEN),
            r#"{"title":"no start field here"}"#,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = body_json(response).await;
    assert!(body["message"].as_str().unwrap().contains("start"));
    assert!(!body["remedy"].as_str().unwrap().is_empty());

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn dry_run_on_an_unknown_source_says_where_to_look() {
    let dir = scratch_dir("dryrun-unknown");
    let app = almanac::shell::build_router_with_probes(admin_state(&dir));

    let response = app
        .oneshot(post("/v1/debug/dry-run/nope", Some(ADMIN_TOKEN), "{}"))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(
        body_json(response).await["remedy"]
            .as_str()
            .unwrap()
            .contains("/v1/debug/status")
    );

    std::fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------
// M13 — the Prometheus scrape target.
// ---------------------------------------------------------------
