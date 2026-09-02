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
use axum::response::IntoResponse;
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

/// Environment variable holding a capture-only token (S2).
///
/// The capture endpoint is the one debug surface a *foreign* system is
/// meant to call: M11 exists to learn what an undocumented webhook
/// sends, which means configuring that webhook to post here. Guarding
/// it with the bootstrap token meant the only way to make that work
/// was to paste the credential that also logs into the dashboard and
/// reveals every source's plaintext token into a third-party config
/// store — defeating the encrypted token store end to end.
///
/// This token authorizes exactly one thing: posting a capture. It
/// cannot log in, cannot read captures back, cannot issue or revoke,
/// and can be rotated without touching anything else.
pub const CAPTURE_TOKEN_ENV: &str = "ALMANAC_CAPTURE_TOKEN";

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

/// Checks the request against the capture-only token, falling back to
/// the admin token.
///
/// Both are accepted deliberately: the operator's own token should not
/// stop working, and the point of the capture token is that a foreign
/// system can be given *only* that one. What must never happen is the
/// reverse — a capture token opening anything else — and it cannot,
/// because nothing else consults it.
fn authorize_capture(state: &AppState, headers: &HeaderMap) -> Result<(), Reply> {
    let presented = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_bearer);

    if let (Some(token), Some(expected)) = (presented, &state.capture_token_hash)
        && verify_token(token, expected)
    {
        return Ok(());
    }

    authorize_admin(state, headers).map_err(|_| {
        error(
            StatusCode::UNAUTHORIZED,
            "invalid or missing capture token",
            "send ALMANAC_CAPTURE_TOKEN (or the bootstrap token) as `Authorization: Bearer \
             <token>`; the capture token is the one to give a system you are still investigating, \
             because it cannot do anything else",
        )
    })
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

/// `GET /metrics` (M13) — the Prometheus scrape target.
///
/// Unauthenticated, like `/healthz` and for the same reason M12 gives:
/// monitoring must not fail closed. A scraper that cannot authenticate
/// reports the service as down, which is a lie that costs an evening.
/// The output is numbers only — see the tests in `core::metrics` — so
/// there is nothing here a token would protect.
///
/// The journal depth is read per scrape rather than tracked, because a
/// counter of "how many are pending" drifts from the file the moment a
/// replay, a compaction or a restart touches it, and the number is only
/// worth having if it is true.
async fn metrics(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let pending = match state.journal.pending() {
        Ok(entries) => Some(entries.len() as u64),
        Err(e) => {
            tracing::warn!(
                error = %e,
                "could not read the journal for a metrics scrape; reporting it as unreadable                  rather than as empty"
            );
            None
        }
    };

    (
        StatusCode::OK,
        // The exposition format's own content type. Prometheus is
        // forgiving about it, but Grafana Agent and the textfile
        // collectors are not.
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        state.metrics.render(pending, env!("CARGO_PKG_VERSION")),
    )
}

/// `GET /v1/debug/status` (K11) — what is loaded, what is waiting, and
/// how the recent events were routed.
async fn debug_status(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Reply {
    if let Err(reply) = authorize_admin(&state, &headers) {
        return reply;
    }

    let loaded = state.profiles();
    let mut profiles: Vec<_> = loaded
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
/// Guarded by `ALMANAC_CAPTURE_TOKEN` (S2), which authorizes this and
/// nothing else, so a system you are still investigating can be given
/// a credential that cannot log in or reveal anything. The bootstrap
/// token also works, for the operator's own use.
async fn capture_post(
    State(state): State<Arc<AppState>>,
    Path(label): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Reply {
    if let Err(reply) = authorize_capture(&state, &headers) {
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

    let profiles = state.profiles();
    let Some(profile) = profiles.get(&source_id) else {
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
        .route("/metrics", axum::routing::get(metrics))
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
