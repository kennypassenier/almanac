//! `POST /try/api/run/{source_id}`: what Almanac would do with one call,
//! answered in the same steps and words as the real dry-run.

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use chassis::Error;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::core::calendar::{ExtendedProperties, GoogleEvent};
use crate::core::dry_run::{self, Outcome};
use crate::core::journal::Entry;
use crate::core::mapping::map_payload;
use crate::core::profile::Profile;
use crate::shell::delivery::{UPSERT_PROPERTY, upsert_key};
use crate::shell::ingest::requires_an_id;
use crate::shell::state::AppState;

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

/// `POST /try/api/run/{source_id}` — what Almanac would do with this call,
/// and nothing else: always 200 with the verdict, the path and the answer
/// the real dry-run gives; a call that is itself unreadable is an error.
pub(super) async fn run(
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
    let urls =
        json!({ "dry_run": format!("{}/v1/debug/dry-run/{source_id}", origin_of(&headers)) });
    let profiles = state.profiles();
    let Some(profile) = profiles.get(&source_id) else {
        return Ok(Json(unknown_source(&source_id, urls)));
    };
    let calendar_name = state.calendar_name(&profile.target_calendar_id);
    let origin_label = format!("profile {source_id}");
    if let Some((status, answer)) = refusal(&request, profile, &origin_label)? {
        let outcome = refused_outcome(&answer, &origin_label);
        return Ok(Json(json!({
            "http": status.as_u16(),
            "answer": answer,
            "hops": dry_run::hops(&source_id, &calendar_name, &outcome),
            "verdict": null, "existing": null, "lookup_failed": false,
            "calendar_name": calendar_name, "urls": urls,
        })));
    }
    let reply = accepted(&state, &request, profile, &origin_label).await?;
    Ok(Json(reply.into_json(
        &source_id,
        profile,
        &calendar_name,
        urls,
    )))
}

fn unknown_source(source_id: &str, urls: Value) -> Value {
    let hops = dry_run::hops(source_id, "", &Outcome::UnknownSource { source_id });
    json!({
        "http": 404,
        "answer": {
            "status": "error",
            "message": format!("no profile with source_id \"{source_id}\""),
            "remedy": "check the loaded profiles at /v1/debug/status",
        },
        "hops": hops, "verdict": null, "existing": null, "lookup_failed": false,
        "calendar_name": null, "urls": urls,
    })
}

/// The same two refusals ingest gives, in ingest's order: the id first,
/// then the mapping. `None` when ingest would accept the call.
fn refusal(
    request: &RunRequest,
    profile: &Profile,
    origin_label: &str,
) -> Result<Option<(StatusCode, Value)>, Error> {
    let mut request_headers = HeaderMap::new();
    if let Some(key) = request.idempotency_key.as_deref() {
        // Before, a key that is not a valid header value was dropped
        // without a word, and the page then said the call lacked an id.
        let value = HeaderValue::from_str(key).map_err(|_| {
            Error::invalid(
                "the Idempotency-Key is not a valid header value",
                "use visible ASCII characters only, as a real header would carry",
            )
        })?;
        request_headers.insert(HeaderName::from_static("idempotency-key"), value);
    }
    Ok(match requires_an_id(&request.event, &request_headers) {
        Err((status, Json(body))) => Some((status, body)),
        Ok(()) => match map_payload(&request.event, profile, origin_label) {
            Ok(_) => None,
            Err(e) => Some((
                StatusCode::UNPROCESSABLE_ENTITY,
                json!({"status": "error", "message": e.to_string(), "remedy": e.remedy()}),
            )),
        },
    })
}

/// The refused step for the path. Its note is the message without the
/// profile's name in front and without serde's long list of allowed
/// fields.
fn refused_outcome(answer: &Value, origin_label: &str) -> Outcome<'static> {
    let message = answer["message"].as_str().unwrap_or_default();
    let note = message
        .split(" — ")
        .next()
        .unwrap_or(message)
        .trim_start_matches(&format!("{origin_label}: "))
        .split(", expected one of")
        .next()
        .unwrap_or_default()
        .to_string();
    Outcome::Refused {
        rule: dry_run::rule_of(message).to_string(),
        note,
    }
}

/// An accepted call: the event Almanac would write, and what is in the
/// calendar under the same key now.
struct Accepted {
    event: GoogleEvent,
    existing: Option<GoogleEvent>,
    lookup_failed: bool,
}

async fn accepted(
    state: &AppState,
    request: &RunRequest,
    profile: &Profile,
    origin_label: &str,
) -> Result<Accepted, Error> {
    // Map again for the event itself (cheap and pure).
    let mut event: GoogleEvent = map_payload(&request.event, profile, origin_label)
        .map_err(|e| Error::internal(e.to_string(), e.remedy().to_string()))?;
    let entry = Entry {
        id: String::new(),
        source_id: profile.source_id.clone(),
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
    let (existing, lookup_failed) = match &key {
        None => (None, false),
        Some(key) => lookup_existing(state, profile, key).await,
    };
    Ok(Accepted {
        event,
        existing,
        lookup_failed,
    })
}

/// Read only: the same lookup delivery does before it creates or updates.
async fn lookup_existing(
    state: &AppState,
    profile: &Profile,
    key: &str,
) -> (Option<GoogleEvent>, bool) {
    match state
        .client
        .find_event_by_property(&profile.target_calendar_id, UPSERT_PROPERTY, key)
        .await
    {
        Ok(found) => (found, false),
        Err(e) => {
            tracing::warn!(source_id = %profile.source_id, error = %e, "try: could not look up the existing event");
            (None, true)
        }
    }
}

impl Accepted {
    fn into_json(
        self,
        source_id: &str,
        profile: &Profile,
        calendar_name: &str,
        urls: Value,
    ) -> Value {
        let verdict =
            (!self.lookup_failed).then(|| dry_run::verdict(self.existing.as_ref(), &self.event));
        let outcome = Outcome::Mapped {
            summary: &self.event.summary,
            all_day: self.event.start.is_all_day(),
            verdict,
        };
        let hops = dry_run::hops(source_id, calendar_name, &outcome);
        let existing = self.existing.map(|mut found| {
            found.id = None;
            found
        });
        json!({
            "http": 200,
            "answer": {
                "status": "ok",
                "would_write_to_calendar": profile.target_calendar_id,
                "event": self.event,
            },
            "hops": hops,
            "verdict": verdict,
            "existing": existing,
            "lookup_failed": self.lookup_failed,
            "calendar_name": calendar_name,
            "urls": urls,
        })
    }
}
