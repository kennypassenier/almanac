//! JSON routes for the status page's kp-themes components (4.3.0): the
//! Journal section's glance and recent deliveries, and the Sources
//! section's list and reload. The sections themselves are
//! `kit::JournalSection` and `kit::ProfilesSection`; `web/js/status.js`
//! fills them from these answers and polls them for the live refresh.

use std::collections::HashSet;
use std::sync::Arc;

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;

use crate::core::status_view;
use crate::shell::ingest::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/status/journal", get(journal))
        .route("/status/sources", get(sources))
        // The JSON twin of `POST /sources/reload`, which answers with a
        // redirect to the Calendars page; a flash button needs a message.
        .route("/status/sources/reload", post(reload))
}

/// Whether the process could append to the journal, without writing to it.
fn store_state(state: &AppState) -> (&'static str, &'static str) {
    let path = state.journal.path();
    let writable = match std::fs::OpenOptions::new().append(true).open(path) {
        Ok(_) => true,
        // No journal file yet: the directory it will be made in decides.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => path
            .parent()
            .and_then(|dir| std::fs::metadata(dir).ok())
            .is_some_and(|m| !m.permissions().readonly()),
        Err(_) => false,
    };
    if writable {
        ("ok", "Writable")
    } else {
        (
            "down",
            "Cannot be written; check the data folder's permissions",
        )
    }
}

/// `GET /status/journal`: the figures, the two health marks, the outcome
/// bar, the per-hour trend and the deliveries, all from what Almanac holds
/// in memory (counters, the route ring) and the journal file.
async fn journal(State(state): State<Arc<AppState>>) -> Response {
    let routes: Vec<_> = state.routes.lock().await.iter().cloned().collect();
    let read = Arc::clone(&state);
    let read = tokio::task::spawn_blocking(move || {
        let pending = read.journal.pending();
        let dead = read.journal.dead();
        let store = store_state(&read);
        (pending, dead, store)
    })
    .await;
    let Ok((pending, dead, store)) = read else {
        return chassis::Error::internal(
            "the journal read did not finish",
            "try again in a moment",
        )
        .into_response();
    };
    let pending_ids: Option<HashSet<String>> = pending
        .as_ref()
        .ok()
        .map(|p| p.iter().map(|e| e.id.clone()).collect());
    let dead_ids: HashSet<String> = dead
        .map(|d| d.into_iter().map(|(e, _, _)| e.id).collect())
        .unwrap_or_default();

    let rows = status_view::fold_deliveries(&routes, pending_ids.as_ref(), &dead_ids);
    let outcomes = status_view::outcomes(&rows);
    let counts = state.metrics.snapshot();
    let trend = status_view::hourly_trend(&routes, chrono::Utc::now());
    // The ring keeps the latest HISTORY_CAPACITY attempts; once deliveries
    // outnumber what it still holds, the bar says it shows the latest ones.
    let partial = (outcomes.created + outcomes.updated) as u64 != counts.delivered;

    let waiting = pending.as_ref().ok().map(Vec::len);
    let journal = match &pending {
        Ok(p) if p.is_empty() => json!({ "state": "ok", "text": "Readable, nothing pending" }),
        Ok(p) => {
            json!({ "state": "warn", "text": format!("Readable, {} waiting to be delivered", p.len()) })
        }
        Err(_) => json!({ "state": "down", "text": "Cannot be read; deliveries are on hold" }),
    };
    let deliveries: Vec<_> = rows
        .iter()
        .map(|r| {
            let calendar = state
                .profiles()
                .get(&r.source_id)
                .map(|p| state.calendar_name(&p.target_calendar_id));
            json!({
                "id": r.id,
                "at": r.at,
                "source": r.source_id,
                "result": r.result.as_str(),
                "calendar": calendar,
                "error": r.error,
            })
        })
        .collect();

    Json(json!({
        "at": chrono::Utc::now().to_rfc3339(),
        "waiting": waiting,
        "accepted": counts.accepted,
        "delivered": counts.delivered,
        "gaveUp": counts.dead,
        "journal": journal,
        "store": { "state": store.0, "text": store.1 },
        "outcomes": {
            "created": outcomes.created,
            "updated": outcomes.updated,
            "failed": outcomes.failed,
            "partial": partial,
        },
        "trend": {
            "start": trend.start_ms,
            "step": trend.step_ms,
            "values": trend.values,
            "lastHour": trend.last_hour,
        },
        "deliveries": deliveries,
    }))
    .into_response()
}

/// `GET /status/sources`: every loaded profile with the calendar it writes
/// to, set against the files on disk so a changed, new or broken file shows
/// as a row that says what to do.
async fn sources(State(state): State<Arc<AppState>>) -> Response {
    let read = Arc::clone(&state);
    let disk =
        tokio::task::spawn_blocking(move || crate::shell::profiles::load_all(&read.profiles_dir))
            .await;
    let Ok(disk) = disk else {
        return chassis::Error::internal(
            "the profile files could not be listed",
            "try again in a moment",
        )
        .into_response();
    };
    let loaded = state.profiles();
    let calendar = |id: &str| state.calendar_name(id);
    let mut rows = Vec::new();
    for p in loaded.values() {
        let on_disk = disk.profiles.iter().find(|d| d.source_id == p.source_id);
        let (status, read, fix) = match on_disk {
            Some(d) if d == p => ("ok", "served", None),
            Some(_) => (
                "warn",
                "the file changed on disk",
                Some("Reload the profiles to serve the changed file."),
            ),
            None => (
                "warn",
                "the file is no longer on disk",
                Some("Reload the profiles to stop serving it."),
            ),
        };
        rows.push(json!({
            "id": p.source_id,
            "detail": format!("{} · {read}", calendar(&p.target_calendar_id)),
            "state": status,
            "fix": fix,
        }));
    }
    for d in &disk.profiles {
        if !loaded.contains_key(&d.source_id) {
            rows.push(json!({
                "id": d.source_id,
                "detail": format!("{} · not served yet", calendar(&d.target_calendar_id)),
                "state": "warn",
                "fix": "Reload the profiles to serve it.",
            }));
        }
    }
    for u in &disk.unusable {
        rows.push(json!({
            "id": u.file_name(),
            "detail": "This file cannot be used",
            "state": "failed",
            "fix": "Repair or remove the file on the Calendars page.",
        }));
    }
    rows.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    Json(json!({
        "schemaVersion": crate::core::profile::SUPPORTED_SCHEMA_VERSION,
        "sources": rows,
    }))
    .into_response()
}

/// `POST /status/sources/reload`: re-reads the profile files, like
/// `POST /sources/reload`, and answers with the real count.
async fn reload(State(state): State<Arc<AppState>>) -> Response {
    let read = Arc::clone(&state);
    let loaded = tokio::task::spawn_blocking(move || {
        let unusable = crate::shell::profiles::load_all(&read.profiles_dir)
            .unusable
            .len();
        (
            crate::shell::profiles::load_map(&read.profiles_dir),
            unusable,
        )
    })
    .await;
    let Ok((profiles, unusable)) = loaded else {
        return chassis::Error::internal(
            "the profile files could not be read",
            "try again in a moment",
        )
        .into_response();
    };
    let count = profiles.len();
    tracing::info!(count, "reloaded profiles from the status page");
    state.set_profiles(profiles);
    let mut message = format!(
        "{count} profile{} reloaded from disk.",
        if count == 1 { "" } else { "s" }
    );
    if unusable > 0 {
        message.push_str(&format!(
            " {unusable} file{} could not be used.",
            if unusable == 1 { "" } else { "s" }
        ));
    }
    Json(json!({ "message": message })).into_response()
}
