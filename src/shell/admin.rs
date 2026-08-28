//! The operator-facing surface: health (M1), debug introspection
//! (K11), raw request capture (M11) and the dry-run mapper (M9).
//!
//! Everything except health sits behind the bootstrap token from the
//! environment (AR17 as amended) — the same token that will log into
//! the L4b dashboard. Health stays open on purpose: a monitoring stack
//! that fails closed lies to you during an outage, which is exactly
//! when you believe it. It carries no secret, so there is nothing to
//! protect.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::{Json, Router};
use serde_json::{Value, json};

use crate::core::mapping::map_payload;
use crate::core::observability::{CaptureRecord, truncate_body};
use crate::core::token::{parse_bearer, verify_token};
use crate::shell::ingest::AppState;

/// Environment variable holding the bootstrap token. Absent means the
/// admin surface refuses every request rather than opening up — a
/// forgotten variable must not silently expose the debug views
/// (fail-closed, standing rule 12).
pub const BOOTSTRAP_TOKEN_ENV: &str = "ALMANAC_BOOTSTRAP_TOKEN";

/// Bodies larger than this are stored cut, with the original size
/// reported (M11).
const MAX_CAPTURE_BODY_BYTES: usize = 64 * 1024;

/// How long a captured request stays in memory. Long enough to fire a
/// webhook and go read it; short enough that a forgotten capture label
/// does not hold someone's payload all week.
pub const CAPTURE_TTL_SECS: u64 = 3600;

/// Headers never echoed back by the capture surface. Capturing an
/// unknown webhook means capturing whatever it sends, including its
/// own credentials — those must not be readable afterwards just
/// because someone pointed it here (standing rule 10).
const REDACTED_HEADERS: [&str; 4] = [
    "authorization",
    "cookie",
    "proxy-authorization",
    "x-api-key",
];

type Reply = (StatusCode, Json<Value>);

fn error(status: StatusCode, message: &str, remedy: &str) -> Reply {
    (
        status,
        Json(json!({"status": "error", "message": message, "remedy": remedy})),
    )
}

/// Checks the request against the bootstrap token.
fn authorize_admin(state: &AppState, headers: &HeaderMap) -> Result<(), Reply> {
    let Some(expected) = &state.bootstrap_token_hash else {
        return Err(error(
            StatusCode::SERVICE_UNAVAILABLE,
            "the admin surface is not configured",
            "set ALMANAC_BOOTSTRAP_TOKEN (via `latch run --`) and restart; without it the debug \
             views stay closed rather than open",
        ));
    };

    let presented = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_bearer);

    match presented {
        Some(token) if verify_token(token, expected) => Ok(()),
        _ => Err(error(
            StatusCode::UNAUTHORIZED,
            "invalid or missing admin token",
            "send the bootstrap token as `Authorization: Bearer <token>`",
        )),
    }
}

/// `GET /healthz` (M1) — deliberately unauthenticated and dependency-
/// free. It answers "this process is alive and serving", nothing more:
/// reporting Google's reachability here would make the health check go
/// red during an outage Almanac is designed to ride out via the
/// journal, which would be a lie about Almanac's own state.
async fn healthz() -> Reply {
    (
        StatusCode::OK,
        Json(json!({"status": "ok", "version": env!("CARGO_PKG_VERSION")})),
    )
}

