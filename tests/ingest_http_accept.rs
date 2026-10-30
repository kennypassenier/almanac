//! The ingest surface as real HTTP (K7, M7): accepted payloads and the guarantee that a 202 only follows durable journalling.
//! Drives the router directly; nothing here needs Google. The door (K6) is the
//! kit's since 4.0.0 and is proven in tests/kit_door.rs.

mod common;

use std::sync::Arc;

use almanac::shell::build_router_with_probes;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::inproc::{HA_TOKEN, ha_payload, ingest_state, post, scratch_dir, state_with_journal};
use tower::ServiceExt;

#[tokio::test]
async fn a_valid_home_assistant_payload_is_accepted_and_journalled() {
    let dir = scratch_dir("accept");
    let state = ingest_state(&dir);
    let app = build_router_with_probes(Arc::clone(&state));

    let response = app
        .oneshot(post(
            "/v1/ingest/home-assistant",
            Some(HA_TOKEN),
            ha_payload(),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // The 202 must mean "durably stored", not "we'll get to it".
    let pending = state.journal.pending().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].source_id, "home-assistant");
    assert_eq!(pending[0].payload["title"], "Wasmachine klaar");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn an_idempotency_key_header_is_recorded_on_the_journal_entry() {
    // M7: the header is what lets a source without a natural external
    // id have its retries converge instead of duplicating.
    let dir = scratch_dir("idempotency");
    let state = ingest_state(&dir);
    let app = build_router_with_probes(Arc::clone(&state));

    let request = Request::builder()
        .method("POST")
        .uri("/v1/ingest/home-assistant")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {HA_TOKEN}"))
        .header("idempotency-key", "run-42")
        .body(Body::from(ha_payload().to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    let pending = state.journal.pending().unwrap();
    assert_eq!(pending[0].idempotency_key.as_deref(), Some("run-42"));

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn two_accepted_payloads_both_survive_in_the_journal() {
    let dir = scratch_dir("two");
    let state = ingest_state(&dir);

    for _ in 0..2 {
        let response = build_router_with_probes(Arc::clone(&state))
            .oneshot(post(
                "/v1/ingest/home-assistant",
                Some(HA_TOKEN),
                ha_payload(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);
    }

    assert_eq!(state.journal.pending().unwrap().len(), 2);

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_journal_that_cannot_be_written_answers_500_so_the_sender_retries() {
    // AR16's one silent-data-loss path. The rule is: if the write
    // fails, say so, because Home Assistant's retry script only tries
    // again on a failure. A regression to "log it and answer 202
    // anyway" means a full disk quietly eats events while every sender
    // believes it succeeded — and nothing on the dashboard would show
    // it either.
    let dir = scratch_dir("journal-readonly");
    let readonly = dir.join("readonly");
    std::fs::create_dir_all(&readonly).unwrap();

    // A state whose journal points inside a directory nothing may
    // write to; the door is the kit's, so the request reaches the write.

    let state = state_with_journal(readonly.join("journal.jsonl"), &["home-assistant"]);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&readonly, std::fs::Permissions::from_mode(0o500)).unwrap();
    }

    let response = build_router_with_probes(Arc::clone(&state))
        .oneshot(post(
            "/v1/ingest/home-assistant",
            Some(HA_TOKEN),
            ha_payload(),
        ))
        .await
        .unwrap();

    // Restore permissions before asserting, so a failure still cleans up.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&readonly, std::fs::Permissions::from_mode(0o700)).ok();
    }
    let status = response.status();
    std::fs::remove_dir_all(&dir).ok();

    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "an unwritable journal must be reported, never answered with 202"
    );
}
