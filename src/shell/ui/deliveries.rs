//! The Deliveries page (/deliveries) and its JSON route.
//!
//! One route, `/deliveries/data`, answers all three sections from the live
//! state: the journal on disk, the worker's watch (backoff step, next pass,
//! strikes, failed tries) and the metrics counters. Times leave as epoch
//! milliseconds; the page words them.

use std::sync::Arc;

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use chassis::Dashboard;
use serde_json::{Value, json};

use crate::core::journal::Entry;
use crate::core::pacing;
use crate::shell::state::AppState;
use crate::shell::worker::{self, EntryWatch, FailureKind, WatchSnapshot};

mod actions;
mod aside;

const PAGE_HTML: &str = include_str!("../../../templates/deliveries.html");

/// How many coming passes the schedule shows.
const COMING_PASSES: usize = 4;
/// Permanent refusals after which the worker sets an entry aside.
const STRIKES_LIMIT: usize = crate::shell::worker::PERMANENT_FAILURES_BEFORE_DEAD;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/deliveries", get(page))
        .route("/deliveries/data", get(data))
        .route("/deliveries/retry", post(actions::retry))
        .route("/deliveries/dismiss", post(actions::dismiss))
}

async fn page(Extension(dash): Extension<Dashboard>) -> Response {
    match dash.render_project(
        "/deliveries",
        PAGE_HTML,
        json!({ "pwa_head": crate::shell::web::pwa_head() }),
    ) {
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
    let watch = state.watch.snapshot();
    let profiles = state.profiles();
    let now = (state.now_unix)();
    let counts = state.metrics.snapshot();

    let set_aside = aside::items(&dead, &profiles, now as i64);
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
        "chain": chain(&waiting, &watch, &set_aside),
        "schedule": schedule(&waiting, &watch, now),
        "aside": set_aside,
        "marks": marks(&profiles),
    })))
}

/// Each source's mark from its profile, where the profile sets one; the
/// page derives the rest from the id.
fn marks(profiles: &std::collections::HashMap<String, crate::core::profile::Profile>) -> Value {
    profiles
        .iter()
        .filter_map(|(id, p)| p.mark.as_ref().map(|m| (id.clone(), json!(m))))
        .collect::<serde_json::Map<_, _>>()
        .into()
}

/// The stage that is failing, from the latest pass, and why.
///
/// Nothing is lit once nothing waits: the last failure is then history, and the
/// entries it set aside show in their own lane, so a lit lane never claims a
/// failure that has no card to show for it.
fn lit_stage(watch: &WatchSnapshot, waiting: usize) -> Value {
    let Some(kind) = watch.last_failure.filter(|_| waiting > 0) else {
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

fn chain(waiting: &[(&Entry, String)], watch: &WatchSnapshot, set_aside: &[Value]) -> Value {
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
    cards.extend(set_aside.iter().map(|item| {
        json!({
            "id": item["id"],
            "lane": "dead",
            "title": item["title"],
            "source": item["source"],
            "at": item["at"],
            "short": item["short"],
        })
    }));
    json!({"lit": lit_stage(watch, waiting.len()), "cards": cards})
}

fn schedule(waiting: &[(&Entry, String)], watch: &WatchSnapshot, now: u64) -> Value {
    if waiting.is_empty() {
        return json!({"next": [], "entries": [], "outage_since": Value::Null, "strikes_limit": STRIKES_LIMIT});
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
        "strikes_limit": STRIKES_LIMIT,
    })
}
