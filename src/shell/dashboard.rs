//! Almanac's own dashboard pages on the kit (M12 as amended 2026-09-06,
//! step 2 of the chassis migration): **Calendars** — the calendars, the
//! sources that write to them and the profile files, as a page that renders
//! at once and fills itself from the JSON routes in `ui/calendars.rs`. The kit brings the layout, the login and session,
//! the clients page (labelled "Sources" here: the per-source tokens),
//! CSRF and CSP; this module fills `content` with minijinja templates.
//!
//! The UI is English per standing rule 1 (Dutch is for conversation and
//! for dashboards meant for Kenny's parents; this is an operator tool).

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Extension, Form, Router};
use chassis::Dashboard;
use serde::Deserialize;
use serde_json::json;

use crate::shell::ingest::AppState;

const CALENDARS_HTML: &str = include_str!("../../templates/calendars.html");

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        // 4.0.2: sources live on the kit's Sources page (profile + token in
        // one issue, S1); Almanac's own page keeps the calendars and the
        // profile files on disk.
        .route("/calendars", get(calendars_page).post(create_calendar))
        .route("/calendars/{calendar_id}/delete", post(delete_calendar))
        .route("/sources/reload", post(reload_profiles))
        .route("/profiles/{file_name}/delete", post(delete_unusable))
        // Older addresses keep working: bookmarks and runbooks point here.
        .route("/sources", get(|| async { Redirect::to("/calendars") }))
        .route("/dashboard", get(|| async { Redirect::to("/") }))
        .route(
            "/dashboard/sources",
            get(|| async { Redirect::to("/calendars") }),
        )
        .route(
            "/dashboard/captures",
            get(|| async { Redirect::to("/clients") }),
        )
        .route("/captures", get(|| async { Redirect::to("/clients") }))
}

async fn calendars_page(
    Extension(dash): Extension<Dashboard>,
    State(_state): State<Arc<AppState>>,
) -> Response {
    render_calendars(&dash, None)
}

/// Renders the Calendars page. It never waits on Google: the page script
/// fetches the lists. `notice` carries the refusal of an old form post
/// (a bookmark or a runbook still posting to the page), which the script
/// raises as a toast.
fn render_calendars(dash: &Dashboard, notice: Option<&str>) -> Response {
    match dash.render_project("/calendars", CALENDARS_HTML, json!({ "notice": notice })) {
        Ok(html) => html.into_response(),
        Err(e) => e.into_response(),
    }
}

/// The sentence an old form post shows when the shared function refuses.
fn notice_of(e: &chassis::Error) -> String {
    format!("{}. {}", e.message.trim_end_matches('.'), e.remedy)
}

/// What the make-a-calendar form sends (K24).
#[derive(Deserialize)]
struct NewCalendar {
    name: String,
}

/// `POST /calendars` — make one and share it (K24). The page itself calls
/// `POST /calendars/make` as JSON; this form post stays for bookmarks.
async fn create_calendar(
    Extension(dash): Extension<Dashboard>,
    State(state): State<Arc<AppState>>,
    Form(form): Form<NewCalendar>,
) -> Response {
    match crate::shell::ui::calendars::make_calendar(&state, &form.name).await {
        Ok(_) => Redirect::to("/calendars").into_response(),
        Err(e) => render_calendars(&dash, Some(&notice_of(&e))),
    }
}

/// `POST /calendars/{calendar_id}/delete` — remove a calendar and
/// everything on it (K24). Guarded on arrival as well as in the page: a
/// source can be added between the render and the click.
async fn delete_calendar(
    Extension(dash): Extension<Dashboard>,
    State(state): State<Arc<AppState>>,
    Path(calendar_id): Path<String>,
) -> Response {
    match crate::shell::ui::calendars::remove_calendar(&state, &calendar_id).await {
        Ok(_) => Redirect::to("/calendars").into_response(),
        Err(e) => render_calendars(&dash, Some(&notice_of(&e))),
    }
}

/// `POST /profiles/{file_name}/delete` — remove a file the service cannot
/// use (K23). Addressed by file name: a broken profile often has no
/// readable source id, which is frequently the thing wrong with it.
async fn delete_unusable(
    Extension(dash): Extension<Dashboard>,
    State(state): State<Arc<AppState>>,
    Path(file_name): Path<String>,
) -> Response {
    match crate::shell::ui::calendars::remove_unusable_file(&state, &file_name) {
        Ok(()) => Redirect::to("/calendars").into_response(),
        Err(e) if e.kind == chassis::Kind::NotFound => {
            (StatusCode::NOT_FOUND, "no such unusable profile").into_response()
        }
        Err(e) => render_calendars(&dash, Some(&notice_of(&e))),
    }
}

/// `POST /sources/reload` — re-read the profiles directory (K21), which
/// is what makes a profile placed by hand usable without a restart. The
/// page asks for JSON and stays where it is; a plain form post still
/// redirects to the page.
async fn reload_profiles(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let summary = crate::shell::ui::calendars::reload_summary(&state);
    let wants_json = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("application/json"));
    if wants_json {
        axum::Json(summary).into_response()
    } else {
        Redirect::to("/calendars").into_response()
    }
}
