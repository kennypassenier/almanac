//! The ingest surface as real HTTP (K7, M7): payloads refused at the door instead of being accepted blindly.
//! Drives the router directly; nothing here needs Google.

mod common;

use std::sync::Arc;

use almanac::shell::build_router_with_probes;
use axum::http::StatusCode;
use common::inproc::{HA_TOKEN, KUMA_TOKEN, ingest_state, post, scratch_dir};
use tower::ServiceExt;

#[tokio::test]
async fn a_payload_using_every_option_is_accepted_at_the_http_layer() {
    // K9's criterion said "an E2E test per alert system", and until
    // 2.0.0 this ran the grafana and uptime-kuma fixtures through auth,
    // the router and the journal — the only place those payloads ever
    // met anything but the mapping engine.
    //
    // Those fixtures went with the translation layer they existed to
    // prove. What still matters is the same claim about the shape that
    // replaced them: a call using every per-event option must survive
    // the whole HTTP path, not just the mapper. A content type or a
    // field the ingest layer refuses would otherwise surface only when
    // a real event failed to appear.
    let dir = scratch_dir("every-option");
    let state = ingest_state(&dir);

    let payload = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/payloads/everything_sample.json"
    ))
    .unwrap();

    let response = build_router_with_probes(Arc::clone(&state))
        .oneshot(post("/v1/ingest/uptime-kuma", Some(KUMA_TOKEN), &payload))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::ACCEPTED,
        "a payload using every option must be accepted as it is actually sent"
    );

    assert_eq!(
        state.journal.pending().unwrap().len(),
        1,
        "and it must be durably journalled"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_payload_almanac_could_never_map_is_refused_at_the_door() {
    // Kenny's decision, 2026-09-03, after the JobTracker session sent a
    // date without `all_day` and got a reassuring answer. A payload that
    // can only ever fail must not be stored: it would be retried until
    // it dead-letters, in a list nobody watches.
    //
    // 422 rather than a new field in the body. HTTP already separates
    // "your request is wrong" from "my upstream broke", and almanac was
    // answering 502 for both.
    let dir = scratch_dir("unmappable");
    let state = ingest_state(&dir);

    let response = build_router_with_probes(Arc::clone(&state))
        .oneshot(post(
            "/v1/ingest/home-assistant",
            Some(HA_TOKEN),
            r#"{"title": "Dagmarkering", "start": "2026-09-03", "external_id": "day-1"}"#,
        ))
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "a date sent without all_day can never become an event; it must be refused, not accepted"
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        body["remedy"]
            .as_str()
            .unwrap_or_default()
            .contains("all_day"),
        "the refusal must name the way out, not only the problem: {body}"
    );

    assert!(
        state.journal.pending().unwrap().is_empty(),
        "nothing that can never be delivered may enter the journal"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_misspelled_field_is_named_instead_of_answered_with_202() {
    // The same fault from the asynchronous side. `deny_unknown_fields`
    // has always caught this, but it caught it in the delivery worker —
    // long after the sender had been told "accepted".
    let dir = scratch_dir("misspelled");
    let state = ingest_state(&dir);

    let response = build_router_with_probes(Arc::clone(&state))
        .oneshot(post(
            "/v1/ingest/home-assistant",
            Some(HA_TOKEN),
            r#"{"title": "Tikfout", "start": "2026-09-03T10:00:00+02:00",
                "external_id": "typo-1", "allDay": true}"#,
        ))
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "a field almanac does not know is a mistake worth naming while the sender can still act"
    );
    assert!(
        state.journal.pending().unwrap().is_empty(),
        "a payload with a misspelled field must not be journalled"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn the_synchronous_path_refuses_an_unmappable_payload_before_journalling_it() {
    // The sync path is where the ambiguity actually hurt: it answered
    // 502 both when Google hiccuped and when the body was unusable, so
    // a caller could not tell "wait" from "waiting will never help".
    // After this, 502 on this path means Google and only Google.
    let dir = scratch_dir("sync-unmappable");
    let state = ingest_state(&dir);

    let response = build_router_with_probes(Arc::clone(&state))
        .oneshot(post(
            "/v1/ingest/home-assistant/sync",
            Some(HA_TOKEN),
            r#"{"title": "Geen titel is geen probleem", "start": "morgenvroeg",
                "external_id": "sync-1"}"#,
        ))
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "an unusable body must not come back as 502; that code now means Google"
    );
    assert!(
        state.journal.pending().unwrap().is_empty(),
        "the sync path must not journal what it just refused"
    );

    std::fs::remove_dir_all(&dir).ok();
}
