//! The Try a source page (/try) and its JSON routes.
//!
//! The run route is the dashboard-session twin of `POST
//! /v1/debug/dry-run/{source_id}`: the same mapping, plus the two things
//! that dry-run leaves out and the page needs. It applies ingest's rule
//! that a call needs an `external_id` or an `Idempotency-Key`, so
//! "accepted" means ingest would accept it, and it looks up the event
//! already in the target calendar (read only) so the page can say new,
//! update or no change and show the difference. Nothing is ever written.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use chassis::{Dashboard, Error};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::core::calendar::{ExtendedProperties, GoogleEvent};
use crate::core::dry_run::{self, Outcome};
use crate::core::journal::Entry;
use crate::core::mapping::map_payload;
use crate::core::profile::{FALLBACK_DURATION_MINUTES, Profile};
use crate::shell::delivery::{UPSERT_PROPERTY, upsert_key};
use crate::shell::ingest::{AppState, requires_an_id};

const PAGE_HTML: &str = include_str!("../../../templates/try.html");

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/try", get(page))
        .route("/try/api/sources", get(sources))
        .route("/try/api/run/{source_id}", post(run))
}

async fn page(Extension(dash): Extension<Dashboard>) -> Response {
    match dash.render_project("/try", PAGE_HTML, json!({})) {
        Ok(html) => html.into_response(),
        Err(e) => e.into_response(),
    }
}

/// The text of the profile file that declares `source_id`, with its file
/// name. Found by what the file says, not by what it is called: a profile
/// placed by hand need not be named after its source.
fn profile_file(dir: &std::path::Path, source_id: &str) -> Option<(String, String)> {
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
        .collect();
    paths.sort();
    paths.into_iter().find_map(|path| {
        let text = std::fs::read_to_string(&path).ok()?;
        let declared = text.parse::<toml::Table>().ok()?;
        (declared.get("source_id")?.as_str()? == source_id).then(|| {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
            (name.unwrap_or_default(), text)
        })
    })
}

fn source_json(state: &AppState, profile: &Profile) -> Value {
    let file = profile_file(&state.profiles_dir, &profile.source_id);
    json!({
        "source_id": profile.source_id,
        "calendar_id": profile.target_calendar_id,
        "calendar_name": state.calendar_name(&profile.target_calendar_id),
        "timezone": profile.timezone,
        "default_duration_minutes": profile.default_duration_minutes.unwrap_or(FALLBACK_DURATION_MINUTES),
        "default_is_fallback": profile.default_duration_minutes.is_none(),
        "schema_version": profile.schema_version,
        "file_name": file.as_ref().map(|(name, _)| name),
        "toml": file.map(|(_, text)| text),
    })
}

/// `GET /try/api/sources` — the loaded profiles, with their files, for the
/// pickers and the profile view.
async fn sources(State(state): State<Arc<AppState>>) -> Json<Value> {
    let loaded = state.profiles();
    let mut profiles: Vec<&Profile> = loaded.values().collect();
    profiles.sort_by(|a, b| a.source_id.cmp(&b.source_id));
    let list: Vec<Value> = profiles.iter().map(|p| source_json(&state, p)).collect();
    Json(json!({ "sources": list }))
}

#[derive(Deserialize)]
struct RunRequest {
    /// The event call, as the source would send it.
    event: Value,
    /// The `Idempotency-Key` header the call would carry, if any.
    #[serde(default)]
    idempotency_key: Option<String>,
}

/// The origin the dashboard was reached at, for the curl command: the
/// proxy's scheme and the Host the browser used.
fn origin_of(headers: &HeaderMap) -> String {
    let text = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    let scheme = text("x-forwarded-proto").unwrap_or("http");
    let host = text("host").unwrap_or("localhost");
    format!("{scheme}://{host}")
}

fn refusal(status: StatusCode, body: &Value) -> (StatusCode, Value) {
    (status, body.clone())
}

