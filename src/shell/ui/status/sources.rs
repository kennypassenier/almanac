//! The `sources` part of `GET /status/data` and its reload twin: the Sources section.

use std::collections::HashMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use chassis::PartResult;
use serde_json::{Value, json};

use crate::core::profile::Profile;
use crate::shell::profiles::Loaded;
use crate::shell::state::AppState;

/// One row for a loaded profile, set against its file on disk.
fn loaded_row(state: &AppState, p: &Profile, on_disk: Option<&Profile>) -> Value {
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
    json!({
        "id": p.source_id,
        "detail": format!("{} · {read}", state.calendar_name(&p.target_calendar_id)),
        "state": status,
        "fix": fix,
    })
}

/// One row for a file on disk that is not served yet.
fn new_file_row(state: &AppState, d: &Profile) -> Value {
    json!({
        "id": d.source_id,
        "detail": format!("{} · not served yet", state.calendar_name(&d.target_calendar_id)),
        "state": "warn",
        "fix": "Reload the profiles to serve it.",
    })
}

/// One row for a file that cannot be used at all.
fn unusable_row(file_name: &str) -> Value {
    json!({
        "id": file_name,
        "detail": "This file cannot be used",
        "state": "failed",
        "fix": "Repair or remove the file on the Calendars page.",
    })
}

/// All rows: loaded profiles, new files, unusable files, sorted by id.
fn source_rows(state: &AppState, loaded: &HashMap<String, Profile>, disk: &Loaded) -> Vec<Value> {
    let mut rows = Vec::new();
    for p in loaded.values() {
        let on_disk = disk.profiles.iter().find(|d| d.source_id == p.source_id);
        rows.push(loaded_row(state, p, on_disk));
    }
    for d in &disk.profiles {
        if !loaded.contains_key(&d.source_id) {
            rows.push(new_file_row(state, d));
        }
    }
    for u in &disk.unusable {
        rows.push(unusable_row(&u.file_name()));
    }
    rows.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    rows
}

/// The `sources` part of `GET /status/data`: every loaded profile with the calendar it writes
/// to, set against the files on disk so a changed, new or broken file shows
/// as a row that says what to do.
pub(super) async fn sources(state: Arc<AppState>) -> PartResult {
    let read = Arc::clone(&state);
    let disk =
        tokio::task::spawn_blocking(move || crate::shell::profiles::load_all(&read.profiles_dir))
            .await;
    let disk = match disk {
        Ok(disk) => disk,
        Err(e) => {
            tracing::warn!(error = %e, "the profile files could not be listed for the status page");
            return Err(chassis::Error::internal(
                "the profile files could not be listed",
                "try again in a moment",
            ));
        }
    };
    let rows = source_rows(&state, &state.profiles(), &disk);
    Ok(json!({
        "schemaVersion": crate::core::profile::SUPPORTED_SCHEMA_VERSION,
        "sources": rows,
    }))
}

/// `POST /status/sources/reload`: re-reads the profile files, like
/// `POST /sources/reload`, and answers with the real count.
pub(super) async fn reload(State(state): State<Arc<AppState>>) -> Response {
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
    let (profiles, unusable) = match loaded {
        Ok(loaded) => loaded,
        Err(e) => {
            tracing::warn!(error = %e, "the profile files could not be read for the status page");
            return chassis::Error::internal(
                "the profile files could not be read",
                "try again in a moment",
            )
            .into_response();
        }
    };
    let count = profiles.len();
    tracing::info!(count, "reloaded profiles from the status page");
    state.set_profiles(profiles);
    Json(json!({ "message": reload_message(count, unusable) })).into_response()
}

/// The flash text after a reload: how many profiles, and how many files
/// could not be used.
fn reload_message(count: usize, unusable: usize) -> String {
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    let mut message = format!("{count} profile{} reloaded from disk.", plural(count));
    if unusable > 0 {
        message.push_str(&format!(
            " {unusable} file{} could not be used.",
            plural(unusable)
        ));
    }
    message
}
