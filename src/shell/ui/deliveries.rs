//! The Deliveries page (/deliveries) and its JSON route.
//!
//! One route, `/deliveries/data`, answers all three sections from the live
//! state: the journal on disk, the worker's watch (backoff step, next pass,
//! strikes, failed tries) and the metrics counters. Times leave as epoch
//! milliseconds; the page words them.

use std::sync::Arc;

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Json, Router};
use chassis::Dashboard;
use serde_json::{Value, json};

use crate::core::journal::Entry;
use crate::core::pacing;
use crate::shell::ingest::AppState;
use crate::shell::worker::{self, EntryWatch, FailureKind, WatchSnapshot};

const PAGE_HTML: &str = include_str!("../../../templates/deliveries.html");

/// How many coming passes the schedule shows.
const COMING_PASSES: usize = 4;
/// Permanent refusals after which an entry is set aside (mirrors the worker).
const STRIKES_LIMIT: usize = 3;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/deliveries", get(page))
        .route("/deliveries/data", get(data))
}

async fn page(Extension(dash): Extension<Dashboard>) -> Response {
    match dash.render_project("/deliveries", PAGE_HTML, json!({})) {
        Ok(html) => html.into_response(),
        Err(e) => e.into_response(),
    }
}

fn millis(seconds: u64) -> u64 {
    seconds * 1000
}

fn received_ms(entry: &Entry) -> u64 {
    chrono::DateTime::parse_from_rfc3339(&entry.received_at)
        .map(|t| t.timestamp_millis().max(0) as u64)
        .unwrap_or(0)
}

fn kind_word(kind: FailureKind) -> &'static str {
    match kind {
        FailureKind::Token => "token",
        FailureKind::Google => "google",
        FailureKind::Local => "local",
    }
}

async fn data(State(state): State<Arc<AppState>>) -> Result<Json<Value>, chassis::Error> {
    let pending = state
        .journal
        .pending()
        .map_err(|e| chassis::Error::internal(e.to_string(), e.remedy().to_string()))?;
    let dead = state
        .journal
        .dead()
        .map_err(|e| chassis::Error::internal(e.to_string(), e.remedy().to_string()))?;
    let watch = worker::watch().snapshot();
    let profiles = state.profiles();
    let now = (state.now_unix)();
    let counts = state.metrics.snapshot();

    let waiting: Vec<(&Entry, String)> = pending
        .iter()
        .map(|e| (e, worker::entry_title(e, profiles.get(&e.source_id))))
        .collect();

    Ok(Json(json!({
        "now": millis(now),
        "counts": {
            "accepted": counts.accepted,
            "delivered": counts.delivered,
            "failed": counts.failed,
            "set_aside": counts.dead,
            "pending": pending.len(),
        },
        "chain": chain(&waiting, &watch),
        "schedule": schedule(&waiting, &watch, now),
        "aside": aside(&dead, &state),
    })))
}

/// The stage that is failing, from the latest pass, and why.
fn lit_stage(watch: &WatchSnapshot) -> Value {
    let Some(kind) = watch.last_failure else {
        return Value::Null;
    };
    let refused = watch.entries.values().any(|e| e.strikes > 0);
    let (stage, reason) = match kind {
        FailureKind::Token => ("worker", "token"),
        FailureKind::Google if refused => ("google", "refused"),
        FailureKind::Google => ("google", "temporary"),
        FailureKind::Local => ("worker", "local"),
    };
    json!({"stage": stage, "reason": reason})
}

fn chain(waiting: &[(&Entry, String)], watch: &WatchSnapshot) -> Value {
    let mut cards: Vec<Value> = waiting
        .iter()
        .map(|(entry, title)| {
            let tries = watch.entries.get(&entry.id);
            let lane = if watch.in_flight.as_deref() == Some(entry.id.as_str()) {
                "worker"
            } else {
                "journal"
            };
            json!({
                "id": entry.id,
                "lane": lane,
                "title": title,
                "source": entry.source_id,
                "at": received_ms(entry),
                "tries": tries.map_or(0, |t| t.failed_at.len()),
                "kind": tries.map(|t| kind_word(t.last_kind)),
                "strikes": tries.map_or(0, |t| t.strikes),
            })
        })
        .collect();
    cards.extend(watch.delivered.iter().map(|d| {
        json!({
            "id": format!("done-{}", d.entry_id),
            "lane": "google",
            "title": d.title,
            "source": d.source_id,
            "at": millis(d.at),
            "created": d.created,
        })
    }));
    json!({"lit": lit_stage(watch), "cards": cards})
}

fn schedule(waiting: &[(&Entry, String)], watch: &WatchSnapshot, now: u64) -> Value {
    if waiting.is_empty() {
        return json!({"next": [], "entries": [], "outage_since": Value::Null});
    }
    let next_at = watch
        .next_pass
        .filter(|at| *at >= now)
        .unwrap_or(now + pacing::POLL_INTERVAL_SECS);
    let passes = pacing::projected_passes(next_at, watch.consecutive_failures, COMING_PASSES);

    let strikes_of = |e: &EntryWatch| e.strikes.min(STRIKES_LIMIT);
    let entries: Vec<Value> = waiting
        .iter()
        .map(|(entry, title)| {
            let tries = watch.entries.get(&entry.id);
            let strikes = tries.map_or(0, strikes_of);
            let limited = strikes > 0;
            json!({
                "id": entry.id,
                "title": title,
                "source": entry.source_id,
                "received": received_ms(entry),
                "tries": tries.map_or(Vec::new(), |t| t.failed_at.iter().map(|s| millis(*s)).collect()),
                "strikes": strikes,
                "limited": limited,
                "left": if limited { json!(STRIKES_LIMIT - strikes) } else { Value::Null },
            })
        })
        .collect();

    json!({
        "next": passes.iter().map(|s| millis(*s)).collect::<Vec<_>>(),
        "entries": entries,
        "outage_since": watch.outage_since.map(millis),
    })
}

fn aside(dead: &[(Entry, String, String)], state: &AppState) -> Vec<Value> {
    let profiles = state.profiles();
    let mut items: Vec<(u64, Value)> = dead
        .iter()
        .map(|(entry, reason, at)| {
            let at_ms = chrono::DateTime::parse_from_rfc3339(at)
                .map(|t| t.timestamp_millis().max(0) as u64)
                .unwrap_or(0);
            let (cause, remedy) = pacing::plain_set_aside_reason(reason);
            (
                at_ms,
                json!({
                    "id": entry.id,
                    "source": entry.source_id,
                    "title": worker::entry_title(entry, profiles.get(&entry.source_id)),
                    "at": at_ms,
                    "cause": cause,
                    "remedy": remedy,
                }),
            )
        })
        .collect();
    items.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
    items.into_iter().map(|(_, v)| v).collect()
}