/// `POST /try/api/run/{source_id}` — what Almanac would do with this call,
/// and nothing else: always 200 with the verdict, the path and the answer
/// the real dry-run gives; a call that is itself unreadable is an error.
async fn run(
    State(state): State<Arc<AppState>>,
    Path(source_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, Error> {
    let request: RunRequest = serde_json::from_slice(&body).map_err(|e| {
        Error::invalid(
            format!("the request to Try is not readable: {e}"),
            "send {\"event\": {...}} as JSON",
        )
    })?;
    let origin = origin_of(&headers);
    let urls = json!({ "dry_run": format!("{origin}/v1/debug/dry-run/{source_id}") });

    let profiles = state.profiles();
    let Some(profile) = profiles.get(&source_id) else {
        let hops = dry_run::hops(
            &source_id,
            "",
            &Outcome::UnknownSource {
                source_id: &source_id,
            },
        );
        return Ok(Json(json!({
            "http": 404,
            "answer": {
                "status": "error",
                "message": format!("no profile with source_id \"{source_id}\""),
                "remedy": "check the loaded profiles at /v1/debug/status",
            },
            "hops": hops, "verdict": null, "existing": null, "lookup_failed": false,
            "calendar_name": null, "urls": urls,
        })));
    };
    let calendar_name = state.calendar_name(&profile.target_calendar_id);

    // The same two refusals ingest gives, in ingest's order: the id first.
    let mut request_headers = HeaderMap::new();
    if let Some(key) = request.idempotency_key.as_deref()
        && let Ok(value) = HeaderValue::from_str(key)
    {
        request_headers.insert(HeaderName::from_static("idempotency-key"), value);
    }
    let origin_label = format!("profile {source_id}");
    let refused = match requires_an_id(&request.event, &request_headers) {
        Err((status, Json(body))) => Some(refusal(status, &body)),
        Ok(()) => match map_payload(&request.event, profile, &origin_label) {
            Ok(_) => None,
            Err(e) => Some(refusal(
                StatusCode::UNPROCESSABLE_ENTITY,
                &json!({"status": "error", "message": e.to_string(), "remedy": e.remedy()}),
            )),
        },
    };
    if let Some((status, answer)) = refused {
        let message = answer["message"].as_str().unwrap_or_default();
        // The step's note is the message without the profile's name in front
        // and without serde's long list of allowed fields.
        let note = message
            .split(" — ")
            .next()
            .unwrap_or(message)
            .trim_start_matches(&format!("{origin_label}: "))
            .split(", expected one of")
            .next()
            .unwrap_or_default()
            .to_string();
        let outcome = Outcome::Refused {
            rule: dry_run::rule_of(message).to_string(),
            note,
        };
        return Ok(Json(json!({
            "http": status.as_u16(),
            "answer": answer,
            "hops": dry_run::hops(&source_id, &calendar_name, &outcome),
            "verdict": null, "existing": null, "lookup_failed": false,
            "calendar_name": calendar_name, "urls": urls,
        })));
    }

    // Accepted: map again for the event itself (cheap and pure).
    let mut event: GoogleEvent = map_payload(&request.event, profile, &origin_label)
        .map_err(|e| Error::internal(e.to_string(), e.remedy().to_string()))?;
    let entry = Entry {
        id: String::new(),
        source_id: source_id.clone(),
        received_at: String::new(),
        payload: request.event.clone(),
        idempotency_key: request
            .idempotency_key
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .map(str::to_string),
    };
    let key = upsert_key(&entry, &event);
    // Delivery stamps the marker on an event whose call had no external_id.
    if let Some(key) = &key
        && event.extended_properties.is_none()
    {
        event.extended_properties = Some(ExtendedProperties {
            private: std::iter::once((UPSERT_PROPERTY.to_string(), key.clone())).collect(),
        });
    }
    // Read only: the same lookup delivery does before it creates or updates.
    let (existing, lookup_failed) = match &key {
        None => (None, false),
        Some(key) => match state
            .client
            .find_event_by_property(&profile.target_calendar_id, UPSERT_PROPERTY, key)
            .await
        {
            Ok(found) => (found, false),
            Err(e) => {
                tracing::warn!(source_id = %source_id, error = %e, "try: could not look up the existing event");
                (None, true)
            }
        },
    };
    let verdict = (!lookup_failed).then(|| dry_run::verdict(existing.as_ref(), &event));
    let outcome = Outcome::Mapped {
        summary: &event.summary,
        all_day: event.start.is_all_day(),
        verdict,
    };
    let hops = dry_run::hops(&source_id, &calendar_name, &outcome);
    let existing = existing.map(|mut found| {
        found.id = None;
        found
    });
    Ok(Json(json!({
        "http": 200,
        "answer": {
            "status": "ok",
            "would_write_to_calendar": profile.target_calendar_id,
            "event": event,
        },
        "hops": hops,
        "verdict": verdict,
        "existing": existing,
        "lookup_failed": lookup_failed,
        "calendar_name": calendar_name,
        "urls": urls,
    })))
}
