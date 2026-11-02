//! Route handlers of the calendar stub. Each one records the call,
//! then answers the way Google would, including the scripted failures.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde_json::{Value, json};

use super::{CalendarState, Created};

/// The 503 body Google actually sends, so `extract_reason` sees the
/// shape it parses in production rather than an empty string.
fn transient_body() -> Value {
    json!({"error": {"errors": [{"reason": "backendError"}], "message": "backend error"}})
}

/// Google's permission-denied 403 — the same status code it uses for
/// rate limiting, which is exactly why the reason string matters.
fn permanent_body() -> Value {
    json!({"error": {"errors": [{"reason": "permissionDenied"}],
                     "message": "the service account cannot access this calendar"}})
}

/// One page of `items`: the `pageToken` query is the offset to start
/// from, and `nextPageToken` is sent while items remain. With no page
/// size set, everything comes back at once.
fn page(state: &CalendarState, query: &HashMap<String, String>, items: Vec<Value>) -> Value {
    let size = state.page_size.load(Ordering::SeqCst);
    if size == 0 {
        return json!({ "items": items });
    }
    let start: usize = query
        .get("pageToken")
        .map(|token| {
            token
                .parse()
                .expect("the stub only hands out numeric page tokens")
        })
        .unwrap_or(0);
    let end = (start + size).min(items.len());
    let mut body = json!({ "items": items[start.min(end)..end] });
    if end < items.len() {
        body["nextPageToken"] = json!(end.to_string());
    }
    body
}

pub(super) async fn delete_calendar(
    State(state): State<Arc<CalendarState>>,
    Path(calendar_id): Path<String>,
) -> StatusCode {
    state.calendars.lock().await.remove(&calendar_id);
    StatusCode::NO_CONTENT
}

/// The calendars this "account" can see — whatever the stub has been
/// asked to create. Enough for K21's find-or-create to be tested
/// without a live Google.
pub(super) async fn list_calendars(
    State(state): State<Arc<CalendarState>>,
    axum::extract::Query(query): axum::extract::Query<HashMap<String, String>>,
) -> (StatusCode, axum::Json<Value>) {
    let calendars = state.calendars.lock().await;
    let items: Vec<Value> = calendars
        .iter()
        .filter(|(_, created)| created.listed)
        .map(|(id, created)| json!({"id": id, "summary": created.name}))
        .collect();
    (StatusCode::OK, axum::Json(page(&state, &query, items)))
}

pub(super) async fn list_events(
    State(state): State<Arc<CalendarState>>,
    Path(calendar_id): Path<String>,
    axum::extract::Query(query): axum::extract::Query<HashMap<String, String>>,
) -> (StatusCode, axum::Json<Value>) {
    state.record("GET", &calendar_id).await;
    if state.take_rejection() {
        return (StatusCode::FORBIDDEN, axum::Json(permanent_body()));
    }
    if state.take_failure() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(transient_body()),
        );
    }

    // Google filters on `privateExtendedProperty=key=value`, and so
    // must this: a stub that returns everything regardless of the
    // query would make an upsert lookup — and the delete that shares
    // it — appear to work while matching the wrong event, which is
    // precisely the cross-source bug the tests are meant to catch.
    let filter = query.get("privateExtendedProperty").cloned();
    // Like Google: a cancelled event is hidden unless the caller asks.
    let show_deleted = query.get("showDeleted").is_some_and(|v| v == "true");
    let existing = state.existing.lock().await;
    let items: Vec<Value> = existing
        .get(&calendar_id)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|event| show_deleted || event["status"] != json!("cancelled"))
        .filter(|event| match &filter {
            None => true,
            Some(filter) => {
                let Some((key, value)) = filter.split_once('=') else {
                    return false;
                };
                event["extendedProperties"]["private"][key] == json!(value)
            }
        })
        .collect();

    (StatusCode::OK, axum::Json(page(&state, &query, items)))
}

pub(super) async fn create_event(
    State(state): State<Arc<CalendarState>>,
    Path(calendar_id): Path<String>,
    axum::Json(body): axum::Json<Value>,
) -> (StatusCode, axum::Json<Value>) {
    state.record("POST", &calendar_id).await;
    if state.take_rejection() {
        return (StatusCode::FORBIDDEN, axum::Json(permanent_body()));
    }
    if state.take_failure() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(transient_body()),
        );
    }

    let id = format!("event-{}", state.next_id.fetch_add(1, Ordering::SeqCst));
    state.created.lock().await.push((calendar_id, body.clone()));

    let mut created = body;
    created["id"] = json!(id);
    (StatusCode::OK, axum::Json(created))
}

pub(super) async fn update_event(
    State(state): State<Arc<CalendarState>>,
    Path((calendar_id, event_id)): Path<(String, String)>,
    axum::Json(body): axum::Json<Value>,
) -> (StatusCode, axum::Json<Value>) {
    state.record("PUT", &calendar_id).await;
    if state.take_rejection() {
        return (StatusCode::FORBIDDEN, axum::Json(permanent_body()));
    }
    if state.take_failure() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(transient_body()),
        );
    }

    let mut updated = body;
    updated["id"] = json!(event_id);
    (StatusCode::OK, axum::Json(updated))
}

pub(super) async fn delete_event(
    State(state): State<Arc<CalendarState>>,
    Path((calendar_id, _event_id)): Path<(String, String)>,
) -> StatusCode {
    state.record("DELETE", &calendar_id).await;
    if state.take_rejection() {
        return StatusCode::FORBIDDEN;
    }
    if state.take_failure() {
        return StatusCode::SERVICE_UNAVAILABLE;
    }
    StatusCode::NO_CONTENT
}

pub(super) async fn create_calendar(
    State(state): State<Arc<CalendarState>>,
    axum::Json(body): axum::Json<Value>,
) -> (StatusCode, axum::Json<Value>) {
    state.record("POST", "<new calendar>").await;
    let id = format!("calendar-{}", state.next_id.fetch_add(1, Ordering::SeqCst));
    state.calendars.lock().await.insert(
        id.clone(),
        Created {
            name: body["summary"].as_str().unwrap_or_default().to_string(),
            shared_with: Vec::new(),
            listed: !state.lag_new_calendars.load(Ordering::SeqCst),
        },
    );
    (StatusCode::OK, axum::Json(json!({"id": id})))
}

pub(super) async fn insert_acl(
    State(state): State<Arc<CalendarState>>,
    Path(calendar_id): Path<String>,
    axum::Json(rule): axum::Json<Value>,
) -> (StatusCode, axum::Json<Value>) {
    state.record("ACL", &calendar_id).await;
    if let Some(created) = state.calendars.lock().await.get_mut(&calendar_id)
        && let Some(who) = rule["scope"]["value"].as_str()
    {
        created.shared_with.push(who.to_string());
    }
    (StatusCode::OK, axum::Json(rule))
}