/// `GET /v1/debug/status` (K11) — what is loaded, what is waiting, and
/// how the recent events were routed.
async fn debug_status(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Reply {
    if let Err(reply) = authorize_admin(&state, &headers) {
        return reply;
    }

    let mut profiles: Vec<_> = state
        .profiles
        .values()
        .map(|p| {
            json!({
                "source_id": p.source_id,
                "target_calendar_id": p.target_calendar_id,
                "schema_version": p.schema_version,
            })
        })
        .collect();
    profiles.sort_by_key(|p| p["source_id"].as_str().unwrap_or_default().to_string());

    let pending = match state.journal.pending() {
        Ok(pending) => json!({
            "count": pending.len(),
            "oldest": pending.first().map(|e| json!({
                "entry_id": e.id,
                "source_id": e.source_id,
                "received_at": e.received_at,
            })),
        }),
        Err(e) => json!({"error": e.to_string(), "remedy": e.remedy()}),
    };

    let routes: Vec<_> = state.routes.lock().await.iter().cloned().collect();

    (
        StatusCode::OK,
        Json(json!({
            "status": "ok",
            "profiles": profiles,
            "journal": pending,
            "recent_routes": routes,
        })),
    )
}

/// `POST /v1/debug/capture/{label}` (M11) — accepts anything, stores it
/// verbatim, interprets nothing. Point an undocumented webhook here to
/// learn its real shape before writing a profile for it.
///
/// Guarded by the bootstrap token, which is also the dashboard login
/// and can reveal every source's token — so configuring a foreign
/// system to call this means handing that system the master
/// credential. The captures page now says so. Splitting out a
/// capture-only credential is open for decision.
async fn capture_post(
    State(state): State<Arc<AppState>>,
    Path(label): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Reply {
    if let Err(reply) = authorize_admin(&state, &headers) {
        return reply;
    }

    let (stored_body, truncated_from_bytes) = truncate_body(&body, MAX_CAPTURE_BODY_BYTES);

    let recorded: Vec<(String, String)> = headers
        .iter()
        .map(|(name, value)| {
            let name = name.as_str().to_ascii_lowercase();
            let value = if REDACTED_HEADERS.contains(&name.as_str()) {
                "<redacted>".to_string()
            } else {
                value.to_str().unwrap_or("<non-utf8>").to_string()
            };
            (name, value)
        })
        .collect();

    let record = CaptureRecord {
        at: (state.now)(),
        at_unix: (state.now_unix)(),
        label: label.clone(),
        method: "POST".to_string(),
        headers: recorded,
        body: stored_body,
        truncated_from_bytes,
    };

    let mut captures = state.captures_after_expiry().await;
    captures.push(record);

    tracing::info!(label = %label, "captured a raw request");

    (
        StatusCode::OK,
        Json(json!({"status": "captured", "label": label})),
    )
}

/// `GET /v1/debug/capture` (M11) — read back what was captured.
async fn capture_list(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Reply {
    if let Err(reply) = authorize_admin(&state, &headers) {
        return reply;
    }

    let captures = state.captures_after_expiry().await;
    let records: Vec<_> = captures.iter().cloned().collect();

    (
        StatusCode::OK,
        Json(json!({
            "status": "ok",
            "ttl_seconds": CAPTURE_TTL_SECS,
            "captures": records,
        })),
    )
}

/// `POST /v1/debug/dry-run/{source_id}` (M9) — shows the calendar event
/// a payload would produce, without writing anything to Google. The
/// point is to check a new or changed profile against a real payload
/// before letting it near a calendar.
async fn dry_run(
    State(state): State<Arc<AppState>>,
    Path(source_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> Reply {
    if let Err(reply) = authorize_admin(&state, &headers) {
        return reply;
    }

    let Some(profile) = state.profiles.get(&source_id) else {
        return error(
            StatusCode::NOT_FOUND,
            &format!("no profile with source_id \"{source_id}\""),
            "check the loaded profiles at /v1/debug/status",
        );
    };

    match map_payload(&payload, profile, &format!("profile {source_id}")) {
        Ok(event) => (
            StatusCode::OK,
            Json(json!({
                "status": "ok",
                "would_write_to_calendar": profile.target_calendar_id,
                "event": event,
            })),
        ),
        Err(e) => error(StatusCode::UNPROCESSABLE_ENTITY, &e.to_string(), e.remedy()),
    }
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/healthz", axum::routing::get(healthz))
        .route("/v1/debug/status", axum::routing::get(debug_status))
        .route(
            "/v1/debug/capture/{label}",
            axum::routing::post(capture_post),
        )
        .route("/v1/debug/capture", axum::routing::get(capture_list))
        .route(
            "/v1/debug/dry-run/{source_id}",
            axum::routing::post(dry_run),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacted_header_names_are_lowercase_so_the_comparison_matches() {
        // Headers are lowercased before comparison; a capitalised entry
        // in this list would silently never match and a credential
        // would be stored in the clear.
        for name in REDACTED_HEADERS {
            assert_eq!(name, name.to_ascii_lowercase(), "{name} must be lowercase");
        }
    }
}
