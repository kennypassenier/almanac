//! JSON routes for the calendars page's kp-themes components.
//!
//! The page itself renders at once and asks these routes for its data, so a
//! slow Google never holds the page back: `GET /calendars/data/google` lists
//! the calendars (the only call that waits on Google), `GET
//! /calendars/data/profiles` reads the profile files and the delivery ring
//! (fast, local). Making and deleting calendars and deleting an unusable
//! profile file are JSON calls too; the old form posts in `dashboard.rs` run
//! the same functions, so a bookmark behaves like the page.
//!
//! User-facing sentences here are plain: no paths, no ids, no raw upstream
//! errors (those go to the log).

use std::sync::Arc;

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use chassis::Error;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::shell::state::AppState;

mod actions;
mod profiles_data;

pub(crate) use actions::{make_calendar, reload_summary, remove_calendar, remove_unusable_file};
#[cfg(test)]
use profiles_data::{hourly_buckets, split_reason};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/calendars/data/google", get(google_data))
        .route(
            "/calendars/data/profiles",
            get(profiles_data::profiles_data),
        )
        .route("/calendars/make", post(make))
        .route("/calendars/remove", post(remove))
        .route("/profiles/remove", post(remove_file))
}

/// Lists the calendars the service account sees, the way the page always
/// did: through the tombstones and the just-created memory, because
/// Google's list lags a create or a delete by seconds.
pub(crate) async fn list_calendars(
    state: &AppState,
) -> Result<Vec<(String, String)>, crate::core::error::AlmanacError> {
    let listed = state.client.list_calendars().await?;
    let calendars = state.without_deleted_calendars(state.with_created_calendars(listed));
    state.remember_calendars(&calendars);
    Ok(calendars)
}

/// `GET /calendars/data/google`. A failed list is a 200 with `ok: false` and
/// the words to show, so the card can offer Retry without a transport error.
/// How long the page waits for Google before it says Google does not
/// answer. The client's own retry budget is a minute, and the kit ends a
/// request after 30 s with a bare 408, so without this the page sat on
/// "Reading…" for half a minute in an outage (UX review 2026-11-01).
const PAGE_GOOGLE_WAIT: std::time::Duration = std::time::Duration::from_secs(8);

async fn google_data(State(state): State<Arc<AppState>>) -> Json<Value> {
    let listed = match tokio::time::timeout(PAGE_GOOGLE_WAIT, list_calendars(&state)).await {
        Ok(listed) => listed,
        Err(_) => Err(crate::core::error::AlmanacError::GoogleApi {
            message: format!(
                "Google did not answer within {} s",
                PAGE_GOOGLE_WAIT.as_secs()
            ),
            remedy: "check again in a moment".to_string(),
            transient: true,
        }),
    };
    match listed {
        Ok(calendars) => Json(json!({
            "ok": true,
            "can_create": state.calendar_owner.is_some(),
            "calendars": calendars
                .iter()
                .map(|(id, name)| json!({ "id": id, "name": name }))
                .collect::<Vec<_>>(),
        })),
        Err(e) => {
            tracing::warn!(error = %e, "could not list the calendars for the page");
            Json(json!({
                "ok": false,
                "can_create": state.calendar_owner.is_some(),
                "calendars": [],
                "error": "Google did not answer, so the calendars could not be listed.",
                "remedy": "Almanac keeps the events and delivers them when Google answers again.",
            }))
        }
    }
}

#[derive(Deserialize)]
struct MakeBody {
    name: String,
}

#[derive(Deserialize)]
struct RemoveBody {
    id: String,
}

#[derive(Deserialize)]
struct RemoveFileBody {
    file: String,
}

/// `POST /calendars/make` `{ "name": … }`.
async fn make(
    State(state): State<Arc<AppState>>,
    Json(body): Json<MakeBody>,
) -> Result<Json<Value>, Error> {
    let (id, name, created) = make_calendar(&state, &body.name).await?;
    Ok(Json(json!({ "id": id, "name": name, "created": created })))
}

/// `POST /calendars/remove` `{ "id": … }`.
async fn remove(
    State(state): State<Arc<AppState>>,
    Json(body): Json<RemoveBody>,
) -> Result<Json<Value>, Error> {
    let name = remove_calendar(&state, &body.id).await?;
    Ok(Json(json!({ "name": name })))
}

/// `POST /profiles/remove` `{ "file": … }`.
async fn remove_file(
    State(state): State<Arc<AppState>>,
    Json(body): Json<RemoveFileBody>,
) -> Result<Json<Value>, Error> {
    remove_unusable_file(&state, &body.file)?;
    Ok(Json(json!({ "file": body.file })))
}

#[cfg(test)]
mod tests;
