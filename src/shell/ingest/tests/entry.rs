//! Tests of how a request becomes a journal entry.

use super::*;

#[tokio::test]
async fn an_idempotency_key_header_lands_on_the_entry() {
    let state = state_with("home-assistant", "t").await;
    let mut headers = headers_with_token("t");
    headers.insert(IDEMPOTENCY_HEADER, "abc123".parse().unwrap());
    let entry = build_entry(&state, "home-assistant", &headers, json!({}));
    assert_eq!(entry.idempotency_key.as_deref(), Some("abc123"));
}

#[tokio::test]
async fn an_absent_or_blank_idempotency_key_is_none_not_an_empty_string() {
    let state = state_with("home-assistant", "t").await;
    let entry = build_entry(
        &state,
        "home-assistant",
        &headers_with_token("t"),
        json!({}),
    );
    assert_eq!(entry.idempotency_key, None);

    let mut headers = headers_with_token("t");
    headers.insert(IDEMPOTENCY_HEADER, "   ".parse().unwrap());
    let entry = build_entry(&state, "home-assistant", &headers, json!({}));
    assert_eq!(entry.idempotency_key, None);
}

#[tokio::test]
async fn each_accepted_payload_gets_its_own_entry_id() {
    let state = state_with("home-assistant", "t").await;
    let a = build_entry(
        &state,
        "home-assistant",
        &headers_with_token("t"),
        json!({}),
    );
    let b = build_entry(
        &state,
        "home-assistant",
        &headers_with_token("t"),
        json!({}),
    );
    assert_ne!(a.id, b.id);
}
