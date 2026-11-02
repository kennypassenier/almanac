//! A 401 from the Calendar API: one fresh token, then the entry waits
//! (Kenny, form auth-401, 2026-11-02).

use super::tests::event;
use super::*;
use crate::shell::testing::{TokenStub, stub_credentials};

/// A Calendar API that answers 401 to its first `refusals` requests and
/// then 200, recording the bearer token of every request. The shared
/// `CalendarStub` cannot script a 401 and is left alone; this stays local
/// to the 401 tests.
async fn unauthorized_api(refusals: usize) -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
    use axum::http::{HeaderMap, StatusCode};
    let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let log = Arc::clone(&seen);
    let router = axum::Router::new().route(
        "/{calendar_id}/events",
        axum::routing::post(move |headers: HeaderMap| {
            let log = Arc::clone(&log);
            async move {
                let auth = headers["authorization"].to_str().unwrap().to_string();
                let mut log = log.lock().unwrap();
                log.push(auth);
                if log.len() <= refusals {
                    (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({})))
                } else {
                    (
                        StatusCode::OK,
                        axum::Json(serde_json::json!({
                            "id": "e1",
                            "summary": "dentist",
                            "start": {"dateTime": "2026-08-28T09:00:00+00:00", "timeZone": "UTC"},
                            "end": {"dateTime": "2026-08-28T10:00:00+00:00", "timeZone": "UTC"}
                        })),
                    )
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (format!("http://{address}"), seen)
}

#[tokio::test]
async fn a_401_is_retried_once_with_a_fresh_token() {
    let tokens = TokenStub::start(3600).await;
    let (url, seen) = unauthorized_api(1).await;
    let client = GoogleCalendarClient::with_base_url(
        Client::new(),
        TokenManager::new(Client::new(), stub_credentials(&tokens.url)),
        url,
    );

    client
        .create_event("household", &event("dentist"))
        .await
        .expect("a stale token must be replaced, not reported");

    let seen = seen.lock().unwrap();
    assert_eq!(*seen, ["Bearer stub-token-0", "Bearer stub-token-1"]);
    assert_eq!(tokens.state.hits(), 2, "exactly one extra token fetch");
}

#[tokio::test]
async fn a_401_that_survives_a_fresh_token_is_transient_and_names_the_sign_in() {
    let tokens = TokenStub::start(3600).await;
    let (url, seen) = unauthorized_api(99).await;
    let client = GoogleCalendarClient::with_base_url(
        Client::new(),
        TokenManager::new(Client::new(), stub_credentials(&tokens.url)),
        url,
    );

    let error = client
        .create_event("household", &event("dentist"))
        .await
        .unwrap_err();

    assert!(error.is_transient(), "the entry must keep waiting");
    let text = error.to_string();
    assert!(text.contains("refused Almanac's sign-in"), "{text}");
    assert!(text.contains("machine clock"), "{text}");
    assert_eq!(seen.lock().unwrap().len(), 2, "one resend, no backoff loop");
}
