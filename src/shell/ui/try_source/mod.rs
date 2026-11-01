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

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use chassis::Dashboard;
use serde_json::{Value, json};

use crate::core::profile::{FALLBACK_DURATION_MINUTES, Profile};
use crate::shell::state::AppState;

mod run;

use run::run;

const PAGE_HTML: &str = include_str!("../../../../templates/try.html");

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/try", get(page))
        .route("/try/api/sources", get(sources))
        .route("/try/api/run/{source_id}", post(run))
}

async fn page(Extension(dash): Extension<Dashboard>) -> Response {
    match dash.render_project(
        "/try",
        PAGE_HTML,
        json!({ "pwa_head": crate::shell::web::pwa_head() }),
    ) {
        Ok(html) => html.into_response(),
        Err(e) => e.into_response(),
    }
}

/// The text of the profile file that declares `source_id`, with its file
/// name. Found by what the file says, not by what it is called: a profile
/// placed by hand need not be named after its source.
fn profile_file(dir: &std::path::Path, source_id: &str) -> Option<(String, String)> {
    let listing = match std::fs::read_dir(dir) {
        Ok(listing) => listing,
        Err(e) => {
            tracing::warn!(dir = %dir.display(), error = %e, "try: could not list the profiles folder; the page shows no profile file");
            return None;
        }
    };
    let mut paths: Vec<_> = listing
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
